//! OpenAI-compatible endpoint client. Port of the Python fork's
//! `auto_captioning/models/openai_compatible.py`: images are sent as
//! base64 JPEG data URIs to `/chat/completions`; the server-side model
//! name is auto-detected from `/models` when unset.

use std::path::Path;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};

/// Reasoning models with thinking enabled can take minutes to answer
/// (the Python client uses the same generous timeout).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(900);

#[derive(Debug, thiserror::Error)]
pub enum CaptionError {
    #[error("no server URL set")]
    NoBaseUrl,
    #[error("invalid server URL: {0}")]
    InvalidBaseUrl(String),
    #[error("could not reach the server at {url}: {source}")]
    Request { url: String, source: reqwest::Error },
    #[error("the server returned an unexpected response: {0}")]
    UnexpectedResponse(String),
}

#[derive(Debug, Clone)]
pub struct EndpointConfig {
    /// For example `http://127.0.0.1:8000/v1` for a local vLLM server.
    pub base_url: String,
    pub api_key: Option<String>,
    /// The server-side model name; `None` auto-detects from `/models`.
    pub model_name: Option<String>,
}

impl EndpointConfig {
    pub fn base_url(&self) -> Result<String, CaptionError> {
        let trimmed = self.base_url.trim().trim_end_matches('/').to_owned();
        if trimmed.is_empty() {
            return Err(CaptionError::NoBaseUrl);
        }
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            return Err(CaptionError::InvalidBaseUrl(trimmed));
        }
        Ok(trimmed)
    }

    /// Resolve the server-side model name, detecting it when unset.
    pub fn resolve_model_name(&self) -> Result<String, CaptionError> {
        if let Some(name) = &self.model_name {
            if !name.trim().is_empty() {
                return Ok(name.trim().to_owned());
            }
        }
        let base = self.base_url()?;
        let url = format!("{base}/models");
        let mut request = reqwest::blocking::Client::new()
            .get(&url)
            .timeout(REQUEST_TIMEOUT);
        if let Some(key) = &self.api_key {
            if !key.trim().is_empty() {
                request = request.bearer_auth(key.trim());
            }
        }
        let response = request
            .send()
            .map_err(|source| CaptionError::Request { url, source })?;
        let body: ModelsResponse = response
            .json()
            .map_err(|source| CaptionError::UnexpectedResponse(source.to_string()))?;
        body.data
            .first()
            .map(|model| model.id.clone())
            .ok_or_else(|| {
                CaptionError::UnexpectedResponse("the /models list was empty".to_owned())
            })
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

/// Sampling parameters, mirroring the Python app's generation settings.
/// OpenAI-style greedy decoding corresponds to `do_sample = false`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationParams {
    pub max_tokens: u32,
    #[serde(default = "default_true")]
    pub do_sample: bool,
    pub temperature: f64,
    pub top_k: u32,
    pub top_p: f64,
    pub repetition_penalty: f64,
}

fn default_true() -> bool {
    true
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            max_tokens: 100,
            do_sample: false,
            temperature: 0.7,
            top_k: 50,
            top_p: 1.0,
            repetition_penalty: 1.0,
        }
    }
}

/// Strip a reasoning model's `<think>...</think>` block. The opening tag
/// is often forced by the chat template (so the response starts
/// mid-reasoning with only a closing tag) - the pattern matches both
/// cases, like `THINK_BLOCK_PATTERN` in the Python app.
pub fn strip_think_block(text: &str) -> String {
    // Find the closing tag first: everything before it (with or without
    // an opening tag) is reasoning.
    match text.find("</think>") {
        Some(end) => text[end + "</think>".len()..].trim_start().to_owned(),
        None => text.trim().to_owned(),
    }
}

/// Encode an image file as the data URI the chat API expects (JPEG
/// re-encode, like the Python client).
pub fn image_to_data_uri(image_path: &Path) -> Result<String, CaptionError> {
    let image = image::open(image_path).map_err(|source| {
        CaptionError::UnexpectedResponse(format!(
            "failed to read {}: {source}",
            image_path.display()
        ))
    })?;
    let mut jpeg = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90);
    image
        .write_with_encoder(encoder)
        .map_err(|source| CaptionError::UnexpectedResponse(source.to_string()))?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 1],
    max_tokens: u32,
    temperature: f64,
    top_p: f64,
    top_k: u32,
    repetition_penalty: f64,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: [ContentPart<'a>; 2],
}

#[derive(Debug, Serialize)]
struct ContentPart<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<ImageUrl<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ImageUrl<'a> {
    url: &'a str,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatChoice {
    message: ChatAnswer,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatAnswer {
    content: Option<String>,
}

/// Caption one image. `prompt` is the full user prompt (already combined
/// with any captioner suffix by the caller).
pub fn caption_image(
    config: &EndpointConfig,
    image_path: &Path,
    prompt: &str,
    params: &GenerationParams,
) -> Result<String, CaptionError> {
    let base = config.base_url()?;
    let model = config.resolve_model_name()?;
    let data_uri = image_to_data_uri(image_path)?;
    let payload = ChatRequest {
        model: &model,
        messages: [ChatMessage {
            role: "user",
            content: [
                ContentPart {
                    kind: "image_url",
                    image_url: Some(ImageUrl { url: &data_uri }),
                    text: None,
                },
                ContentPart {
                    kind: "text",
                    image_url: None,
                    text: Some(prompt),
                },
            ],
        }],
        // OpenAI-style greedy decoding: a temperature of 0 disables
        // sampling (the Python client maps do_sample the same way).
        temperature: if params.do_sample {
            params.temperature
        } else {
            0.0
        },
        max_tokens: params.max_tokens,
        top_p: params.top_p,
        top_k: params.top_k,
        repetition_penalty: params.repetition_penalty,
    };
    let url = format!("{base}/chat/completions");
    let mut request = reqwest::blocking::Client::new()
        .post(&url)
        .timeout(REQUEST_TIMEOUT)
        .json(&payload);
    if let Some(key) = &config.api_key {
        if !key.trim().is_empty() {
            request = request.bearer_auth(key.trim());
        }
    }
    let response = request
        .send()
        .map_err(|source| CaptionError::Request { url, source })?;
    let body: ChatResponse = response
        .json()
        .map_err(|source| CaptionError::UnexpectedResponse(source.to_string()))?;
    let content = body
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .ok_or_else(|| {
            CaptionError::UnexpectedResponse(serde_json::to_string(&body).unwrap_or_default())
        })?;
    Ok(strip_think_block(&content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_templates;

    #[test]
    fn think_blocks_are_stripped() {
        // Self-contained block.
        assert_eq!(strip_think_block("<think>hm</think>a cat"), "a cat");
        // Only a closing tag (the chat template forced the opener).
        assert_eq!(strip_think_block("reasoning...</think>a cat"), "a cat");
        // No block at all.
        assert_eq!(strip_think_block("a cat"), "a cat");
    }

    #[test]
    fn base_url_validation() {
        let config = EndpointConfig {
            base_url: String::new(),
            api_key: None,
            model_name: None,
        };
        assert!(matches!(config.base_url(), Err(CaptionError::NoBaseUrl)));
        let config = EndpointConfig {
            base_url: String::from("127.0.0.1:8000"),
            api_key: None,
            model_name: None,
        };
        assert!(matches!(
            config.base_url(),
            Err(CaptionError::InvalidBaseUrl(_))
        ));
        let config = EndpointConfig {
            base_url: String::from("http://127.0.0.1:8000/v1/"),
            api_key: None,
            model_name: None,
        };
        assert_eq!(config.base_url().unwrap(), "http://127.0.0.1:8000/v1");
    }

    #[test]
    fn data_uri_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img.png");
        image::save_buffer_with_format(
            &path,
            &[255, 0, 0, 255],
            1,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let data_uri = image_to_data_uri(&path).unwrap();
        assert!(data_uri.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn generated_templates_are_present() {
        // The generated module must expose the same shape as Python.
        assert_eq!(prompt_templates::PURPOSES.len(), 5);
        assert!(prompt_templates::prompt_template("SDXL", "Style")
            .unwrap()
            .contains("Do not mention the art style"));
        assert!(prompt_templates::captioner_suffix("JoyCaption").is_some());
        assert!(prompt_templates::captioner_suffix("(none)").is_none());
    }
}
