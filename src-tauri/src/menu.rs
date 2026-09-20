//! App menu: About, Settings, new conversation, view shortcuts, and text size.

use serde::Serialize;
#[cfg(target_os = "macos")]
use tauri::menu::WINDOW_SUBMENU_ID;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter};

fn tr(key: &str) -> String {
    rust_i18n::t!(key).into()
}

/// Windows menu accelerators. macOS does not use `&`.
#[cfg(not(target_os = "macos"))]
fn accel(text: String) -> String {
    if text.contains('&') {
        return text;
    }
    if let Some(ch) = text.chars().find(|c| c.is_alphabetic()) {
        return text.replacen(ch, &format!("&{ch}"), 1);
    }
    text
}

const ABOUT: &str = "about";
const VIEW_SETTINGS: &str = "view-settings";
const NEW_CONVERSATION: &str = "new-conversation";
const VIEW_CHAT: &str = "view-chat";
const VIEW_SHELVES: &str = "view-shelves";
const VIEW_RECIPES: &str = "view-recipes";
const TEXT_LARGER: &str = "text-larger";
const TEXT_SMALLER: &str = "text-smaller";

#[derive(Clone, Serialize)]
struct MenuAction {
    action: &'static str,
}

#[cfg(target_os = "macos")]
pub fn build_menu(handle: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let pkg_info = handle.package_info();
    let about = MenuItem::with_id(handle, ABOUT, tr("menu.about"), true, None::<&str>)?;
    let settings = MenuItem::with_id(
        handle,
        VIEW_SETTINGS,
        tr("menu.settings"),
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let new_conversation = MenuItem::with_id(
        handle,
        NEW_CONVERSATION,
        tr("menu.newConversation"),
        true,
        Some("CmdOrCtrl+N"),
    )?;
    let chat = MenuItem::with_id(
        handle,
        VIEW_CHAT,
        tr("menu.chat"),
        true,
        Some("CmdOrCtrl+1"),
    )?;
    let shelves = MenuItem::with_id(
        handle,
        VIEW_SHELVES,
        tr("menu.shelves"),
        true,
        Some("CmdOrCtrl+2"),
    )?;
    let recipes = MenuItem::with_id(
        handle,
        VIEW_RECIPES,
        tr("menu.recipes"),
        true,
        Some("CmdOrCtrl+3"),
    )?;
    let text_larger = MenuItem::with_id(
        handle,
        TEXT_LARGER,
        tr("menu.largerText"),
        true,
        Some("CmdOrCtrl+Plus"),
    )?;
    let text_smaller = MenuItem::with_id(
        handle,
        TEXT_SMALLER,
        tr("menu.smallerText"),
        true,
        Some("CmdOrCtrl+Minus"),
    )?;

    let window_menu = Submenu::with_id_and_items(
        handle,
        WINDOW_SUBMENU_ID,
        tr("menu.window"),
        true,
        &[
            &PredefinedMenuItem::minimize(handle, None)?,
            &PredefinedMenuItem::maximize(handle, None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::close_window(handle, None)?,
        ],
    )?;

    Menu::with_items(
        handle,
        &[
            &Submenu::with_items(
                handle,
                pkg_info.name.clone(),
                true,
                &[
                    &about,
                    &PredefinedMenuItem::separator(handle)?,
                    &settings,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::services(handle, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::hide(handle, None)?,
                    &PredefinedMenuItem::hide_others(handle, None)?,
                    &PredefinedMenuItem::show_all(handle, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::quit(handle, None)?,
                ],
            )?,
            &Submenu::with_items(
                handle,
                tr("menu.file"),
                true,
                &[
                    &new_conversation,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::close_window(handle, None)?,
                ],
            )?,
            &Submenu::with_items(
                handle,
                tr("menu.edit"),
                true,
                &[
                    &PredefinedMenuItem::undo(handle, None)?,
                    &PredefinedMenuItem::redo(handle, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::cut(handle, None)?,
                    &PredefinedMenuItem::copy(handle, None)?,
                    &PredefinedMenuItem::paste(handle, None)?,
                    &PredefinedMenuItem::select_all(handle, None)?,
                ],
            )?,
            &Submenu::with_items(
                handle,
                tr("menu.view"),
                true,
                &[
                    &chat,
                    &shelves,
                    &recipes,
                    &PredefinedMenuItem::separator(handle)?,
                    &text_larger,
                    &text_smaller,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::fullscreen(handle, None)?,
                ],
            )?,
            &window_menu,
        ],
    )
}

#[cfg(not(target_os = "macos"))]
pub fn build_menu(handle: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let about = MenuItem::with_id(handle, ABOUT, accel(tr("menu.about")), true, None::<&str>)?;
    let settings = MenuItem::with_id(
        handle,
        VIEW_SETTINGS,
        accel(tr("menu.settingsWin")),
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let new_conversation = MenuItem::with_id(
        handle,
        NEW_CONVERSATION,
        accel(tr("menu.newConversationWin")),
        true,
        Some("CmdOrCtrl+N"),
    )?;
    let chat = MenuItem::with_id(
        handle,
        VIEW_CHAT,
        accel(tr("menu.chat")),
        true,
        Some("CmdOrCtrl+1"),
    )?;
    let shelves = MenuItem::with_id(
        handle,
        VIEW_SHELVES,
        accel(tr("menu.shelves")),
        true,
        Some("CmdOrCtrl+2"),
    )?;
    let recipes = MenuItem::with_id(
        handle,
        VIEW_RECIPES,
        accel(tr("menu.recipes")),
        true,
        Some("CmdOrCtrl+3"),
    )?;
    let text_larger = MenuItem::with_id(
        handle,
        TEXT_LARGER,
        accel(tr("menu.largerTextWin")),
        true,
        Some("CmdOrCtrl+Plus"),
    )?;
    let text_smaller = MenuItem::with_id(
        handle,
        TEXT_SMALLER,
        accel(tr("menu.smallerTextWin")),
        true,
        Some("CmdOrCtrl+Minus"),
    )?;
    Menu::with_items(
        handle,
        &[
            &Submenu::with_items(
                handle,
                accel(tr("menu.file")),
                true,
                &[&new_conversation, &settings],
            )?,
            &Submenu::with_items(
                handle,
                accel(tr("menu.edit")),
                true,
                &[
                    &PredefinedMenuItem::undo(handle, None)?,
                    &PredefinedMenuItem::redo(handle, None)?,
                    &PredefinedMenuItem::separator(handle)?,
                    &PredefinedMenuItem::cut(handle, None)?,
                    &PredefinedMenuItem::copy(handle, None)?,
                    &PredefinedMenuItem::paste(handle, None)?,
                    &PredefinedMenuItem::select_all(handle, None)?,
                ],
            )?,
            &Submenu::with_items(
                handle,
                accel(tr("menu.view")),
                true,
                &[
                    &chat,
                    &shelves,
                    &recipes,
                    &PredefinedMenuItem::separator(handle)?,
                    &text_larger,
                    &text_smaller,
                ],
            )?,
            &Submenu::with_items(handle, accel(tr("menu.help")), true, &[&about])?,
        ],
    )
}

pub fn on_menu_event(app: &AppHandle, event: &MenuEvent) {
    if event.id() == ABOUT {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::about::open(&app) {
                log::error!("about window: {error}");
            }
        });
        return;
    }

    let action = if event.id() == NEW_CONVERSATION {
        "new-conversation"
    } else if event.id() == VIEW_CHAT {
        "view-chat"
    } else if event.id() == VIEW_SHELVES {
        "view-shelves"
    } else if event.id() == VIEW_RECIPES {
        "view-recipes"
    } else if event.id() == VIEW_SETTINGS {
        "view-settings"
    } else if event.id() == TEXT_LARGER {
        "text-larger"
    } else if event.id() == TEXT_SMALLER {
        "text-smaller"
    } else {
        return;
    };

    if let Err(error) = app.emit("larra://menu", MenuAction { action }) {
        log::warn!("emit larra://menu: {error}");
    }
}
