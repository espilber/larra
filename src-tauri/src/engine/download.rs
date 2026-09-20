//! Streaming downloads with progress events, SHA-256 verification and
//! cancellation. Used for the engine build and AI model files.
//!
//! Large files use a few parallel HTTP range requests (the same approach
//! Hugging Face's own `hf_transfer` uses) so a single TCP connection is not
//! the ceiling. If the server does not support ranges, we fall back to one
//! stream. Interrupted `.part` files resume instead of starting over, including
//! parallel remainder ranges. 429s wait for `Retry-After` / `RateLimit` then retry.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, ACCEPT_ENCODING, CONTENT_RANGE, RANGE, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::core::Events;

/// Six connections fill a typical home link. Dozens against `/resolve/` looks
/// like scraping; Hub Xet clients go higher only against chunk storage.
const MAX_PARTS: usize = 6;
const PARALLEL_MIN_BYTES: u64 = 32 * 1024 * 1024;
const STALL_TIMEOUT: Duration = Duration::from_secs(90);
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RATE_LIMIT_RETRIES: u32 = 5;
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(300);
const RATE_LIMIT_BACKOFF_START: Duration = Duration::from_secs(5);

pub struct DownloadTicket {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
}

/// Cancel the transfer, or skip SHA-256 after the bytes are on disk.
#[derive(Clone, Default)]
pub struct DownloadControl {
    cancel: Arc<AtomicBool>,
    skip_verify: Arc<AtomicBool>,
}

impl DownloadControl {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            skip_verify: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn request_skip_verify(&self) {
        self.skip_verify.store(true, Ordering::Relaxed);
    }
}

/// Download `url` to `dest` (atomically via a `.part` file), emitting
/// `larra://download` progress events. Verifies SHA-256 when given.
///
/// Hugging Face CDN signatures are bound to one Range header. Every request
/// therefore starts from the original `/resolve/` URL so each range gets its
/// own redirect. Reusing a signed URL from a 0-0 probe yields 403.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    ticket: &DownloadTicket,
    expected_sha256: Option<&str>,
    known_size: Option<u64>,
    events: &Arc<dyn Events>,
    control: &DownloadControl,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let emit_progress =
        |received: u64, total: Option<u64>, done: bool, error: Option<&str>, phase: &str| {
            events.emit(
                "larra://download",
                json!({
                    "kind": ticket.kind,
                    "id": ticket.id,
                    "name": ticket.name,
                    "received": received,
                    "total": total,
                    "done": done,
                    "error": error,
                    "phase": phase,
                }),
            );
        };
    if dest.is_file() {
        if let Some(expected) = expected_sha256 {
            emit_progress(
                dest.metadata().map(|m| m.len()).unwrap_or(0),
                known_size,
                false,
                None,
                "verifying",
            );
            match verify_sha256(
                dest,
                expected,
                known_size,
                &emit_progress,
                &control.cancel,
                &control.skip_verify,
            )
            .await
            {
                Ok(()) => {
                    let received = dest.metadata().map(|m| m.len()).unwrap_or(0);
                    emit_progress(
                        received,
                        known_size.or(Some(received)),
                        true,
                        None,
                        "downloading",
                    );
                    return Ok(());
                }
                Err(error) if error.to_string() == "cancelled" => return Err(error),
                Err(_) => {
                    let _ = tokio::fs::remove_file(dest).await;
                }
            }
        } else {
            let received = dest.metadata().map(|m| m.len()).unwrap_or(0);
            emit_progress(
                received,
                known_size.or(Some(received)),
                true,
                None,
                "downloading",
            );
            return Ok(());
        }
    }
    let part = dest.with_extension("part");
    if part_is_untrusted(&part, known_size) {
        log::warn!("discarding incomplete download {}", part.display());
        let _ = tokio::fs::remove_file(&part).await;
        remove_parallel_sidecars(&part);
    }

    let mut attempt = 0;
    loop {
        let part_len = trusted_part_len(&part, known_size);
        emit_progress(part_len, known_size, false, None, "downloading");

        let skip_transfer = known_size.is_some_and(|total| part_len == total);
        if skip_transfer {
            remove_parallel_sidecars(&part);
        }
        if !skip_transfer {
            let parallel_ok = should_use_parallel(&part, part_len, known_size);
            let result = if parallel_ok {
                match download_parallel(
                    client,
                    url,
                    &part,
                    known_size.expect("parallel_ok requires a known size"),
                    part_len,
                    &emit_progress,
                    &control.cancel,
                )
                .await
                {
                    Ok(()) => Ok(()),
                    Err(error) if is_fatal_download_error(&error) => Err(error),
                    Err(_) => {
                        remove_parallel_sidecars(&part);
                        let prefix = tokio::fs::metadata(&part)
                            .await
                            .ok()
                            .map(|m| m.len())
                            .unwrap_or(0);
                        download_sequential(
                            client,
                            url,
                            &part,
                            known_size,
                            prefix,
                            &emit_progress,
                            &control.cancel,
                        )
                        .await
                    }
                }
            } else {
                download_sequential(
                    client,
                    url,
                    &part,
                    known_size,
                    part_len,
                    &emit_progress,
                    &control.cancel,
                )
                .await
            };
            result?;
        }

        if let Some(expected) = expected_sha256 {
            if control.skip_verify.load(Ordering::Relaxed) {
                log::warn!("SHA-256 check skipped by user");
                break;
            }
            let total = known_size.or(tokio::fs::metadata(&part).await.ok().map(|m| m.len()));
            emit_progress(0, total, false, None, "verifying");
            match verify_sha256(
                &part,
                expected,
                total,
                &emit_progress,
                &control.cancel,
                &control.skip_verify,
            )
            .await
            {
                Ok(()) => break,
                Err(error) if error.to_string() == "cancelled" => return Err(error),
                Err(error) if skip_transfer && attempt == 0 => {
                    log::warn!("existing download failed verification; starting over: {error:#}");
                    let _ = tokio::fs::remove_file(&part).await;
                    attempt += 1;
                    continue;
                }
                Err(error) => {
                    let _ = tokio::fs::remove_file(&part).await;
                    emit_progress(
                        0,
                        known_size,
                        false,
                        Some("verification failed"),
                        "verifying",
                    );
                    return Err(error);
                }
            }
        } else {
            break;
        }
    }

    let received = tokio::fs::metadata(&part).await?.len();
    tokio::fs::rename(&part, dest).await?;
    emit_progress(
        received,
        known_size.or(Some(received)),
        true,
        None,
        "downloading",
    );
    Ok(())
}

async fn download_sequential(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    known_total: Option<u64>,
    mut received: u64,
    emit_progress: &impl Fn(u64, Option<u64>, bool, Option<&str>, &str),
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    if let Some(total) = known_total {
        if received > total {
            let _ = tokio::fs::remove_file(part).await;
            received = 0;
        } else if received == total {
            return Ok(());
        }
    }

    let mut retried_resume = false;
    let response = loop {
        let range_header = (received > 0).then(|| format!("bytes={received}-"));
        let response =
            send_with_rate_limit_retry(client, url, range_header, cancel, None, None).await?;
        let status = response.status();

        let resume = status == StatusCode::PARTIAL_CONTENT && received > 0;
        if received > 0 && !resume {
            let _ = tokio::fs::remove_file(part).await;
            received = 0;
            if status == StatusCode::FORBIDDEN && !retried_resume {
                retried_resume = true;
                continue;
            }
            if !status.is_success() {
                return Err(anyhow!("download failed ({status})"));
            }
        } else if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
            return Err(anyhow!("download failed ({status})"));
        }
        break response;
    };
    let resume = received > 0;

    let total = if resume {
        parse_content_range_total(
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or(""),
        )
        .or(known_total)
        .or_else(|| response.content_length().map(|len| received + len))
    } else {
        response.content_length().or(known_total)
    };

    let mut file = if resume {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(part)
            .await?
    } else {
        tokio::fs::File::create(part).await?
    };

    emit_progress(received, total, false, None, "downloading");
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut stream = response.bytes_stream();
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = tokio::fs::remove_file(part).await;
            emit_progress(received, total, false, Some("cancelled"), "downloading");
            return Err(anyhow!("cancelled"));
        }
        let chunk = match next_chunk(&mut stream).await? {
            Some(chunk) => chunk,
            None => break,
        };
        received += chunk.len() as u64;
        file.write_all(&chunk).await?;
        if last_emit.elapsed() >= Duration::from_millis(150) {
            emit_progress(received, total, false, None, "downloading");
            last_emit = Instant::now();
        }
    }
    file.flush().await?;
    Ok(())
}

async fn download_parallel(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    total: u64,
    prefix: u64,
    emit_progress: &impl Fn(u64, Option<u64>, bool, Option<&str>, &str),
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let _ = std::fs::remove_file(part_sibling(part, ".complete"));
    let meta = load_or_create_meta(part, total, prefix)?;
    let already = prefix + range_bytes_on_disk(part, &meta);
    let received = Arc::new(AtomicU64::new(already));
    let abort = Arc::new(AtomicBool::new(false));
    let waiting = Arc::new(AtomicU64::new(0));

    emit_progress(already, Some(total), false, None, "downloading");

    let mut tasks = Vec::new();
    for (i, (start, end)) in meta.ranges.iter().copied().enumerate() {
        let expected = end - start;
        let range_path = part_sibling(part, &format!(".r{i}"));
        if range_file_len(&range_path, expected) >= expected {
            continue;
        }
        let client = client.clone();
        let url = url.to_string();
        let received = received.clone();
        let cancel = cancel.clone();
        let abort = abort.clone();
        let waiting = waiting.clone();
        tasks.push(tokio::spawn(async move {
            fetch_range(
                &client,
                &url,
                start,
                end,
                &range_path,
                &received,
                &cancel,
                &abort,
                &waiting,
            )
            .await
        }));
    }

    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut last_bytes = already;
    let mut last_progress = Instant::now();
    loop {
        if tasks.iter().all(|task| task.is_finished()) {
            break;
        }
        let now = received.load(Ordering::Relaxed);
        if now > last_bytes {
            last_bytes = now;
            last_progress = Instant::now();
        } else if waiting.load(Ordering::Relaxed) == 0 && last_progress.elapsed() >= STALL_TIMEOUT {
            abort.store(true, Ordering::Relaxed);
            emit_progress(now, Some(total), false, Some("stalled"), "downloading");
            return Err(stalled());
        }
        if last_emit.elapsed() >= Duration::from_millis(150) {
            emit_progress(now, Some(total), false, None, "downloading");
            last_emit = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let mut first_error: Option<anyhow::Error> = None;
    for task in tasks {
        match task
            .await
            .unwrap_or_else(|_| Err(anyhow!("download task failed")))
        {
            Ok(()) => {}
            Err(error) if error.to_string() == "cancelled" => {
                abort.store(true, Ordering::Relaxed);
                let _ = tokio::fs::remove_file(part).await;
                remove_parallel_sidecars(part);
                emit_progress(
                    received.load(Ordering::Relaxed),
                    Some(total),
                    false,
                    Some("cancelled"),
                    "downloading",
                );
                return Err(error);
            }
            Err(error) => {
                abort.store(true, Ordering::Relaxed);
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }

    let part_buf = part.to_path_buf();
    let meta_clone = meta.clone();
    tokio::task::spawn_blocking(move || assemble_ranges(&part_buf, &meta_clone)).await??;
    remove_parallel_sidecars(part);
    emit_progress(total, Some(total), false, None, "downloading");
    Ok(())
}

async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
    range_path: &Path,
    received: &AtomicU64,
    cancel: &AtomicBool,
    abort: &AtomicBool,
    waiting: &AtomicU64,
) -> Result<()> {
    let expected = end - start;
    let mut have = range_file_len(range_path, expected);
    let mut retried_resume = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("cancelled"));
        }
        if abort.load(Ordering::Relaxed) {
            return Ok(());
        }
        if have >= expected {
            return Ok(());
        }

        let abs_start = start + have;
        let response = match send_with_rate_limit_retry(
            client,
            url,
            Some(format!("bytes={abs_start}-{}", end - 1)),
            cancel,
            Some(abort),
            Some(waiting),
        )
        .await
        {
            Ok(response) => response,
            Err(error) if error.to_string() == "cancelled" => return Err(error),
            Err(error) if error.to_string() == "aborted" => return Ok(()),
            Err(error) => {
                abort.store(true, Ordering::Relaxed);
                return Err(error).with_context(|| format!("range {start}-{}", end - 1));
            }
        };

        let status = response.status();
        let remaining = expected - have;
        let resume = status == StatusCode::PARTIAL_CONTENT && have > 0;
        if have > 0 && !resume {
            let _ = tokio::fs::remove_file(range_path).await;
            let _ = received.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_sub(have))
            });
            have = 0;
            if status == StatusCode::FORBIDDEN && !retried_resume {
                retried_resume = true;
                continue;
            }
            if !status.is_success() {
                abort.store(true, Ordering::Relaxed);
                return Err(anyhow!("download failed ({status})"));
            }
            if response.content_length() != Some(expected) {
                abort.store(true, Ordering::Relaxed);
                return Err(anyhow!("server ignored range request"));
            }
        } else if status != StatusCode::PARTIAL_CONTENT {
            if status.is_success() {
                if response.content_length() != Some(remaining) {
                    abort.store(true, Ordering::Relaxed);
                    return Err(anyhow!("server ignored range request"));
                }
            } else {
                abort.store(true, Ordering::Relaxed);
                return Err(anyhow!("download failed ({status})"));
            }
        }

        let mut file = if have > 0 {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(range_path)
                .await?
        } else {
            tokio::fs::File::create(range_path).await?
        };
        let mut stream = response.bytes_stream();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow!("cancelled"));
            }
            if abort.load(Ordering::Relaxed) {
                return Ok(());
            }
            let chunk = match next_chunk(&mut stream).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => {
                    abort.store(true, Ordering::Relaxed);
                    return Err(error);
                }
            };
            file.write_all(&chunk).await?;
            have += chunk.len() as u64;
            received.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        file.flush().await?;
        if have == expected {
            return Ok(());
        }
        if have > expected {
            let _ = tokio::fs::remove_file(range_path).await;
            abort.store(true, Ordering::Relaxed);
            return Err(anyhow!("range wrote past end {start}-{end}"));
        }
        abort.store(true, Ordering::Relaxed);
        return Err(anyhow!("incomplete range {start}-{end}"));
    }
}

/// SHA-256 a file on the calling thread. Used for `REBOST_ENGINE_ARCHIVE`
/// so a local copy is checked the same way as a network download.
pub fn verify_sha256_blocking(path: &Path, expected: &str) -> Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).with_context(|| path.display().to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if hex.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(anyhow!("SHA-256 mismatch: got {hex}, expected {expected}"))
    }
}

async fn verify_sha256(
    path: &Path,
    expected: &str,
    total: Option<u64>,
    emit_progress: &impl Fn(u64, Option<u64>, bool, Option<&str>, &str),
    cancel: &Arc<AtomicBool>,
    skip_verify: &Arc<AtomicBool>,
) -> Result<()> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    let mut hashed = 0u64;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    loop {
        if cancel.load(Ordering::Relaxed) {
            emit_progress(hashed, total, false, Some("cancelled"), "verifying");
            return Err(anyhow!("cancelled"));
        }
        if skip_verify.load(Ordering::Relaxed) {
            log::warn!("SHA-256 check skipped by user after {hashed} bytes");
            return Ok(());
        }
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        hashed += n as u64;
        if last_emit.elapsed() >= Duration::from_millis(150) {
            emit_progress(hashed, total, false, None, "verifying");
            last_emit = Instant::now();
        }
    }
    emit_progress(hashed, total.or(Some(hashed)), false, None, "verifying");
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if hex.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(anyhow!("SHA-256 mismatch: got {hex}, expected {expected}"))
    }
}

async fn first_byte(
    send: impl std::future::Future<Output = reqwest::Result<reqwest::Response>>,
) -> Result<reqwest::Response> {
    match tokio::time::timeout(FIRST_BYTE_TIMEOUT, send).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(error).context("request"),
        Err(_) => Err(stalled()),
    }
}

async fn next_chunk(
    stream: &mut (impl StreamExt<Item = reqwest::Result<bytes::Bytes>> + Unpin),
) -> Result<Option<bytes::Bytes>> {
    match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(chunk))) => Ok(Some(chunk)),
        Ok(Some(Err(error))) => Err(error).context("download stream"),
        Ok(None) => Ok(None),
        Err(_) => Err(stalled()),
    }
}

fn stalled() -> anyhow::Error {
    anyhow!("The download stalled. Check your connection and try again.")
}

fn rate_limited() -> anyhow::Error {
    anyhow!("The download was rate-limited. Wait a moment and try again.")
}

async fn send_with_rate_limit_retry(
    client: &reqwest::Client,
    url: &str,
    range_header: Option<String>,
    cancel: &AtomicBool,
    abort: Option<&AtomicBool>,
    waiting: Option<&AtomicU64>,
) -> Result<reqwest::Response> {
    let mut attempt = 0u32;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("cancelled"));
        }
        if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(anyhow!("aborted"));
        }
        let mut request = client.get(url).header(ACCEPT_ENCODING, "identity");
        if let Some(range) = &range_header {
            request = request.header(RANGE, range);
        }
        let response = first_byte(request.send()).await?;
        if response.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(response);
        }
        attempt += 1;
        if attempt > MAX_RATE_LIMIT_RETRIES {
            return Err(rate_limited());
        }
        let wait = retry_wait_from_headers(response.headers(), attempt);
        log::warn!(
            "download rate-limited; waiting {}s (attempt {attempt})",
            wait.as_secs()
        );
        drop(response);
        if let Some(waiting) = waiting {
            waiting.fetch_add(1, Ordering::Relaxed);
        }
        let slept = sleep_cancellable(wait, cancel, abort).await;
        if let Some(waiting) = waiting {
            waiting.fetch_sub(1, Ordering::Relaxed);
        }
        slept?;
    }
}

async fn sleep_cancellable(
    wait: Duration,
    cancel: &AtomicBool,
    abort: Option<&AtomicBool>,
) -> Result<()> {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("cancelled"));
        }
        if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(anyhow!("aborted"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(remaining.min(Duration::from_millis(200))).await;
    }
    Ok(())
}

fn retry_wait_from_headers(headers: &HeaderMap, attempt: u32) -> Duration {
    parse_retry_after(headers)
        .or_else(|| parse_ratelimit_t(headers))
        .unwrap_or_else(|| rate_limit_backoff(attempt))
        .min(MAX_RATE_LIMIT_WAIT)
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let raw = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let dt = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
    let wait = dt.timestamp() - chrono::Utc::now().timestamp();
    Some(Duration::from_secs(wait.max(0) as u64))
}

fn parse_ratelimit_t(headers: &HeaderMap) -> Option<Duration> {
    let mut max_t = 0u64;
    let mut found = false;
    for value in headers.get_all("ratelimit") {
        let Ok(text) = value.to_str() else {
            continue;
        };
        for t in ratelimit_t_values(text) {
            found = true;
            max_t = max_t.max(t);
        }
    }
    found.then_some(Duration::from_secs(max_t))
}

fn ratelimit_t_values(header: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut rest = header;
    while let Some(idx) = rest.find("t=") {
        rest = &rest[idx + 2..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(n) = digits.parse() {
            out.push(n);
        }
        rest = rest.get(digits.len()..).unwrap_or("");
    }
    out
}

fn rate_limit_backoff(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(6);
    RATE_LIMIT_BACKOFF_START * (1u32 << shift)
}

fn is_fatal_download_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message == "cancelled" || message.contains("rate-limited") || message.contains("stalled")
}

fn parse_content_range_total(header: &str) -> Option<u64> {
    let total = header.rsplit('/').next()?;
    if total == "*" {
        None
    } else {
        total.parse().ok()
    }
}

/// A leftover parallel download preallocates the full size with holes.
/// Treating that as finished shows 100% while hashing empty bytes for minutes.
fn part_is_untrusted(path: &Path, known_size: Option<u64>) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    part_looks_hollow(meta.len(), allocated_bytes(&meta), known_size)
}

/// Bytes we can resume from. Logical `len()` is not that on Windows.
fn trusted_part_len(path: &Path, known_size: Option<u64>) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if part_looks_hollow(meta.len(), allocated_bytes(&meta), known_size) {
        0
    } else {
        meta.len()
    }
}

/// `allocated` is bytes known to be on disk. `None` means we only have the
/// logical size — do not treat that as complete (Windows `SetEndOfFile` /
/// seek-writes inflate `len()` after a crash).
fn part_looks_hollow(logical_len: u64, allocated: Option<u64>, known_size: Option<u64>) -> bool {
    if logical_len == 0 {
        return false;
    }
    if let Some(total) = known_size {
        if logical_len > total {
            return true;
        }
    }
    match allocated {
        Some(allocated) => allocated < logical_len.saturating_mul(8) / 10,
        None => known_size.is_some_and(|total| logical_len >= total),
    }
}

fn allocated_bytes(meta: &std::fs::Metadata) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(meta.blocks().saturating_mul(512))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
        if meta.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE != 0 {
            // Holes. Logical size is not bytes on disk.
            Some(0)
        } else {
            None
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        None
    }
}

/// Inclusive-exclusive byte ranges covering `total`, split across up to `n` parts.
fn split_parts(total: u64, n: usize) -> Vec<(u64, u64)> {
    if total == 0 {
        return Vec::new();
    }
    let n = (n as u64).min(total).max(1);
    let chunk = total / n;
    (0..n)
        .map(|i| {
            let start = i * chunk;
            let end = if i + 1 == n { total } else { start + chunk };
            (start, end)
        })
        .filter(|(start, end)| start < end)
        .collect()
}

fn split_parts_from(start: u64, total: u64, n: usize) -> Vec<(u64, u64)> {
    if start >= total {
        return Vec::new();
    }
    split_parts(total - start, n)
        .into_iter()
        .map(|(from, to)| (from + start, to + start))
        .collect()
}

fn should_use_parallel(part: &Path, part_len: u64, known_size: Option<u64>) -> bool {
    let Some(total) = known_size else {
        return false;
    };
    if part_len >= total {
        return false;
    }
    if parallel_meta_matches(part, total, part_len) {
        return true;
    }
    total - part_len >= PARALLEL_MIN_BYTES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParallelMeta {
    total: u64,
    prefix: u64,
    ranges: Vec<(u64, u64)>,
}

fn part_sibling(part: &Path, suffix: &str) -> PathBuf {
    let mut name = part.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn parallel_meta_matches(part: &Path, total: u64, prefix: u64) -> bool {
    load_meta(part)
        .is_some_and(|meta| meta.total == total && meta.prefix == prefix && ranges_valid(&meta))
}

fn load_meta(part: &Path) -> Option<ParallelMeta> {
    let bytes = std::fs::read(part_sibling(part, ".meta")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn load_or_create_meta(part: &Path, total: u64, prefix: u64) -> Result<ParallelMeta> {
    if let Some(meta) = load_meta(part) {
        if meta.total == total && meta.prefix == prefix && ranges_valid(&meta) {
            return Ok(meta);
        }
    }
    remove_parallel_sidecars(part);
    let meta = ParallelMeta {
        total,
        prefix,
        ranges: split_parts_from(prefix, total, MAX_PARTS),
    };
    std::fs::write(part_sibling(part, ".meta"), serde_json::to_vec(&meta)?)?;
    Ok(meta)
}

fn ranges_valid(meta: &ParallelMeta) -> bool {
    if meta.prefix > meta.total {
        return false;
    }
    if meta.ranges.is_empty() {
        return meta.prefix == meta.total;
    }
    meta.ranges[0].0 == meta.prefix
        && meta
            .ranges
            .last()
            .is_some_and(|range| range.1 == meta.total)
        && meta.ranges.iter().all(|(start, end)| start < end)
        && meta.ranges.windows(2).all(|pair| pair[0].1 == pair[1].0)
}

fn range_file_len(path: &Path, expected: u64) -> u64 {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
        return 0;
    };
    if len > expected {
        let _ = std::fs::remove_file(path);
        0
    } else {
        len
    }
}

fn range_bytes_on_disk(part: &Path, meta: &ParallelMeta) -> u64 {
    meta.ranges
        .iter()
        .enumerate()
        .map(|(i, (start, end))| {
            range_file_len(&part_sibling(part, &format!(".r{i}")), end - start)
        })
        .sum()
}

fn remove_parallel_sidecars(part: &Path) {
    let count = load_meta(part)
        .map(|meta| meta.ranges.len())
        .unwrap_or(MAX_PARTS.max(16));
    for i in 0..count {
        let _ = std::fs::remove_file(part_sibling(part, &format!(".r{i}")));
    }
    let _ = std::fs::remove_file(part_sibling(part, ".meta"));
    let _ = std::fs::remove_file(part_sibling(part, ".complete"));
}

fn assemble_ranges(part: &Path, meta: &ParallelMeta) -> Result<()> {
    let complete = part_sibling(part, ".complete");
    let mut out = std::fs::File::create(&complete)?;
    if meta.prefix > 0 {
        let mut src = std::fs::File::open(part)?;
        let copied = std::io::copy(&mut src, &mut out)?;
        if copied != meta.prefix {
            return Err(anyhow!(
                "prefix length mismatch: got {copied}, expected {}",
                meta.prefix
            ));
        }
    }
    for (i, (start, end)) in meta.ranges.iter().enumerate() {
        let expected = end - start;
        let mut src = std::fs::File::open(part_sibling(part, &format!(".r{i}")))?;
        let copied = std::io::copy(&mut src, &mut out)?;
        if copied != expected {
            return Err(anyhow!("range {i} length mismatch"));
        }
    }
    out.sync_all()?;
    drop(out);
    std::fs::rename(&complete, part)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_parts_even() {
        assert_eq!(
            split_parts(100, 4),
            vec![(0, 25), (25, 50), (50, 75), (75, 100)]
        );
    }

    #[test]
    fn split_parts_remainder_on_last() {
        assert_eq!(split_parts(10, 3), vec![(0, 3), (3, 6), (6, 10)]);
    }

    #[test]
    fn split_parts_fewer_than_requested_when_tiny() {
        assert_eq!(split_parts(2, 8), vec![(0, 1), (1, 2)]);
    }

    #[test]
    fn split_parts_from_offset() {
        assert_eq!(
            split_parts_from(100, 200, 4),
            vec![(100, 125), (125, 150), (150, 175), (175, 200)]
        );
    }

    #[test]
    fn parallel_resume_when_remainder_is_large() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.part");
        assert!(should_use_parallel(&part, 0, Some(PARALLEL_MIN_BYTES)));
        assert!(should_use_parallel(
            &part,
            10,
            Some(PARALLEL_MIN_BYTES + 10)
        ));
        assert!(!should_use_parallel(&part, 0, Some(PARALLEL_MIN_BYTES - 1)));
        assert!(!should_use_parallel(
            &part,
            PARALLEL_MIN_BYTES,
            Some(PARALLEL_MIN_BYTES)
        ));
    }

    #[test]
    fn parallel_resume_when_meta_exists() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.part");
        let meta = ParallelMeta {
            total: 1000,
            prefix: 900,
            ranges: vec![(900, 1000)],
        };
        std::fs::write(
            part_sibling(&part, ".meta"),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
        assert!(should_use_parallel(&part, 900, Some(1000)));
    }

    #[test]
    fn retry_after_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "12".parse().unwrap());
        assert_eq!(
            retry_wait_from_headers(&headers, 1),
            Duration::from_secs(12)
        );
    }

    #[test]
    fn ratelimit_t_is_used_without_retry_after() {
        let mut headers = HeaderMap::new();
        headers.insert("ratelimit", r#""resolvers";r=0;t=8"#.parse().unwrap());
        assert_eq!(retry_wait_from_headers(&headers, 1), Duration::from_secs(8));
    }

    #[test]
    fn retry_after_caps_at_five_minutes() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "9999".parse().unwrap());
        assert_eq!(retry_wait_from_headers(&headers, 1), MAX_RATE_LIMIT_WAIT);
    }

    #[test]
    fn backoff_without_headers() {
        assert_eq!(rate_limit_backoff(1), Duration::from_secs(5));
        assert_eq!(rate_limit_backoff(2), Duration::from_secs(10));
        assert_eq!(rate_limit_backoff(3), Duration::from_secs(20));
    }

    #[test]
    fn ratelimit_t_values_take_digits() {
        assert_eq!(
            ratelimit_t_values(r#""api";r=3;t=1, "resolvers";r=0;t=15"#),
            vec![1, 15]
        );
    }

    #[test]
    fn content_range_total() {
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
    }

    #[test]
    fn hollow_part_without_allocation_is_untrusted() {
        assert!(part_looks_hollow(10_000_000, None, Some(10_000_000)));
        assert!(part_looks_hollow(10_000_001, None, Some(10_000_000)));
        assert!(!part_looks_hollow(5_000_000, None, Some(10_000_000)));
        assert!(!part_looks_hollow(0, None, Some(10_000_000)));
    }

    #[test]
    fn allocated_part_uses_on_disk_bytes() {
        assert!(part_looks_hollow(10_000_000, Some(0), Some(10_000_000)));
        assert!(!part_looks_hollow(
            64 * 1024,
            Some(64 * 1024),
            Some(64 * 1024)
        ));
    }

    #[test]
    fn preallocated_sparse_part_is_untrusted() {
        let path = std::env::temp_dir().join(format!(
            "larra-sparse-part-{}.gguf.part",
            std::process::id()
        ));
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(10_000_000).unwrap();
        drop(file);
        let untrusted = part_is_untrusted(&path, Some(10_000_000));
        let resume_from = trusted_part_len(&path, Some(10_000_000));
        let _ = std::fs::remove_file(&path);
        assert!(untrusted);
        assert_eq!(resume_from, 0);
    }

    #[cfg(unix)]
    #[test]
    fn written_file_is_trusted() {
        let path = std::env::temp_dir().join(format!(
            "larra-written-part-{}.gguf.part",
            std::process::id()
        ));
        std::fs::write(&path, vec![1u8; 64 * 1024]).unwrap();
        let untrusted = part_is_untrusted(&path, Some(64 * 1024));
        let _ = std::fs::remove_file(&path);
        assert!(!untrusted);
    }

    #[test]
    fn blocking_sha256_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.tar.gz");
        std::fs::write(&path, b"not-the-archive").unwrap();
        let error = verify_sha256_blocking(&path, &"00".repeat(32)).unwrap_err();
        assert!(error.to_string().contains("mismatch"), "got {error:#}");
    }

    #[tokio::test]
    async fn http_download_verifies_sha256() {
        let body = b"larra-engine-fixture";
        let expected = {
            let mut hasher = Sha256::new();
            hasher.update(body);
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 2048];
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let _ = stream.read(&mut buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes()).await;
            let _ = stream.write_all(body).await;
        });

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("engine.bin");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        struct Noop;
        impl crate::core::Events for Noop {
            fn emit(&self, _event: &str, _payload: serde_json::Value) {}
        }
        let events: Arc<dyn crate::core::Events> = Arc::new(Noop);
        download(
            &client,
            &format!("http://127.0.0.1:{port}/engine.bin"),
            &dest,
            &DownloadTicket {
                kind: "engine",
                id: "test".into(),
                name: "test".into(),
            },
            Some(&expected),
            Some(body.len() as u64),
            &events,
            &DownloadControl::new(),
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), body);
        server.abort();
    }

    #[tokio::test]
    async fn sha256_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("weights.bin");
        tokio::fs::write(&path, b"not-the-model").await.unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let skip = Arc::new(AtomicBool::new(false));
        let expected = "00".repeat(32);
        let error = verify_sha256(&path, &expected, None, &|_, _, _, _, _| {}, &cancel, &skip)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("mismatch"), "got {error:#}");
    }

    #[tokio::test]
    async fn sha256_can_be_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("weights.bin");
        tokio::fs::write(&path, b"not-the-model").await.unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let skip = Arc::new(AtomicBool::new(true));
        let expected = "00".repeat(32);
        verify_sha256(&path, &expected, None, &|_, _, _, _, _| {}, &cancel, &skip)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancel_wins_over_skip_verify() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("weights.bin");
        tokio::fs::write(&path, b"not-the-model").await.unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        let skip = Arc::new(AtomicBool::new(true));
        let expected = "00".repeat(32);
        let error = verify_sha256(&path, &expected, None, &|_, _, _, _, _| {}, &cancel, &skip)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "cancelled");
    }

    #[tokio::test]
    async fn skipped_verify_keeps_an_already_downloaded_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let body = b"already-downloaded";
        // The finished file, not a leftover `.part`. On Windows a complete-looking
        // `.part` without allocated-size info is discarded before skip-verify runs.
        tokio::fs::write(&dest, body).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        struct Noop;
        impl crate::core::Events for Noop {
            fn emit(&self, _event: &str, _payload: serde_json::Value) {}
        }
        let events: Arc<dyn crate::core::Events> = Arc::new(Noop);
        let control = DownloadControl::new();
        control.request_skip_verify();
        download(
            &client,
            "http://127.0.0.1:1/unused",
            &dest,
            &DownloadTicket {
                kind: "model",
                id: "test".into(),
                name: "test".into(),
            },
            Some(&"00".repeat(32)),
            Some(body.len() as u64),
            &events,
            &control,
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), body);
    }

    struct Noop;
    impl crate::core::Events for Noop {
        fn emit(&self, _event: &str, _payload: serde_json::Value) {}
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .http1_only()
            .build()
            .unwrap()
    }

    fn parse_request_range(req: &str, total: u64) -> (u64, u64) {
        let lower = req.to_ascii_lowercase();
        let Some(rest) = lower.split("range: bytes=").nth(1) else {
            return (0, total);
        };
        let spec = rest.split_whitespace().next().unwrap_or("");
        let Some((start_s, end_s)) = spec.split_once('-') else {
            return (0, total);
        };
        let start: u64 = start_s.parse().unwrap_or(0);
        let end_incl: u64 = if end_s.is_empty() {
            total.saturating_sub(1)
        } else {
            end_s.parse().unwrap_or(total.saturating_sub(1))
        };
        (start, (end_incl + 1).min(total))
    }

    async fn handle_download_conn(
        mut stream: tokio::net::TcpStream,
        body: &[u8],
        remaining_429: &AtomicU64,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = vec![0u8; 4096];
        let Ok(n) = stream.read(&mut buf).await else {
            return;
        };
        let req = String::from_utf8_lossy(&buf[..n]);
        if remaining_429.load(Ordering::Relaxed) > 0 {
            remaining_429.fetch_sub(1, Ordering::Relaxed);
            let _ = stream
                .write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nRateLimit: \"resolvers\";r=0;t=0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return;
        }
        let total = body.len() as u64;
        let (start, end) = parse_request_range(&req, total);
        let slice = &body[start as usize..end as usize];
        let header = if start == 0 && end == total && !req.to_ascii_lowercase().contains("range:") {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                slice.len()
            )
        } else {
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{total}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                start,
                end.saturating_sub(1),
                slice.len()
            )
        };
        let _ = stream.write_all(header.as_bytes()).await;
        let _ = stream.write_all(slice).await;
    }

    async fn spawn_download_server(
        body: Vec<u8>,
        remaining_429: Arc<AtomicU64>,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = Arc::new(body);
        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let body = body.clone();
                let remaining_429 = remaining_429.clone();
                tokio::spawn(async move {
                    handle_download_conn(stream, &body, &remaining_429).await;
                });
            }
        });
        (port, handle)
    }

    #[tokio::test]
    async fn waits_then_retries_on_429() {
        let body = b"rate-limit-then-ok".to_vec();
        let remaining_429 = Arc::new(AtomicU64::new(1));
        let (port, server) = spawn_download_server(body.clone(), remaining_429).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let events: Arc<dyn crate::core::Events> = Arc::new(Noop);
        download(
            &test_client(),
            &format!("http://127.0.0.1:{port}/model.bin"),
            &dest,
            &DownloadTicket {
                kind: "model",
                id: "test".into(),
                name: "test".into(),
            },
            None,
            Some(body.len() as u64),
            &events,
            &DownloadControl::new(),
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), body);
        server.abort();
    }

    #[tokio::test]
    async fn exhausted_429_is_fatal() {
        let body = b"never-delivered".to_vec();
        let remaining_429 = Arc::new(AtomicU64::new(100));
        let (port, server) = spawn_download_server(body, remaining_429).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let events: Arc<dyn crate::core::Events> = Arc::new(Noop);
        let error = download(
            &test_client(),
            &format!("http://127.0.0.1:{port}/model.bin"),
            &dest,
            &DownloadTicket {
                kind: "model",
                id: "test".into(),
                name: "test".into(),
            },
            None,
            Some(16),
            &events,
            &DownloadControl::new(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("rate-limited"), "got {error:#}");
        server.abort();
    }

    #[tokio::test]
    async fn parallel_resume_keeps_prefix() {
        let body: Vec<u8> = (0u8..=255).cycle().take(600).collect();
        let prefix = 180usize;
        let remaining_429 = Arc::new(AtomicU64::new(0));
        let (port, server) = spawn_download_server(body.clone(), remaining_429).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let part = dest.with_extension("part");
        tokio::fs::write(&part, &body[..prefix]).await.unwrap();
        download_parallel(
            &test_client(),
            &format!("http://127.0.0.1:{port}/model.bin"),
            &part,
            body.len() as u64,
            prefix as u64,
            &|_, _, _, _, _| {},
            &Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&part).await.unwrap(), body);
        assert!(!part_sibling(&part, ".meta").exists());
        server.abort();
    }

    #[tokio::test]
    async fn parallel_fresh_six_ranges() {
        let body: Vec<u8> = (0u8..=255).cycle().take(600).collect();
        let remaining_429 = Arc::new(AtomicU64::new(0));
        let (port, server) = spawn_download_server(body.clone(), remaining_429).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.bin");
        let part = dest.with_extension("part");
        download_parallel(
            &test_client(),
            &format!("http://127.0.0.1:{port}/model.bin"),
            &part,
            body.len() as u64,
            0,
            &|_, _, _, _, _| {},
            &Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&part).await.unwrap(), body);
        server.abort();
    }
}
