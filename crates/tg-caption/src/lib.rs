//! tg-caption: captioning backends for taggui-rs. Port of the
//! Python fork's auto-captioning: OpenAI-compatible endpoint, local GGUF
//! models via llama.cpp mtmd, and WD Tagger via ONNX Runtime.

pub mod client;
pub mod hf;
pub mod local;
pub mod prompt_templates;
pub mod wd_tagger;
