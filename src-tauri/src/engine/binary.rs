//! Download, verify, and unpack the pinned llama.cpp archive.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

use super::download;
use super::pin::{
    current_engine_pin, extract_dir_name, llama_server_file_name, preferred_engine_pin,
    runtime_for, EnginePin, ENGINE_RELEASE,
};
use super::process::unpack_engine_archive;
use super::{Engine, EngineState};

struct ArchiveDownload<'a> {
    control: download::DownloadControl,
    events: &'a std::sync::Arc<dyn crate::core::Events>,
    finished: bool,
}
impl Drop for ArchiveDownload<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.control.request_cancel();
            self.events.emit("larra://download", serde_json::json!({
                "kind": "engine", "id": "engine", "name": "AI engine", "done": true, "error": "cancelled"
            }));
        }
    }
}

impl Engine {
    fn engine_binary(&self, pin: &EnginePin) -> PathBuf {
        let tagged = self
            .ctx
            .paths
            .engine_dir()
            .join(extract_dir_name(pin))
            .join(llama_server_file_name());
        if tagged.exists() {
            return tagged;
        }
        let bundled_accel = current_engine_pin()
            .map(|bundled| bundled.accelerator)
            .unwrap_or("");
        if bundled_accel == pin.accelerator {
            let legacy = self
                .ctx
                .paths
                .engine_dir()
                .join(ENGINE_RELEASE)
                .join(llama_server_file_name());
            if legacy.exists() {
                return legacy;
            }
        }
        tagged
    }

    fn pin_is_ready(&self, pin: &EnginePin) -> bool {
        let binary = self.engine_binary(pin);
        if !binary.exists() {
            return false;
        }
        match runtime_for(pin) {
            Some(runtime) => binary
                .parent()
                .map(|dir| dir.join(runtime.sidecar).is_file())
                .unwrap_or(false),
            None => true,
        }
    }

    /// Preferred GPU build when the hardware is there; otherwise the installer pin.
    /// Callers fall back to the bundled pin if this download or spawn fails.
    pub async fn ensure_binary(&self) -> Result<(PathBuf, &'static EnginePin)> {
        let bundled = current_engine_pin()?;
        if self
            .skip_optional
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let path = self.ensure_pin_binary(bundled, true).await?;
            return Ok((path, bundled));
        }
        let preferred = preferred_engine_pin()?;
        if !std::ptr::eq(preferred, bundled) {
            match self.ensure_pin_binary(preferred, false).await {
                Ok(path) => return Ok((path, preferred)),
                Err(error) => {
                    self.skip_optional
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    log::warn!(
                        "optional {} engine unavailable ({error:#}); using bundled {}",
                        preferred.accelerator,
                        bundled.accelerator
                    );
                }
            }
        }
        let path = self.ensure_pin_binary(bundled, true).await?;
        Ok((path, bundled))
    }

    /// Make sure one llama.cpp build is present. Preference: already extracted,
    /// `REBOST_ENGINE_ARCHIVE` / the installer bundle (bundled pin only), then GitHub.
    pub(super) async fn ensure_pin_binary(
        &self,
        pin: &'static EnginePin,
        allow_local: bool,
    ) -> Result<PathBuf> {
        if self.pin_is_ready(pin) {
            return Ok(self.engine_binary(pin));
        }
        self.set_status(EngineState::Starting, Some("Preparing the AI…".into()));
        let engine_dir = self.ctx.paths.engine_dir();
        std::fs::create_dir_all(&engine_dir)?;
        let target = engine_dir.join(extract_dir_name(pin));

        if !self.engine_binary(pin).exists() {
            self.unpack_server_archive(pin, allow_local, &engine_dir, &target)
                .await?;
        }
        if let Some(runtime) = runtime_for(pin) {
            if !self.pin_is_ready(pin) {
                self.unpack_runtime(runtime, &engine_dir, &target).await?;
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if target.is_dir() {
                for entry in std::fs::read_dir(&target)?.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let _ =
                            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
                    }
                }
            }
        }

        let binary = self.engine_binary(pin);
        if !self.pin_is_ready(pin) {
            return Err(anyhow!("engine archive did not contain llama-server"));
        }
        log::info!(
            "AI engine {ENGINE_RELEASE} {} is ready",
            pin.accelerator.to_ascii_lowercase()
        );
        Ok(binary)
    }

    async fn unpack_server_archive(
        &self,
        pin: &EnginePin,
        allow_local: bool,
        engine_dir: &Path,
        target: &Path,
    ) -> Result<()> {
        let download_dest = engine_dir.join(pin.file_name);
        let env_archive = std::env::var_os("REBOST_ENGINE_ARCHIVE").filter(|p| !p.is_empty());
        let bundled = self.ctx.paths.bundled_engine_archive();

        let (archive, verify_pin_sha, delete_after_unpack) = if allow_local {
            if let Some(local) = resolve_local_archive(env_archive, bundled) {
                log::info!(
                    "unpacking AI engine {ENGINE_RELEASE} from {}",
                    local.path.display()
                );
                (local.path, local.verify_pin_sha, local.delete_after_unpack)
            } else {
                self.download_archive(
                    pin.url,
                    &download_dest,
                    pin.sha256,
                    pin.file_name,
                    "AI engine",
                )
                .await?;
                (download_dest, false, true)
            }
        } else {
            log::info!(
                "downloading optional {} engine {ENGINE_RELEASE} ({})",
                pin.accelerator,
                pin.file_name
            );
            self.download_archive(
                pin.url,
                &download_dest,
                pin.sha256,
                pin.file_name,
                "AI engine",
            )
            .await?;
            (download_dest, false, true)
        };

        if verify_pin_sha {
            download::verify_sha256_blocking(&archive, pin.sha256)
                .context("engine archive SHA-256")?;
        }

        let extract_dir = engine_dir.join(format!(
            "extract-tmp-{}",
            pin.accelerator.to_ascii_lowercase()
        ));
        let _ = std::fs::remove_dir_all(&extract_dir);
        std::fs::create_dir_all(&extract_dir)?;
        unpack_engine_archive(&archive, &extract_dir)?;
        let found = super::process::find_llama_server(&extract_dir)
            .ok_or_else(|| anyhow!("engine archive did not contain llama-server"))?;
        let home = found
            .parent()
            .ok_or_else(|| anyhow!("engine binary has no parent directory"))?;
        let _ = std::fs::remove_dir_all(target);
        if home == extract_dir.as_path() {
            std::fs::rename(&extract_dir, target)?;
        } else {
            std::fs::rename(home, target)?;
            let _ = std::fs::remove_dir_all(&extract_dir);
        }
        if delete_after_unpack {
            let _ = std::fs::remove_file(&archive);
        }
        Ok(())
    }

    async fn unpack_runtime(
        &self,
        runtime: &super::pin::EngineRuntimePin,
        engine_dir: &Path,
        target: &Path,
    ) -> Result<()> {
        let dest = engine_dir.join(runtime.file_name);
        log::info!("downloading CUDA runtime ({})", runtime.file_name);
        self.download_archive(
            runtime.url,
            &dest,
            runtime.sha256,
            runtime.file_name,
            "AI engine",
        )
        .await?;
        let extract_dir = engine_dir.join("extract-tmp-runtime");
        let _ = std::fs::remove_dir_all(&extract_dir);
        std::fs::create_dir_all(&extract_dir)?;
        unpack_engine_archive(&dest, &extract_dir)?;
        std::fs::create_dir_all(target)?;
        copy_dir_contents(&extract_dir, target)?;
        let _ = std::fs::remove_dir_all(&extract_dir);
        let _ = std::fs::remove_file(&dest);
        if !target.join(runtime.sidecar).is_file() {
            return Err(anyhow!(
                "CUDA runtime archive did not contain {}",
                runtime.sidecar
            ));
        }
        Ok(())
    }

    async fn download_archive(
        &self,
        url: &str,
        dest: &Path,
        sha256: &str,
        file_name: &str,
        name: &str,
    ) -> Result<()> {
        log::info!("downloading AI engine {ENGINE_RELEASE} ({file_name})");
        let mut transfer = ArchiveDownload {
            control: download::DownloadControl::new(),
            events: &self.ctx.events,
            finished: false,
        };
        let result = download::download(
            &self.download_client,
            url,
            dest,
            &download::DownloadTicket {
                kind: "engine",
                id: "engine".into(),
                name: name.into(),
            },
            Some(sha256),
            None,
            &self.ctx.events.clone(),
            &transfer.control,
        )
        .await;
        transfer.finished = true;
        result
    }
}

fn copy_dir_contents(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_contents(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Local archive to unpack: env override, then the installer bundle.
/// GitHub is only the fallback when neither is present. Never copies the
/// bundle into app data — unpack from the source path.
struct LocalArchive {
    path: PathBuf,
    verify_pin_sha: bool,
    delete_after_unpack: bool,
}

fn resolve_local_archive(
    env_archive: Option<std::ffi::OsString>,
    bundled: Option<&Path>,
) -> Option<LocalArchive> {
    if let Some(path) = env_archive.filter(|p| !p.is_empty()) {
        return Some(LocalArchive {
            path: PathBuf::from(path),
            verify_pin_sha: true,
            delete_after_unpack: false,
        });
    }
    bundled
        .filter(|path| path.is_file())
        .map(|path| LocalArchive {
            path: path.to_path_buf(),
            // Notary unpacks this tar.gz, so signed Mac builds re-sign Mach-O
            // inside it and the bytes no longer match the GitHub pin SHA.
            verify_pin_sha: false,
            delete_after_unpack: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_archive_wins_over_bundle() {
        let bundled = PathBuf::from("/bundle/engine.tar.gz");
        let found = resolve_local_archive(
            Some(std::ffi::OsString::from("/tmp/test-engine.tar.gz")),
            Some(&bundled),
        )
        .unwrap();
        assert_eq!(found.path, PathBuf::from("/tmp/test-engine.tar.gz"));
        assert!(found.verify_pin_sha);
        assert!(!found.delete_after_unpack);
    }

    #[test]
    fn bundle_is_unpacked_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("engine.tar.gz");
        std::fs::write(&archive, b"x").unwrap();
        let found = resolve_local_archive(None, Some(&archive)).unwrap();
        assert_eq!(found.path, archive);
        assert!(!found.verify_pin_sha);
        assert!(!found.delete_after_unpack);
        assert!(resolve_local_archive(None, None).is_none());
        assert!(resolve_local_archive(Some(std::ffi::OsString::new()), Some(&archive)).is_some());
    }

    #[test]
    fn pin_file_name_is_stable() {
        let pin = current_engine_pin().unwrap();
        assert!(!pin.file_name.is_empty());
    }

    #[test]
    fn copy_dir_contents_merges_sidecars() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("cudart64_12.dll"), b"rt").unwrap();
        std::fs::write(dest.path().join("llama-server"), b"bin").unwrap();
        copy_dir_contents(src.path(), dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("cudart64_12.dll")).unwrap(),
            b"rt"
        );
        assert_eq!(
            std::fs::read(dest.path().join("llama-server")).unwrap(),
            b"bin"
        );
    }
}
