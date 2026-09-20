# Development

See [CONTRIBUTING.md](../CONTRIBUTING.md) for prerequisites, first 30 minutes, and PR expectations. Installer cuts: [releasing.md](releasing.md).

## Commands

| Command | What |
|---------|------|
| `pnpm install` | Frontend deps |
| `pnpm tauri dev` | Dev app + Vite |
| `pnpm tauri build` | Fetches the host engine archive, then a DMG (macOS) or NSIS (Windows). Unsigned unless `.env.signing` is loaded. |
| `pnpm fetch-engine` | Download/stage the pinned llama.cpp archive (`--all`, `--triple=…`) |
| `pnpm check` | `svelte-check` |
| `pnpm format:check` | Prettier |
| `pnpm lint` | oxlint |
| `pnpm test` | Vitest |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Full Rust tests (lib + integration). GitHub CI runs `--lib` only. |
| `just check` / `just test` / `just reset` / `just seed` | Wrappers |

## Environment

Copy `.env.example`. Vite variables:

- `VITE_START_VIEW`: `chat` \| `shelves` \| `recipes` \| `settings`
- `VITE_START_SHELF=first`: open the first shelf in Shelves view
- `VITE_START_ONBOARD=model`: first-run install step
- `VITE_START_SOURCE=first` / `VITE_START_THINKING=first` / `VITE_START_DOC=first`: open a citation, Thinking, or the first file
- `VITE_SNAPSHOT_PATH`: write a PNG of the window after start (debug Mac builds)

## Reset and seed

`./scripts/reset.sh` (macOS) or `./scripts/reset.ps1` (Windows) deletes Larra app data and related caches, and keeps `library/` (managed Shelf files). Settings → Reset Larra does the same from inside the running app (type `DELETE` to confirm). Older `~/Documents/Larra` folders, if any, also stay.

`cargo run --manifest-path src-tauri/Cargo.toml --example seed -- [--fresh] [--empty] [--model GGUF] [--ai-name "Muse Glimmer"]` fills a demo library. Harbor (seven files) and Notes (three), plus a list of conversations. `--empty` finishes first run with no Shelves. Seed refuses if Larra is open. `REBOST_FORCE_RECOMMEND=Muse Glimmer` makes first run offer that AI.

## Logging

- App: stdout + OS log dir (`~/Library/Logs/io.larra.desktop/` on macOS; `%LOCALAPPDATA%\io.larra.desktop\logs` on Windows)
- Engine: `logs/engine.log` under app data
- Default level: Info (`tauri-plugin-log`). Tantivy is Warn. The updater plugin is silent; a missing or private `latest.json` is not an error.
- macOS may print `Task policy set failed: 4 ((os/kern) invalid argument)` to the terminal. That is WebKit/AppKit, common when the process was started from a terminal, and it is safe to ignore.

## Toolchain

Pinned: Rust 1.98.0, pnpm 11.25.0, Node 22 (`.nvmrc`).
