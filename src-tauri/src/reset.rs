//! Wipe Larra app data back to first-run.
//!
//! The running app holds file handles (Tantivy, the engine, logs), so the
//! command only writes a marker and restarts. The wipe runs at process
//! start, before Tauri creates the window — `.setup()` is too late, because
//! Tauri builds the webview first and deleting `~/Library/WebKit/<id>` then
//! leaves a white window. User Shelf files in `library/` (and any folder
//! outside app data) are not deleted.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Confirmation the UI requires before invoking the reset command.
pub const CONFIRMATION: &str = "DELETE";

pub const MARKER_NAME: &str = ".reset-pending";

/// Same as `tauri.conf.json` `identifier`. Needed before an `AppHandle` exists.
/// Last component must not be a macOS package extension (`.app`, `.bundle`):
/// Library folders use this name, and a `.app` suffix makes them look like apps.
pub const BUNDLE_IDENTIFIER: &str = "io.larra.desktop";

pub fn marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MARKER_NAME)
}

pub fn mark_pending(data_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(marker_path(data_dir), b"")
}

pub fn is_pending(data_dir: &Path) -> bool {
    marker_path(data_dir).is_file()
}

/// App-data directory Tauri would resolve for this bundle id.
pub fn app_data_dir(identifier: &str) -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join(identifier))
}

fn app_log_dir(identifier: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|dir| dir.join("Library/Logs").join(identifier))
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_local_dir().map(|dir| dir.join(identifier).join("logs"))
    }
}

/// Cache, logs, and OS-specific extras next to the app-data directory.
/// Extra paths are deleted only when they contain the bundle identifier.
pub fn extra_paths(
    identifier: &str,
    cache: Option<PathBuf>,
    logs: Option<PathBuf>,
    local_data: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut extras = Vec::new();
    extras.extend(cache);
    extras.extend(logs);
    extras.extend(local_data);
    extras.extend(platform_extras(identifier));
    extras
}

pub fn extra_paths_for_identifier(identifier: &str) -> Vec<PathBuf> {
    extra_paths(
        identifier,
        dirs::cache_dir().map(|dir| dir.join(identifier)),
        app_log_dir(identifier),
        dirs::data_local_dir().map(|dir| dir.join(identifier)),
    )
}

/// Wipe a pending reset before any window exists.
pub fn apply_before_launch(identifier: &str) -> std::io::Result<bool> {
    let Some(data_dir) = app_data_dir(identifier) else {
        return Ok(false);
    };
    apply_pending(
        &data_dir,
        &extra_paths_for_identifier(identifier),
        identifier,
    )
}

/// In `tauri dev`, `request_restart()` spawns a sibling binary and exits,
/// which detaches from the CLI and often leaves a blank window. Replace
/// this process so the parent `pnpm tauri dev` session stays attached.
#[cfg(debug_assertions)]
pub fn relaunch_current_exe() -> ! {
    let exe = std::env::current_exe().expect("current executable");
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(exe).args(args).exec();
        panic!("failed to relaunch Larra: {error}");
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new(&exe).args(&args).spawn();
        std::process::exit(0);
    }
}

#[cfg(target_os = "macos")]
fn platform_extras(identifier: &str) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![
        home.join("Library/WebKit").join(identifier),
        home.join("Library/Preferences")
            .join(format!("{identifier}.plist")),
        home.join("Library/Saved Application State")
            .join(format!("{identifier}.savedState")),
    ]
}

#[cfg(not(target_os = "macos"))]
fn platform_extras(_identifier: &str) -> Vec<PathBuf> {
    Vec::new()
}

/// True when a path looks like Larra app state, not a user folder.
pub fn is_safe_reset_target(path: &Path, identifier: &str) -> bool {
    if identifier.is_empty() || path.parent().is_none() {
        return false;
    }
    path.components()
        .any(|c| c.as_os_str().to_string_lossy().contains(identifier))
}

/// Names under app data that Reset leaves in place.
const KEEP_ON_RESET: &[&str] = &[
    MARKER_NAME,
    crate::instance::LOCK_NAME,
    crate::paths::LIBRARY_DIR,
];

/// Delete app-data contents except user Shelf files in `library/`.
pub fn wipe_app_data_contents(data_dir: &Path) -> std::io::Result<()> {
    wipe_dir_contents(data_dir, KEEP_ON_RESET)
}

/// If a reset was requested on the previous run, delete app data and extras.
/// Returns whether a wipe ran.
pub fn apply_pending(
    data_dir: &Path,
    extras: &[PathBuf],
    identifier: &str,
) -> std::io::Result<bool> {
    if !is_pending(data_dir) {
        return Ok(false);
    }
    wipe_app_data_contents(data_dir)?;
    for extra in extras {
        if extra == data_dir || extra.starts_with(data_dir) {
            continue;
        }
        if !is_safe_reset_target(extra, identifier) {
            log::warn!("skipping reset path {}", extra.display());
            continue;
        }
        if let Err(error) = remove_path_retry(extra) {
            log::warn!("could not remove {}: {error}", extra.display());
        }
    }
    let marker = marker_path(data_dir);
    if marker.exists() {
        std::fs::remove_file(&marker)?;
    }
    Ok(true)
}

fn wipe_dir_contents(dir: &Path, keep_names: &[&str]) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if keep_names.iter().any(|keep| name == *keep) {
            continue;
        }
        remove_path_retry(&entry.path())?;
    }
    Ok(())
}

fn remove_path_retry(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let mut last_error = None;
    for attempt in 0..10 {
        match remove_path(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(40 * (attempt + 1)));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("reset path still in use")))
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || meta.is_file() {
        std::fs::remove_file(path)
    } else {
        std::fs::remove_dir_all(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated_data_dir() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("appdata");
        std::fs::create_dir_all(data.join("models")).unwrap();
        std::fs::write(data.join("settings.json"), b"{\"onboardingDone\":true}").unwrap();
        std::fs::write(data.join("models").join("model.gguf"), b"gguf").unwrap();
        (root, data)
    }

    #[test]
    fn apply_pending_is_noop_without_marker() {
        let (_root, data) = populated_data_dir();
        assert!(!apply_pending(&data, &[], BUNDLE_IDENTIFIER).unwrap());
        assert!(data.join("settings.json").is_file());
    }

    #[test]
    fn apply_pending_wipes_app_data_and_removes_marker() {
        let (_root, data) = populated_data_dir();
        mark_pending(&data).unwrap();
        assert!(apply_pending(&data, &[], BUNDLE_IDENTIFIER).unwrap());
        assert!(!data.join("settings.json").exists());
        assert!(!data.join("models").exists());
        assert!(!marker_path(&data).exists());
        assert!(data.is_dir());
    }

    #[test]
    fn apply_pending_keeps_the_instance_lock_file() {
        let (_root, data) = populated_data_dir();
        std::fs::write(data.join(crate::instance::LOCK_NAME), b"").unwrap();
        mark_pending(&data).unwrap();
        assert!(apply_pending(&data, &[], BUNDLE_IDENTIFIER).unwrap());
        assert!(data.join(crate::instance::LOCK_NAME).is_file());
        assert!(!data.join("settings.json").exists());
    }

    #[test]
    fn apply_pending_leaves_files_outside_app_data() {
        let (root, data) = populated_data_dir();
        let documents = root.path().join("Documents").join("Larra").join("Notes");
        std::fs::create_dir_all(&documents).unwrap();
        let kept = documents.join("letter.txt");
        std::fs::write(&kept, b"keep me").unwrap();
        mark_pending(&data).unwrap();
        apply_pending(&data, &[], BUNDLE_IDENTIFIER).unwrap();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "keep me");
    }

    #[test]
    fn apply_pending_keeps_library_shelf_files() {
        let (_root, data) = populated_data_dir();
        let note = data
            .join(crate::paths::LIBRARY_DIR)
            .join("Notes")
            .join("letter.txt");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::write(&note, b"keep me").unwrap();
        mark_pending(&data).unwrap();
        apply_pending(&data, &[], BUNDLE_IDENTIFIER).unwrap();
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "keep me");
        assert!(!data.join("settings.json").exists());
    }

    #[test]
    fn extras_with_identifier_are_removed() {
        let (_root, data) = populated_data_dir();
        let cache_root = tempfile::tempdir().unwrap();
        let cache = cache_root.path().join(BUNDLE_IDENTIFIER);
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("blob"), b"x").unwrap();
        mark_pending(&data).unwrap();
        apply_pending(&data, std::slice::from_ref(&cache), BUNDLE_IDENTIFIER).unwrap();
        assert!(!cache.exists());
    }

    #[test]
    fn extras_without_identifier_are_left_alone() {
        let (_root, data) = populated_data_dir();
        let other_root = tempfile::tempdir().unwrap();
        let other = other_root.path().join("Documents");
        std::fs::create_dir_all(&other).unwrap();
        let kept = other.join("photo.jpg");
        std::fs::write(&kept, b"img").unwrap();
        mark_pending(&data).unwrap();
        apply_pending(&data, std::slice::from_ref(&other), BUNDLE_IDENTIFIER).unwrap();
        assert!(kept.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_app_data_is_unlinked_not_followed() {
        let (_root, data) = populated_data_dir();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real-file");
        std::fs::write(&target, b"untouched").unwrap();
        let link = data.join("sneaky");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        mark_pending(&data).unwrap();
        apply_pending(&data, &[], BUNDLE_IDENTIFIER).unwrap();
        assert!(!link.exists());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
    }

    #[test]
    fn confirmation_is_delete() {
        assert_eq!(CONFIRMATION, "DELETE");
    }

    #[test]
    fn app_data_dir_joins_the_bundle_id() {
        let dir = app_data_dir(BUNDLE_IDENTIFIER).expect("data dir");
        assert_eq!(dir.file_name().unwrap(), BUNDLE_IDENTIFIER);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn extras_include_webkit_and_logs() {
        let extras = extra_paths_for_identifier(BUNDLE_IDENTIFIER);
        assert!(
            extras
                .iter()
                .any(|path| path.ends_with(Path::new("WebKit").join(BUNDLE_IDENTIFIER))),
            "missing WebKit extra: {extras:?}"
        );
        assert!(
            extras
                .iter()
                .any(|path| path.ends_with(Path::new("Logs").join(BUNDLE_IDENTIFIER))),
            "missing Logs extra: {extras:?}"
        );
    }

    #[test]
    fn unsafe_root_paths_are_rejected() {
        assert!(!is_safe_reset_target(Path::new("/"), BUNDLE_IDENTIFIER));
        assert!(!is_safe_reset_target(
            Path::new("/Users/someone/Documents"),
            BUNDLE_IDENTIFIER
        ));
        assert!(is_safe_reset_target(
            &Path::new("/Users/someone/Library/Application Support").join(BUNDLE_IDENTIFIER),
            BUNDLE_IDENTIFIER
        ));
    }

    #[test]
    fn bundle_identifier_matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(conf["identifier"].as_str(), Some(BUNDLE_IDENTIFIER));
    }

    #[test]
    fn bundle_identifier_is_not_a_macos_package_name() {
        let last = BUNDLE_IDENTIFIER.rsplit('.').next().unwrap_or("");
        assert!(
            !matches!(
                last,
                "app" | "bundle" | "framework" | "plugin" | "kext" | "xpc"
            ),
            "{BUNDLE_IDENTIFIER} ends with a package extension"
        );
    }
}
