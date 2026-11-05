# Port guide

How the Python -> Rust port keeps itself honest. This file is the
taggui-rs equivalent of nd-world's `FORK_RUST_PORT_GUIDE.md`.

## The oracle principle

The Python app (`github.com/8bit-boom/taggui`) is the **oracle**: wherever
a behavior is ported, the Rust implementation must match what the Python
app actually does - verified by fixtures generated from the real Python
code, not from our reading of it.

For an HTTP gateway (nd-world), parity is per-route byte equality. For a
desktop GUI, the parity surface is the **data layer**: whatever the Rust
app writes to disk must be byte-identical to what the Python app writes,
and both must read each other's files interchangeably.

## What must stay byte-identical

- Sidecar `.txt` writes: tags joined with the effective separator (default
  `, `), UTF-8, **no trailing newline**
  (`taggui/models/image_list_model.py`, `write_image_tags_to_disk`).
- Sidecar reads: UTF-8 with replacement characters (`errors='replace'`),
  split on the separator, each tag trimmed, empties dropped.
- The filter grammar's parse acceptance and evaluation results.
- Tag operations: caption positions, find & replace (which runs on the
  *joined caption* and can cross tag boundaries!), rename/delete tags.

## Golden fixtures

`tools/gen_golden.py` runs against a checkout of the Python app (its
real `ImageListModel`, real `add_caption_to_tags`, real pyparsing
grammar, real filter evaluator) and writes `crates/tg-core/golden/*.json`
fixtures. `crates/tg-core/tests/golden_parity.rs` asserts the Rust
implementation reproduces every fixture.

Regenerate after changing the fixtures or after pulling Python-side
changes worth capturing:

```
python tools/gen_golden.py <path-to-python-taggui-checkout>
```

Requires the Python app's dependencies (PySide6, torch, transformers, ...)
- the Python repo's `venv` works.

## Generated code

`crates/tg-caption/src/prompt_templates.rs` is generated from the
Python app's `prompt_templates.py` by `tools/gen_prompts.py` - the
Python file is the source of truth for template wording and the
rationale comments. Regenerate with:

```
python tools/gen_prompts.py <path-to-python-taggui-checkout>
```

## Deliberate divergences (documented, not accidental)

- **Settings**: `config.toml` in the platform config dir, NOT the Python
  app's QSettings store. The schema mirrors `utils/settings.py` defaults.
- **Regex replacement templates** in find & replace accept Python syntax
  (`\1`, `\g<name>`, `\\`) and translate to the Rust regex crate
  internally; a literal `$` stays literal.
- **Auto-captioning** ships endpoint-only (vLLM/SGLang over HTTP); local
  transformers inference stays in the Python app for now.

## Testing

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all --check
```

CI (`.github/workflows/ci.yml`) runs all three on Windows and Linux.
