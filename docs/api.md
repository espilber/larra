# HTTP / event contract

Canonical types live in Rust. The TypeScript mirror is `src/lib/api.ts`. Prefer `camelCase` on the wire. On-disk JSON that used `snake_case` is still accepted via serde aliases.

Handlers live under `src-tauri/src/commands/`, plus `about.rs`, `menu.rs`, and `updater.rs`. New commands need a handler, a registration in `lib.rs` `generate_handler!`, and an `api.ts` wrapper in the same PR.

## Events

| Name | Payload |
|------|---------|
| `larra://engine` | `EngineStatus` |
| `larra://download` | `DownloadEvent` |
| `larra://ingest` | per-document status |
| `larra://shelf-stats` | `{ shelfId, stats }` (`waiting` is files accepted but not yet being read) |
| `larra://shelves` | empty object (reload list) |
| `larra://chat` | stream machine (`queued` … `done` / `error`) |
| `larra://menu` | `{ action }` (`new-conversation` \| `view-chat` \| `view-shelves` \| `view-recipes` \| `view-settings` \| `text-larger` \| `text-smaller`) |
| `larra://update` | `{ version, currentVersion, notes? }` when a newer release exists |
| `larra://update-progress` | `{ event: started \| progress \| finished }` while installing |

## Commands

| Command | Args | Returns |
|---------|------|---------|
| `shelves_list` | — | `ShelfView[]` (`linkedFolders[].available` is false when that folder is not present on the machine) |
| `shelf_create` | `name` | `ShelfView` |
| `shelf_remove` | `shelfId` | `()` |
| `shelf_add_linked` | `shelfId` | `AddLinkedResult \| null` (`shelf`, `queued`, `atLimit`; null if the folder picker was cancelled) |
| `shelf_remove_source` | `shelfId`, `sourceId` | `()` |
| `shelf_import_paths` | `shelfId`, `paths` | `ImportResult` (`queued`, `names`, `cancelled`, `atLimit`, `skippedLong`). Empty `paths` imports the latest native drop. Other paths must already be on the drop/picker allowlist; anything else is ignored. |
| `shelf_import_dialog` | `shelfId` | `ImportResult` (`cancelled` if the picker was dismissed; same fields as `shelf_import_paths`) |
| `pick_files` | — | `string[] \| null` (null if cancelled; returned paths are allowlisted for a following `shelf_import_paths`) |
| `shelf_documents` | `shelfId` | `DocumentMeta[]` |
| `document_card` | `shelfId`, `docId` | `Card` |
| `document_text` | `shelfId`, `docId`, `startChar?`, `page?`, `section?`, `around?` | window of extracted text (`text`, `startChar`, `endChar`, `totalChars`, `windowChars`) |
| `document_reprocess` | `shelfId`, `docId` | `()` |
| `shelf_retry_failed` | `shelfId` | `u32` (how many failed files were queued again) |
| `open_original` | `path` | `()` (allowlisted Shelf paths only) |
| `reveal_item` | `path` | `()` (allowlisted Shelf paths only) |
| `threads_list` | — | `ThreadMeta[]` |
| `thread_create` | `shelfId?` | `ThreadMeta` |
| `thread_messages` | `threadId`, `beforeId?` | `{ messages, hasOlder }` (latest window, or the window older than `beforeId`) |
| `thread_set_shelf` | `threadId`, `shelfId?` | `()` |
| `thread_rename` | `threadId`, `title` | `()` |
| `thread_export` | `threadId` | `bool` (false if the save dialog was cancelled) |
| `thread_delete` | `threadId` | `()` |
| `chat_send` | `threadId`, `text`, `shelfId?` | `()` (answer arrives on `larra://chat`; `queued` fires immediately, then waits if another answer is in flight) |
| `chat_cancel` | `messageId` | `()` |
| `warm_engine` | — | `()` |
| `engine_status` | — | `EngineStatus` |
| `engine_remeasure` | — | `()` (recalibrates when Chat is idle) |
| `machine_profile` | — | `MachineView` |
| `active_model` | — | `ActiveModel \| null` (Rust command; the UI reads the model from `settings_get`) |
| `models_search` | `query` | `ModelSearchResult[]` |
| `open_model_page` | `source`, `reference` | `()` (system browser; Hugging Face or Ollama page only) |
| `model_install` | `source`, `reference`, `name`, `license?` | `()` (progress on `larra://download`) |
| `download_cancel` | `id` | `()` |
| `download_skip_verify` | `id` | `()` (model downloads only; finishes install without hashing) |
| `settings_get` | — | `SettingsView` |
| `settings_set_house_rules` | `text` | `()` |
| `settings_set_allow_online_research` | `enabled` | `()` |
| `settings_set_text_size` | `size` (`default` \| `large` \| `larger`) | `()` |
| `settings_finish_onboarding` | — | `()` |
| `settings_reset_workspace` | `confirmation` (`DELETE`) | `()` (writes a marker, stops the engine, restarts; wipe runs on next launch before the window is created) |
| `redact_text` | `text` | redacted string |
| `text_has_pii` | `text` | `bool` |
| `diagnostics` | — | `Diagnostics` (no engine log body) |
| `open_engine_log` | — | `()` (opens the on-disk engine log; no path from the UI) |
| `recipes_list` | — | `Recipe[]` |
| `recipe_create` | `name`, `prompt` | `Recipe` |
| `recipe_delete` | `id` | `()` |
| `recipes_restore_defaults` | — | `Recipe[]` (adds missing defaults; preserves existing Recipes) |
| `recipe_reset_default` | `id` | `Recipe` (resets one existing built-in Recipe) |
| `about_info` | — | `AboutInfo` |
| `show_about_window` | — | `()` |
| `open_external` | `link` | `()` |
| `update_info` | — | `AppUpdate \| null` (null unless a newer GitHub release was confirmed) |
| `show_update_window` | — | `()` |
| `install_update` | — | `()` (progress on `larra://update-progress`; app restarts) |
