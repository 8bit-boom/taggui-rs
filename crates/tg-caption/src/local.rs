//! Local GGUF captioning via llama.cpp's multimodal (mtmd) interface.
//!
//! This module is a contained unsafe layer over the C API (mirroring
//! llama.cpp's `llava-cli` flow): load a GGUF model (+ an `mmproj`
//! projector for vision), tokenize prompt + image into chunks, evaluate
//! them with `mtmd_helper_eval_chunks`, then sample tokens.
//!
//! Targets the Qwen-VL chat format (`<|im_start|>` family: Qwen2.5-VL,
//! Qwen3-VL, ...), which is what the Python fork captioned with. The
//! image goes where the mtmd media marker is, and mtmd expands it to the
//! model's own vision tokens.
//!
//! Runs sequentially on one thread (decode is not thread-safe); the GUI
//! calls this on a dedicated worker.

use std::ffi::{c_int, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use llama_cpp_sys_2 as sys;

use crate::client::strip_think_block;

/// The default mtmd media marker (placeholder in the prompt where the
/// image is inserted).
pub const MEDIA_MARKER: &str = "<__media__>";

/// `LLAMA_DEFAULT_SEED`
const DEFAULT_SEED: u32 = 0xFFFF_FFFF;

static BACKEND_INIT: OnceLock<()> = OnceLock::new();

pub fn init_backend() {
    BACKEND_INIT.get_or_init(|| unsafe {
        sys::llama_backend_init();
    });
}

#[derive(Debug, Clone)]
pub struct LocalModelConfig {
    /// Path to the main GGUF file (or the first shard - llama.cpp finds
    /// sibling `*-00001-of-*` shards automatically when they sit next to
    /// it).
    pub model_path: PathBuf,
    /// Path to the `mmproj` projector GGUF; `None` for text-only models.
    pub mmproj_path: Option<PathBuf>,
    /// Layers to offload to the GPU: `0` = CPU only, large values (e.g.
    /// 999) = everything, for CUDA/Vulkan builds.
    pub n_gpu_layers: i32,
    pub n_ctx: u32,
    pub n_threads: u32,
}

impl Default for LocalModelConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            mmproj_path: None,
            n_gpu_layers: 0,
            n_ctx: 4096,
            n_threads: 0, // 0 = hardware default
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    #[error("failed to read {path}: {source}")]
    Image {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error("the image does not contain any data")]
    EmptyImage,
    #[error("failed to load the model: {0}")]
    ModelLoad(String),
    #[error("failed to create the context: {0}")]
    ContextLoad(String),
    #[error("failed to load the mmproj projector: {0}")]
    MmprojLoad(String),
    #[error("tokenization failed: {0}")]
    Tokenize(i32),
    #[error("evaluation failed: {0}")]
    Eval(i32),
}

/// Progress event streamed back to the GUI per image.
pub struct LocalProgress {
    pub index: usize,
    pub caption: Option<String>,
    pub error: Option<String>,
    pub elapsed_seconds: f64,
}

fn cstring(path: &Path) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| format!("path contains an interior NUL: {}", path.display()))
}

/// Build the chat-formatted prompt with the media marker. Qwen-VL-family
/// chat template (im_start/im_end); `parse_special = true` in mtmd makes
/// the tokenizer understand the special tokens.
fn build_prompt(prompt: &str, with_vision: bool) -> String {
    let marker = if with_vision {
        format!("{MEDIA_MARKER}\n")
    } else {
        String::new()
    };
    format!("<|im_start|>user\n{marker}{prompt}<|im_end|>\n<|im_start|>assistant\n")
}

/// Decode an image into opaque-composited RGB8 bytes.
fn load_rgb(image_path: &Path) -> Result<(u32, u32, Vec<u8>), LocalError> {
    let decoded = image::open(image_path).map_err(|source| LocalError::Image {
        path: image_path.to_path_buf(),
        source,
    })?;
    let rgba = decoded.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    let mut rgb = vec![255u8; width as usize * height as usize * 3];
    for (pixel, chunk) in rgba.pixels().zip(rgb.chunks_mut(3)) {
        let alpha = pixel.0[3] as u32;
        for (channel, channel_byte) in chunk.iter_mut().enumerate() {
            *channel_byte = ((pixel.0[channel] as u32 * alpha
                + *channel_byte as u32 * (255 - alpha))
                / 255) as u8;
        }
    }
    if width == 0 || height == 0 {
        return Err(LocalError::EmptyImage);
    }
    Ok((width, height, rgb))
}

/// Caption one image with an already-loaded model context (the argument
/// count mirrors the C API surfaces it wraps).
///
/// # Safety
/// All pointers come from the handles created by `run_local_captioning`,
/// which outlives this call and are used from a single thread.
#[allow(clippy::needless_range_loop)]
#[allow(clippy::too_many_arguments)]
unsafe fn caption_single(
    vocab: *const sys::llama_vocab,
    lctx: *mut sys::llama_context,
    mtmd: *mut sys::mtmd_context,
    image_path: &Path,
    prompt: &str,
    params: &crate::client::GenerationParams,
    cancel: &AtomicBool,
    n_batch: c_int,
) -> Result<String, LocalError> {
    let (width, height, rgb) = load_rgb(image_path)?;
    let bitmap = sys::mtmd_bitmap_init(width, height, rgb.as_ptr().cast());
    if bitmap.is_null() {
        return Err(LocalError::EmptyImage);
    }
    let result = (|| {
        let full_prompt = build_prompt(prompt, !mtmd.is_null());
        let prompt_c = CString::new(full_prompt).map_err(|_| LocalError::Tokenize(0))?;
        let text_len = prompt_c.as_bytes().len();

        let mut n_past: sys::llama_pos = 0;
        if mtmd.is_null() {
            // Text-only GGUF: two-pass tokenize, then decode each token.
            let needed = sys::llama_tokenize(
                vocab,
                prompt_c.as_ptr(),
                text_len as c_int,
                std::ptr::null_mut(),
                0,
                false,
                true,
            );
            if needed < 0 {
                return Err(LocalError::Tokenize(needed));
            }
            let mut tokens = vec![0 as sys::llama_token; needed as usize];
            let written = sys::llama_tokenize(
                vocab,
                prompt_c.as_ptr(),
                text_len as c_int,
                tokens.as_mut_ptr(),
                tokens.len() as c_int,
                false,
                true,
            );
            #[allow(unused_mut)]
            if written < 0 {
                return Err(LocalError::Tokenize(written));
            }
            tokens.truncate(written as usize);
            for token in tokens {
                let mut token = token;
                let batch = sys::llama_batch_get_one(&mut token, 1);
                if sys::llama_decode(lctx, batch) != 0 {
                    return Err(LocalError::Eval(-1));
                }
                n_past += 1;
            }
        } else {
            let input_text = sys::mtmd_input_text {
                text: prompt_c.as_ptr(),
                text_len,
                add_special: false,
                parse_special: true,
            };
            let chunks: *mut sys::mtmd_input_chunks = std::ptr::null_mut();
            let bitmaps = [bitmap as *const sys::mtmd_bitmap];
            let code = sys::mtmd_tokenize(mtmd, chunks, &input_text, bitmaps.as_ptr(), 1);
            if code != 0 {
                return Err(LocalError::Tokenize(code));
            }
            let mut new_n_past: sys::llama_pos = 0;
            let code = sys::mtmd_helper_eval_chunks(
                mtmd,
                lctx,
                chunks,
                n_past,
                0,
                n_batch,
                true,
                &mut new_n_past,
            );
            sys::mtmd_input_chunks_free(chunks);
            if code != 0 {
                return Err(LocalError::Eval(code));
            }
            n_past = new_n_past;
            let _ = n_past;
        }

        // Sampling loop.
        let chain_params = sys::llama_sampler_chain_default_params();
        let sampler = sys::llama_sampler_chain_init(chain_params);
        if sampler.is_null() {
            return Err(LocalError::Eval(-2));
        }
        if params.do_sample {
            sys::llama_sampler_chain_add(
                sampler,
                sys::llama_sampler_init_temp_ext(params.temperature as f32, 0.0, 1.0),
            );
            if params.top_k > 0 {
                sys::llama_sampler_chain_add(
                    sampler,
                    sys::llama_sampler_init_top_k(params.top_k as c_int),
                );
            }
            sys::llama_sampler_chain_add(
                sampler,
                sys::llama_sampler_init_top_p(params.top_p as f32, 1),
            );
            sys::llama_sampler_chain_add(sampler, sys::llama_sampler_init_dist(DEFAULT_SEED));
        } else {
            sys::llama_sampler_chain_add(sampler, sys::llama_sampler_init_greedy());
        }

        let mut output = String::new();
        let mut generated: u32 = 0;
        while generated < params.max_tokens {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut token = sys::llama_sampler_sample(sampler, lctx, -1);
            if sys::llama_vocab_is_eog(vocab, token) {
                break;
            }
            sys::llama_sampler_accept(sampler, token);
            let piece = sys::llama_vocab_get_text(vocab, token);
            if !piece.is_null() {
                output.push_str(&std::ffi::CStr::from_ptr(piece).to_string_lossy());
            }
            generated += 1;
            let batch = sys::llama_batch_get_one(&mut token, 1);
            if sys::llama_decode(lctx, batch) != 0 {
                break;
            }
            n_past += 1;
            let _ = n_past;
        }
        sys::llama_sampler_free(sampler);
        Ok(strip_think_block(&output))
    })();
    sys::mtmd_bitmap_free(bitmap);
    result
}

/// Caption a list of images sequentially with one loaded model.
/// Progress events are streamed through `send`.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn run_local_captioning(
    jobs: Vec<(usize, PathBuf)>,
    config: &LocalModelConfig,
    prompt: &str,
    params: &crate::client::GenerationParams,
    cancel: &AtomicBool,
    mut send: impl FnMut(LocalProgress),
) {
    init_backend();
    let mut fail = |message: String| {
        send(LocalProgress {
            index: 0,
            caption: None,
            error: Some(message),
            elapsed_seconds: 0.0,
        });
    };
    let model_path = match cstring(&config.model_path) {
        Ok(path) => path,
        Err(message) => return fail(message),
    };
    unsafe {
        let mut model_params = sys::llama_model_default_params();
        model_params.n_gpu_layers = config.n_gpu_layers;
        let model = sys::llama_model_load_from_file(model_path.as_ptr(), model_params);
        if model.is_null() {
            return fail(
                LocalError::ModelLoad(config.model_path.display().to_string()).to_string(),
            );
        }
        let vocab = sys::llama_model_get_vocab(model);
        let mut ctx_params = sys::llama_context_default_params();
        ctx_params.n_ctx = config.n_ctx;
        ctx_params.n_batch = 512;
        ctx_params.n_ubatch = 512;
        if config.n_threads > 0 {
            ctx_params.n_threads = config.n_threads as i32;
            ctx_params.n_threads_batch = config.n_threads as i32;
        }
        let lctx = sys::llama_init_from_model(model, ctx_params);
        if lctx.is_null() {
            sys::llama_model_free(model);
            return fail(
                LocalError::ContextLoad("llama_init_from_model returned null".to_owned())
                    .to_string(),
            );
        }
        let mut mtmd: *mut sys::mtmd_context = std::ptr::null_mut();
        if let Some(mmproj_path) = &config.mmproj_path {
            if let Ok(mmproj_c) = cstring(mmproj_path) {
                let mut mtmd_params = sys::mtmd_context_params_default();
                mtmd_params.print_timings = false;
                mtmd_params.use_gpu = config.n_gpu_layers > 0;
                mtmd = sys::mtmd_init_from_file(mmproj_c.as_ptr(), model, mtmd_params);
            }
            if mtmd.is_null() {
                sys::llama_free(lctx);
                sys::llama_model_free(model);
                return fail(LocalError::MmprojLoad(mmproj_path.display().to_string()).to_string());
            }
        }
        let n_batch = ctx_params.n_batch as c_int;
        for (index, image_path) in jobs {
            let started = std::time::Instant::now();
            let (caption, error) = match caption_single(
                vocab,
                lctx,
                mtmd,
                &image_path,
                prompt,
                params,
                cancel,
                n_batch,
            ) {
                Ok(caption) => (Some(caption), None),
                Err(error) => (None, Some(error.to_string())),
            };
            send(LocalProgress {
                index,
                caption,
                error,
                elapsed_seconds: started.elapsed().as_secs_f64(),
            });
            if cancel.load(Ordering::Relaxed) {
                break;
            }
        }
        if !mtmd.is_null() {
            sys::mtmd_free(mtmd);
        }
        sys::llama_free(lctx);
        sys::llama_model_free(model);
    }
}
