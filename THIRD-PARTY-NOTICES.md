# Third-party notices

Larra is AGPL-3.0-or-later licensed ([LICENSE](LICENSE), [NOTICE.md](NOTICE.md)). Copyright © 2026 Larra contributors. Rebost-derived code remains MIT (Copyright © Frontierz).

This file lists bundled and runtime third-party material. Direct dependencies are declared in `src-tauri/Cargo.toml` and `package.json`; lockfiles record the exact graph. A more complete dump can be generated at release time with `cargo about` and `pnpm licenses` (see `scripts/generate-notices.sh`).

## Bundled with the app

### Tesseract OCR traineddata

- Location: `src-tauri/resources/tessdata/`
- Project: [tesseract-ocr/tessdata](https://github.com/tesseract-ocr/tessdata)
- License: Apache License 2.0 (`src-tauri/resources/tessdata/LICENSE`)

OCR itself is provided by the `xberg` crate (vendored Tesseract, statically linked). English data is also pulled in via xberg's `bundle-tessdata-eng` feature. Any `*.traineddata` file in that directory is copied at startup and used for OCR.

### llama.cpp (`llama-server`)

- Not compiled into the Larra binary.
- Downloaded on first need from the pinned official llama.cpp release in `src-tauri/src/engine/pin.rs` (`ENGINE_RELEASE`, archive tag `ENGINE_BUILD`, URL, SHA-256).
- License: MIT (ggml-org/llama.cpp)

## Direct Rust dependencies (high level)

| Crate | Role | License (as declared) |
|-------|------|------------------------|
| tauri and Tauri plugins | Desktop shell | MIT OR Apache-2.0 |
| xberg | Document extraction / OCR | MIT |
| tantivy / tantivy-stemmers | Local search | MIT |
| pii-vault | Privacy Lens | MIT |
| reqwest, tokio, serde, anyhow | Plumbing | MIT OR Apache-2.0 |

Transitive crates include **MPL-2.0** file-level copyleft (for example CSS-related crates used through the frontend toolchain). Source for those files remains available via the lockfile and upstream repositories.

## Direct frontend dependencies

Svelte, Vite, Tailwind CSS, marked, DOMPurify, Lucide, svelte-sonner, `@tauri-apps/api`. Licenses are MIT/Apache-2.0 unless a package README says otherwise.

## AI model weights

**Not part of this repository.** Models are downloaded at the user's request from Hugging Face or Ollama and are subject to **their** licenses (Apache-2.0, MIT, Gemma Terms, and others). The in-app catalog shows the license before install. Installing Larra does not grant rights to those weights.

## Disclaimer

This notice is for attribution. It is not legal advice. If a lockfile entry disagrees with this summary, the lockfile and the upstream license file win.
