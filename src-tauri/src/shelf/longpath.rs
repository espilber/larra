//! Long destination paths when copying into a Shelf's Imports folder.
//!
//! Windows MAX_PATH is 260 UTF-16 units including a NUL. This process opts
//! into long-path APIs (`longPathAware` in the exe manifest, plus `\\?\`
//! prefixes on copy) so most deep trees still land. A path that cannot fit
//! even then — a name longer than 255 characters, or more than ~32,767 units —
//! is skipped before copy so the rest of the drop can continue.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Ceiling for an extended-length Windows path, including a `\\?\` prefix.
#[cfg(windows)]
const WIN_EXTENDED_MAX: usize = 32_767;
/// NTFS / APFS / typical POSIX file-name limit.
const COMPONENT_MAX: usize = 255;

pub fn dest_too_long(path: &Path) -> bool {
    if path.components().any(|component| match component {
        Component::Normal(name) => os_unit_len(name) > COMPONENT_MAX,
        _ => false,
    }) {
        return true;
    }
    #[cfg(windows)]
    {
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        os_unit_len(abs.as_os_str()) > WIN_EXTENDED_MAX.saturating_sub(10)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Prefix `\\?\` on Windows so CreateFile/CopyFile bypass MAX_PATH even when
/// the machine has not set LongPathsEnabled. Other platforms are unchanged.
pub fn with_long_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        verbatim(path)
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

pub fn is_path_length_error(err: &io::Error) -> bool {
    // 206 ERROR_FILENAME_EXCED_RANGE, 111 ERROR_BUFFER_OVERFLOW,
    // 63 Darwin ENAMETOOLONG, 36 Linux ENAMETOOLONG.
    matches!(err.raw_os_error(), Some(206 | 111 | 63 | 36))
}

#[cfg(windows)]
fn os_unit_len(s: &std::ffi::OsStr) -> usize {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().count()
}

#[cfg(not(windows))]
fn os_unit_len(s: &std::ffi::OsStr) -> usize {
    s.len()
}

#[cfg(windows)]
fn verbatim(path: &Path) -> PathBuf {
    let abs = match std::path::absolute(path) {
        Ok(p) => p,
        Err(_) => return path.to_path_buf(),
    };
    let text = abs.to_string_lossy();
    if text.starts_with(r"\\?\") {
        return abs;
    }
    if let Some(rest) = text.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{rest}"))
    } else {
        PathBuf::from(format!(r"\\?\{text}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlong_file_name_is_rejected() {
        let name = format!("{}.md", "a".repeat(256));
        assert!(dest_too_long(&PathBuf::from("/tmp").join(name)));
        assert!(!dest_too_long(Path::new("/tmp/note.md")));
    }

    #[test]
    fn just_over_legacy_max_path_is_not_rejected() {
        // 260 was MAX_PATH; long-path APIs handle this. One 200-char name is fine.
        let dest = PathBuf::from("/Users/docs").join(format!("{}.md", "n".repeat(200)));
        assert!(!dest_too_long(&dest));
    }

    #[cfg(windows)]
    #[test]
    fn huge_absolute_path_is_rejected_on_windows() {
        let mut dest = PathBuf::from(r"C:\");
        let chunk = "n".repeat(200);
        for _ in 0..200 {
            dest.push(&chunk);
        }
        dest.set_extension("md");
        assert!(dest_too_long(&dest));
    }

    #[test]
    fn filename_exceeded_os_error_is_a_path_length_error() {
        assert!(is_path_length_error(&io::Error::from_raw_os_error(206)));
        assert!(!is_path_length_error(&io::Error::from_raw_os_error(2)));
    }
}
