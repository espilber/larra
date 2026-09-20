# Adding an ingest format

1. Confirm [xberg](https://crates.io/crates/xberg) already understands the format, or add the crate feature in `src-tauri/Cargo.toml`.
2. Extend `REBOST_EXTENSIONS` / `is_supported_file` in `src-tauri/src/ingest/extract.rs`.
3. Add a small fixture under `src-tauri/tests/fixtures/` and a case in `pipeline_test.rs` (and retrieval eval if the format should be searchable).
4. Mention the extension in Diagnostics (`supported_extensions`).

Unsupported files are skipped. They should not appear as errors on the Shelf.
