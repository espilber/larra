//! In-app updates. Checks GitHub `latest.json` on startup and stays quiet
//! when the network is down, the file is missing, the repo is private, or
//! this build is current. Debug builds skip the check: they are unsigned
//! and cannot apply a GitHub Release.
//!
//! The endpoint is `{Cargo.toml package.repository}/releases/latest/download/latest.json`.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::{friendly, CmdResult};
use crate::core::mutex_lock;

const WINDOW_LABEL: &str = "update";
const CHECK_TIMEOUT: Duration = Duration::from_secs(8);

pub struct PendingUpdate(pub Mutex<Option<StoredUpdate>>);

pub struct StoredUpdate {
    pub meta: UpdateMeta,
    pub update: Update,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMeta {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data")]
enum UpdateProgress {
    #[serde(rename = "started", rename_all = "camelCase")]
    Started { content_length: Option<u64> },
    #[serde(rename = "progress", rename_all = "camelCase")]
    Progress { chunk_length: usize },
    #[serde(rename = "finished")]
    Finished,
}

/// `{repository}/releases/latest/download/latest.json`, or `None` if the
/// Cargo.toml value is not an `https://` GitHub-style repo URL.
pub fn latest_json_url(repository: &str) -> Option<String> {
    let repo = repository
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git");
    if !repo.starts_with("https://") || repo.eq_ignore_ascii_case("https://") {
        return None;
    }
    Some(format!("{repo}/releases/latest/download/latest.json"))
}

/// Startup check. Never toasts, never warns; a failure is the same as "no update".
pub async fn check_silently(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let Some(raw) = latest_json_url(env!("CARGO_PKG_REPOSITORY")) else {
        return;
    };
    let Ok(endpoint) = raw.parse() else {
        return;
    };
    let Ok(builder) = app
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .endpoints(vec![endpoint])
    else {
        return;
    };
    let Ok(updater) = builder.build() else {
        return;
    };
    let Ok(Some(update)) = updater.check().await else {
        return;
    };

    let notes = update
        .body
        .as_ref()
        .map(|body| body.trim().to_string())
        .filter(|body| !body.is_empty());
    let meta = UpdateMeta {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes,
    };
    {
        let pending = app.state::<PendingUpdate>();
        *mutex_lock(&pending.0) = Some(StoredUpdate {
            meta: meta.clone(),
            update,
        });
    }
    let _ = app.emit("larra://update", meta);
}

/// Pending in-app update metadata, if any.
#[tauri::command]
pub fn update_info(pending: State<'_, PendingUpdate>) -> Option<UpdateMeta> {
    mutex_lock(&pending.0)
        .as_ref()
        .map(|stored| stored.meta.clone())
}

/// Download and install the pending update.
#[tauri::command]
pub async fn install_update(app: AppHandle, pending: State<'_, PendingUpdate>) -> CmdResult<()> {
    let update = mutex_lock(&pending.0)
        .as_ref()
        .map(|stored| stored.update.clone())
        .ok_or_else(|| "No update is waiting.".to_string())?;

    let chunk_app = app.clone();
    let done_app = app.clone();
    let mut started = false;
    update
        .download_and_install(
            move |chunk_length, content_length| {
                if !started {
                    started = true;
                    let _ = chunk_app.emit(
                        "larra://update-progress",
                        UpdateProgress::Started { content_length },
                    );
                }
                let _ = chunk_app.emit(
                    "larra://update-progress",
                    UpdateProgress::Progress { chunk_length },
                );
            },
            move || {
                let _ = done_app.emit("larra://update-progress", UpdateProgress::Finished);
            },
        )
        .await
        .map_err(|error| {
            log::error!("update install: {error}");
            "The update couldn't be installed. Try again.".to_string()
        })?;

    #[cfg(not(windows))]
    {
        if let Ok(data_dir) = app.path().app_data_dir() {
            let _ = crate::instance::mark_relaunch(&data_dir);
        }
        app.restart();
    }

    #[cfg(windows)]
    {
        Ok(())
    }
}

/// Open or focus the update window.
#[tauri::command]
pub async fn show_update_window(app: AppHandle) -> CmdResult<()> {
    open(&app)
}

fn open(app: &AppHandle) -> CmdResult<()> {
    if let Some(existing) = app.get_webview_window(WINDOW_LABEL) {
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }
    create(app).map_err(friendly)
}

fn create(app: &AppHandle) -> tauri::Result<()> {
    let builder = WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        WebviewUrl::App("index.html?window=update".into()),
    )
    .title(rust_i18n::t!("update.windowTitle").to_string())
    .inner_size(400.0, 440.0)
    .min_inner_size(400.0, 440.0)
    .max_inner_size(400.0, 440.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .skip_taskbar(true)
    .accept_first_mouse(true)
    .center()
    .focused(true);

    #[cfg(windows)]
    let builder = builder.drag_and_drop(false);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    builder.build()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_json_follows_cargo_repository() {
        let repo = env!("CARGO_PKG_REPOSITORY");
        let url = latest_json_url(repo).expect("Cargo.toml repository should be an https URL");
        let stripped = repo.trim().trim_end_matches('/').trim_end_matches(".git");
        assert_eq!(
            url,
            format!("{stripped}/releases/latest/download/latest.json")
        );
        assert!(url.starts_with("https://"));
    }

    #[test]
    fn latest_json_strips_git_suffix_and_slash() {
        assert_eq!(
            latest_json_url("https://github.com/acme/larra.git/").as_deref(),
            Some("https://github.com/acme/larra/releases/latest/download/latest.json")
        );
        assert_eq!(
            latest_json_url("  https://github.com/acme/larra  ").as_deref(),
            Some("https://github.com/acme/larra/releases/latest/download/latest.json")
        );
    }

    #[test]
    fn latest_json_rejects_non_https() {
        assert_eq!(latest_json_url("http://github.com/acme/larra"), None);
        assert_eq!(latest_json_url(""), None);
        assert_eq!(latest_json_url("   "), None);
        assert_eq!(latest_json_url("git@github.com:acme/larra.git"), None);
    }
}
