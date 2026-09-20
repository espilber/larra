//! Custom About window. The desktop menu that opens it lives in `menu.rs`.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

use crate::commands::{friendly, CmdResult};

const ABOUT_LABEL: &str = "about";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    pub version: String,
    pub repository_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExternalLink {
    Repository,
}

impl ExternalLink {
    fn url(self) -> &'static str {
        match self {
            ExternalLink::Repository => env!("CARGO_PKG_REPOSITORY"),
        }
    }
}

/// Version and source URL for the About window.
#[tauri::command]
pub fn about_info(app: AppHandle) -> AboutInfo {
    AboutInfo {
        version: app.package_info().version.to_string(),
        repository_url: env!("CARGO_PKG_REPOSITORY").to_string(),
    }
}

/// Open or focus the About window.
#[tauri::command]
pub async fn show_about_window(app: AppHandle) -> CmdResult<()> {
    open(&app)
}

/// Open an allowlisted URL in the system browser.
#[tauri::command]
pub fn open_external(app: AppHandle, link: ExternalLink) -> CmdResult<()> {
    app.opener()
        .open_url(link.url(), None::<String>)
        .map_err(friendly)
}

/// Close About so the next locale can open a fresh window.
#[tauri::command]
pub fn close_about_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window(ABOUT_LABEL) {
        let _ = window.close();
    }
}

pub(crate) fn open(app: &AppHandle) -> CmdResult<()> {
    let park = crate::window::shot_park_enabled();
    if let Some(existing) = app.get_webview_window(ABOUT_LABEL) {
        let _ = existing.unminimize();
        let _ = existing.show();
        if park {
            crate::window::park_for_shots(&existing);
        } else {
            let _ = existing.set_focus();
        }
        return Ok(());
    }
    create(app, park).map_err(friendly)
}

fn create(app: &AppHandle, park: bool) -> tauri::Result<()> {
    let builder = WebviewWindowBuilder::new(
        app,
        ABOUT_LABEL,
        WebviewUrl::App("index.html?window=about".into()),
    )
    .title(rust_i18n::t!("about.windowTitle").to_string())
    .inner_size(400.0, 356.0)
    .min_inner_size(400.0, 356.0)
    .max_inner_size(400.0, 356.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .skip_taskbar(true)
    .accept_first_mouse(true)
    .focused(!park);
    let builder = if park { builder } else { builder.center() };

    #[cfg(windows)]
    let builder = builder.drag_and_drop(false);

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    let window = builder.build()?;
    if park {
        crate::window::park_for_shots(&window);
    }
    Ok(())
}
