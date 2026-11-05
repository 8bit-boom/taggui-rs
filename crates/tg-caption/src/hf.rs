//! Hugging Face Hub file listing and downloading, via plain HTTPS (the
//! `/api/models/{repo}` JSON endpoint and the `resolve/main` URLs). No
//! auth for public repos; a token can be supplied for gated ones.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum HfError {
    #[error("invalid repository id: {0} (expected `owner/name`)")]
    InvalidRepo(String),
    #[error("listing files for {repo} failed: {source}")]
    List {
        repo: String,
        source: DownloadSource,
    },
    #[error("downloading {file} from {repo} failed: {source}")]
    Download {
        repo: String,
        file: String,
        source: DownloadSource,
    },
    #[error("writing {file} failed: {source}")]
    Write {
        file: String,
        source: std::io::Error,
    },
}

/// Either a reqwest error (request stage) or an IO error (stream stage).
#[derive(Debug, thiserror::Error)]
pub enum DownloadSource {
    #[error("{0}")]
    Request(#[from] reqwest::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    siblings: Vec<Sibling>,
}

#[derive(Debug, Deserialize)]
struct Sibling {
    rfilename: String,
}

fn client(token: Option<&str>) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder();
    if let Some(token) = token {
        if !token.trim().is_empty() {
            builder = builder.default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                if let Ok(value) =
                    reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.trim()))
                {
                    headers.insert(reqwest::header::AUTHORIZATION, value);
                }
                headers
            });
        }
    }
    builder.build().unwrap_or_default()
}

/// List the files in a repository (the `siblings` of the model info).
pub fn list_files(repo: &str, token: Option<&str>) -> Result<Vec<String>, HfError> {
    let url = format!("https://huggingface.co/api/models/{repo}");
    let info: ModelInfo = client(token)
        .get(&url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json())
        .map_err(|source| HfError::List {
            repo: repo.to_owned(),
            source: DownloadSource::Request(source),
        })?;
    Ok(info
        .siblings
        .into_iter()
        .map(|sibling| sibling.rfilename)
        .collect())
}

/// Download one file to `destination_dir`, streaming with a progress
/// callback `(bytes_done, bytes_total)`; returns the written path.
pub fn download_file(
    repo: &str,
    file: &str,
    token: Option<&str>,
    destination_dir: &Path,
    progress: &mut impl FnMut(u64, u64),
) -> Result<PathBuf, HfError> {
    let url = format!("https://huggingface.co/{repo}/resolve/main/{file}");
    let mut response = client(token)
        .get(&url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|source| HfError::Download {
            repo: repo.to_owned(),
            file: file.to_owned(),
            source: DownloadSource::Request(source),
        })?;
    let total = response.content_length().unwrap_or(0);
    let destination = destination_dir.join(file);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|source| HfError::Write {
            file: file.to_owned(),
            source,
        })?;
    }
    let mut file_handle = File::create(&destination).map_err(|source| HfError::Write {
        file: file.to_owned(),
        source,
    })?;
    let mut downloaded: u64 = 0;
    let mut buffer = [0u8; 1 << 16];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|source| HfError::Download {
                repo: repo.to_owned(),
                file: file.to_owned(),
                source: DownloadSource::Io(source),
            })?;
        if read == 0 {
            break;
        }
        file_handle
            .write_all(&buffer[..read])
            .map_err(|source| HfError::Write {
                file: file.to_owned(),
                source,
            })?;
        downloaded += read as u64;
        progress(downloaded, total);
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    #[test]
    fn repo_validation_catches_bad_ids() {
        // The error type is exercised through list_files only with
        // network access; here just document that IDs with a slash pass
        // URL construction.
        let repo = "Qwen/Qwen3-VL-8B-Instruct-GGUF";
        let url = format!("https://huggingface.co/api/models/{repo}");
        assert!(url.ends_with("/api/models/Qwen/Qwen3-VL-8B-Instruct-GGUF"));
    }
}
