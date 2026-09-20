# Run with `just <recipe>`. Install: https://github.com/casey/just

dev:
    pnpm tauri dev

build:
    pnpm tauri build

# Retina DMG background (src-tauri/dmg/background.png). --preview also writes preview.png.
dmg-background:
    swift src-tauri/dmg/render-background.swift
    just _compress-dmg-background

# Throwaway Finder window using the same layout as Tauri's bundle_dmg.sh.
dmg-preview:
    swift src-tauri/dmg/render-background.swift --preview
    just _compress-dmg-background
    ./src-tauri/dmg/preview-window.sh

[private]
_compress-dmg-background:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v pngquant >/dev/null; then
      pngquant --force --quality=82-95 --speed 1 --output src-tauri/dmg/background.png src-tauri/dmg/background.png
      sips -s dpiWidth 144 -s dpiHeight 144 src-tauri/dmg/background.png >/dev/null
    fi

# Signed + notarized DMG. Reads gitignored .env.signing.
release-macos:
    ./scripts/release-macos.sh

release-macos-intel:
    ./scripts/release-macos.sh x86_64-apple-darwin

# Signed NSIS. Needs Windows and gitignored .env.signing.
release-windows:
    pwsh scripts/release-windows.ps1

release-windows-arm:
    pwsh scripts/release-windows.ps1 -Target aarch64-pc-windows-msvc

fetch-engine *args:
    node scripts/fetch-engine.mjs {{args}}

# svelte-check, Prettier, oxlint, rustfmt, clippy (debug + release).
check:
    pnpm check
    pnpm format:check
    pnpm lint
    cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings
    cd src-tauri && cargo clippy --lib --tests --release -- -D warnings

test:
    cd src-tauri && cargo test
    pnpm test

# Full verification for broad changes and final release source; see CONTRIBUTING.md.
gate:
    just fmt
    just check
    just test

test-smoke:
    cd src-tauri && cargo test --test core_smoke -- --ignored --nocapture

seed *args:
    cargo run --manifest-path src-tauri/Cargo.toml --example seed -- {{args}}

reset:
    ./scripts/reset.sh

fmt:
    cd src-tauri && cargo fmt
    pnpm exec prettier --write "src/**/*.{ts,svelte,css}"
