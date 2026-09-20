//! Shared helpers for integration tests: a headless Ctx over a temp dir and
//! realistic fixture files.

#![allow(dead_code)]

use larra::core::{Ctx, NoopEvents};
use larra::ingest::extract::ExtractorSettings;
use larra::paths::Paths;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct TestApp {
    pub ctx: Arc<Ctx>,
    pub dir: tempfile::TempDir,
}

pub fn test_app() -> TestApp {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(dir.path().join("appdata"));
    let tessdata = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("tessdata");
    let ctx = Ctx::new(
        paths,
        Arc::new(NoopEvents),
        ExtractorSettings {
            tessdata_dir: Some(tessdata),
            timeout_secs: 120,
            ..Default::default()
        },
    )
    .expect("ctx");
    TestApp { ctx, dir }
}

/// Create a shelf and return its id.
pub fn make_shelf(app: &TestApp, name: &str) -> String {
    let root = app.ctx.paths.library_dir();
    let mut library = app.ctx.library.write().unwrap();
    let shelf = library
        .create_shelf(&app.ctx.paths, name, &root)
        .expect("create shelf");
    shelf.id
}

/// Run the full pipeline synchronously for one file.
pub async fn ingest_file(
    app: &TestApp,
    shelf_id: &str,
    path: &Path,
) -> larra::types::DocumentMeta {
    let shelf = app
        .ctx
        .library
        .read()
        .unwrap()
        .shelf(shelf_id)
        .cloned()
        .expect("shelf");
    let rel = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap();
    let job = larra::ingest::ProcessJob {
        shelf_id: shelf_id.to_string(),
        source_id: larra::shelf::Shelf::IMPORTED_SOURCE.to_string(),
        source_type: larra::types::SourceType::Imported,
        source_label: "Imported".into(),
        abs_path: path.to_path_buf(),
        rel_path: rel,
        force: false,
        epoch: 0,
    };
    larra::ingest::process_file(&app.ctx, &job)
        .await
        .expect("pipeline");
    let doc_id = larra::ids::document_id(
        shelf_id,
        larra::shelf::Shelf::IMPORTED_SOURCE,
        &job.rel_path,
    );
    let _ = shelf;
    app.ctx
        .library
        .read()
        .unwrap()
        .document(shelf_id, &doc_id)
        .expect("document meta")
}

// Synthetic fixtures. Checksums are valid; names are not real people.

pub fn fixture_contract_md(dir: &Path) -> PathBuf {
    let path = dir.join("Framework Services Agreement - Northwind.md");
    std::fs::write(
        &path,
        r#"# Framework Services Agreement — Northwind Trading S.L.

## Object

This agreement covers the delivery of consulting services by the Provider to
Northwind Trading S.L. across all Spanish branches.

## Payment

Invoices are payable net 30 days to account ES7620770024003102575766.
The billing contact is billing@northwind.example.

## Confidentiality

Both parties keep all exchanged material strictly confidential for five years.

## Termination

Either party may terminate this agreement with ninety (90) days written
notice. In case of material breach, Northwind Trading S.L. may terminate with
immediate effect after a fifteen (15) day cure period.

## Governing law

Spanish law governs this agreement; courts of Madrid have jurisdiction.
"#,
    )
    .unwrap();
    path
}

pub fn fixture_invoice_md(dir: &Path) -> PathBuf {
    let path = dir.join("Invoice INV-2026-0042.md");
    std::fs::write(
        &path,
        r#"# Invoice INV-2026-0042

Client: Acme Logistics SL
Date: 2026-07-31
Amount due: 4,850.00 EUR
Due date: 2026-08-30

Concept: July consulting retainer, 22 days on site.
"#,
    )
    .unwrap();
    path
}

pub fn fixture_policies_ca_md(dir: &Path) -> PathBuf {
    let path = dir.join("Politica de vacances.md");
    std::fs::write(
        &path,
        r#"# Política de vacances i permisos

## Dies de vacances

Cada persona treballadora té dret a vint-i-tres (23) dies laborables de
vacances per any complet treballat. Les vacances es demanen amb un mínim de
quinze dies d'antelació al responsable d'equip.

## Permisos retribuïts

Es concedeixen permisos per matrimoni, naixement i trasllat de domicili
segons el conveni col·lectiu aplicable.
"#,
    )
    .unwrap();
    path
}

/// Payroll spreadsheet (sheet "July": 3 employees with NIF/IBAN/email) —
/// static fixture, checked in at tests/fixtures/.
pub fn fixture_payroll_xlsx(dir: &Path) -> PathBuf {
    let path = dir.join("Payroll July.xlsx");
    std::fs::write(&path, include_bytes!("../fixtures/payroll-july.xlsx")).unwrap();
    path
}

/// PDF with a native text layer ("18,000 EUR" on page 1) — static fixture.
pub fn fixture_brief_pdf(dir: &Path) -> PathBuf {
    let path = dir.join("Office move brief.pdf");
    std::fs::write(&path, include_bytes!("../fixtures/office-move-brief.pdf")).unwrap();
    path
}

pub fn fixture_policies_es_md(dir: &Path) -> PathBuf {
    let path = dir.join("Politica de vacaciones.md");
    std::fs::write(
        &path,
        r#"# Política de vacaciones y permisos

Cada trabajador tiene derecho a veintitrés (23) días laborables de
vacaciones por año completo trabajado. Las vacaciones se solicitan con
quince días de antelación al responsable de equipo.
"#,
    )
    .unwrap();
    path
}

pub fn fixture_clients_csv(dir: &Path) -> PathBuf {
    let path = dir.join("clients.csv");
    std::fs::write(
        &path,
        "name,iban,email\nAcme Logistics,ES7620770024003102575766,ops@acme.example\n",
    )
    .unwrap();
    path
}

pub fn fixture_notice_eml(dir: &Path) -> PathBuf {
    let path = dir.join("notice.eml");
    std::fs::write(
        &path,
        "From: legal@example.com\r\n\
Subject: Termination notice period\r\n\
\r\n\
Please remember the ninety (90) days written notice in the Northwind agreement.\r\n",
    )
    .unwrap();
    path
}

pub fn fixture_notice_html(dir: &Path) -> PathBuf {
    let path = dir.join("kitchen.html");
    std::fs::write(
        &path,
        "<!DOCTYPE html><html><body><h1>Kitchen</h1><p>The office kitchen is restocked every Tuesday morning.</p></body></html>\n",
    )
    .unwrap();
    path
}

pub fn fixture_office_note_docx(dir: &Path) -> PathBuf {
    let path = dir.join("Office note.docx");
    std::fs::write(&path, include_bytes!("../fixtures/office-note.docx")).unwrap();
    path
}

/// Image-only PDF (no text layer) so extraction must go through OCR.
pub fn fixture_scanned_pdf(dir: &Path) -> PathBuf {
    let path = dir.join("Scanned note.pdf");
    std::fs::write(&path, include_bytes!("../fixtures/scanned-larra.pdf")).unwrap();
    path
}
