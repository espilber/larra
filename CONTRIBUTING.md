# Contributing to Larra

Larra is a Tauri 2 + Svelte 5 + Rust desktop app. Supported platforms: macOS (Apple Silicon and Intel) and Windows 10/11 (x64 with Vulkan, ARM64). Linux is not a supported platform.

Issues, pull requests, and discussions here are covered by the [Code of Conduct](CODE_OF_CONDUCT.md).

## First 30 minutes

1. Install [Rust](https://rustup.rs/) 1.98.0 (`rust-toolchain.toml` pins this), Node 22+ (`.nvmrc`), [pnpm](https://pnpm.io) 11, and [just](https://github.com/casey/just).
2. macOS: Xcode Command Line Tools. Windows: MSVC toolchain. First OCR build needs **CMake** and a C++ compiler (Xberg compiles Tesseract). Windows x64 also needs a Vulkan driver from the GPU vendor. Windows 11 includes WebView2; on Windows 10 install the [Evergreen WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) before `pnpm tauri dev`.
3. From the repo root:

```bash
pnpm install
just check          # svelte-check, Prettier, oxlint, cargo fmt, clippy
just test           # cargo test + Vitest
pnpm tauri dev
```

`pnpm tauri build` fetches the pinned llama.cpp archive for that target (~11–33 MB) and ships it in the bundle. Without signing credentials the DMG / NSIS is unsigned. See [docs/releasing.md](docs/releasing.md).

For `pnpm tauri dev`, run `pnpm fetch-engine` once so first chat can stay offline. Otherwise the running app downloads the pin.

## One test

```bash
cargo test --manifest-path src-tauri/Cargo.toml recipes::tests::old_contract_key_terms_id_is_renamed
cargo test --manifest-path src-tauri/Cargo.toml --test pipeline_test markdown_contract
pnpm exec vitest run src/lib/focus-trap.test.ts
```

Optional full-loop smoke (downloads nothing if local files are passed):

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test core_smoke -- --ignored --nocapture
```

Needs `REBOST_ENGINE_ARCHIVE` (the **host** archive from [docs/engine.md](docs/engine.md)) and `REBOST_TEST_MODEL` (see `.env.example`). Network catalog tests:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test model_catalog -- --ignored
```

CI runs frontend checks and `cargo deny` on every push. The Rust job runs only when `src-tauri/`, `rust-toolchain.toml`, or this workflow change. On Ubuntu it does `cargo fmt`, `clippy --lib --tests`, `cargo test --lib` (as `rlib`, so it does not link the desktop library), and a check that llama.cpp pin URLs still exist. Full `cargo test` (integration tests, cdylib) stays on `just test` and before a release. Linux is not a supported product platform. Windows is covered by the manual release workflow, not every push.

## TypeScript versions

`pnpm check` (`svelte-check`) uses **TypeScript 6** (`typescript` in `package.json`). `@typescript/native` is a TypeScript 7 preview for editors. Version-mismatch warnings between those two are expected.

## Verification

Run commands from the repo root with pnpm. Match verification to the changed behavior:

| Change | Checks |
|--------|--------|
| Documentation or instruction prose | Review accuracy, links, and examples; `git diff --check`. App compilation and runtime tests are unnecessary. |
| Frontend code | Format changed files; `pnpm check`, `pnpm format:check`, `pnpm lint`, and relevant Vitest tests for changed behavior. |
| UI catalogs | Check JSON, interpolation tokens, and catalog parity with `pnpm exec vitest run src/lib/i18n.test.ts`. |
| Rust code | `cargo fmt --manifest-path src-tauri/Cargo.toml --check`, relevant Rust tests, and Clippy for affected targets. Include release Clippy when debug-only code or imports changed. |
| Build or release helpers | Check script syntax and exercise changed behavior using isolated fixtures or mocked build/publish commands. |
| Broad changes across subsystems | `just gate` (format, check, test). |
| Installer release | Full `just gate` on the final source before building, plus the applicable release-candidate checks in `docs/experience-quality.md`. |

Reuse successful checks until a later change, failure, or unresolved concern invalidates them. Rerun the affected checks, including behavioral tests when logic changes. Do not add tests that only restate a trivial implementation. Fix failures while making progress; report the actual blocker when progress requires unavailable input or tools. Do not bypass failing required checks to publish.

`just check` and `just test` remain convenient full-check wrappers. Normal tests do not require ignored network or real-model tests; those are explicit checks for the relevant work and release candidates.

## Dev utilities

- `./scripts/reset.sh` (macOS) or `./scripts/reset.ps1` (Windows): wipe Larra app data (Shelf files in `library/` are kept). Settings → Reset Larra does the same from the running app.
- `cargo run --manifest-path src-tauri/Cargo.toml --example seed -- [--model PATH] [--fresh] [--empty] [--ai-name "Muse Glimmer"]`: fills a demo library with two invented Shelves, Harbor and Notes, plus a full chat list (refuses if Larra is open). `--ai-name` is only the label shown for the installed AI. `--empty` finishes first run with no Shelves.
- `VITE_START_VIEW=shelves VITE_START_SHELF=first pnpm tauri dev`: land on a specific screen

Logs: Settings → Diagnostics (paths only; the engine log body stays on disk). On macOS also `~/Library/Logs/io.larra.desktop/`. Engine stdout is `logs/engine.log` under app data.

## Where to change things

| Task | Start here |
|------|------------|
| Tauri commands | `src-tauri/src/commands/` and `src/lib/api.ts` |
| Model catalog / recommend | `src-tauri/src/engine/catalog.rs` |
| Engine URL / SHA matrix | `src-tauri/src/engine/pin.rs` |
| Add a file format | `docs/ingest-formats.md` |
| Chat prompts | `src-tauri/src/chat/prompts.rs` |
| UI copy | `locales/*.json` (`en.json` is the source; `t` / `t!` in the view or command) |
| Languages | `docs/i18n.md` |
| UI colors, buttons, shapes | `docs/ui.md`, `src/app.css` |
| In-app updates | `src-tauri/src/updater.rs`, `src/lib/views/UpdateWindow.svelte` |

## House style

Formatters own spaces and wrapping. `just check` runs svelte-check, Prettier, oxlint, rustfmt, and Clippy as errors.

**Indent.** 4 spaces in Rust, 2 in TypeScript / Svelte / CSS. Never tabs. Wrap at 100 characters, including comments.

**Names.** Follow the language: `snake_case` functions in Rust, `camelCase` in TypeScript. Tauri command names stay the Rust function name (`chat_send`). JSON fields are camelCase (`rename_all = "camelCase"`). Component callback props are camelCase (`onChooseShelf`); DOM events stay lowercase (`onclick`).

**Comments.** Explain why, not the next line.

- Rust: `//!` at the top of a module, `///` on public items (one sentence, period), `//` for invariants and `// SAFETY:` on `unsafe`.
- TypeScript / Svelte: `/** */` on exports that are not obvious from the name, `//` for implementation. Do not use `///` (that is rustdoc). Do not draw HTML section banners.

Sentence case. Delete anything the name or types already say.

**User-facing copy.** Lives in `locales/*.json`. Name the outcome, not the machinery. Prefer AI over model. Do not put GGUF, llama.cpp, or SHA-256 in the UI or in the README above Develop. Errors say what failed and what to do. Nav, View, and view titles translate Chat, Shelves, and Recipes. Running copy translates Shelf and Shelves with the catalog. Recipe, Chat, House rules, and Online stay English. The app is the subject, not "you". Draft catalogs (`pt`, `fr`, `ja`, `de`, `it`, `sv`, `nb`, `nl`, `cs`, `el`, `da`, `fi`) start with a `_review` note; native-speaker fixes belong in those files. See [docs/i18n.md](docs/i18n.md).

**Register.** The app UI is read at the machine, so "here" and "this computer" work there. The README and `docs/` are read on github.com, so name the machine instead: "the machine where Larra is installed", "that machine", or "locally".

## Pull requests

- Open a PR against `main`. Direct pushes to `main` are for maintainers.
- Issues tagged `good first issue` or `help wanted` are a reasonable place to start.
- Commit messages are **prose that explains why**, not Conventional Commit prefixes. Match the existing history.
- Include a test when changing ingest, retrieval, PII, downloads, or chat orchestration.
- There is no CLA to sign. A `Signed-off-by` line (DCO) is welcome and optional.

## Platform policy

Supported platforms: macOS and Windows 10/11. Release installers bundle one llama.cpp archive per OS/arch (`engine/pin.rs`). Linux URLs are in that matrix so the crate compiles on Linux CI. There is no Linux installer. See [docs/engine.md](docs/engine.md).

## License

By contributing you agree that your work is licensed under the AGPL-3.0-or-later License in `LICENSE`.
