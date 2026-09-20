//! Hugging Face and Ollama search, and GGUF install resolution.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::cmp::Ordering;

pub use crate::engine::catalog::{
    recommend, runtime_need_bytes, smaller_alternatives, uninstalled_suggestions, MachineProfile,
    Recommendation,
};

mod hf;
mod ollama;

use hf::{parse_hf_repo_query, search_huggingface, HfTreeEntry};
use ollama::{ollama_manifest, search_ollama};

/// Quantization preference for automatic file selection.
const QUANT_PREFERENCE: &[&str] = &[
    "Q4_K_M", "Q4_K_XL", "Q4_K_S", "IQ4_XS", "Q5_K_M", "Q4_0", "Q5_0", "Q6_K", "Q8_0", "F16",
    "BF16",
];

/// How many catalog hits to return after ranking (Explore paginates these).
const HF_RESULT_LIMIT: usize = 75;
/// Resolve sizes for this many Hugging Face hits before merge.
const HF_SIZE_CAP: usize = 80;
const RECENT_DAYS: i64 = 90;

/// Names that signal a file/repo llama-server can't chat with.
fn is_unusable_artifact(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "mmproj",
        "embedding",
        "embed-",
        "reranker",
        "rerank",
        "whisper",
        "clip",
        "vae",
        "encoder",
    ]
    .iter()
    .any(|t| lower.contains(t))
}

/// Research packs that use unofficial `custom_*` tensor types.
fn is_custom_quant_artifact(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("packed_1bit") || lower.contains("packed-1bit") || lower.contains("1-bit") {
        return true;
    }
    if lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|part| part.starts_with("custom_"))
    {
        return true;
    }
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == "1bit")
}

/// CI stubs and moved-weight placeholders (e.g. ggml-org/models-moved).
fn is_stub_catalog_name(name: &str) -> bool {
    let compact: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.contains("modelsmoved")
}

fn is_hidden_catalog_name(name: &str) -> bool {
    is_unusable_artifact(name) || is_custom_quant_artifact(name) || is_stub_catalog_name(name)
}

/// Hub task for a chat / document AI. `conversational` only means a chat template.
const TEXT_GENERATION: &str = "text-generation";
const IMAGE_TEXT_TO_TEXT: &str = "image-text-to-text";

/// Typed query tokens that mean “show specialists” (ocr / coder).
const SPECIALIST_QUERY_TOKENS: &[&str] = &["ocr", "coder", "code"];

/// Hub tag tokens for OCR or coding specialists. Not product names.
const SPECIALIST_TAG_TOKENS: &[&str] = &["ocr", "coder", "pdf", "layout"];

fn task_eq(value: &str, expected: &str) -> bool {
    value.eq_ignore_ascii_case(expected)
}

fn alnum_tokens(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
}

fn has_task(pipeline_tag: Option<&str>, tags: &[String], task: &str) -> bool {
    pipeline_tag.is_some_and(|tag| task_eq(tag, task)) || tags.iter().any(|tag| task_eq(tag, task))
}

fn is_text_generation(pipeline_tag: Option<&str>, tags: &[String]) -> bool {
    has_task(pipeline_tag, tags, TEXT_GENERATION)
}

fn is_general_vision_chat(pipeline_tag: Option<&str>, tags: &[String]) -> bool {
    has_task(pipeline_tag, tags, IMAGE_TEXT_TO_TEXT) && !has_specialist_tag(tags)
}

fn is_general_chat(pipeline_tag: Option<&str>, tags: &[String]) -> bool {
    is_text_generation(pipeline_tag, tags) || is_general_vision_chat(pipeline_tag, tags)
}

fn has_projector_sibling(files: &[(String, Option<u64>)]) -> bool {
    files
        .iter()
        .any(|(name, _)| name.to_ascii_lowercase().contains("mmproj"))
}

/// Typed Explore query is asking for a specialist (ocr, coder), not general chat.
fn query_wants_specialist(query: &str) -> bool {
    alnum_tokens(query).any(|token| SPECIALIST_QUERY_TOKENS.contains(&token.as_str()))
}

fn has_specialist_tag(tags: &[String]) -> bool {
    tags.iter()
        .any(|tag| alnum_tokens(tag).any(|token| SPECIALIST_TAG_TOKENS.contains(&token.as_str())))
}

/// Empty browse: general chat only. A query with ocr/coder/code can include specialists.
fn include_explore_hit(
    query: &str,
    pipeline_tag: Option<&str>,
    tags: &[String],
    files: &[(String, Option<u64>)],
) -> bool {
    if query_wants_specialist(query) {
        return true;
    }
    if has_specialist_tag(tags) {
        return false;
    }
    if has_projector_sibling(files) && !is_general_chat(pipeline_tag, tags) {
        return false;
    }
    is_general_chat(pipeline_tag, tags)
}

/// A pasted `owner/repo` is kept even when browse would hide specialists.
fn keep_explore_hit(
    query: &str,
    repo: &str,
    pipeline_tag: Option<&str>,
    tags: &[String],
    files: &[(String, Option<u64>)],
) -> bool {
    if parse_hf_repo_query(query).is_some_and(|exact| exact.eq_ignore_ascii_case(repo)) {
        return true;
    }
    include_explore_hit(query, pipeline_tag, tags, files)
}

/// Pick the best single-file GGUF from a repo file listing.
pub fn pick_gguf(files: &[(String, Option<u64>)]) -> Option<(String, Option<u64>)> {
    let candidates: Vec<&(String, Option<u64>)> = files
        .iter()
        .filter(|(name, _)| name.to_lowercase().ends_with(".gguf"))
        .filter(|(name, _)| !is_hidden_catalog_name(name))
        // Multi-part files (…-00001-of-00003.gguf) need merging — skip.
        .filter(|(name, _)| !name.contains("-of-"))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    for quant in QUANT_PREFERENCE {
        if let Some(hit) = candidates
            .iter()
            .find(|(name, _)| name.to_uppercase().contains(quant))
        {
            return Some((*hit).clone());
        }
    }
    Some(candidates[0].clone())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSearchResult {
    /// Merge key (normalized base name).
    pub id: String,
    /// Display name, e.g. "Muse Glimmer 7B".
    pub name: String,
    /// "huggingface" | "ollama" | "huggingface+ollama"
    pub source: String,
    /// HF repo id or Ollama library name.
    pub reference: String,
    /// Chosen GGUF file, when known.
    pub file: Option<String>,
    pub size_bytes: Option<u64>,
    pub license: Option<String>,
    /// Repo creation date (`YYYY-MM-DD`), from Hugging Face `createdAt`.
    pub released: Option<String>,
    /// Hugging Face all-time downloads when the Hub publishes them.
    pub downloads: Option<u64>,
    /// Hugging Face namespace (the publisher), e.g. "Qwen".
    pub publisher: Option<String>,
    /// True when the publisher is an original-lab Hugging Face namespace.
    pub official: bool,
    /// Whether it fits this computer (None = size unknown).
    pub fits: Option<bool>,
}

fn normalize_model_key(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    let base = base
        .trim_end_matches(".gguf")
        .replace("-gguf", "")
        .replace("_gguf", "");
    base.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn display_name_from_repo(repo: &str) -> String {
    let base = repo.rsplit('/').next().unwrap_or(repo);
    base.replace("-GGUF", "")
        .replace("-gguf", "")
        .replace(['-', '_'], " ")
        .trim()
        .to_string()
}

fn parse_released_on(value: &str) -> Option<chrono::NaiveDate> {
    let day = value.chars().take(10).collect::<String>();
    chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
        .ok()
        .or_else(|| {
            if day.len() == 7 {
                chrono::NaiveDate::parse_from_str(&format!("{day}-01"), "%Y-%m-%d").ok()
            } else {
                None
            }
        })
}

fn recency_score(released: Option<&str>, today: chrono::NaiveDate) -> i64 {
    let Some(date) = released.and_then(parse_released_on) else {
        return 0;
    };
    let days = today.signed_duration_since(date).num_days();
    if !(0..=365).contains(&days) {
        return 0;
    }
    if days <= RECENT_DAYS {
        100 - days / 2
    } else if days <= 180 {
        42
    } else {
        16
    }
}

fn usage_score(size_bytes: Option<u64>, budget: u64) -> i64 {
    let Some(size) = size_bytes else {
        return 32;
    };
    if budget == 0 {
        return 32;
    }
    let need = runtime_need_bytes(size);
    if need > budget {
        return 0;
    }
    let ratio = need as f64 / budget as f64;
    if (0.45..=0.85).contains(&ratio) {
        100
    } else if ratio > 0.85 {
        78
    } else if ratio >= 0.30 {
        68
    } else if ratio >= 0.15 {
        44
    } else {
        22
    }
}

fn download_score(downloads: Option<u64>) -> i64 {
    let n = downloads.unwrap_or(0) as f64;
    (((n + 1.0).log10() * 8.0).round() as i64).min(48)
}

fn official_score(official: bool) -> i64 {
    if official {
        48
    } else {
        0
    }
}

fn fit_score(fits: Option<bool>) -> i64 {
    match fits {
        Some(true) => 120,
        None => 28,
        Some(false) => 0,
    }
}

/// Mix of fit, recency, how well the file uses this machine, Official, then downloads.
fn best_score(result: &ModelSearchResult, today: chrono::NaiveDate, budget: u64) -> i64 {
    if result.fits == Some(false) {
        return download_score(result.downloads);
    }
    fit_score(result.fits)
        + recency_score(result.released.as_deref(), today)
        + usage_score(result.size_bytes, budget)
        + official_score(result.official)
        + download_score(result.downloads)
}

pub(super) fn released_newest_first(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Default Explore order: fit, recent, good use of memory, Official, downloads.
/// The UI keeps this order for Best. Other sorts happen in the webview.
fn rank_search_results(results: &mut [ModelSearchResult], budget: u64) {
    rank_search_results_on(results, chrono::Utc::now().date_naive(), budget);
}

fn rank_search_results_on(
    results: &mut [ModelSearchResult],
    today: chrono::NaiveDate,
    budget: u64,
) {
    results.sort_by(|a, b| {
        best_score(b, today, budget)
            .cmp(&best_score(a, today, budget))
            .then_with(|| released_newest_first(a.released.as_deref(), b.released.as_deref()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Explore other models: Hugging Face + Ollama, duplicates merged.
/// An empty query browses popular Hugging Face GGUFs. Company files are
/// never sent anywhere — only this query string.
pub async fn search_models(
    client: &reqwest::Client,
    query: &str,
    profile: &MachineProfile,
) -> Result<Vec<ModelSearchResult>> {
    let query = parse_hf_repo_query(query.trim()).unwrap_or_else(|| query.trim().to_string());
    let query = query.as_str();
    let exact_repo = parse_hf_repo_query(query);
    let (hf, ollama) = tokio::join!(search_huggingface(client, query, profile), async {
        if query.is_empty() || exact_repo.is_some() {
            Vec::new()
        } else {
            search_ollama(client, query, profile).await
        }
    });
    let mut results = hf.unwrap_or_else(|error| {
        log::warn!("hugging face search: {error:#}");
        Vec::new()
    });
    for extra in ollama {
        if let Some(existing) = results.iter_mut().find(|r| r.id == extra.id) {
            existing.source = "huggingface+ollama".into();
            if existing.license.is_none() {
                existing.license = extra.license.clone();
            }
            if existing.size_bytes.is_none() {
                existing.size_bytes = extra.size_bytes;
                existing.fits = extra.fits;
            }
        } else {
            results.push(extra);
        }
    }
    rank_search_results(&mut results, profile.model_budget_bytes());
    if let Some(repo) = exact_repo {
        if let Some(idx) = results
            .iter()
            .position(|result| result.reference.eq_ignore_ascii_case(&repo))
        {
            let hit = results.remove(idx);
            results.insert(0, hit);
        }
    }
    results.truncate(HF_RESULT_LIMIT);
    Ok(results)
}

async fn reject_if_header_incompatible(client: &reqwest::Client, url: &str) -> Result<()> {
    let response = match client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-524287")
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return Ok(()),
    };
    if !response.status().is_success() {
        return Ok(());
    }
    if response.status() == reqwest::StatusCode::OK
        && response.content_length().unwrap_or(0) > 2 * 1024 * 1024
    {
        return Ok(());
    }
    let Ok(bytes) = response.bytes().await else {
        return Ok(());
    };
    match super::gguf::inspect_header(&bytes) {
        super::gguf::GgufCompat::CustomFormat | super::gguf::GgufCompat::UnsupportedTensors => {
            Err(anyhow!("incompatible-format"))
        }
        super::gguf::GgufCompat::Ok
        | super::gguf::GgufCompat::Incomplete
        | super::gguf::GgufCompat::Unreadable => Ok(()),
    }
}

/// A concrete, verifiable model download.
pub struct ResolvedDownload {
    pub url: String,
    pub file_name: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

/// Map UI source strings onto the two catalogs we actually fetch from.
pub fn normalize_source(source: &str) -> Result<&'static str> {
    match source {
        "huggingface" | "huggingface+ollama" => Ok("huggingface"),
        "ollama" => Ok("ollama"),
        _ => Err(anyhow!("unsupported model source")),
    }
}

/// Hugging Face `owner/repo` or an Ollama library name (optional `:tag`).
pub fn validate_reference(source: &str, reference: &str) -> Result<()> {
    if reference.is_empty() || reference.len() > 200 {
        return Err(anyhow!("invalid model reference"));
    }
    if reference.contains("..") || reference.contains('\\') || reference.contains('\0') {
        return Err(anyhow!("invalid model reference"));
    }
    match source {
        "huggingface" => {
            let mut parts = reference.split('/');
            let owner = parts.next().unwrap_or("");
            let repo = parts.next().unwrap_or("");
            if parts.next().is_some() || owner.is_empty() || repo.is_empty() {
                return Err(anyhow!("invalid Hugging Face reference"));
            }
            let ok = |s: &str| {
                s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                    && s.starts_with(|c: char| c.is_ascii_alphanumeric())
            };
            if !ok(owner) || !ok(repo) {
                return Err(anyhow!("invalid Hugging Face reference"));
            }
            Ok(())
        }
        "ollama" => {
            let (name, tag) = match reference.split_once(':') {
                Some((name, tag)) => (name, Some(tag)),
                None => (reference, None),
            };
            if name.is_empty()
                || name.contains('/')
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            {
                return Err(anyhow!("invalid Ollama reference"));
            }
            if let Some(tag) = tag {
                if tag.is_empty()
                    || !tag
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                {
                    return Err(anyhow!("invalid Ollama reference"));
                }
            }
            Ok(())
        }
        _ => Err(anyhow!("unsupported model source")),
    }
}

/// Public catalog page for a validated Hugging Face repo or Ollama library name.
pub fn catalog_page_url(source: &str, reference: &str) -> Result<String> {
    let source = normalize_source(source)?;
    validate_reference(source, reference)?;
    match source {
        "huggingface" => Ok(format!("https://huggingface.co/{reference}")),
        "ollama" => {
            let name = reference
                .split_once(':')
                .map(|(name, _)| name)
                .unwrap_or(reference);
            Ok(format!("https://ollama.com/library/{name}"))
        }
        _ => Err(anyhow!("unsupported model source")),
    }
}

/// Keep GGUF weights inside the models directory: basename only, `.gguf`.
pub fn safe_model_file_name(name: &str) -> Result<String> {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if base != name || base.is_empty() || base.contains("..") {
        return Err(anyhow!("invalid model file name"));
    }
    if !base.to_ascii_lowercase().ends_with(".gguf") {
        return Err(anyhow!("model file must be .gguf"));
    }
    if !base
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(anyhow!("invalid model file name"));
    }
    Ok(base.to_string())
}

/// Resolve the concrete GGUF (url + file name + size + checksum) for an
/// install. Quantization and exact file name are chosen automatically.
pub async fn resolve_download(
    client: &reqwest::Client,
    source: &str,
    reference: &str,
) -> Result<ResolvedDownload> {
    let source = normalize_source(source)?;
    validate_reference(source, reference)?;
    match source {
        "ollama" => {
            let (layer, _) = ollama_manifest(client, reference).await?;
            let url = format!(
                "https://registry.ollama.ai/v2/library/{reference}/blobs/{}",
                layer.digest
            );
            let file_name = format!("{}.gguf", reference.replace([':', '/'], "-"));
            let sha256 = layer.digest.strip_prefix("sha256:").map(str::to_string);
            Ok(ResolvedDownload {
                url,
                file_name,
                size: layer.size,
                sha256,
            })
        }
        _ => {
            let entries: Vec<HfTreeEntry> = client
                .get(format!(
                    "https://huggingface.co/api/models/{reference}/tree/main"
                ))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
                .context("hugging face file listing")?;
            let files: Vec<(String, Option<u64>)> = entries
                .iter()
                .map(|e| {
                    (
                        e.path.clone(),
                        e.size.or(e.lfs.as_ref().and_then(|l| l.size)),
                    )
                })
                .collect();
            let (file, size) =
                pick_gguf(&files).ok_or_else(|| anyhow!("no usable model file in {reference}"))?;
            let sha256 = entries
                .iter()
                .find(|e| e.path == file)
                .and_then(|e| e.lfs.as_ref())
                .and_then(|l| l.oid.clone());
            let url =
                format!("https://huggingface.co/{reference}/resolve/main/{file}?download=true");
            let file_name = file.rsplit('/').next().unwrap_or(&file).to_string();
            reject_if_header_incompatible(client, &url).await?;
            Ok(ResolvedDownload {
                url,
                file_name,
                size,
                sha256,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::hf::{
        author_matches_search, base_model_owner, is_original_maker, official_namespaces,
        publisher_guesses, publisher_namespace, HfModel,
    };
    use super::*;

    #[test]
    fn gguf_picker_prefers_q4_k_m_and_skips_junk() {
        let files = vec![
            ("model-mmproj-F16.gguf".to_string(), None),
            ("Model-Q8_0.gguf".to_string(), None),
            ("Model-Q4_K_M.gguf".to_string(), None),
            ("Model-Q4_K_M-00001-of-00002.gguf".to_string(), None),
            ("README.md".to_string(), None),
        ];
        let (picked, _) = pick_gguf(&files).unwrap();
        assert_eq!(picked, "Model-Q4_K_M.gguf");
    }

    #[test]
    fn gguf_picker_hides_custom_one_bit_packs() {
        let files = vec![
            ("gemma4_31b_packed_1bit_v11.gguf".to_string(), None),
            ("gemma-4-31B-it-Q4_K_M.gguf".to_string(), None),
        ];
        let (picked, _) = pick_gguf(&files).unwrap();
        assert_eq!(picked, "gemma-4-31B-it-Q4_K_M.gguf");
        assert!(pick_gguf(&[("gemma4_31b_packed_1bit_v11.gguf".into(), None)]).is_none());
        assert!(is_custom_quant_artifact("arcticoneai/gemma4-31B-1bit"));
        assert!(is_custom_quant_artifact("custom_1bit_packed.gguf"));
        assert!(!is_custom_quant_artifact("gemma-4-31B-it-IQ1_S.gguf"));
        assert!(!is_custom_quant_artifact("unsloth/gemma-4-31B-it-GGUF"));
        assert!(is_hidden_catalog_name("ggml-org/models-moved"));
        assert!(is_hidden_catalog_name("models moved"));
        assert!(!is_hidden_catalog_name("unsloth/Muse-Glimmer-30B-GGUF"));
    }

    #[test]
    fn explore_keeps_chat_ais_and_hides_specialists_until_searched() {
        let chat_files = vec![("Qwen3-8B-Q4_K_M.gguf".into(), None)];
        let lfm_tags = [
            "gguf".into(),
            "text-generation".into(),
            "conversational".into(),
        ];
        let paddle_files = vec![
            ("PaddleOCR-VL-1.6-GGUF.gguf".into(), None),
            ("PaddleOCR-VL-1.6-GGUF-mmproj.gguf".into(), None),
        ];
        let surya_tags = [
            "gguf".into(),
            "image-text-to-text".into(),
            "ocr".into(),
            "conversational".into(),
        ];
        let surya_files = vec![
            ("surya-2.gguf".into(), None),
            ("surya-2-mmproj.gguf".into(), None),
        ];
        let coder_tags = [
            "gguf".into(),
            "text-generation".into(),
            "base_model:01-ai/Yi-Coder-1.5B-Chat".into(),
        ];

        assert!(include_explore_hit(
            "",
            Some("text-generation"),
            &lfm_tags,
            &chat_files
        ));
        assert!(include_explore_hit(
            "",
            Some("image-text-to-text"),
            &[
                "gguf".into(),
                "image-text-to-text".into(),
                "conversational".into()
            ],
            &[
                ("gemma-3-4b-it-Q4_K_M.gguf".into(), None),
                ("mmproj-model-f16-4B.gguf".into(), None),
            ],
        ));
        assert!(!include_explore_hit(
            "",
            None,
            &["gguf".into(), "conversational".into()],
            &chat_files,
        ));
        assert!(!include_explore_hit(
            "",
            None,
            &["gguf".into()],
            &paddle_files
        ));
        assert!(!include_explore_hit(
            "",
            Some("image-text-to-text"),
            &surya_tags,
            &surya_files,
        ));
        assert!(!include_explore_hit(
            "",
            Some("text-generation"),
            &coder_tags,
            &chat_files
        ));
        assert!(include_explore_hit("ocr", None, &surya_tags, &surya_files));
        assert!(include_explore_hit(
            "coder",
            Some("text-generation"),
            &coder_tags,
            &chat_files,
        ));
        assert!(!keep_explore_hit(
            "surya",
            "someone/surya",
            Some("image-text-to-text"),
            &surya_tags,
            &surya_files,
        ));
        assert!(keep_explore_hit(
            "someone/surya",
            "someone/surya",
            Some("image-text-to-text"),
            &surya_tags,
            &surya_files,
        ));
        assert!(query_wants_specialist("ocr"));
        assert!(query_wants_specialist("Yi coder"));
        assert!(!query_wants_specialist("Qwen3"));
    }

    #[test]
    fn explore_query_reads_owner_repo_and_hub_urls() {
        assert_eq!(
            parse_hf_repo_query("OBLITERATUS/Qwen3.8-27B-OBLITERATED").as_deref(),
            Some("OBLITERATUS/Qwen3.8-27B-OBLITERATED")
        );
        assert_eq!(
            parse_hf_repo_query(
                " https://huggingface.co/OBLITERATUS/Qwen3.8-27B-OBLITERATED/tree/main "
            )
            .as_deref(),
            Some("OBLITERATUS/Qwen3.8-27B-OBLITERATED")
        );
        assert_eq!(
            parse_hf_repo_query("hf.co/unsloth/Qwen3-0.6B-GGUF").as_deref(),
            Some("unsloth/Qwen3-0.6B-GGUF")
        );
        assert_eq!(
            parse_hf_repo_query("<https://huggingface.co/Qwen/Qwen3-8B>").as_deref(),
            Some("Qwen/Qwen3-8B")
        );
        assert_eq!(
            publisher_guesses("OBLITERATUS/Qwen3.8-27B-OBLITERATED"),
            vec!["OBLITERATUS"]
        );
        assert!(parse_hf_repo_query("Qwen3").is_none());
        assert!(parse_hf_repo_query("https://evil.example/owner/repo").is_none());
        assert!(parse_hf_repo_query("https://huggingface.co/datasets/owner/repo").is_none());
        assert!(parse_hf_repo_query("owner/repo/extra").is_none());
    }

    #[test]
    fn merge_keys_align_hf_and_ollama() {
        assert_eq!(
            normalize_model_key("Qwen/Qwen3-14B-GGUF"),
            normalize_model_key("qwen3-14b")
        );
    }

    #[test]
    fn install_rejects_malicious_references_and_names() {
        assert!(normalize_source("ftp").is_err());
        assert_eq!(
            normalize_source("huggingface+ollama").unwrap(),
            "huggingface"
        );
        assert!(validate_reference("huggingface", "../../etc/passwd").is_err());
        assert!(validate_reference("huggingface", "owner/repo/../../../tmp").is_err());
        assert!(validate_reference("huggingface", "https://evil.example/x").is_err());
        assert!(validate_reference("huggingface", "owner/repo?download=1").is_err());
        assert!(validate_reference("ollama", "library/../../../etc").is_err());
        assert!(validate_reference("huggingface", "unsloth/Muse-Glimmer-30B-GGUF").is_ok());
        assert_eq!(
            catalog_page_url("huggingface", "unsloth/Muse-Glimmer-30B-GGUF").unwrap(),
            "https://huggingface.co/unsloth/Muse-Glimmer-30B-GGUF"
        );
        assert_eq!(
            catalog_page_url("huggingface+ollama", "unsloth/Muse-Glimmer-30B-GGUF").unwrap(),
            "https://huggingface.co/unsloth/Muse-Glimmer-30B-GGUF"
        );
        assert_eq!(
            catalog_page_url("ollama", "llama3.2:latest").unwrap(),
            "https://ollama.com/library/llama3.2"
        );
        assert!(catalog_page_url("huggingface", "https://evil.example/x").is_err());
        assert!(catalog_page_url("ftp", "unsloth/Muse-Glimmer-30B-GGUF").is_err());
        assert!(safe_model_file_name("../x.gguf").is_err());
        assert!(safe_model_file_name("foo/bar.gguf").is_err());
        assert!(safe_model_file_name("weights.bin").is_err());
        assert_eq!(
            safe_model_file_name("Muse-Glimmer-30B-Q4_K_M.gguf").unwrap(),
            "Muse-Glimmer-30B-Q4_K_M.gguf"
        );
    }

    fn hf_hit(author: &str, tags: &[&str]) -> HfModel {
        HfModel {
            id: Some(format!("{author}/model")),
            model_id: None,
            author: Some(author.into()),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            pipeline_tag: None,
            created_at: None,
            last_modified: None,
            downloads: None,
            downloads_all_time: None,
            siblings: None,
        }
    }

    fn search_hit(
        name: &str,
        official: bool,
        released: Option<&str>,
        downloads: Option<u64>,
        fits: Option<bool>,
        size_bytes: Option<u64>,
    ) -> ModelSearchResult {
        ModelSearchResult {
            id: name.into(),
            name: name.into(),
            source: "huggingface".into(),
            reference: format!("org/{name}"),
            file: None,
            size_bytes,
            license: None,
            released: released.map(str::to_string),
            downloads,
            publisher: Some("org".into()),
            official,
            fits,
        }
    }

    #[test]
    fn qwen_search_guesses_the_qwen_org() {
        assert_eq!(publisher_guesses("Qwen"), vec!["Qwen"]);
        assert_eq!(publisher_guesses("Qwen3"), vec!["Qwen3", "Qwen"]);
        assert_eq!(publisher_guesses("Qwen2.5"), vec!["Qwen2.5", "Qwen"]);
        assert_eq!(
            publisher_guesses("https://huggingface.co/Qwen/Qwen3-8B"),
            vec!["Qwen"]
        );
        assert!(author_matches_search("Qwen", "Qwen3"));
        assert!(author_matches_search("meta-llama", "llama"));
        assert!(!author_matches_search("unsloth", "Qwen"));
    }

    #[test]
    fn original_makers_are_allowlisted_forks_are_not() {
        for name in [
            "Qwen",
            "google",
            "meta-llama",
            "meta-models",
            "mistralai",
            "openai",
            "ornith-ai",
            "ibm-granite",
            "deepseek-ai",
            "microsoft",
            "zai-org",
            "HuggingFaceTB",
        ] {
            assert!(is_original_maker(name), "{name}");
        }
        for name in [
            "unsloth",
            "bartowski",
            "Blackfrost-AI",
            "huihui-ai",
            "lmstudio-community",
            "HuggingFaceH4",
            "TheBloke",
            "mradermacher",
            "mlx-community",
            "Abliterated",
        ] {
            assert!(!is_original_maker(name), "{name}");
        }
    }

    #[test]
    fn publisher_falls_back_to_repo_owner() {
        assert_eq!(
            publisher_namespace(None, "Qwen/Qwen3.8-27B-GGUF").as_deref(),
            Some("Qwen")
        );
        assert!(publisher_namespace(None, "Qwen/Qwen3.8-27B-GGUF")
            .as_deref()
            .is_some_and(is_original_maker));
        assert!(
            !publisher_namespace(Some("Blackfrost-AI"), "Blackfrost-AI/foo")
                .as_deref()
                .is_some_and(is_original_maker)
        );
    }

    #[test]
    fn official_namespaces_prefer_the_maker_not_quantizers() {
        let models = vec![
            hf_hit("unsloth", &["base_model:Qwen/Qwen3-8B"]),
            hf_hit("unsloth", &["base_model:Qwen/Qwen3-14B"]),
            hf_hit("bartowski", &["base_model:Qwen/Qwen3-8B"]),
        ];
        let names = official_namespaces("Qwen", &models);
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("Qwen")),
            "Qwen org should be inferred: {names:?}"
        );
        assert!(!names.iter().any(|n| n.eq_ignore_ascii_case("unsloth")));
    }

    #[test]
    fn gemma_search_uses_base_model_owner_when_org_is_not_the_query() {
        let models = vec![
            hf_hit("unsloth", &["base_model:google/gemma-4-9b-it"]),
            hf_hit("bartowski", &["base_model:google/gemma-4-9b-it"]),
            hf_hit("google", &["base_model:google/gemma-4-9b-it"]),
        ];
        let names = official_namespaces("Gemma", &models);
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("google")),
            "google should win via base_model tags: {names:?}"
        );
    }

    #[test]
    fn abliterated_fork_is_not_inferred_as_official() {
        let models = vec![
            hf_hit(
                "Blackfrost-AI",
                &["base_model:Blackfrost-AI/Qwen3.8-27B-ABLITERATED"],
            ),
            hf_hit(
                "Blackfrost-AI",
                &["base_model:Blackfrost-AI/Qwen3.8-14B-ABLITERATED"],
            ),
            hf_hit(
                "unsloth",
                &["base_model:Blackfrost-AI/Qwen3.8-27B-ABLITERATED"],
            ),
        ];
        let names = official_namespaces("Abliterated", &models);
        assert!(
            names.is_empty(),
            "fork authors must not become Official: {names:?}"
        );
    }

    #[test]
    fn rank_mixes_fit_recency_usage_and_official() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 20).unwrap();
        let budget = 20 * 1024 * 1024 * 1024;
        let right = Some(9 * 1024 * 1024 * 1024);
        let tiny = Some(400 * 1024 * 1024);
        let huge = Some(40 * 1024 * 1024 * 1024);
        let mut results = vec![
            search_hit(
                "too-large",
                false,
                Some("2026-08-01"),
                Some(42_000_000),
                Some(false),
                huge,
            ),
            search_hit(
                "official-old-rightsize",
                true,
                Some("2025-01-01"),
                Some(8_000_000),
                Some(true),
                right,
            ),
            search_hit(
                "unofficial-recent-tiny",
                false,
                Some("2026-08-10"),
                Some(50),
                Some(true),
                tiny,
            ),
            search_hit(
                "official-recent-rightsize",
                true,
                Some("2026-08-01"),
                Some(120_000),
                Some(true),
                right,
            ),
            search_hit("undated", false, None, Some(1), None, None),
        ];
        rank_search_results_on(&mut results, today, budget);
        let names: Vec<_> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names[0], "official-recent-rightsize");
        assert_eq!(names[1], "official-old-rightsize");
        assert!(
            names.iter().position(|n| *n == "unofficial-recent-tiny")
                < names.iter().position(|n| *n == "too-large")
        );
        assert_eq!(*names.last().unwrap(), "too-large");
    }

    #[test]
    fn base_model_owner_ignores_quantized_and_adapter_prefixes() {
        assert_eq!(base_model_owner("base_model:Qwen/Qwen3-8B"), Some("Qwen"));
        assert_eq!(base_model_owner("base_model:quantized:Qwen/Qwen3-8B"), None);
        assert_eq!(base_model_owner("base_model:adapter:DavidAU/foo"), None);
    }
}
