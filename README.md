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

Local models run through **llama.cpp** (GGUF checkpoints with an
`mmproj` projector for vision - Qwen2.5-VL/Qwen3-VL, LLaVA-family,
JoyCaption GGUFs) with GPU offload; WD taggers run via ONNX Runtime.
The transformers-only features (SageAttention, bitsandbytes,
safetensors checkpoints) stay in the Python app by design.

Not yet ported: dataset statistics, convert-images, tag presets, caption
profiles, Danbooru autocomplete.

### Windows build requirements (handled by install.bat)

`install.bat` detects your GPU and sets everything up automatically:

- **NVIDIA GPU found** -> offers to install the CUDA toolkit (winget),
  then builds with CUDA: local captioning offloads all layers to the GPU
  and enables llama.cpp's flash-attention kernels (the Rust stack's
  equivalent of the Python app's SageAttention/FlashAttention - nothing
  extra to install).
- **AMD/Intel GPU found** -> offers the Vulkan SDK and builds with
  Vulkan.
- **No GPU** -> builds CPU-only (the remote API backend is unaffected).

Everything else (Rust, CMake, LLVM/libclang for the bindings, the Visual
Studio C++ Build Tools when needed) is installed too; the app itself
reports what it found at startup in the Auto-Captioner panel and the
console. If the GPU build fails for any reason, the installer falls back
to a working CPU build automatically.

Downloads are handled in-app via the Model Manager: pick a repo (or a
quick-pick button), press `Find model files` to auto-select the GGUF
shards/mmproj/ONNX files, then Download.

## Installation

### Windows (automated)

1. Install [Git](https://git-scm.com/downloads) if you don't already have
   it (only needed for updates; you can also download a ZIP instead).
2. Clone the repository (or download and extract the ZIP from the green
   `Code` button):
   ```
   git clone https://github.com/8bit-boom/taggui-rs.git
   cd taggui-rs
   ```
3. Run `install.bat`. It installs everything needed - the Rust toolchain
   (via winget or rustup-init), the Microsoft C++ build tools if the
   build needs them, and builds TagGUI-rs. The first build can take
   5-15 minutes; after that, updates are much faster.
4. Double-click `taggui-rs.bat` to start the app.
5. To update later, run `update.bat`. It pulls the latest code and
   rebuilds.

`install.bat` is safe to re-run at any time - it skips whatever is
already installed.

### Linux / manual (any platform)

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
