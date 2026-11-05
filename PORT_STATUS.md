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
| All-tags editor (rename/delete across images) | deferred | Core ops exist in tg-core |
| Tag autocomplete (most-used + Danbooru CSV) | deferred | |
| Dataset statistics dock | deferred | |
| Tag presets dock | deferred | |
| Saved filters menu | deferred | |
| Find & replace dialog UI | deferred | Core ops exist in tg-core |
| Batch reorder tags dialog | deferred | Core ops exist in tg-core |
| Convert images dialog | deferred | |
| Export dataset dialog | deferred | |
| EXIF orientation on thumbnails | deferred | |

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
| Local transformers models (Qwen3-VL etc.) | n/a | Stays in the Python app; Rust local inference (ort/candle) is a future project |
| WD Tagger (ONNX) | deferred | via `ort` crate |
| Caption profiles | deferred | |
| Compare prompts dialog | deferred | |
| Pre-quantized checkpoint detection | n/a | Server-side concern |
