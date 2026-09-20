# Releases

Signed installers are on [GitHub Releases](https://github.com/espilber/larra/releases). Installed copies check for a newer version on startup and offer an update when one exists. A failed check is ignored. The app-data layout may change without a migration.

## Version

The same version must appear in:

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml` (package `version` only)
- `src-tauri/Cargo.lock` (`name = "larra"`)

Current-version references in README, SECURITY, CHANGELOG, the bug template, and `docs/privacy.md` should match. Accessibility's "verified against" version changes only after that verification is performed. The HTTP user agent reads its version from `CARGO_PKG_VERSION`, so it follows the bump without an edit. Historical CHANGELOG sections stay as they were.

Finish source changes, synchronization, and tracked release documentation before building. Run the full verification gate on the final source. Every installer in a release must correspond to the published commit; rebuild after source or build-input changes. Record manual and hardware checks from `experience-quality.md` separately from automated checks.

## What a contributor can build

```bash
pnpm tauri build
```

That writes an unsigned DMG or NSIS for this machine. Gatekeeper will warn on macOS. Linux is not a supported platform. llama.cpp pins for Linux exist only so the crate compiles on Ubuntu CI; there is no Linux installer. Windows ARM64 NSIS is `pnpm tauri build --target aarch64-pc-windows-msvc` on ARM Windows, or the `windows-11-arm` job in `release-windows.yml`.

Signed installers need credentials that are not in this repository. See [signing.md](signing.md).

The in-app updater endpoint is `{Cargo.toml package.repository}/releases/latest/download/latest.json`. Change the repository URL in `src-tauri/Cargo.toml` if the GitHub repo moves; do not hardcode it elsewhere. Installed copies fetch that file without credentials, so the GitHub repository must be public or the check is ignored. `pnpm tauri dev` does not check.
