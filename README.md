# Larra

Private AI that works with your files. What happens on your computer stays on your computer.

Larra is a desktop application for Mac and Windows, forked from
[Rebost](https://github.com/Frontierz-AI/Rebost). It runs an AI on the machine
where it is installed and answers questions about documents kept there — with
citations that link every answer back to the file it drew on.

Questions go in **Chat**. Documents reach the AI through a **Shelf**, a folder
on the same machine that a conversation can be pointed at. A **Recipe** is a
prompt saved for reuse, and **House rules** are standing instructions that apply
to every conversation.

The first launch offers an AI sized for the machine's memory and downloads it.
The application is free software, AGPL-3.0-or-later, and works without an
account. Documents on a Shelf are not uploaded.

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)

## What Larra changes from Rebost

Larra keeps everything Rebost 0.9.3 shipped and is being extended with:

- **Local endpoints**: point the app at an OpenAI-compatible server on the same
  machine (LM Studio, Ollama, llama.cpp) instead of installing a model.
- **Office editing**: an embedded editing environment built from ONLYOFFICE,
  downloaded on first use, wired to the Shelves so edits are re-indexed and
  citations stay current.
- **Multimodal chat**: Office files are already searchable today; images, audio
  and video follow, model-permitting.

See [CHANGELOG.md](CHANGELOG.md) and [NOTICE.md](NOTICE.md).

## Status

Pre-release. The renaming (Rebost → Larra) is applied end to end; the
extension work listed above is in progress. Upstream Rebost remains the
reference for stability: [Frontierz-AI/Rebost](https://github.com/Frontierz-AI/Rebost).

## Build

Requirements: macOS (Apple Silicon or Intel) or Windows 10/11, Rust 1.98.0,
Node 22+, pnpm 11.

```
pnpm install
pnpm tauri dev
```

Checks:

```
pnpm check          # svelte-check
cargo test --manifest-path src-tauri/Cargo.toml
pnpm test           # Vitest
```

Full developer docs: [CONTRIBUTING.md](CONTRIBUTING.md),
[docs/development.md](docs/development.md), [docs/architecture.md](docs/architecture.md).

## License

[AGPL-3.0-or-later](LICENSE) for Larra as a whole. Code inherited from Rebost
remains MIT — see [NOTICE.md](NOTICE.md) and
[docs/licensing.md](docs/licensing.md). Trademark: [docs/branding.md](docs/branding.md).
