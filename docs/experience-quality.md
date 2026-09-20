# Experience quality gate

The normal test suite covers retrieval, file extraction, queued cancellation, UTF-8 stream boundaries, interrupted streams, metadata recovery, import/navigation races, event subscription ordering, and large-table bounds. CI runs frontend checks on Linux and Rust unit/integration checks on macOS and Windows. The existing Linux Rust job remains useful for static checks.

Run the real-model gate on each release candidate using a GGUF and the pinned engine archive already on the test computer:

```sh
node scripts/quality-gate.mjs /path/model.gguf /path/pinned-engine-archive /tmp/quality.json
```

This uses a temporary workspace and synthetic files. It never opens the user's library or downloads a model. On Unix it links the model; on Windows it uses a hard link, falling back to a copy if the volume requires it. The ignored `experience_quality` integration test is intentionally separate from ordinary CI, which cannot represent users' GPU performance.

The JSON report records platform, architecture, model size, engine warmup, ingestion time and bytes, first visible answer time, total answer time, cancellation time, grounded facts, and exact citation counts. The long fixture places a known fact near the end; separate English, Spanish and Catalan questions must return the correct fact and cite the exact saved text and file version. A wrong answer or unverifiable citation fails the gate.

Defaults are first visible answer under 30 seconds per warm query, ingestion of the synthetic long Markdown file under 30 seconds, and cancellation under 250 ms. Override with positive `REBOST_MAX_TTFT_MS`, `REBOST_MAX_INGEST_MS`, and `REBOST_MAX_CANCEL_MS` for a documented device/model baseline. Do not loosen a threshold merely to make a regression pass. Compare the same model, engine, power mode, device and corpus across builds; three language cases are a smoke sample, not a statistically useful p95.

Before a download-button release, run on Apple Silicon, Intel macOS, Windows x64 and Windows ARM64, including the actual GPU/CPU fallback used on that machine. Save the report alongside the release validation. CI's macOS and Windows runners complement this hardware check; they do not certify every accelerator or architecture.

Manual acceptance checks:

- VoiceOver on macOS and Narrator on Windows: find the composer, open and dismiss file suggestions with arrows/Escape, choose a file with Enter/Tab, and switch the selected library using only the keyboard. Confirm focus returns after closing panels.
- Receive a streamed answer without hearing every token repeatedly. Confirm the preparing/writing/completed announcements and read the final answer and citation controls normally.
- Type Japanese with an IME: Enter commits composition without sending. Send the completed draft with the next Enter.
- At the largest text size and narrowest supported window: inspect settings, source panels, composer, and file-table horizontal scrolling. Navigate the virtual table with arrows/Home/End, or enable “Show all rows” for full native table navigation.
- Attach files, ask while they are processing, then Stop. Test one unreadable attachment, retry its processing, and retry the question. Switch conversations while a file picker is open; files must remain on the original conversation.
- Interrupt the model process after visible text arrives. The partial answer must remain after reopening the conversation, with Retry and Continue available. Simulate a failed send and verify the draft returns.
- Enable online research and inspect the exact outbound query or URL. Deny it, cancel the answer, and disable online access while a decision is pending. No request should leave the app without the corresponding approval. Public lookup responses remain untrusted source material.
- Edit standing instructions, change language or text size, and navigate away/back. Unsaved text must survive. Make settings persistence fail and verify the UI does not claim the save succeeded.
- Open a citation, edit and reprocess its file, then reopen it. The saved quoted evidence must remain available with a changed-version notice and separate current context. Old conversations without saved anchors retain their legacy location lookup.

Automatic tests do not establish screen-reader usability, general answer quality, or performance on hardware that was not exercised. Record those manual results explicitly.
