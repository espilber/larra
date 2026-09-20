# Bundled engine archive

Release builds copy **one** pinned llama.cpp archive into the installer (the host
this `tauri build` is targeting). Archives are not committed.

```bash
node scripts/fetch-engine.mjs                 # this machine
node scripts/fetch-engine.mjs --triple=x86_64-apple-darwin
node scripts/fetch-engine.mjs --all           # cache macOS + Windows
```

`pnpm tauri build` runs the fetch first. First chat unpacks the bundle and
SHA-256-checks it. GitHub is only used if this folder is empty (`tauri dev`
without a prior fetch).
