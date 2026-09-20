//! Hugging Face Hub search and file listing.

use anyhow::Result;
use futures_util::future::join_all;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use url::Url;

use super::{
    display_name_from_repo, is_hidden_catalog_name, keep_explore_hit, normalize_model_key,
    pick_gguf, released_newest_first, validate_reference, ModelSearchResult, HF_SIZE_CAP,
};
use crate::engine::catalog::MachineProfile;

#[derive(Deserialize)]
pub(super) struct HfModel {
    /// The API sends both `id` and `modelId` — keep them as separate
    /// optional fields (an alias would trip serde's duplicate-field check).
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default, rename = "modelId")]
    pub(super) model_id: Option<String>,
    #[serde(default)]
    pub(super) author: Option<String>,
    #[serde(default)]
    pub(super) tags: Vec<String>,
    #[serde(default, rename = "pipeline_tag")]
    pub(super) pipeline_tag: Option<String>,
    #[serde(default, rename = "createdAt")]
    pub(super) created_at: Option<String>,
    #[serde(default, rename = "lastModified")]
    pub(super) last_modified: Option<String>,
    #[serde(default)]
    pub(super) downloads: Option<u64>,
    #[serde(default, rename = "downloadsAllTime")]
    pub(super) downloads_all_time: Option<u64>,
    #[serde(default)]
    pub(super) siblings: Option<Vec<HfSibling>>,
}

impl HfModel {
    fn repo_id(&self) -> Option<String> {
        self.model_id.clone().or_else(|| self.id.clone())
    }

    fn download_count(&self) -> Option<u64> {
        self.downloads_all_time.or(self.downloads)
    }

    fn released_on(&self) -> Option<String> {
        self.created_at
            .as_deref()
            .or(self.last_modified.as_deref())
            .map(|d| d.chars().take(10).collect())
    }
}

#[derive(Deserialize)]
pub(super) struct HfSibling {
    rfilename: String,
}

#[derive(Deserialize)]
pub(super) struct HfTreeEntry {
    pub(super) path: String,
    #[serde(default)]
    pub(super) size: Option<u64>,
    #[serde(default)]
    pub(super) lfs: Option<HfLfs>,
}

#[derive(Deserialize)]
pub(super) struct HfLfs {
    #[serde(default)]
    pub(super) size: Option<u64>,
    /// SHA-256 of the file — used to verify model downloads.
    #[serde(default)]
    pub(super) oid: Option<String>,
}

fn is_hf_namespace(value: &str) -> bool {
    let value = value.trim();
    (2..=64).contains(&value.len())
        && value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Hub paths that are not model repos (`/datasets/…`, `/spaces/…`).
const HF_NON_MODEL_ROOTS: &[&str] = &[
    "api",
    "blog",
    "chat",
    "collections",
    "datasets",
    "docs",
    "join",
    "learn",
    "login",
    "metrics",
    "models",
    "organizations",
    "papers",
    "pricing",
    "settings",
    "spaces",
    "tasks",
];

fn strip_query_noise(query: &str) -> &str {
    query
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '<' | '>' | '\u{201c}' | '\u{201d}'))
        .trim()
}

fn is_hf_hub_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "huggingface.co" | "www.huggingface.co" | "hf.co" | "www.hf.co"
    )
}

fn looks_like_hf_url(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();
    lower.contains("://")
        || lower.starts_with("huggingface.co/")
        || lower.starts_with("www.huggingface.co/")
        || lower.starts_with("hf.co/")
        || lower.starts_with("www.hf.co/")
}

fn hf_repo_id(owner: &str, repo: &str) -> Option<String> {
    if HF_NON_MODEL_ROOTS
        .iter()
        .any(|root| root.eq_ignore_ascii_case(owner))
    {
        return None;
    }
    let reference = format!("{owner}/{repo}");
    validate_reference("huggingface", &reference)
        .ok()
        .map(|_| reference)
}

fn parse_hf_url_repo(query: &str) -> Option<String> {
    let parsed = Url::parse(query)
        .ok()
        .or_else(|| Url::parse(&format!("https://{query}")).ok())?;
    if !is_hf_hub_host(parsed.host_str()?) {
        return None;
    }
    let mut segments = parsed.path_segments()?;
    let owner = segments.next()?;
    let repo = segments.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    hf_repo_id(owner, repo)
}

/// Hugging Face `owner/repo`, or a huggingface.co / hf.co model page URL.
pub(super) fn parse_hf_repo_query(query: &str) -> Option<String> {
    let query = strip_query_noise(query);
    if query.is_empty() {
        return None;
    }
    if looks_like_hf_url(query) {
        return parse_hf_url_repo(query);
    }
    let (owner, repo) = query.split_once('/')?;
    if repo.contains('/') {
        return None;
    }
    hf_repo_id(owner, repo)
}

/// `Qwen3` / `Qwen2.5` → `Qwen`, so an author= query can hit the maker's org.
fn strip_trailing_version(query: &str) -> String {
    query
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
        .to_string()
}

pub(super) fn publisher_guesses(query: &str) -> Vec<String> {
    let query = query.trim();
    if let Some(repo) = parse_hf_repo_query(query) {
        let owner = repo.split('/').next().unwrap_or("");
        if is_hf_namespace(owner) {
            return vec![owner.to_string()];
        }
    }
    let mut names = Vec::new();
    if is_hf_namespace(query) {
        names.push(query.to_string());
    }
    let stripped = strip_trailing_version(query);
    if is_hf_namespace(&stripped) && !stripped.eq_ignore_ascii_case(query) {
        names.push(stripped);
    }
    names
}

/// Hub namespaces of labs that trained the original weights.
/// Quantizers, forks, and abliterations are never Official.
/// Ollama library names have no maker handle; those hits stay unofficial.
const ORIGINAL_MAKERS: &[&str] = &[
    "01-ai",
    "ai21labs",
    "allenai",
    "apple",
    "baichuan-inc",
    "baidu",
    "bigcode",
    "bigscience",
    "BlinkDL",
    "ByteDance-Seed",
    "CohereLabs",
    "deepseek-ai",
    "EleutherAI",
    "EssentialAI",
    "facebook",
    "google",
    "HuggingFaceTB",
    "ibm-granite",
    "inclusionAI",
    "internlm",
    "LGAI-EXAONE",
    "LiquidAI",
    "meituan-longcat",
    "meta-llama",
    "meta-models",
    "microsoft",
    "MiniMaxAI",
    "mistralai",
    "moonshotai",
    "mosaicml",
    "naver-hyperclovax",
    "nvidia",
    "openai",
    "openbmb",
    "ornith-ai",
    "Qwen",
    "RWKV",
    "ServiceNow-AI",
    "Snowflake",
    "stabilityai",
    "stepfun-ai",
    "tencent",
    "THUDM",
    "tiiuae",
    "TinyLlama",
    "upstage",
    "Writer",
    "xai-org",
    "XiaomiMiMo",
    "zai-org",
];

pub(super) fn is_original_maker(name: &str) -> bool {
    ORIGINAL_MAKERS
        .iter()
        .any(|maker| maker.eq_ignore_ascii_case(name))
}

pub(super) fn publisher_namespace(author: Option<&str>, repo: &str) -> Option<String> {
    if let Some(author) = author.map(str::trim) {
        if !author.is_empty() {
            return Some(author.to_string());
        }
    }
    let owner = repo.split('/').next().unwrap_or("").trim();
    (!owner.is_empty()).then(|| owner.to_string())
}

pub(super) fn author_matches_search(author: &str, query: &str) -> bool {
    let author = author.to_ascii_lowercase();
    let query = query.trim().to_ascii_lowercase();
    if query.len() < 2 {
        return false;
    }
    if author == query {
        return true;
    }
    // Search "Qwen3" should still treat org "Qwen" as the maker.
    if query.starts_with(&author) && author.len() >= 3 {
        return true;
    }
    if author.starts_with(&query) && query.len() >= 3 {
        return true;
    }
    // Search "llama" vs org "meta-llama".
    query.len() >= 4 && author.contains(&query)
}

pub(super) fn base_model_owner(tag: &str) -> Option<&str> {
    let rest = tag.strip_prefix("base_model:")?;
    // Skip Hub prefixes like quantized:, finetune:, adapter:.
    if rest.contains(':') {
        return None;
    }
    let owner = rest.split('/').next()?;
    (!owner.is_empty()).then_some(owner)
}

fn majority_base_model_owner(models: &[HfModel]) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for model in models {
        for tag in &model.tags {
            if let Some(owner) = base_model_owner(tag) {
                *counts.entry(owner.to_string()).or_insert(0) += 1;
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .max_by_key(|(_, n)| *n)
        .map(|(owner, _)| owner)
}

fn author_count(models: &[HfModel], name: &str) -> usize {
    models
        .iter()
        .filter(|model| {
            model
                .author
                .as_deref()
                .is_some_and(|author| author.eq_ignore_ascii_case(name))
        })
        .count()
}

fn base_model_count(models: &[HfModel], name: &str) -> usize {
    models
        .iter()
        .flat_map(|model| model.tags.iter())
        .filter_map(|tag| base_model_owner(tag))
        .filter(|owner| owner.eq_ignore_ascii_case(name))
        .count()
}

pub(super) fn official_namespaces(query: &str, models: &[HfModel]) -> Vec<String> {
    let mut candidates = publisher_guesses(query);
    for model in models {
        if let Some(author) = &model.author {
            if author_matches_search(author, query)
                && !candidates.iter().any(|n| n.eq_ignore_ascii_case(author))
            {
                candidates.push(author.clone());
            }
        }
    }
    let strong: Vec<String> = candidates
        .iter()
        .filter(|name| author_count(models, name) >= 2 || base_model_count(models, name) >= 2)
        .cloned()
        .collect();
    let inferred = if !strong.is_empty() {
        strong
    } else if let Some(owner) = majority_base_model_owner(models) {
        vec![owner]
    } else {
        candidates
            .into_iter()
            .filter(|name| author_count(models, name) > 0)
            .collect()
    };
    inferred
        .into_iter()
        .filter(|name| is_original_maker(name))
        .collect()
}

fn dedup_models(models: &mut Vec<HfModel>) {
    let mut seen = HashSet::new();
    models.retain(|model| {
        model
            .repo_id()
            .is_some_and(|id| seen.insert(id.to_ascii_lowercase()))
    });
}

const HF_EXPAND: &[&str] = &[
    "author",
    "createdAt",
    "downloads",
    "downloadsAllTime",
    "pipeline_tag",
    "siblings",
    "tags",
];

/// Hugging Face Hub list endpoint. Empty on network/HTTP errors so search
/// can still return the other catalog.
#[derive(Clone, Copy)]
enum HfListSort {
    Created,
    Downloads,
}

async fn hf_list_models(
    client: &reqwest::Client,
    search: Option<&str>,
    author: Option<&str>,
    limit: u32,
    sort: HfListSort,
    task: Option<&str>,
) -> Vec<HfModel> {
    let mut query: Vec<(&str, String)> =
        vec![("filter", "gguf".into()), ("limit", limit.to_string())];
    if let Some(task) = task {
        query.push(("filter", task.to_string()));
    }
    if let Some(search) = search {
        query.push(("search", search.to_string()));
    }
    if let Some(author) = author {
        query.push(("author", author.to_string()));
    }
    match sort {
        HfListSort::Created => {
            query.push(("sort", "createdAt".into()));
            query.push(("direction", "-1".into()));
        }
        HfListSort::Downloads => {
            query.push(("sort", "downloads".into()));
            query.push(("direction", "-1".into()));
        }
    }
    for field in HF_EXPAND {
        query.push(("expand", (*field).into()));
    }
    let response = match client
        .get("https://huggingface.co/api/models")
        .query(&query)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            log::debug!("hugging face list: {error:#}");
            return Vec::new();
        }
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    response.json().await.unwrap_or_default()
}

async fn hf_get_model(client: &reqwest::Client, repo: &str) -> Option<HfModel> {
    if validate_reference("huggingface", repo).is_err() {
        return None;
    }
    let mut query: Vec<(&str, &str)> = Vec::new();
    for field in HF_EXPAND {
        query.push(("expand", field));
    }
    let response = match client
        .get(format!("https://huggingface.co/api/models/{repo}"))
        .query(&query)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            log::debug!("hugging face model {repo}: {error:#}");
            return None;
        }
    };
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

async fn hf_models_for_repo(client: &reqwest::Client, repo: &str) -> Vec<HfModel> {
    let (owner, name) = repo.split_once('/').unwrap_or((repo, repo));
    let (exact, search, author) = tokio::join!(
        hf_get_model(client, repo),
        hf_list_models(client, Some(name), None, 60, HfListSort::Downloads, None),
        hf_list_models(client, None, Some(owner), 20, HfListSort::Created, None),
    );
    let mut models = Vec::new();
    if let Some(model) = exact {
        models.push(model);
    }
    models.extend(search);
    models.extend(author);
    dedup_models(&mut models);
    models
}

async fn hf_models_for_query(client: &reqwest::Client, query: &str) -> Vec<HfModel> {
    let query = query.trim();
    if let Some(repo) = parse_hf_repo_query(query) {
        return hf_models_for_repo(client, &repo).await;
    }
    if query.is_empty() {
        let (generation, vision) = tokio::join!(
            hf_list_models(
                client,
                None,
                None,
                80,
                HfListSort::Downloads,
                Some("text-generation"),
            ),
            hf_list_models(
                client,
                None,
                None,
                40,
                HfListSort::Downloads,
                Some("image-text-to-text"),
            ),
        );
        let mut models = generation;
        models.extend(vision);
        dedup_models(&mut models);
        return models;
    }
    let guesses = publisher_guesses(query);
    let author_fetches = guesses.iter().map(|guess| {
        let client = client.clone();
        let author = guess.clone();
        let search = if guess.eq_ignore_ascii_case(query) {
            None
        } else {
            Some(query.to_string())
        };
        async move {
            hf_list_models(
                &client,
                search.as_deref(),
                Some(&author),
                20,
                HfListSort::Created,
                None,
            )
            .await
        }
    });
    let (mut models, author_lists) = tokio::join!(
        hf_list_models(client, Some(query), None, 60, HfListSort::Downloads, None),
        join_all(author_fetches)
    );
    for list in author_lists {
        models.extend(list);
    }
    dedup_models(&mut models);

    let namespaces = official_namespaces(query, &models);
    let missing: Vec<String> = namespaces
        .iter()
        .filter(|name| author_count(&models, name) < 3)
        .cloned()
        .collect();
    if !missing.is_empty() {
        let extra = join_all(missing.into_iter().map(|name| {
            let client = client.clone();
            let search = if name.eq_ignore_ascii_case(query) {
                None
            } else {
                Some(query.to_string())
            };
            async move {
                hf_list_models(
                    &client,
                    search.as_deref(),
                    Some(&name),
                    20,
                    HfListSort::Created,
                    None,
                )
                .await
            }
        }))
        .await;
        for list in extra {
            models.extend(list);
        }
        dedup_models(&mut models);
    }
    models
}

async fn gguf_file_size(client: &reqwest::Client, repo: &str, file: &str) -> Option<u64> {
    let response = client
        .get(format!(
            "https://huggingface.co/api/models/{repo}/tree/main"
        ))
        .query(&[("recursive", "true")])
        .send()
        .await
        .ok()?;
    let entries: Vec<HfTreeEntry> = response.json().await.ok()?;
    entries
        .iter()
        .find(|entry| entry.path == file)
        .and_then(|entry| entry.size.or(entry.lfs.as_ref().and_then(|lfs| lfs.size)))
}

/// Search the Hugging Face catalog for GGUF builds.
pub(super) async fn search_huggingface(
    client: &reqwest::Client,
    query: &str,
    profile: &MachineProfile,
) -> Result<Vec<ModelSearchResult>> {
    let models = hf_models_for_query(client, query).await;
    let mut results = Vec::new();
    for model in models {
        let Some(repo) = model.repo_id() else {
            continue;
        };
        if is_hidden_catalog_name(&repo) {
            continue;
        }
        let files: Vec<(String, Option<u64>)> = model
            .siblings
            .iter()
            .flatten()
            .map(|s| (s.rfilename.clone(), None))
            .collect();
        if !keep_explore_hit(
            query,
            &repo,
            model.pipeline_tag.as_deref(),
            &model.tags,
            &files,
        ) {
            continue;
        }
        let Some((file, _)) = pick_gguf(&files) else {
            continue;
        };
        let license = model
            .tags
            .iter()
            .find_map(|t| t.strip_prefix("license:"))
            .map(|l| l.to_string());
        let publisher = publisher_namespace(model.author.as_deref(), &repo);
        let official = publisher.as_deref().is_some_and(is_original_maker);
        results.push(ModelSearchResult {
            id: normalize_model_key(&repo),
            name: display_name_from_repo(&repo),
            source: "huggingface".into(),
            reference: repo,
            file: Some(file),
            size_bytes: None,
            license,
            released: model.released_on(),
            downloads: model.download_count(),
            publisher,
            official,
            fits: None,
        });
    }
    results.sort_by(|a, b| {
        b.downloads
            .unwrap_or(0)
            .cmp(&a.downloads.unwrap_or(0))
            .then_with(|| released_newest_first(a.released.as_deref(), b.released.as_deref()))
    });
    results.truncate(HF_SIZE_CAP);

    let budget = profile.model_budget_bytes();
    let sizes = join_all(results.iter().map(|result| {
        let client = client.clone();
        let repo = result.reference.clone();
        let file = result.file.clone();
        async move {
            match file {
                Some(file) => gguf_file_size(&client, &repo, &file).await,
                None => None,
            }
        }
    }))
    .await;
    for (result, size_bytes) in results.iter_mut().zip(sizes) {
        result.size_bytes = size_bytes;
        result.fits = size_bytes.map(|size| profile.runtime_need_bytes(size) <= budget);
    }
    Ok(results)
}
