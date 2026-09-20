//! llama-server process hygiene: logs, ports, leftover PIDs, archive unpack.

use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

use super::pin::is_llama_server_file_name;

pub(crate) const USER_AGENT: &str = concat!(
    "Larra/",
    env!("CARGO_PKG_VERSION"),
    " (local-first open-source desktop AI; https://github.com/espilber/larra)"
);

pub(crate) fn is_compute_failure(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("compute error")
        || lower.contains("insufficient memory")
        || lower.contains("outofmemory")
        || lower.contains("failed to decode")
}

/// The chat template rejected the message list itself, so the shape of the
/// list is what has to change before trying again.
pub(crate) fn is_template_failure(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("jinja") || lower.contains("chat template")
}

pub(crate) const ENGINE_LOG_MAX_LINES: usize = 1_000;
const ENGINE_LOG_TRIM_EVERY: usize = 200;

pub(crate) fn trim_log_file(path: &Path, max_lines: usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let count = text.lines().count();
    if count <= max_lines {
        return;
    }
    let keep: String = text
        .lines()
        .rev()
        .take(max_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .flat_map(|line| [line, "\n"])
        .collect();
    let _ = crate::paths::atomic_write(path, keep);
}

pub(crate) async fn pipe_to_log(reader: impl tokio::io::AsyncRead + Unpin, log_path: PathBuf) {
    use tokio::io::BufReader;
    trim_log_file(&log_path, ENGINE_LOG_MAX_LINES);
    let mut lines = BufReader::new(reader).lines();
    let mut since_trim = 0usize;
    while let Ok(Some(line)) = lines.next_line().await {
        echo_engine_line(&line);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
        since_trim += 1;
        if since_trim >= ENGINE_LOG_TRIM_EVERY {
            trim_log_file(&log_path, ENGINE_LOG_MAX_LINES);
            since_trim = 0;
        }
    }
}

fn echo_engine_line(line: &str) {
    let lower = line.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("fail") || lower.contains("panic") {
        log::error!(target: "llama", "{line}");
        return;
    }
    if lower.contains("load")
        || lower.contains("listen")
        || lower.contains("metal")
        || lower.contains("vulkan")
        || lower.contains("cuda")
        || lower.contains("opencl")
        || lower.contains("warn")
    {
        log::info!(target: "llama", "{line}");
    }
}

pub(crate) fn engine_log_tail(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return "(no engine.log yet)".into();
    };
    let lines: Vec<&str> = text.lines().rev().take(40).collect();
    if lines.is_empty() {
        "(engine.log empty)".into()
    } else {
        lines.into_iter().rev().collect::<Vec<_>>().join("\n")
    }
}

pub(crate) fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Unpack a gzip tar or zip, skipping `..` path components.
pub(crate) fn unpack_engine_archive(archive: &Path, extract_dir: &Path) -> Result<()> {
    if archive_is_zip(archive)? {
        unpack_zip(archive, extract_dir)
    } else {
        unpack_tar_gz(archive, extract_dir)
    }
}

fn archive_is_zip(archive: &Path) -> Result<bool> {
    let mut file = std::fs::File::open(archive).context("open engine archive")?;
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).context("read archive magic")?;
    Ok(&magic == b"PK")
}

fn unpack_tar_gz(archive: &Path, extract_dir: &Path) -> Result<()> {
    let archive_file = std::fs::File::open(archive)?;
    let tar = flate2::read::GzDecoder::new(archive_file);
    let mut archive_reader = tar::Archive::new(tar);
    for entry in archive_reader.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        entry.unpack_in(extract_dir)?;
    }
    Ok(())
}

fn unpack_zip(archive: &Path, extract_dir: &Path) -> Result<()> {
    let archive_file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(archive_file).context("read zip engine archive")?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(relative) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        if relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let out = extract_dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut dest = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut dest)?;
    }
    Ok(())
}

/// Locate `llama-server` / `llama-server.exe` after unpack. Prefers the
/// shallowest match so a nested copy does not win over the real binary.
pub(crate) fn find_llama_server(root: &Path) -> Option<PathBuf> {
    let mut best: Option<PathBuf> = None;
    let mut best_depth = usize::MAX;
    find_llama_server_walk(root, 0, &mut best, &mut best_depth);
    best
}

fn find_llama_server_walk(
    dir: &Path,
    depth: usize,
    best: &mut Option<PathBuf>,
    best_depth: &mut usize,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_llama_server_walk(&path, depth + 1, best, best_depth);
            continue;
        }
        if is_llama_server_file_name(path.file_name()) && depth < *best_depth {
            *best_depth = depth;
            *best = Some(path);
        }
    }
}

fn pid_file(data_dir: &Path) -> PathBuf {
    data_dir.join("engine").join("llama-server.pid")
}

pub(crate) fn write_server_pid(data_dir: &Path, pid: u32) {
    let path = pid_file(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&path, format!("{pid}\n")) {
        log::warn!("write llama-server pid: {error}");
    }
}

fn read_server_pid(data_dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(pid_file(data_dir)).ok()?;
    text.trim().parse().ok()
}

/// Kill leftover llama-server processes.
///
/// The fast path is the PID we wrote last. If that file is missing (upgrade
/// from a build that never wrote one, or a spawn that crashed first), scan
/// for `llama-server` binaries under this app-data directory.
pub(crate) fn kill_stale_llama_servers(data_dir: &Path) {
    let path = pid_file(data_dir);
    if let Some(pid) = read_server_pid(data_dir) {
        kill_recorded_pid(data_dir, pid);
        let _ = std::fs::remove_file(&path);
        return;
    }
    kill_orphans_under_data_dir(data_dir);
}

fn kill_recorded_pid(data_dir: &Path, pid: u32) {
    if !process_is_our_llama(data_dir, pid) {
        return;
    }
    log::info!("stopping leftover llama-server pid {pid}");
    force_kill_pid(pid);
    wait_until_dead(pid);
}

fn kill_orphans_under_data_dir(data_dir: &Path) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    let pids: Vec<u32> = sys
        .processes()
        .iter()
        .filter(|(_, process)| is_our_llama_server(process, data_dir))
        .map(|(pid, _)| pid.as_u32())
        .collect();
    for pid in pids {
        log::info!("stopping leftover llama-server pid {pid} (no pid file)");
        force_kill_pid(pid);
        wait_until_dead(pid);
    }
}

fn wait_until_dead(pid: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while process_is_alive(pid) {
        if std::time::Instant::now() >= deadline {
            log::error!("llama-server pid {pid} ignored SIGKILL");
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !process_is_alive(pid) {
        log::info!("all leftover llama-server processes have exited");
    }
}

fn process_is_our_llama(data_dir: &Path, pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    sys.process(pid)
        .is_some_and(|process| is_our_llama_server(process, data_dir))
}

fn process_is_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).is_some()
}

fn is_our_llama_server(process: &sysinfo::Process, data_dir: &Path) -> bool {
    if let Some(exe) = process.exe() {
        if exe.starts_with(data_dir) && is_llama_server_file_name(exe.file_name()) {
            return true;
        }
    }
    let marker = data_dir.to_string_lossy();
    let cmd = process.cmd();
    cmd.iter()
        .any(|part| part.to_string_lossy().contains("llama-server"))
        && cmd
            .iter()
            .any(|part| part.to_string_lossy().contains(marker.as_ref()))
}

pub(crate) fn force_kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: SIGKILL on this pid and its process group. The pid came from
        // a llama-server we spawned (or a stale one under our app-data path).
        // Negative pid is POSIX killpg; the child is started in its own group.
        unsafe {
            extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            let pid = pid as i32;
            kill(pid, 9);
            kill(-pid, 9);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_port_is_ephemeral() {
        let a = free_port().unwrap();
        assert!(a > 1024);
    }

    #[test]
    fn compute_failure_detects_known_strings() {
        assert!(is_compute_failure("Compute error: failed to decode"));
        assert!(!is_compute_failure("ok"));
    }

    #[test]
    fn trim_log_keeps_the_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.log");
        let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, body).unwrap();
        trim_log_file(&path, 5);
        let kept = std::fs::read_to_string(&path).unwrap();
        assert_eq!(kept.lines().count(), 5);
        assert!(kept.contains("line 19"));
        assert!(!kept.contains("line 0"));
        trim_log_file(&path, 5);
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 5);
    }

    fn write_targz(path: &Path, files: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, name, *bytes).unwrap();
        }
        builder.finish().unwrap();
    }

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            use std::io::Write;
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn unpack_targz_finds_nested_llama_server() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("engine.tar.gz");
        write_targz(
            &archive,
            &[
                ("build/llama-server", b"fake-bin"),
                ("build/libllama.so", b"lib"),
            ],
        );
        let extract = dir.path().join("out");
        std::fs::create_dir_all(&extract).unwrap();
        unpack_engine_archive(&archive, &extract).unwrap();
        let found = find_llama_server(&extract).unwrap();
        assert_eq!(found.file_name().unwrap(), "llama-server");
        assert!(found.parent().unwrap().ends_with("build"));
    }

    #[test]
    fn unpack_zip_finds_root_llama_server_exe() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("engine.zip");
        write_zip(
            &archive,
            &[
                ("llama-server.exe", b"fake-exe"),
                ("ggml-vulkan.dll", b"dll"),
            ],
        );
        let extract = dir.path().join("out");
        std::fs::create_dir_all(&extract).unwrap();
        unpack_engine_archive(&archive, &extract).unwrap();
        let found = find_llama_server(&extract).unwrap();
        assert_eq!(found.file_name().unwrap(), "llama-server.exe");
        assert_eq!(found.parent().unwrap(), extract.as_path());
    }

    #[test]
    fn pid_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        write_server_pid(dir.path(), 4242);
        assert_eq!(read_server_pid(dir.path()), Some(4242));
    }

    #[test]
    fn kill_stale_clears_a_dead_pid_file() {
        let dir = tempfile::tempdir().unwrap();
        write_server_pid(dir.path(), 999_999_999);
        kill_stale_llama_servers(dir.path());
        assert!(read_server_pid(dir.path()).is_none());
        assert!(!pid_file(dir.path()).exists());
    }

    #[test]
    fn kill_stale_without_pid_file_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        kill_stale_llama_servers(dir.path());
        assert!(!pid_file(dir.path()).exists());
    }

    #[test]
    fn unpack_zip_skips_parent_dir_entries() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("slip.zip");
        write_zip(&archive, &[("../outside.bin", b"nope"), ("ok.txt", b"yes")]);
        let extract = dir.path().join("out");
        std::fs::create_dir_all(&extract).unwrap();
        unpack_engine_archive(&archive, &extract).unwrap();
        assert!(!dir.path().join("outside.bin").exists());
        assert!(extract.join("ok.txt").exists());
    }
}
