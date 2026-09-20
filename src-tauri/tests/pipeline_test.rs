//! Pipeline integration: real files through hash → Xberg → PII → Card →
//! passages → Tantivy, using the deterministic path only (no LLM).

mod common;

use common::*;
use larra::types::DocStatus;

#[tokio::test(flavor = "multi_thread")]
async fn markdown_contract_becomes_ready_with_card_and_pii() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Contracts");
    let file = fixture_contract_md(app.dir.path());

    let meta = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(meta.status, DocStatus::Ready);
    assert!(meta.passage_count >= 3, "sections should become passages");
    assert!(!meta.ocr, "native text needs no OCR");

    // Card
    let card = larra::ingest::card::read_card(&app.ctx.paths.card_path(&shelf_id, &meta.id))
        .expect("card written");
    assert_eq!(card.schema, "larra-card/v1");
    assert!(card.title.contains("Northwind"));
    assert_eq!(card.quality, "full");
    assert!(!card.summary.is_empty(), "TextRank summary expected");
    assert!(!card.keywords.is_empty(), "YAKE keywords expected");
    assert!(
        card.outline.iter().any(|o| o.title.contains("Termination")),
        "outline should list the Termination heading, got {:?}",
        card.outline
    );

    // Privacy Lens: IBAN + email present, values never stored.
    assert!(card.privacy.total >= 2);
    assert_eq!(card.privacy.categories.get("iban"), Some(&1));
    assert!(card.privacy.categories.get("email").copied().unwrap_or(0) >= 1);
    let card_text = std::fs::read_to_string(app.ctx.paths.card_path(&shelf_id, &meta.id)).unwrap();
    assert!(
        !card_text.contains("ES7620770024003102575766"),
        "cards must never store PII values"
    );

    // Extracted-text cache backs "View extracted text".
    let extracted =
        std::fs::read_to_string(app.ctx.paths.extracted_path(&shelf_id, &meta.id)).unwrap();
    assert!(extracted.contains("ninety (90) days"));
}

#[tokio::test(flavor = "multi_thread")]
async fn xlsx_payroll_counts_pii_per_sheet() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Finance");
    let file = fixture_payroll_xlsx(app.dir.path());

    let meta = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(meta.status, DocStatus::Ready, "error: {:?}", meta.error);
    assert!(meta.passage_count >= 1);

    let card = larra::ingest::card::read_card(&app.ctx.paths.card_path(&shelf_id, &meta.id))
        .expect("card");
    assert_eq!(
        card.privacy.categories.get("nif"),
        Some(&3),
        "{:?}",
        card.privacy
    );
    assert_eq!(card.privacy.categories.get("iban"), Some(&3));
    assert_eq!(card.privacy.categories.get("email"), Some(&3));
}

#[tokio::test(flavor = "multi_thread")]
async fn pdf_with_native_text_extracts_pages() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Operations");
    let file = fixture_brief_pdf(app.dir.path());

    let meta = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(meta.status, DocStatus::Ready, "error: {:?}", meta.error);
    assert!(!meta.ocr, "typst PDFs carry a text layer");
    let extracted =
        std::fs::read_to_string(app.ctx.paths.extracted_path(&shelf_id, &meta.id)).unwrap();
    assert!(extracted.contains("18,000 EUR"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unchanged_files_are_skipped_and_changed_files_reprocessed() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Docs");
    let file = fixture_invoice_md(app.dir.path());

    let first = ingest_file(&app, &shelf_id, &file).await;
    let first_updated = first.updated_at.clone();

    // Same content → skip (updated_at unchanged).
    let second = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(second.updated_at, first_updated);

    // Changed content → same id, new hash, reprocessed.
    std::fs::write(
        &file,
        "# Invoice INV-2026-0042\n\nAmount due: 9,999.00 EUR\n",
    )
    .unwrap();
    let third = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(third.id, first.id);
    assert_ne!(third.hash, first.hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_index_entry_is_rebuilt_from_cache() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Docs");
    let file = fixture_invoice_md(app.dir.path());
    let first = ingest_file(&app, &shelf_id, &file).await;
    assert!(app.ctx.search.has_document(&first.id));
    app.ctx.search.remove_document(&first.id).unwrap();
    assert!(!app.ctx.search.has_document(&first.id));

    let second = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(second.updated_at, first.updated_at);
    assert!(app.ctx.search.has_document(&first.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_files_are_ready_not_errors() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Notes");

    // 0-byte file and a whitespace-only file — both fine, both Ready.
    let empty = app.dir.path().join("empty.md");
    std::fs::write(&empty, "").unwrap();
    let blank = app.dir.path().join("Changelog-PENDING.md");
    std::fs::write(&blank, "\n\n").unwrap();

    for file in [&empty, &blank] {
        let meta = ingest_file(&app, &shelf_id, file).await;
        assert_eq!(meta.status, DocStatus::Ready, "error: {:?}", meta.error);
        assert_eq!(meta.passage_count, 0);
        assert!(meta.error.is_none());
        let again = ingest_file(&app, &shelf_id, file).await;
        assert_eq!(again.updated_at, meta.updated_at);
    }
    let stats = app.ctx.library.read().unwrap().stats(&shelf_id);
    assert_eq!(stats.searchable, 2);
    assert_eq!(stats.errors, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_files_stay_visible_with_error() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Broken");
    let file = app.dir.path().join("broken.pdf");
    std::fs::write(&file, b"this is not a pdf at all").unwrap();

    let meta = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(meta.status, DocStatus::Error);
    assert!(meta.error.is_some());
    // Not indexed as evidence.
    assert_eq!(meta.passage_count, 0);

    // The shelf table still shows it.
    let stats = app.ctx.library.read().unwrap().stats(&shelf_id);
    assert_eq!(stats.files, 1);
    assert_eq!(stats.errors, 1);
    assert_eq!(stats.searchable, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn error_files_are_not_reread_until_forced() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Broken");
    let file = app.dir.path().join("broken.pdf");
    std::fs::write(&file, b"this is not a pdf at all").unwrap();

    let first = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(first.status, DocStatus::Error);
    let second = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(second.updated_at, first.updated_at);

    let job = larra::ingest::ProcessJob {
        shelf_id: shelf_id.clone(),
        source_id: larra::shelf::Shelf::IMPORTED_SOURCE.to_string(),
        source_type: larra::types::SourceType::Imported,
        source_label: "Imported".into(),
        abs_path: file.clone(),
        rel_path: "broken.pdf".into(),
        force: true,
        epoch: 0,
    };
    larra::ingest::process_file(&app.ctx, &job)
        .await
        .expect("force");
    let third = app
        .ctx
        .library
        .read()
        .unwrap()
        .document(&shelf_id, &first.id)
        .unwrap();
    assert_eq!(third.status, DocStatus::Error);
    assert_ne!(third.updated_at, first.updated_at);
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_reading_finishes_on_the_next_pass() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Docs");
    let file = fixture_invoice_md(app.dir.path());
    let fs_meta = std::fs::metadata(&file).unwrap();
    let mtime_ms = fs_meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let rel = file.file_name().unwrap().to_string_lossy().to_string();
    let doc_id = larra::ids::document_id(&shelf_id, larra::shelf::Shelf::IMPORTED_SOURCE, &rel);
    {
        let mut library = app.ctx.library.write().unwrap();
        library.upsert_document(larra::types::DocumentMeta {
            id: doc_id.clone(),
            shelf_id: shelf_id.clone(),
            source_id: larra::shelf::Shelf::IMPORTED_SOURCE.to_string(),
            source_type: larra::types::SourceType::Imported,
            path: file.to_string_lossy().into(),
            rel_path: rel,
            file_name: file.file_name().unwrap().to_string_lossy().into(),
            format: "md".into(),
            size_bytes: fs_meta.len(),
            mtime_ms,
            hash: String::new(),
            status: DocStatus::Reading,
            error: None,
            passage_count: 0,
            pages: None,
            pii_total: 0,
            pii_categories: Default::default(),
            ocr: false,
            updated_at: "2026-01-01T00:00:00Z".into(),
            source_label: "Imported".into(),
        });
    }
    let meta = ingest_file(&app, &shelf_id, &file).await;
    assert_eq!(meta.status, DocStatus::Ready, "error: {:?}", meta.error);
    assert_eq!(meta.id, doc_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn removal_deletes_derived_data_only() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Cleanup");
    let file = fixture_invoice_md(app.dir.path());
    let meta = ingest_file(&app, &shelf_id, &file).await;

    let card_path = app.ctx.paths.card_path(&shelf_id, &meta.id);
    assert!(card_path.exists());

    larra::ingest::remove_document(&app.ctx, &shelf_id, &meta.id);
    assert!(!card_path.exists());
    assert!(
        !app.ctx.paths.extracted_path(&shelf_id, &meta.id).exists(),
        "extracted cache removed"
    );
    // The original file is never touched.
    assert!(file.exists());
    assert!(app
        .ctx
        .library
        .read()
        .unwrap()
        .document(&shelf_id, &meta.id)
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn csv_and_eml_become_ready() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Inbox");
    let csv = ingest_file(&app, &shelf_id, &fixture_clients_csv(app.dir.path())).await;
    assert_eq!(csv.status, DocStatus::Ready, "csv error: {:?}", csv.error);
    let eml = ingest_file(&app, &shelf_id, &fixture_notice_eml(app.dir.path())).await;
    assert_eq!(eml.status, DocStatus::Ready, "eml error: {:?}", eml.error);
    let extracted =
        std::fs::read_to_string(app.ctx.paths.extracted_path(&shelf_id, &eml.id)).unwrap();
    assert!(extracted.contains("ninety (90) days"));
}

#[tokio::test(flavor = "multi_thread")]
async fn html_and_json_are_not_ingested() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Web");
    let html = fixture_notice_html(app.dir.path());
    let json = app.dir.path().join("blob.json");
    std::fs::write(&json, "{\"ok\":true}").unwrap();
    for path in [&html, &json] {
        let job = larra::ingest::ProcessJob {
            shelf_id: shelf_id.clone(),
            source_id: larra::shelf::Shelf::IMPORTED_SOURCE.to_string(),
            source_type: larra::types::SourceType::Imported,
            source_label: "Imported".into(),
            abs_path: path.clone(),
            rel_path: path.file_name().unwrap().to_string_lossy().into(),
            force: false,
            epoch: 0,
        };
        larra::ingest::process_file(&app.ctx, &job)
            .await
            .expect("skip");
    }
    assert!(
        app.ctx
            .library
            .read()
            .unwrap()
            .documents(&shelf_id)
            .is_empty(),
        "HTML and JSON must not land on a Shelf"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn office_lock_files_and_tmp_are_not_ingested() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Office");
    let lock = app.dir.path().join("~$note.md");
    let tmp = app.dir.path().join("scratch.tmp");
    std::fs::write(&lock, "# Secret\n\nShould not be read.\n").unwrap();
    std::fs::write(&tmp, "tmp").unwrap();
    for path in [&lock, &tmp] {
        let job = imported_job(&shelf_id, path);
        larra::ingest::process_file(&app.ctx, &job)
            .await
            .expect("skip");
    }
    assert!(
        app.ctx
            .library
            .read()
            .unwrap()
            .documents(&shelf_id)
            .is_empty(),
        "lock and tmp files must not land on a Shelf"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_shelf_does_not_take_another_file() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Full");
    {
        let mut library = app.ctx.library.write().unwrap();
        for i in 0..larra::shelf::MAX_FILES_PER_SHELF {
            library.upsert_document(larra::types::DocumentMeta {
                id: format!("d_fill{i}"),
                shelf_id: shelf_id.clone(),
                source_id: larra::shelf::Shelf::IMPORTED_SOURCE.to_string(),
                source_type: larra::types::SourceType::Imported,
                path: format!("/tmp/fill{i}.md"),
                rel_path: format!("fill{i}.md"),
                file_name: format!("fill{i}.md"),
                format: "md".into(),
                size_bytes: 1,
                mtime_ms: 0,
                hash: "sha256:x".into(),
                status: DocStatus::Ready,
                error: None,
                passage_count: 1,
                pages: None,
                pii_total: 0,
                pii_categories: Default::default(),
                ocr: false,
                updated_at: "2026-01-01T00:00:00Z".into(),
                source_label: "Imported".into(),
            });
        }
    }
    let extra = app.dir.path().join("extra.md");
    std::fs::write(&extra, "should not be read").unwrap();
    let job = larra::ingest::ProcessJob {
        shelf_id: shelf_id.clone(),
        source_id: larra::shelf::Shelf::IMPORTED_SOURCE.to_string(),
        source_type: larra::types::SourceType::Imported,
        source_label: "Imported".into(),
        abs_path: extra,
        rel_path: "extra.md".into(),
        force: false,
        epoch: 0,
    };
    larra::ingest::process_file(&app.ctx, &job)
        .await
        .expect("skip when full");
    assert_eq!(
        app.ctx.library.read().unwrap().document_count(&shelf_id),
        larra::shelf::MAX_FILES_PER_SHELF
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn docx_office_note_becomes_ready() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Office");
    let meta = ingest_file(&app, &shelf_id, &fixture_office_note_docx(app.dir.path())).await;
    assert_eq!(
        meta.status,
        DocStatus::Ready,
        "docx error: {:?}",
        meta.error
    );
    assert!(!meta.ocr, "docx has a text layer");
    let extracted =
        std::fs::read_to_string(app.ctx.paths.extracted_path(&shelf_id, &meta.id)).unwrap();
    assert!(
        extracted.contains("18,000 EUR") || extracted.contains("ninety"),
        "docx text missing, got {extracted:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scanned_pdf_uses_ocr() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Scans");
    let meta = ingest_file(&app, &shelf_id, &fixture_scanned_pdf(app.dir.path())).await;
    assert_eq!(meta.status, DocStatus::Ready, "error: {:?}", meta.error);
    assert!(meta.ocr, "image-only PDF should go through OCR");
    let extracted =
        std::fs::read_to_string(app.ctx.paths.extracted_path(&shelf_id, &meta.id)).unwrap();
    assert!(
        extracted.to_uppercase().contains("REBOST"),
        "OCR should read the page, got {extracted:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn new_folder_scan_leaves_existing_files_alone() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Dump");
    let managed = app
        .ctx
        .library
        .read()
        .unwrap()
        .shelf(&shelf_id)
        .unwrap()
        .managed_path
        .clone();
    let old = managed.join("old.md");
    std::fs::write(&old, "# Old\n\nAlready on the Shelf.\n").unwrap();
    let first = ingest_file(&app, &shelf_id, &old).await;

    let dump = managed.join("dump");
    std::fs::create_dir(&dump).unwrap();
    std::fs::write(dump.join("a.md"), "# A\n\nNew file A body.\n").unwrap();
    std::fs::write(dump.join("b.md"), "# B\n\nNew file B body.\n").unwrap();

    let ingestor = larra::ingest::Ingestor::start(app.ctx.clone());
    let outcome = ingestor
        .queue_new_under(
            &shelf_id,
            larra::shelf::Shelf::IMPORTED_SOURCE,
            larra::types::SourceType::Imported,
            "Imported",
            &managed,
            &dump,
        )
        .await;
    assert_eq!(outcome.new_files, 2);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        let docs = app.ctx.library.read().unwrap().documents(&shelf_id);
        let new_ready = docs
            .iter()
            .filter(|d| {
                (d.file_name == "a.md" || d.file_name == "b.md") && d.status == DocStatus::Ready
            })
            .count();
        if new_ready == 2 {
            let old_meta = docs.iter().find(|d| d.file_name == "old.md").unwrap();
            assert_eq!(old_meta.updated_at, first.updated_at);
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "dump files did not become ready; docs={:?}",
                docs.iter()
                    .map(|d| (&d.file_name, d.status))
                    .collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn imported_job(shelf_id: &str, path: &std::path::Path) -> larra::ingest::ProcessJob {
    larra::ingest::ProcessJob {
        shelf_id: shelf_id.to_string(),
        source_id: larra::shelf::Shelf::IMPORTED_SOURCE.to_string(),
        source_type: larra::types::SourceType::Imported,
        source_label: "Imported".into(),
        abs_path: path.to_path_buf(),
        rel_path: path.file_name().unwrap().to_string_lossy().into_owned(),
        force: false,
        epoch: 0,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn process_file_does_not_resurrect_a_deleted_shelf() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Gone");
    let file = app.dir.path().join("note.md");
    std::fs::write(&file, "# Hello\n\nBody.\n").unwrap();
    {
        let mut library = app.ctx.library.write().unwrap();
        library.remove_shelf(&app.ctx.paths, &shelf_id).unwrap();
    }
    larra::ingest::process_file(&app.ctx, &imported_job(&shelf_id, &file))
        .await
        .expect("skip gone shelf");
    let library = app.ctx.library.read().unwrap();
    assert!(library.shelf(&shelf_id).is_none());
    assert!(library.documents(&shelf_id).is_empty());
    assert!(!app.ctx.paths.documents_registry(&shelf_id).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn process_file_skips_when_the_job_token_is_stale() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Still here");
    let file = app.dir.path().join("note.md");
    std::fs::write(&file, "# Hello\n\nBody.\n").unwrap();
    app.ctx.ingest_queue.cancel_shelf(&shelf_id);
    larra::ingest::process_file(&app.ctx, &imported_job(&shelf_id, &file))
        .await
        .expect("skip stale");
    assert!(app
        .ctx
        .library
        .read()
        .unwrap()
        .documents(&shelf_id)
        .is_empty());
    let mut job = imported_job(&shelf_id, &file);
    job.epoch = app.ctx.ingest_queue.stamp();
    larra::ingest::process_file(&app.ctx, &job)
        .await
        .expect("fresh token");
    let meta = app
        .ctx
        .library
        .read()
        .unwrap()
        .documents(&shelf_id)
        .into_iter()
        .next()
        .expect("fresh job should land");
    assert_eq!(meta.status, DocStatus::Ready);
}

#[tokio::test(flavor = "multi_thread")]
async fn process_file_skips_an_unlinked_folder() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Docs");
    let linked = app.dir.path().join("incoming");
    std::fs::create_dir_all(&linked).unwrap();
    let file = linked.join("note.md");
    std::fs::write(&file, "# Hello\n\nBody.\n").unwrap();
    let source_id = {
        let mut library = app.ctx.library.write().unwrap();
        library.add_linked_folder(&shelf_id, &linked).unwrap();
        let source_id = larra::ids::source_id(&linked.canonicalize().unwrap().to_string_lossy());
        library.remove_linked_folder(&shelf_id, &source_id).unwrap();
        source_id
    };
    let job = larra::ingest::ProcessJob {
        shelf_id: shelf_id.clone(),
        source_id: source_id.clone(),
        source_type: larra::types::SourceType::Linked,
        source_label: "incoming".into(),
        abs_path: file.clone(),
        rel_path: "note.md".into(),
        force: false,
        epoch: 0,
    };
    larra::ingest::process_file(&app.ctx, &job)
        .await
        .expect("skip unlinked");
    assert!(app
        .ctx
        .library
        .read()
        .unwrap()
        .documents(&shelf_id)
        .is_empty());
    assert!(!app.ctx.paths.documents_registry(&shelf_id).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_linked_root_keeps_documents() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Travel");
    let linked = app.dir.path().join("usb");
    std::fs::create_dir_all(&linked).unwrap();
    let file = linked.join("note.md");
    std::fs::write(&file, "# Hello\n\nKeep me when the volume is away.\n").unwrap();
    let source_id = {
        let mut library = app.ctx.library.write().unwrap();
        library.add_linked_folder(&shelf_id, &linked).unwrap();
        larra::ids::source_id(&linked.canonicalize().unwrap().to_string_lossy())
    };
    let job = larra::ingest::ProcessJob {
        shelf_id: shelf_id.clone(),
        source_id: source_id.clone(),
        source_type: larra::types::SourceType::Linked,
        source_label: "usb".into(),
        abs_path: file.clone(),
        rel_path: "note.md".into(),
        force: false,
        epoch: 0,
    };
    larra::ingest::process_file(&app.ctx, &job)
        .await
        .expect("ingest linked");
    let doc_id = larra::ids::document_id(&shelf_id, &source_id, "note.md");
    assert!(app.ctx.search.has_document(&doc_id));

    std::fs::remove_dir_all(&linked).unwrap();

    let ingestor = larra::ingest::Ingestor::start(app.ctx.clone());
    let _ = ingestor
        .sync_source(
            &shelf_id,
            &source_id,
            larra::types::SourceType::Linked,
            "usb",
            &linked,
            false,
        )
        .await;
    larra::ingest::process_file(&app.ctx, &job)
        .await
        .expect("offline process");

    let docs = app.ctx.library.read().unwrap().documents(&shelf_id);
    assert_eq!(docs.len(), 1, "offline linked root must not purge");
    assert_eq!(docs[0].id, doc_id);
    assert!(app.ctx.search.has_document(&doc_id));
}
