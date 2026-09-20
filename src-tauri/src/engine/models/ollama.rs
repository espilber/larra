//! Ollama library search (HTML scrape) and registry manifests.

use anyhow::{anyhow, Result};
use serde::Deserialize;

use super::{is_hidden_catalog_name, normalize_model_key, ModelSearchResult};
use crate::engine::catalog::MachineProfile;

#[derive(Deserialize)]
struct OllamaManifest {
    #[serde(default)]
    layers: Vec<OllamaLayer>,
}

#[derive(Deserialize, Clone)]
pub(super) struct OllamaLayer {
    #[serde(rename = "mediaType")]
    media_type: String,
    pub(super) digest: String,
    #[serde(default)]
    pub(super) size: Option<u64>,
}

/// Resolve one Ollama library model via its registry manifest.
pub(super) async fn ollama_manifest(
    client: &reqwest::Client,
    name: &str,
) -> Result<(OllamaLayer, Option<String>)> {
    let url = format!("https://registry.ollama.ai/v2/library/{name}/manifests/latest");
    let manifest: OllamaManifest = client
        .get(&url)
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.v2+json",
        )
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let model_layer = manifest
        .layers
        .iter()
        .find(|l| l.media_type == "application/vnd.ollama.image.model")
        .cloned()
        .ok_or_else(|| anyhow!("no model layer for {name}"))?;
    let license_digest = manifest
        .layers
        .iter()
        .find(|l| l.media_type == "application/vnd.ollama.image.license")
        .map(|l| l.digest.clone());

    let license = if let Some(digest) = license_digest {
        let blob_url = format!("https://registry.ollama.ai/v2/library/{name}/blobs/{digest}");
        match client.get(&blob_url).send().await {
            Ok(response) => response
                .text()
                .await
                .ok()
                .map(|text| classify_license(&text)),
            Err(_) => None,
        }
    } else {
        None
    };
    Ok((model_layer, license))
}

fn classify_license(text: &str) -> String {
    let head: String = text.chars().take(400).collect::<String>().to_lowercase();
    if head.contains("apache license") {
        "Apache-2.0".into()
    } else if head.contains("mit license") {
        "MIT".into()
    } else if head.contains("gemma") {
        "Gemma".into()
    } else if head.contains("llama 3") {
        "Llama 3".into()
    } else if head.contains("qwen") {
        "Qwen".into()
    } else {
        "See model page".into()
    }
}

/// Search the Ollama library (best effort — the library has no official
/// API, so this parses the search page and degrades gracefully).
/// Hits are Ollama's packaging, not original labs, so they are never Official.
pub(super) async fn search_ollama(
    client: &reqwest::Client,
    query: &str,
    profile: &MachineProfile,
) -> Vec<ModelSearchResult> {
    let Ok(response) = client
        .get("https://ollama.com/search")
        .query(&[("q", query)])
        .send()
        .await
    else {
        return Vec::new();
    };
    let Ok(html) = response.text().await else {
        return Vec::new();
    };
    let re = regex::Regex::new(r#"href="/library/([a-z0-9][a-z0-9._-]*)""#).unwrap();
    let mut names: Vec<String> = Vec::new();
    for capture in re.captures_iter(&html) {
        let name = capture[1].to_string();
        if !names.contains(&name) {
            names.push(name);
        }
        if names.len() >= 6 {
            break;
        }
    }
    let budget = profile.model_budget_bytes();
    let mut results = Vec::new();
    for name in names {
        if is_hidden_catalog_name(&name) {
            continue;
        }
        match ollama_manifest(client, &name).await {
            Ok((layer, license)) => {
                let fits = layer.size.map(|s| profile.runtime_need_bytes(s) <= budget);
                results.push(ModelSearchResult {
                    id: normalize_model_key(&name),
                    name: name.replace(['-', '_'], " "),
                    source: "ollama".into(),
                    reference: name.clone(),
                    file: Some(format!("{}.gguf", name.replace([':', '/'], "-"))),
                    size_bytes: layer.size,
                    license,
                    released: None,
                    downloads: None,
                    publisher: None,
                    official: false,
                    fits,
                });
            }
            Err(error) => {
                log::debug!("ollama manifest {name}: {error:#}");
            }
        }
    }
    results
}
