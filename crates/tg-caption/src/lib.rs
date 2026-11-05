//! tg-caption: auto-captioning backends for taggui-rs. Port of the
//! Python fork's `auto_captioning/models/openai_compatible.py` - images
//! are sent to an OpenAI-compatible server (vLLM, SGLang, ...) as base64
//! JPEG data URIs over `/chat/completions`.

pub mod client;
pub mod prompt_templates;
