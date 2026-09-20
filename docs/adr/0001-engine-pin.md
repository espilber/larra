# ADR 0001: Pin llama.cpp instead of compiling it

- Status: accepted
- Date: 2026-08

Contributors should not need a llama.cpp toolchain. Larra pins an official llama.cpp `v*` release (`ENGINE_RELEASE`) and fetches the OS archives from the nightly tag that release names (`ENGINE_BUILD` + per-OS URL + SHA-256 in `engine/pin.rs`). It does not follow later nightlies. `llama-server` runs on loopback.

Release installers **bundle that archive** (one OS/arch per artifact) so first chat does not hit GitHub. The binary is still not compiled from source: official builds include shared libraries, so the pin is shipped as a resource and unpacked into app data, not as a single `externalBin` sidecar.

macOS uses Metal builds (arm64 and Intel, separate `.app`s). Windows x64 ships Vulkan and may download CUDA 12.4 at warmup when an NVIDIA driver is present. Windows ARM64 ships CPU and may download Adreno OpenCL on Snapdragon; GitHub Releases attach that NSIS from `release-windows.yml`. Linux pins are in the matrix so the crate compiles. Linux is not a supported platform.
