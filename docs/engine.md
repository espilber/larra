# Local engine

The language model is llama.cpp `llama-server`, managed as a child process. It is not a Tauri `externalBin` sidecar: official builds ship dylibs (macOS) and DLLs (Windows), so Larra bundles the **pinned archive** as a resource and unpacks it into app data.

Each installer contains **one** archive for that OS/arch. There is no universal Mac or Windows binary. llama.cpp does not ship fat libraries, and a universal `.app` would carry two engines.

## Pin

Larra follows official llama.cpp `v*` releases, not nightlies. The `v*` tag does not attach OS archives; those live on the nightly tag named in that release (0.3.0 names `b10621`).

`src-tauri/src/engine/pin.rs`:

- `ENGINE_RELEASE` (e.g. `0.3.0`) — official semver; Settings diagnostics and extract folders use this
- `ENGINE_BUILD` (e.g. `b10621`) — GitHub tag that hosts the archives
- `ENGINE_PINS`: one GitHub archive URL + SHA-256 per OS/arch (what the installer ships)
- `ENGINE_OPTIONAL_PINS`: faster GPU archives downloaded at first warmup when the hardware matches. Not bundled.

| Host | Archive | Accelerator | Typical archive size |
|------|---------|-------------|----------------------|
| macOS arm64 | `macos-arm64.tar.gz` | Metal | ~11 MB |
| macOS x64 | `macos-x64.tar.gz` | Metal | ~11 MB |
| Windows x64 | `win-vulkan-x64.zip` | Vulkan | ~33 MB |
| Windows arm64 | `win-cpu-arm64.zip` | CPU | ~12 MB |
| Linux x64 / arm64 | `ubuntu-vulkan-*.tar.gz` | Vulkan (not a supported platform; pin exists so the crate compiles on Ubuntu CI). There is no Linux installer overlay. | ~26–32 MB |

Optional downloads (app data, not the installer):

| Host | When | Archive | Extra |
|------|------|---------|-------|
| Windows x64 | NVIDIA driver (`nvcuda.dll`) | `win-cuda-12.4-x64.zip` (~250 MB) | `cudart-llama-bin-win-cuda-12.4-x64.zip` (~370 MB), because llama.cpp does not ship `cudart`/`cublas` in the CUDA zip |
| Windows arm64 | Snapdragon / Adreno | `win-opencl-adreno-arm64.zip` (~13 MB) | — |

CUDA 12.4 rather than 13.x so older NVIDIA GPUs still load. If the optional download fails, or llama-server **exits or never becomes healthy** (missing DLL, wrong GPU, hung driver), Larra uses the bundled Vulkan or CPU build and skips the optional pin for the rest of that session. A slow load of the *bundled* pin is still a timeout, not a reason to fetch CUDA.

CI HEAD-checks every URL. `node scripts/fetch-engine.mjs` downloads the **bundled** pin for this build target into `src-tauri/resources/engine/` (gitignored). `pnpm tauri build` runs that fetch first. `--all` fills the cache with macOS + Windows installer pins only; it does not pull CUDA.

Cross-compile:

```bash
node scripts/fetch-engine.mjs --triple=x86_64-apple-darwin
pnpm tauri build --target x86_64-apple-darwin
```

Signed Mac installers **codesign every Mach-O inside the archive** (notary unpacks `.tar.gz`), then notarize. llama.cpp still unpacks on first chat; it is not a Tauri sidecar.

Windows x64 needs a working Vulkan driver for the bundled pin. Windows arm64 ships CPU and can add Adreno OpenCL after warmup. GitHub Releases attach both NSIS installers from `release-windows.yml` (x64 on `windows-latest`, ARM64 on `windows-11-arm`). An x64 copy running on Windows ARM stays on CPU (`-ngl 0`) so Adreno Vulkan is not used.

## Flags

Spawn flags follow the machine and the loaded file (`src-tauri/src/engine/tune.rs`): context 4k–16k (GGUF `context_length` is a ceiling; never below 4k; no 32k). 16k only on Metal with enough unified memory. Answer cap 768–2,048, batch/ubatch, `--cache-type-k/v q8_0` (OpenCL uses `f16`; q8_0 KV crashes Adreno), and `-fa on` (Metal/Vulkan/CUDA) or `-fa auto` (CPU/OpenCL). CPU and the x64-on-ARM Vulkan copy use `-ngl 0`. `--no-mmap` only on discrete Vulkan and CUDA. CPU and OpenCL stay at 4k–6k. Vulkan/CUDA stay at 4k–8k. OpenCL is probed with a tiny completion after `/health`; a hang or empty reply falls back to the bundled CPU pin.

Extracted binaries live in `engine/<release>-<accelerator>/` (for example `0.3.0-metal`, `0.3.0-cuda`). An older `engine/<release>/` folder is still used for the bundled pin.

## First run

1. If `engine/<release>-<accelerator>/llama-server` (`.exe` on Windows) exists, use it. CUDA pins also need the `cudart64_12.dll` sidecar present. The bundled pin also accepts the older `engine/<release>/` layout.
2. Else if the host matches an optional GPU pin, download that archive (and the CUDA runtime zip on NVIDIA Windows).
3. Else if `REBOST_ENGINE_ARCHIVE` is set, unpack that archive and SHA-256-verify it (tests / air-gapped). Used for the bundled pin only.
4. Else if the installer bundled the pin, unpack it in place. Signed Mac builds re-sign Mach-O inside it for notarization, so that archive is **not** checked against the GitHub pin SHA.
5. Else download the host pin and verify SHA-256 (`tauri dev` without a prior fetch).
6. Unpack tar.gz or zip (path-safe), chmod `0755` on Unix, spawn on `127.0.0.1`. Working directory is the engine folder so Windows DLLs load.

Logs live under the OS app-data directory (`logs/engine.log`). Settings → Diagnostics shows the path, not the log body.

## Offline

A release build already contains the bundled archive. For tests, point `REBOST_ENGINE_ARCHIVE` at a copy of the **host** pin (same SHA). Models still have to be present under `models/`. Optional CUDA/OpenCL still need a network the first time, or they stay on the bundled pin.

## Warmup

`warm_engine` and Chat both call `ensure_ready`. The UI shows "Warming up..." while weights load. Closing Larra kills the process (and leftover PIDs from crashed dev builds).

Calibration is tied to the model file, engine build, active accelerator, and context/batch settings. Older or mismatched measurements are discarded when the engine starts. The next idle pause after an answer measures again; Settings → Diagnostics → Measure speed again reruns it on demand. Chat interrupts calibration, and failed measurements do not replace a valid result.
