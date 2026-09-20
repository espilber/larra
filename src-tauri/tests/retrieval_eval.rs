//! Retrieval eval suite — the ranking weights and gate constants are tuned
//! against these cases. Lexical, deterministic, multilingual.

mod common;

use common::*;
use larra::search::gate;
use tokio::sync::OnceCell;

struct Corpus {
    app: TestApp,
    shelf_id: String,
}

static CORPUS: OnceCell<Corpus> = OnceCell::const_new();

async fn corpus() -> &'static Corpus {
    CORPUS
        .get_or_init(|| async {
            let app = test_app();
            let shelf_id = make_shelf(&app, "Everything");
            for fixture in [
                fixture_contract_md(app.dir.path()),
                fixture_invoice_md(app.dir.path()),
                fixture_policies_ca_md(app.dir.path()),
                fixture_payroll_xlsx(app.dir.path()),
                fixture_brief_pdf(app.dir.path()),
            ] {
                ingest_file(&app, &shelf_id, &fixture).await;
            }
            Corpus { app, shelf_id }
        })
        .await
}

fn gated(corpus: &Corpus, query: &str) -> Vec<larra::types::SourcePassage> {
    let files: Vec<(String, String)> = corpus
        .app
        .ctx
        .library
        .read()
        .unwrap()
        .documents(&corpus.shelf_id)
        .into_iter()
        .map(|d| (d.id, d.file_name))
        .collect();
    let tokens = corpus.app.ctx.search.query_tokens(query);
    let (hits, named) = corpus
        .app
        .ctx
        .search
        .search_and_merge_named(query, &corpus.shelf_id, &files, 24)
        .expect("search");
    apply_gate(hits, &tokens, &named)
}

fn apply_gate(
    hits: Vec<larra::types::SourcePassage>,
    tokens: &[String],
    named: &[String],
) -> Vec<larra::types::SourcePassage> {
    gate::gate_passages(hits, tokens, named, gate::GateCaps::default(), false)
}

#[tokio::test(flavor = "multi_thread")]
async fn termination_question_finds_the_termination_section() {
    let corpus = corpus().await;
    let results = gated(corpus, "When can Northwind terminate our agreement?");
    assert!(!results.is_empty(), "gate must pass relevant passages");
    let top = &results[0];
    assert!(
        top.body.contains("terminate") || top.section.as_deref() == Some("Termination"),
        "top hit should be the termination clause, got section {:?}",
        top.section
    );
    assert_eq!(results[0].sid, "S1");
}

#[tokio::test(flavor = "multi_thread")]
async fn exact_invoice_number_ranks_first() {
    let corpus = corpus().await;
    let results = gated(corpus, "INV-2026-0042");
    assert!(!results.is_empty());
    assert!(
        results[0].title.contains("INV-2026-0042") || results[0].body.contains("INV-2026-0042"),
        "exact identifiers must stay searchable exactly"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn iban_is_searchable_as_one_token() {
    let corpus = corpus().await;
    let results = gated(corpus, "ES7620770024003102575766");
    assert!(!results.is_empty());
    assert!(results[0].body.contains("ES7620770024003102575766"));
}

#[tokio::test(flavor = "multi_thread")]
async fn typos_still_match_via_fuzzy() {
    let corpus = corpus().await;
    // "terminaton" (missing i), "agrement" (missing e)
    let results = gated(corpus, "terminaton of the agrement notice period");
    assert!(
        !results.is_empty(),
        "reasonable typos should still reach the contract"
    );
    assert!(
        results.iter().any(|r| r.title.contains("Northwind")),
        "expected the Northwind agreement among results"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn catalan_stemming_matches_inflections() {
    let corpus = corpus().await;
    // Query uses "dies" (plural) and "vacances"; document says "23 dies laborables de vacances".
    let results = gated(corpus, "Quants dies de vacances tenim per any?");
    assert!(
        !results.is_empty(),
        "Catalan question should hit the policy"
    );
    assert!(
        results[0].title.to_lowercase().contains("vacances")
            || results[0].body.contains("vint-i-tres"),
        "vacation policy should rank first, got {:?}",
        results[0].title
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn spanish_stemming_matches_inflections() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "HR-ES");
    ingest_file(&app, &shelf_id, &fixture_policies_es_md(app.dir.path())).await;
    let tokens = app.ctx.search.query_tokens("días de vacaciones por año");
    let hits = app
        .ctx
        .search
        .search_passages("días de vacaciones por año", &shelf_id, 24)
        .expect("search");
    let results = apply_gate(hits, &tokens, &[]);
    assert!(
        !results.is_empty(),
        "Spanish vacation policy should be found"
    );
    assert!(
        results[0].title.to_lowercase().contains("vacaciones")
            || results[0].body.contains("veintitrés"),
        "got {:?}",
        results[0].title
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_document_is_idempotent() {
    let app = test_app();
    app.ctx
        .search
        .remove_document("missing-doc")
        .expect("first remove");
    app.ctx
        .search
        .remove_document("missing-doc")
        .expect("second remove");
}

#[tokio::test(flavor = "multi_thread")]
async fn unrelated_queries_are_gated_out() {
    let corpus = corpus().await;
    let results = gated(corpus, "quantum entanglement in photosynthesis experiments");
    assert!(
        results.is_empty(),
        "nothing in the corpus is relevant; the gate must return empty, got {:?}",
        results.iter().map(|r| r.title.clone()).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn shelf_scoping_is_strict() {
    let app = test_app();
    let finance = make_shelf(&app, "Finance");
    let contracts = make_shelf(&app, "Contracts");
    ingest_file(&app, &contracts, &fixture_contract_md(app.dir.path())).await;

    // Searching the Finance shelf must not surface Contracts documents.
    let hits = app
        .ctx
        .search
        .search_passages("termination notice Northwind", &finance, 24)
        .expect("search");
    assert!(hits.is_empty(), "shelf scoping must be strict");
}

#[tokio::test(flavor = "multi_thread")]
async fn conversation_memory_is_searchable_and_skips_prompt_turns() {
    let app = test_app();
    let now = chrono::Utc::now();
    app.ctx
        .search
        .index_message(
            "t_old",
            "m_1",
            "user",
            "We agreed the office move budget is 18000 EUR",
            Some("en"),
            now,
        )
        .unwrap();
    app.ctx
        .search
        .index_message(
            "t_current",
            "m_2",
            "user",
            "The office move budget question again",
            Some("en"),
            now,
        )
        .unwrap();

    let skip = vec!["m_2".to_string()];
    let hits = app
        .ctx
        .search
        .search_messages("office move budget", None, None, &skip, 8)
        .unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter().all(|h| h.message_id != "m_2"),
        "turns already in the prompt must be skipped"
    );
    assert!(hits.iter().any(|h| h.thread_id == "t_old"));
}

#[tokio::test(flavor = "multi_thread")]
async fn budget_fitting_respects_the_measured_budget() {
    let corpus = corpus().await;
    let tokens = corpus
        .app
        .ctx
        .search
        .query_tokens("Northwind agreement payment termination");
    let hits = corpus
        .app
        .ctx
        .search
        .search_passages(
            "Northwind agreement payment termination",
            &corpus.shelf_id,
            24,
        )
        .unwrap();
    let passages = apply_gate(hits, &tokens, &[]);
    let fitted = gate::take_passages(passages, 1200);
    let total: usize = fitted.iter().map(|p| p.body.chars().count()).sum();
    assert!(
        total <= 1400,
        "budget fitting must bound context, got {total}"
    );
    assert!(
        !fitted.is_empty(),
        "at least one (truncated) passage survives"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn naming_a_file_retrieves_that_document() {
    let app = test_app();
    let shelf_id = make_shelf(&app, "Deal");
    ingest_file(&app, &shelf_id, &fixture_contract_md(app.dir.path())).await;
    let services = app
        .dir
        .path()
        .join("NORTHWIND - Services agreement Studio Lead (J.K) (DRAFT).md");
    std::fs::write(
        &services,
        r#"# SERVICES AGREEMENT FOR STUDIO LEAD

## Parties

The studio and the lead agree the recitals of this commercial relationship.

## Termination

If the lead leaves before the two-year stay period, the studio agreement applies.
"#,
    )
    .unwrap();
    let meta = ingest_file(&app, &shelf_id, &services).await;
    let query =
        "Check the important info from NORTHWIND - Services agreement Studio Lead (J.K) (DRAFT).md";
    let files: Vec<(String, String)> = app
        .ctx
        .library
        .read()
        .unwrap()
        .documents(&shelf_id)
        .into_iter()
        .map(|d| (d.id, d.file_name))
        .collect();
    let tokens = app.ctx.search.query_tokens(query);
    let (hits, named) = app
        .ctx
        .search
        .search_and_merge_named(query, &shelf_id, &files, 24)
        .expect("search");
    assert!(
        named.contains(&meta.id),
        "expected the named file, got {named:?}"
    );
    let results = apply_gate(hits, &tokens, &named);
    assert!(
        results.iter().any(|r| r.document_id == meta.id),
        "expected the named services file among gated hits"
    );
    assert!(
        results.iter().any(|r| r.body.contains("two-year stay")
            || r.body.contains("studio agreement")
            || r.body.contains("Studio Lead")),
        "expected services-agreement content"
    );
}
