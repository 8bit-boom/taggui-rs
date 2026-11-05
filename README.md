# TagGUI-rs

A Rust port of [TagGUI](https://github.com/8bit-boom/taggui), the tag
manager and captioner for image datasets. The Python app is a PySide6
desktop tool; this is the same concept rebuilt in Rust with egui - it
reads and writes the **same sidecar `.txt` caption files**, so both apps
can work on the same dataset interchangeably.

See [PORT_STATUS.md](PORT_STATUS.md) for exactly what is ported and
[PORT_GUIDE.md](PORT_GUIDE.md) for how the port is verified against the
Python app.

## Status: early MVP

- **Data layer (tg-core)**: complete and golden-parity-tested against the
  Python app - sidecar read/write (byte-identical), tag operations, the
  image filter grammar, settings.
- **GUI (tg-gui)**: open folder, image list with thumbnails, zoom/pan
  viewer, tag editor with save-on-change, undo/redo, image filters.
- **Auto-captioning (tg-caption)**: remote OpenAI-compatible endpoint
  (vLLM / SGLang) with concurrent requests, ETA, per-image error
  isolation, cancel, and test runs. The prompt-template library is ported
  verbatim (generated from the Python source).

Not yet ported: WD Tagger (ONNX), dataset statistics, convert-images,
tag presets, caption profiles, Danbooru autocomplete.

## Building

Requires the Rust toolchain (https://rustup.rs).

```
cargo build --release -p tg-gui
```

The binary lands in `target/release/tag-gui.exe` (or `tag-gui` on Linux).

## Running

```
cargo run -p tg-gui
```

1. **Open Folder...** - loads every image with a supported extension and
   its sidecar `.txt` tags.
2. Click images in the left list; edit tags on the right. Every change is
   written to the sidecar file immediately (same behavior as the Python
   app). `Up`/`Down` switch images; `Ctrl+Z` / `Ctrl+Y` undo/redo.
3. The filter box speaks the Python app's filter grammar:
   `tags:>3 AND ext:png`, `tag:"blue eyes"`, `width:>=1024`,
   `NOT tag:1boy`, `(tag:1girl OR tag:2girl) AND size:<2mb`.
4. **Auto-Captioner** (right panel): point it at an OpenAI-compatible
   server (e.g. `vllm serve Qwen/Qwen3-VL-8B-Instruct` or an SGLang
   server), set the prompt, press Start. `Batch size` is the number of
   concurrent requests. Test runs caption only the first N images.

## Workspace layout

| Crate       | Purpose |
|-------------|---------|
| `tg-core`   | Data layer: sidecar files, tag ops, filters, settings. Golden-parity-tested. |
| `tg-caption`| OpenAI-compatible endpoint client + prompt templates. |
| `tg-gui`    | The egui desktop app. |

## License

GPL-3.0 - this is a port of GPL-3.0 TagGUI.
