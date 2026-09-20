# Architecture

Larra is a desktop app: a Svelte 5 UI in a Tauri 2 webview, and a Rust core that owns documents, search, and the language-model process. The shell uses the OS for the title bar, menus, dialogs, context menus, and light/dark appearance.

```
Chat / Shelves / Recipes / Settings
            │  invoke + events (larra://…)
            ▼
     Tauri commands (thin adapters)
            │
    ┌───────┼────────┬──────────┬─────────┐
    ▼       ▼        ▼          ▼         ▼
  shelf   ingest   search     chat     engine
  watcher extract  tantivy   prompts   llama-server
          PII      gate                download
```

## Product pieces

**Chat.** Where questions go. The AI answers locally. Choose a Shelf when the answer should come from those files. Retrieve from the selected Shelf, gate, local `llama-server`, streamed answer with `[S1]` citations. Files attached in Chat live on a hidden per-conversation Shelf. Attached and named files take the local-context budget first; the library Shelf gets what is left. Short files are sent whole; longer ones are searched. A summarize-this-file ask samples the start, middle, and end. Hits include a little text before and after the match. When excerpts are not enough, the AI can search the Shelf again, read more around a citation, or open a named file one window at a time (calling open again continues; other excerpts from that file stay). Light and Deep extra queries run against a named attachment; they skip the library Shelf when the question names an upload. Card outline and summary for a named file go in the system prompt. Earlier turns of this conversation, and other conversations on the same Shelf, are searched when the AI asks and there is something older than the recent window. A No Shelf turn still searches this conversation when it is long enough, and skips the look-through step when there is nothing to retrieve. If Settings allows it, Chat can also search the public web from that machine (Wikipedia, DuckDuckGo Instant Answer, You.com) and open a page as markdown; those notes are not `[S1]` citations. Any source that does not answer is skipped. A library Shelf's Off / Light / Deep setting (Deep on new shelves; missing yaml is Off; older `think` / `think-harder` still load) applies to the whole turn, including attachments, and sets extra queries, neighbor radius, and gate caps. Deep allows more look-throughs when a window was truncated. Light and Deep add three fused search queries. Deep also stuffs the top matching files when they fit the budget (named attachments first), and turns on the loaded AI's cheapest native thinking when the chat template supports it. Generations are serialized (one answer at a time); a second send waits, including from another conversation. Conversation JSONL stores citation ids and titles, not passage bodies; thinking is clipped; look-through steps are kept. Clicking a citation opens a window of that file around the cited spot. The open thread loads a window of recent messages.

**Shelves.** A Shelf is a folder Chat answers from. Attach a folder already on disk, or drop files in. Each has a managed folder for imports plus optional linked folders that `notify` watches. A linked folder that is missing (unmounted volume) is paused: files stay on the Shelf and in the index until the folder is back or the user unlinks it. Conversation upload shelves do not appear here. Drop and Add files copy into a dated `Imports/` folder. On Windows, copy is long-path aware (`longPathAware` and `\\?\`); a destination that still cannot fit is skipped (`skippedLong` on `ImportResult`).

**Ingest.** Xberg extraction for documents (PDF, Office, email, and text, but not HTML, JSON, or XML), a skip list for hidden files and package folders (`node_modules`, caches, and similar; not `Library`), a 1,000-file cap per Shelf, optional OCR from whatever `*.traineddata` packs ship in `resources/tessdata`, Card YAML, passages, Tantivy. Linking a folder walks at most 30,000 entries so a home directory cannot stall Add folder. Startup skips the content hash when size and mtime match; Error files stay until Try again. Opening Shelves (list or a Shelf) marks Reading files older than five minutes as Error so a stuck file becomes Sync error with Resume. There is no background timer. A new folder in a watched source is scanned on its own; extra files wait in the ingest queue instead of being dropped. Deleting a Shelf or unlinking a folder drops waiting work; an in-flight read does not persist if that Shelf or folder is gone. Word/Excel lock files (`~$…`) and `*.tmp` are skipped. If a file's size changes during the content hash, ingest waits and retries once; a file still changing is left for a later pass.

**Privacy Lens.** Counts of emails, phones, IBANs, Spanish tax ids, US Social Security numbers, labeled names, and similar. Counts only on the Card. Full text still lives in `extracted/*.md`, the index, and conversation JSONL. Redact-on-copy is a display action, not encryption at rest.

**Recipes.** A saved question reused on any Shelf.

**House rules.** How the AI should answer. Set once; injected into the system prompt on every message. Never mixed into retrieved excerpts.

**Languages.** Shared JSON catalogs under `locales/` feed rust-i18n and a thin Svelte `t()`. The OS language is the default; Settings can pin a shipped catalog. English, Spanish, and Catalan are maintained; the others are drafts marked for review. See [i18n.md](i18n.md).

**Engine.** A pinned llama.cpp `llama-server` archive is bundled in the installer, unpacked into app data, and spawned on `127.0.0.1`. Not a Tauri `externalBin` sidecar. GitHub downloads are SHA-256 verified. Signed Mac copies are re-signed for notarization, so they skip the GitHub pin SHA; see [engine.md](engine.md). Switching AIs keeps the previous file until the new process is Ready; Chat can use the current AI while the next file downloads. Context and answer length follow the loaded AI and the host machine (about 768 tokens out on a 4k window, 1,536–2,048 on a wider one; the window stays 4k–16k). Short replies still stop when the AI is done.

## Data on disk

The OS app-data directory (see `src-tauri/src/paths.rs`): macOS `~/Library/Application Support/io.larra.desktop/`, Windows `%APPDATA%\io.larra.desktop\`.

```
library/                managed Shelf folders (user files; kept on Reset)
shelves/<id>/{cards,extracted,documents.json}
search/tantivy/
conversations/          threads.json + one JSONL per thread
                        + <thread-id>/uploads/ for chat attachments (hidden Shelf)
models/                 GGUF files (previous file stays until the new AI is Ready)
engine/<release>-<accel>/ llama-server (legacy engine/<release>/ still works)
recipes.json
settings.json
instance.lock           exclusive while Larra is open
logs/engine.log
```

The layout can change without a migration. New library Shelves are created under `library/`. Existing Shelves keep the folder already stored in the registry (including older `Documents/Larra` paths). Settings → Reset Larra (or `scripts/reset.sh`) returns to first-run without deleting `library/` or Shelf folders outside app data. Replacing the app does not touch this directory.

## Module map (`src-tauri/src/`)

| Module | Job |
|--------|-----|
| `commands` | IPC only |
| `shelf` | library + linked folders + watcher + scan skip list |
| `ingest` | queue, extract, card, passages; 1,000-file Shelf cap |
| `search` | Tantivy + multilingual stem + relevance gate |
| `chat` | orchestration + prompts + conversations |
| `engine` | process, download, catalog (standing, first document row that fits), stream split |
| `pii` | Privacy Lens |
| `updater` | Silent GitHub `latest.json` check; in-app install |
| `instance` | Exclusive lock in app data; a second copy focuses the first window |
| `recipes` / `settings` / `reset` / `paths` / `ids` | support |

The frontend contract is `src/lib/api.ts`. Event names are `larra://engine|download|ingest|shelf-stats|shelves|chat|update|update-progress`.
