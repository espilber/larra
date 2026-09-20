//! Debug-only window snapshot for website screenshots.

#[cfg(all(debug_assertions, target_os = "macos"))]
use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
#[cfg(all(debug_assertions, target_os = "macos"))]
use tauri::Manager;

#[cfg(all(debug_assertions, target_os = "macos"))]
use crate::commands::friendly;
use crate::commands::CmdResult;

/// One marketing frame. The capture script writes `job.json`; the UI applies it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotJob {
    pub id: String,
    pub path: String,
    pub locale: Option<String>,
    pub onboarding: Option<bool>,
    pub onboard: Option<String>,
    pub onboard_more: Option<bool>,
    pub view: Option<String>,
    pub thread: Option<u32>,
    pub source: Option<bool>,
    pub thinking: Option<bool>,
    pub shelf: Option<String>,
    pub doc: Option<String>,
    pub recipe: Option<String>,
    pub explore: Option<bool>,
    pub about: Option<bool>,
    pub label: Option<String>,
    pub settle_ms: Option<u64>,
}

pub fn shot_control_enabled() -> bool {
    matches!(std::env::var("REBOST_SHOT_CONTROL").as_deref(), Ok("1"))
}

fn shot_dir() -> std::path::PathBuf {
    std::env::var("REBOST_SHOT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/larra-shots"))
}

/// Tell the capture script the webview is ready for jobs.
#[tauri::command]
pub fn dev_shot_ready() -> CmdResult<()> {
    let dir = shot_dir();
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    std::fs::write(dir.join("ready"), "1").map_err(|error| error.to_string())
}

/// Surface a failed job so the runner can stop instead of waiting on a PNG.
#[tauri::command]
pub fn dev_shot_fail(message: String) -> CmdResult<()> {
    let dir = shot_dir();
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    std::fs::write(dir.join("fail"), message).map_err(|error| error.to_string())
}

/// Sleep off the main thread so the webview can paint, then snapshot.
#[tauri::command]
pub async fn dev_snapshot(
    app: AppHandle,
    path: String,
    label: Option<String>,
    settle_ms: Option<u64>,
) -> CmdResult<()> {
    if let Some(ms) = settle_ms.filter(|ms| *ms > 0) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    #[cfg(not(all(debug_assertions, target_os = "macos")))]
    {
        let _ = (app, path, label);
        Err("Snapshots are only available in a debug Mac build.".into())
    }
    #[cfg(all(debug_assertions, target_os = "macos"))]
    {
        macos_snapshot(&app, &path, label.as_deref().unwrap_or("main"))
    }
}

/// Watch `job.json` and emit each new id. Polls on a thread so occluded
/// webview timers are not the only way a job is noticed.
pub fn spawn_shot_watcher(app: AppHandle) {
    if !shot_control_enabled() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("larra-shot-watch".into())
        .spawn(move || {
            let mut last_id: Option<String> = None;
            loop {
                if let Some(job) = read_job() {
                    if last_id.as_deref() != Some(job.id.as_str()) {
                        last_id = Some(job.id.clone());
                        use tauri::Emitter;
                        let _ = app.emit("larra://shot-job", job);
                    }
                }
                std::thread::sleep(Duration::from_millis(80));
            }
        });
}

fn read_job() -> Option<ShotJob> {
    let path = shot_dir().join("job.json");
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(all(debug_assertions, target_os = "macos"))]
fn macos_snapshot(app: &AppHandle, path: &str, label: &str) -> CmdResult<()> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| format!("No window labelled {label}."))?;
    let (tx, rx) = mpsc::channel();
    window
        .with_webview(move |webview| {
            // SAFETY: Tauri hands a live WKWebView pointer for this window.
            let result = unsafe { png_from_view(webview.inner()) };
            let _ = tx.send(result);
        })
        .map_err(friendly)?;
    let png = rx
        .recv()
        .map_err(|_| "The snapshot did not finish.".to_string())??;
    std::fs::write(path, png).map_err(friendly)?;
    Ok(())
}

/// # Safety
/// `ptr` must be a live `NSView` (WKWebView).
#[cfg(all(debug_assertions, target_os = "macos"))]
unsafe fn png_from_view(ptr: *mut std::ffi::c_void) -> CmdResult<Vec<u8>> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRepPropertyKey, NSView};
    use objc2_foundation::NSDictionary;

    if ptr.is_null() {
        return Err("The window has no webview.".into());
    }
    // SAFETY: caller passes the WKWebView pointer from Tauri's PlatformWebview.
    let view = unsafe { &*ptr.cast::<NSView>() };
    let bounds = view.bounds();
    let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(bounds) else {
        return Err("Couldn't make a bitmap of the window.".into());
    };
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let props = NSDictionary::<NSBitmapImageRepPropertyKey, objc2::runtime::AnyObject>::new();
    // SAFETY: empty properties dictionary is valid for PNG encoding.
    let Some(data) =
        (unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) })
    else {
        return Err("Couldn't encode the snapshot.".into());
    };
    Ok(data.to_vec())
}
