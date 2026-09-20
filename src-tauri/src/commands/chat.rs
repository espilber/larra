//! Thread and chat commands.

use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use super::shelves::{shelf_view, ShelfView};
use super::{friendly, require_id, require_optional_id, CmdResult};
use crate::chat::conversations::{Conversations, ThreadMeta, ThreadPage};
use crate::chat::ChatService;
use crate::core::Ctx;
use crate::engine::Engine;
use crate::shelf::watcher::WatcherHub;

/// List saved conversation threads.
#[tauri::command]
pub fn threads_list(ctx: State<'_, Arc<Ctx>>) -> Vec<ThreadMeta> {
    Conversations::list(&ctx.paths)
}

/// Start a new thread, optionally scoped to a Shelf.
#[tauri::command]
pub fn thread_create(ctx: State<'_, Arc<Ctx>>, shelf_id: Option<String>) -> CmdResult<ThreadMeta> {
    require_optional_id(shelf_id.as_deref())?;
    Conversations::create(&ctx.paths, shelf_id).map_err(friendly)
}

/// Load a window of messages (latest first, then older via `beforeId`).
#[tauri::command]
pub fn thread_messages(
    ctx: State<'_, Arc<Ctx>>,
    thread_id: String,
    before_id: Option<String>,
) -> CmdResult<ThreadPage> {
    require_id(&thread_id)?;
    require_optional_id(before_id.as_deref())?;
    Ok(Conversations::page(
        &ctx.paths,
        &thread_id,
        before_id.as_deref(),
        crate::chat::conversations::THREAD_PAGE_SIZE,
    ))
}

/// Attach or detach a thread from a Shelf.
#[tauri::command]
pub fn thread_set_shelf(
    ctx: State<'_, Arc<Ctx>>,
    thread_id: String,
    shelf_id: Option<String>,
) -> CmdResult<()> {
    require_id(&thread_id)?;
    require_optional_id(shelf_id.as_deref())?;
    Conversations::set_shelf(&ctx.paths, &thread_id, shelf_id).map_err(friendly)
}

/// Rename a conversation. The first user message no longer overwrites it.
#[tauri::command]
pub fn thread_rename(ctx: State<'_, Arc<Ctx>>, thread_id: String, title: String) -> CmdResult<()> {
    require_id(&thread_id)?;
    Conversations::rename(&ctx.paths, &thread_id, &title).map_err(friendly)
}

/// Save the conversation as a Markdown file. False if the save dialog was cancelled.
#[tauri::command]
pub async fn thread_export(
    app: AppHandle,
    ctx: State<'_, Arc<Ctx>>,
    thread_id: String,
) -> CmdResult<bool> {
    require_id(&thread_id)?;
    let meta = Conversations::get(&ctx.paths, &thread_id).ok_or("thread not found")?;
    let messages = Conversations::messages(&ctx.paths, &thread_id);
    let markdown = crate::chat::conversations::thread_markdown(&meta.title, &messages);
    let file_name = format!(
        "{}.md",
        crate::chat::conversations::export_file_stem(&meta.title)
    );
    let Some(path) = app
        .dialog()
        .file()
        .set_title(rust_i18n::t!("chat.exportConversation").as_ref())
        .set_file_name(&file_name)
        .add_filter("Markdown", &["md"])
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(friendly)?;
    std::fs::write(&path, markdown).map_err(friendly)?;
    Ok(true)
}

/// Hidden Shelf for files attached in this conversation.
#[tauri::command]
pub async fn thread_ensure_upload_shelf(
    ctx: State<'_, Arc<Ctx>>,
    watcher: State<'_, Arc<WatcherHub>>,
    thread_id: String,
) -> CmdResult<ShelfView> {
    require_id(&thread_id)?;
    if Conversations::get(&ctx.paths, &thread_id).is_none() {
        return Err("That item is no longer available.".into());
    }
    let shelf = {
        let mut library = crate::core::write_lock(&ctx.library);
        library
            .ensure_conversation_shelf(&ctx.paths, &thread_id)
            .map_err(friendly)?
    };
    Conversations::set_upload_shelf(&ctx.paths, &thread_id, shelf.id.clone()).map_err(friendly)?;
    watcher.rebuild();
    Ok(shelf_view(&shelf, super::stats_for(&ctx, &shelf.id)))
}

/// Delete a thread, its search records, and any conversation-only uploads.
#[tauri::command]
pub fn thread_delete(
    ctx: State<'_, Arc<Ctx>>,
    watcher: State<'_, Arc<WatcherHub>>,
    thread_id: String,
) -> CmdResult<()> {
    require_id(&thread_id)?;
    crate::chat::delete_thread(&ctx, &thread_id).map_err(friendly)?;
    watcher.rebuild();
    Ok(())
}

/// Send a message — returns immediately; `larra://chat` events stream the
/// answer (`queued` while waiting or warming up). A second send waits until
/// the current answer finishes.
#[tauri::command]
pub fn chat_send(
    chat: State<'_, Arc<ChatService>>,
    thread_id: String,
    text: String,
    shelf_id: Option<String>,
) -> CmdResult<()> {
    require_id(&thread_id)?;
    require_optional_id(shelf_id.as_deref())?;
    if text.trim().is_empty() || text.chars().count() > crate::limits::PROMPT_MAX_CHARS {
        return Err(rust_i18n::t!("errors.promptTooLong").to_string());
    }
    let chat = chat.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = chat.send_message(&thread_id, &text, shelf_id).await {
            log::error!("chat send failed: {error:#}");
            chat.notify_send_failed(&thread_id);
        }
    });
    Ok(())
}

/// Stop streaming the current answer.
#[tauri::command]
pub fn chat_cancel(chat: State<'_, Arc<ChatService>>, message_id: String) -> CmdResult<()> {
    require_id(&message_id)?;
    chat.cancel(&message_id);
    Ok(())
}

#[tauri::command]
pub fn chat_approve_web(request_id: String, allowed: bool) -> CmdResult<()> {
    require_id(&request_id)?;
    crate::chat::web_approval::resolve(&request_id, allowed);
    Ok(())
}

/// The composer was focused — start the engine in the background so the
/// first answer needs no wait ("Warming up…" otherwise).
#[tauri::command]
pub fn warm_engine(engine: State<'_, Arc<Engine>>) {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = engine.ensure_ready().await {
            log::warn!("warm engine: {error:#}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_cancel_reject_unsafe_ids() {
        assert!(require_id("../thread").is_err());
        assert!(require_id("a/b").is_err());
        assert!(require_optional_id(Some("..")).is_err());
        assert_eq!(require_optional_id(None), Ok(()));
        assert!(require_id("t_ab12cd").is_ok());
    }
}
