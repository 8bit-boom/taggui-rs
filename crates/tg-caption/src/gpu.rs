//! GPU detection and build-capability reporting.
//!
//! `detect_gpus` queries the system for installed GPUs (NVIDIA via
//! `nvidia-smi`, everything via Windows' CIM video-controller list),
//! `gpu_backend` reports what the compiled binary supports, and
//! `detect_cuda_toolkit` checks for a usable `nvcc` so installers and
//! the GUI can guide the user.

use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuInfo {
    pub name: String,
    /// VRAM in MiB where reported (NVIDIA reports exact values; the
    /// Windows CIM query's `AdapterRAM` is a 32-bit byte count that
    /// saturates around 4 GiB, so treat it as best-effort).
    pub vram_mb: Option<u64>,
    pub is_nvidia: bool,
}

/// What this binary was built for (compile-time: the tg-caption
/// `cuda`/`vulkan` features), as set by its build script.
pub fn gpu_backend() -> &'static str {
    env!("TG_GPU_BACKEND")
}

/// Whether the local llama.cpp build can use a GPU at all.
pub fn gpu_supported_build() -> bool {
    gpu_backend() != "cpu"
}

/// Parse `nvidia-smi --query-gpu=name,memory.total
/// --format=csv,noheader,nounits` output (name, MiB per line).
pub fn parse_nvidia_smi(output: &str) -> Vec<GpuInfo> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.split(',');
            let name = parts.next()?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let vram_mb = parts
                .next()
                .and_then(|value| value.trim().parse::<u64>().ok());
            Some(GpuInfo {
                name,
                vram_mb,
                is_nvidia: true,
            })
        })
        .collect()
}

/// Parse `Get-CimInstance Win32_VideoController | Select-Object
/// Name,AdapterRAM | ConvertTo-Json -Compress` output (an object for one
/// GPU, an array for several; `AdapterRAM` is bytes or null).
pub fn parse_video_controllers(json: &str) -> Result<Vec<GpuInfo>, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Controller {
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "AdapterRAM", default)]
        adapter_ram: Option<u64>,
    }
    let value = serde_json::from_str::<serde_json::Value>(json)?;
    let controllers: Vec<Controller> = match value {
        serde_json::Value::Array(items) => serde_json::from_value(serde_json::Value::Array(items))?,
        object => vec![serde_json::from_value(object)?],
    };
    Ok(controllers
        .into_iter()
        .filter(|controller| !controller.name.is_empty())
        .map(|controller| GpuInfo {
            is_nvidia: controller.name.to_lowercase().contains("nvidia"),
            vram_mb: controller.adapter_ram.map(|bytes| bytes / (1024 * 1024)),
            name: controller.name,
        })
        .collect())
}

/// Detect installed GPUs: NVIDIA entries first (exact name + VRAM via
/// `nvidia-smi`), then any other display adapters via Windows CIM (AMD,
/// Intel, ...). Empty when nothing is found.
pub fn detect_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    if let Ok(output) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            gpus.extend(parse_nvidia_smi(&text));
        }
    }
    let nvidia_names: Vec<String> = gpus
        .iter()
        .filter(|gpu| gpu.is_nvidia)
        .map(|gpu| gpu.name.clone())
        .collect();
    // Non-NVIDIA adapters via CIM (Windows); on other platforms this is
    // the whole list.
    let script = "Get-CimInstance Win32_VideoController | \
        Select-Object Name,AdapterRAM | ConvertTo-Json -Compress";
    if let Ok(output) = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Ok(controllers) = parse_video_controllers(&text) {
                for controller in controllers {
                    let is_nvidia = controller.is_nvidia;
                    let already_listed = is_nvidia
                        && nvidia_names.iter().any(|name| {
                            controller.name.contains(name.as_str())
                                || name.contains(&controller.name)
                        });
                    if !already_listed && !is_nvidia {
                        gpus.push(controller);
                    }
                }
            }
        }
    }
    gpus
}

/// Detect a CUDA toolkit installation; returns the parsed release
/// string (e.g. `12.4`) when `nvcc` is on the PATH.
pub fn detect_cuda_toolkit() -> Option<String> {
    let output = Command::new("nvcc").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| {
            line.split("release ")
                .nth(1)
                .and_then(|rest| rest.split(',').next())
                .map(str::trim)
                .map(str::to_owned)
        })
        .filter(|release| !release.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_smi_lines() {
        let gpus =
            parse_nvidia_smi("NVIDIA GeForce RTX 4090, 24564\nNVIDIA GeForce RTX 3060, 12288\n");
        assert_eq!(gpus.len(), 2);
        assert!(gpus[0].is_nvidia);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(gpus[0].vram_mb, Some(24564));
        assert_eq!(gpus[1].vram_mb, Some(12288));
    }

    #[test]
    fn tolerates_garbage_nvidia_smi_output() {
        let gpus = parse_nvidia_smi("\n, \nbad line no comma here\n");
        // 'bad line' parses as a name with no VRAM (name non-empty).
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].vram_mb, None);
    }

    #[test]
    fn parses_single_video_controller_object() {
        let gpus = parse_video_controllers(
            r#"{"Name":"AMD Radeon RX 7900 XTX","AdapterRAM":17175674880}"#,
        )
        .unwrap();
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "AMD Radeon RX 7900 XTX");
        assert!(!gpus[0].is_nvidia);
        assert_eq!(gpus[0].vram_mb, Some(16380));
    }

    #[test]
    fn parses_video_controller_array_and_nulls() {
        let gpus = parse_video_controllers(
            r#"[{"Name":"Intel(R) UHD Graphics","AdapterRAM":null},
                {"Name":"NVIDIA GeForce RTX 4080","AdapterRAM":4293918720}]"#,
        )
        .unwrap();
        assert_eq!(gpus.len(), 2);
        assert!(gpus[0].vram_mb.is_none());
        assert!(gpus[1].is_nvidia);
    }

    #[test]
    fn build_backend_is_reported() {
        // Always one of the three; the exact value depends on features.
        assert!(matches!(gpu_backend(), "cpu" | "cuda" | "vulkan"));
    }
}
