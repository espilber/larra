//! Live model-catalog checks (network): Hugging Face search + download
//! resolution, Ollama best-effort merge. Run explicitly:
//!
//!   cargo test --test model_catalog -- --ignored
//!
//! Only catalog queries leave the machine, never a file from a Shelf.

use larra::engine::models::{self, MachineProfile};

fn profile() -> MachineProfile {
    MachineProfile {
        total_ram_bytes: 48 * 1024 * 1024 * 1024,
        cpu: "test".into(),
        accelerator: "Metal".into(),
        free_disk_bytes: 500 * 1024 * 1024 * 1024,
        process_arch: "test".into(),
        os_arch: "test".into(),
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!(
            "Larra/",
            env!("CARGO_PKG_VERSION"),
            " (private desktop AI)"
        ))
        .build()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network"]
async fn search_merges_catalogs_and_labels_fit() {
    let results = models::search_models(&client(), "Qwen3", &profile())
        .await
        .expect("search");
    assert!(!results.is_empty(), "expected Qwen3 results");
    let with_size = results.iter().filter(|r| r.size_bytes.is_some()).count();
    assert!(with_size >= 3, "sizes should resolve for most results");
    assert!(
        results.iter().any(|r| r.fits == Some(true)),
        "a 48 GB machine fits several Qwen3 builds"
    );
    assert!(
        results.iter().any(|r| r.downloads.unwrap_or(0) > 0),
        "Hugging Face publishes download counts"
    );
    for result in &results {
        assert!(!result.name.is_empty());
        assert!(!result.reference.is_empty());
        let blob = format!(
            "{} {} {}",
            result.reference,
            result.file.as_deref().unwrap_or(""),
            result.name
        )
        .to_ascii_lowercase();
        assert!(
            !blob.contains("1bit")
                && !blob.contains("packed_1bit")
                && !blob.contains("custom_")
                && !blob.contains("models-moved")
                && !blob.contains("modelsmoved")
                && !blob.contains("wan2")
                && !blob.contains("parakeet"),
            "custom packs, CI stubs, and non-chat AIs must stay hidden: {blob}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network"]
async fn search_looks_up_owner_repo_and_page_url() {
    for query in [
        "unsloth/Qwen3-0.6B-GGUF",
        "https://huggingface.co/unsloth/Qwen3-0.6B-GGUF",
    ] {
        let results = models::search_models(&client(), query, &profile())
            .await
            .expect("search");
        assert_eq!(
            results.first().map(|r| r.reference.as_str()),
            Some("unsloth/Qwen3-0.6B-GGUF"),
            "{query}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network"]
async fn resolve_picks_single_file_gguf_with_checksum() {
    let resolved = models::resolve_download(&client(), "huggingface", "unsloth/Qwen3-0.6B-GGUF")
        .await
        .expect("resolve");
    assert!(resolved.url.contains("/resolve/main/"));
    assert!(resolved.url.contains("download=true"));
    assert!(resolved.file_name.to_lowercase().ends_with(".gguf"));
    assert!(
        !resolved.file_name.contains("-of-"),
        "multi-part must be skipped"
    );
    assert!(resolved.size.unwrap_or(0) > 100_000_000);
    assert!(
        resolved.sha256.map(|s| s.len() == 64).unwrap_or(false),
        "HF LFS oid provides a verifiable sha256"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "network"]
async fn ollama_registry_resolves_model_layer() {
    // Best effort: if the registry is unreachable this test is inconclusive,
    // but when reachable the blob must be a GGUF-sized model layer.
    match models::resolve_download(&client(), "ollama", "qwen3").await {
        Ok(resolved) => {
            assert!(resolved.url.contains("registry.ollama.ai"));
            assert!(resolved.size.unwrap_or(0) > 100_000_000);
            assert!(resolved.sha256.is_some());
        }
        Err(error) => {
            eprintln!("ollama registry unreachable (tolerated): {error:#}");
        }
    }
}
