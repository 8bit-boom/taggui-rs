# Port status

Feature parity tracker for the Python -> Rust port. Python reference:
`8bit-boom/taggui` (this repo's FORK_NOTES.md documents the Python side).

Legend: **done** | partial | deferred | n/a (not applicable)

## Core data layer (tg-core)

| Python feature | Status | Notes |
|---|---|---|
| Sidecar `.txt` read/write | **done** | Byte-identical; golden-tested |
| Directory scan (formats, sort by path) | **done** | Dimensions via `image` crate |
| Tag separator settings | **done** | `config.toml`, not QSettings (documented divergence) |
| Caption positions (5) | **done** | Golden-tested |
| Find & replace (joined-caption semantics) | **done** | Python replacement templates translated |
| Rename/delete tags (whole-tag, regex) | **done** | |
| Batch sort/reverse keeping first tag | **done** | |
| Image list filter grammar | **done** | Full grammar, 43 golden-evaluated filters |
| `tokens:` filter (CLIP tokenizer) | **done** | Bundled tokenizer via HF `tokenizers` |

## GUI (tg-gui)

| Python feature | Status | Notes |
|---|---|---|
| Open folder | **done** | |
| Image list with thumbnails | **done** | Single-column list; grid view deferred |
| Image viewer (zoom/pan) | **done** | Scroll-zoom, drag-pan |
| Tag editor with save-on-change | **done** | |
| Keyboard navigation (Up/Down) | **done** | Ctrl+Z/Y undo/redo |
| Undo/redo stack | **done** | Snapshot-based, like Python |
| Image filters UI | **done** | |
| All-tags editor (rename/delete across images) | **done** | Extras panel: rename, delete everywhere (whole-tag + regex) |
| Tag autocomplete (Danbooru CSV) | **done** | Suggestion buttons under the tag editor; most-used counts deferred |
| Dataset statistics | **done** | Counts, avg tags, unique tags, aspect summary, top tags (click = filter) |
| Saved filters | **done** | Persisted in config.toml, apply/delete |
| Find & replace dialog UI | **done** | Extras panel (substring/whole-tag/regex, visible scope) |
| Batch reorder tags | **done** | Sort A-Z / reverse keeping first tag (visible images) |
| Export dataset | **done** | tags.csv, captions.txt, tag_frequency.csv into the dataset folder |
| EXIF orientation | **done** | Thumbnails and viewer |
| Tag presets dock | deferred | Schema in settings; UI deferred |
| Convert images dialog | deferred | |

## Auto-captioning (tg-caption)

| Python feature | Status | Notes |
|---|---|---|
| Remote OpenAI-compatible endpoint (vLLM/SGLang) | **done** | Base64 JPEG, model auto-detect |
| Concurrent requests (batch size) | **done** | Thread pool, mpsc to UI |
| Generation params (tokens, sampling, temp) | **done** | |
| Think-block stripping | **done** | |
| Prompt template library | **done** | Generated from Python source |
| Per-image error isolation + failure summary | **done** | |
| Cancel mid-run | **done** | |
| Test-run mode (first N) | **done** | |
| ETA in progress UI | **done** | |
| **Local models** | **done** | Via llama.cpp GGUF + mtmd (`local.rs`): Qwen-VL-family chat format, mmproj vision, GPU offload via `n_gpu_layers` (CUDA/Vulkan builds), quantization via GGUF quants. Sequential (decode is single-threaded) |
| **WD Tagger (ONNX)** | **done** | `ort` crate; exact port of preprocessing (white composite, square pad, bicubic resize, RGB->BGR), kaomoji handling, category filtering, thresholds, sort, truncate |
| Model manager (downloads) | **done** | HF repo file listing + streamed downloads with progress |
| Local transformers checkpoints (safetensors) | n/a | By design: local inference uses GGUF; the transformers architectures stay in the Python app |
| SageAttention / bnb quantization / attn picker | n/a | transformers-specific; GGUF quantization replaces them |
| Caption profiles | deferred | |
| Compare prompts dialog | deferred | |
