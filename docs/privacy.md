# Privacy

Documents on a Shelf stay on the machine where Larra is installed, and reading, search, and answers all run there. Larra uses the network to search for or install an AI, to fetch the runtime when the installer did not include it, and to check for a newer release. Settings → Online, when on, also looks things up on the public web after each query or address is approved. The language of the menus is read from that machine (or from a Settings choice) and is not sent anywhere. Counts of personal information are counts, not a legal opinion.

## Claims

| Claim | Reality |
|-------|---------|
| Shelf documents are not uploaded | True. Reading, search, and answers all run locally. |
| The AI runs locally | True. Answers are generated on the machine with or without Online. |
| Chat text is not uploaded | **True while Online is off.** With Online on, the search query or page URL Chat writes leaves the machine only after that exact text is approved. Those lookups do not go through Larra. Prompts tell Chat not to put Shelf text or personal details in them. That is an instruction, not a filter. |
| Larra never uses the network to process documents or to train an AI | **True.** Text extraction, search, and answers all run on the machine where Larra is installed, and nothing on a Shelf is uploaded or used as training data. The network is used elsewhere. Searching for or installing an AI needs it. The software that runs the AI is included in release builds; GitHub is contacted if that piece is missing, and on some Windows machines a faster GPU build may be downloaded at first warmup. A startup check may fetch `latest.json` from GitHub Releases (`tauri dev` skips it); if that fails, the app continues as usual. Settings → Online, when on, also contacts Wikipedia, DuckDuckGo Instant Answer, You.com, and any page Chat opens, after each query or address is approved. |
| Personal-information counts are a legal opinion | **False.** They are detector hits, not a compliance assessment. |

## Network egress

| Destination | When | Payload |
|-------------|------|---------|
| `github.com/ggml-org/llama.cpp` | Runtime missing from the installer (`tauri dev` without fetch, or a broken bundle); Windows may also fetch CUDA 12.4 (NVIDIA) or Adreno OpenCL (Snapdragon) at first warmup | HTTPS GET of a pinned archive; SHA-256 checked |
| GitHub Releases (`latest.json` on the repo in `Cargo.toml` `package.repository`) | Startup of a release build, in the background. `tauri dev` does not check. | HTTPS GET of a small JSON file. Failure is ignored; no UI. The GitHub repo must be public or the fetch 404s. |
| `huggingface.co` | Explore / install | Search query, IP, and the user agent `Larra/0.9.3 (local-first open-source desktop AI; https://github.com/espilber/larra)` |
| `ollama.com` / `registry.ollama.ai` | Explore / install | Same |
| `127.0.0.1:<port>` | Chat, benchmark | Full prompts including retrieved passages |
| `en.wikipedia.org`, `api.duckduckgo.com`, `api.you.com` | Chat, only if Settings → Online is on and the exact query is approved | The search query Chat writes; IP; and the user agent `Larra/0.9.3 (https://github.com/espilber/larra; github.com/espilber/larra/issues)`, which names the project and a contact address. Prompts ask Chat not to put Shelf text or personal details in the query. |
| The page Chat opens | Chat, only if Settings → Online is on, the AI asks to read a URL, and that address is approved | HTTPS GET of that URL, sent from the machine, with the same user agent as the row above. Loopback and private addresses are refused. Prompts ask Chat not to put private text in the URL. |

No analytics SDK is bundled.

## Data at rest

Unencrypted in the app-data directory (mode `0700` on Unix):

- `extracted/*.md`: full text, including personal information
- Tantivy index
- Conversation JSONL (citation ids, titles, and the quoted passage used for each answer)
- Files added to a Shelf (`library/<name>/`); kept when Larra is reset
- Files attached in Chat (`conversations/<thread>/uploads/`): copies, deleted with the conversation
- Cards store **counts**, not the matched strings

Assume anyone with access to the OS user account can read this. Settings → Reset Larra deletes this app-data directory (and caches), except the folders that hold Shelf files.

## Trust boundaries

1. **The operator.** Linked folders are trusted to the extent they were linked. Symlinks inside a linked tree are **skipped** on scan and cannot be opened through the allowlist.
2. **Documents** are untrusted as *instructions*. Prompts tell the AI to treat retrieved text as data, and not to put Shelf text or personal details in Online lookups. This is not a sandbox or a filter.
3. **AI output** is untrusted HTML. The UI sanitizes Markdown (DOMPurify) under a strict CSP. A bug here plus a malicious document could historically reach `invoke()`; treat XSS in the webview as a document-exfil path.
4. **AI files** are untrusted binaries. Installs without a checksum are refused. After download, Skip the check and use the file uses the file without hashing it.
5. **Compromised renderer.** Commands that open or reveal a file only accept paths under a known Shelf root. Import copies only files a native drop or the OS picker just offered; JavaScript cannot pass an arbitrary path. The webview cannot ask the opener plugin to reveal a folder.

## House rules

Standing instructions are attacker-controlled if someone else can edit Settings on the machine. They are injected into the system prompt on every message.
