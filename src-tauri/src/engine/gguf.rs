//! Read just enough of a GGUF header for tune and install checks.

use anyhow::{anyhow, Result};
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

const GGUF_MAGIC: &[u8; 4] = b"GGUF";
const MAX_HEADER_BYTES: u64 = 2 * 1024 * 1024;
const INSPECT_HEADER_BYTES: u64 = 32 * 1024 * 1024;
const MAX_KV: u64 = 512;
const MAX_KEY: u64 = 256;
const MAX_TENSORS: u64 = 8_192;
const MAX_ARRAY: u64 = 400_000;
/// Tensor types the pinned llama.cpp build accepts (`[0, 43)` on 0.3.0).
const ENGINE_GGML_TYPE_COUNT: u32 = 43;

const TY_UINT8: u32 = 0;
const TY_INT8: u32 = 1;
const TY_UINT16: u32 = 2;
const TY_INT16: u32 = 3;
const TY_UINT32: u32 = 4;
const TY_INT32: u32 = 5;
const TY_FLOAT32: u32 = 6;
const TY_BOOL: u32 = 7;
const TY_STRING: u32 = 8;
const TY_ARRAY: u32 = 9;
const TY_UINT64: u32 = 10;
const TY_INT64: u32 = 11;
const TY_FLOAT64: u32 = 12;

/// Whether a header can load on the pinned engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GgufCompat {
    Ok,
    CustomFormat,
    UnsupportedTensors,
    /// Truncated peek, or a header we could not finish reading.
    Incomplete,
    Unreadable,
}

/// Trained context (`*.context_length`) when the header is readable.
pub fn read_context_length(path: &Path) -> Option<u32> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != GGUF_MAGIC {
        return None;
    }
    let version = read_u32(&mut file)?;
    if !(2..=3).contains(&version) {
        return None;
    }
    let _n_tensors = read_u64(&mut file)?;
    let n_kv = read_u64(&mut file)?;
    if n_kv == 0 || n_kv > MAX_KV {
        return None;
    }
    for _ in 0..n_kv {
        if file.stream_position().ok()? > MAX_HEADER_BYTES {
            return None;
        }
        let key = read_string(&mut file)?;
        let ty = read_u32(&mut file)?;
        if key.ends_with(".context_length") || key == "context_length" {
            return read_int_value(&mut file, ty);
        }
        skip_value(&mut file, ty, MAX_HEADER_BYTES, 10_000)?;
    }
    None
}

/// Full-file check used after download, before the file becomes the active AI.
pub fn require_engine_compatible(path: &Path) -> Result<()> {
    match inspect_path(path) {
        GgufCompat::Ok => Ok(()),
        GgufCompat::CustomFormat
        | GgufCompat::UnsupportedTensors
        | GgufCompat::Incomplete
        | GgufCompat::Unreadable => Err(anyhow!("incompatible-format")),
    }
}

pub fn inspect_path(path: &Path) -> GgufCompat {
    let Ok(file) = File::open(path) else {
        return GgufCompat::Unreadable;
    };
    inspect_reader(file, INSPECT_HEADER_BYTES)
}

/// Range-request peek. Only `CustomFormat` / `UnsupportedTensors` are decisive;
/// a short buffer is `Incomplete` and must not reject the download.
pub fn inspect_header(bytes: &[u8]) -> GgufCompat {
    inspect_reader(Cursor::new(bytes), bytes.len() as u64)
}

fn inspect_reader<R: Read + Seek>(mut reader: R, max_header: u64) -> GgufCompat {
    let mut magic = [0u8; 4];
    if reader.read_exact(&mut magic).is_err() || &magic != GGUF_MAGIC {
        return GgufCompat::Unreadable;
    }
    let Some(version) = read_u32(&mut reader) else {
        return GgufCompat::Incomplete;
    };
    if !(2..=3).contains(&version) {
        return GgufCompat::Unreadable;
    }
    let Some(n_tensors) = read_u64(&mut reader) else {
        return GgufCompat::Incomplete;
    };
    let Some(n_kv) = read_u64(&mut reader) else {
        return GgufCompat::Incomplete;
    };
    if n_tensors > MAX_TENSORS || n_kv > MAX_KV {
        return GgufCompat::Unreadable;
    }

    let mut custom = false;
    for _ in 0..n_kv {
        let Ok(pos) = reader.stream_position() else {
            return GgufCompat::Incomplete;
        };
        if pos > max_header {
            return GgufCompat::Incomplete;
        }
        let Some(key) = read_string(&mut reader) else {
            return GgufCompat::Incomplete;
        };
        let Some(ty) = read_u32(&mut reader) else {
            return GgufCompat::Incomplete;
        };
        if key == "general.file_type" && ty == TY_STRING {
            let Some(value) = read_string(&mut reader) else {
                return GgufCompat::Incomplete;
            };
            if value.to_ascii_lowercase().starts_with("custom_") {
                custom = true;
            }
            continue;
        }
        if skip_value(&mut reader, ty, max_header, MAX_ARRAY).is_none() {
            return GgufCompat::Incomplete;
        }
    }
    if custom {
        return GgufCompat::CustomFormat;
    }

    for _ in 0..n_tensors {
        let Ok(pos) = reader.stream_position() else {
            return GgufCompat::Incomplete;
        };
        if pos > max_header {
            return GgufCompat::Incomplete;
        }
        if read_string(&mut reader).is_none() {
            return GgufCompat::Incomplete;
        }
        let Some(n_dims) = read_u32(&mut reader) else {
            return GgufCompat::Incomplete;
        };
        if n_dims > 4 {
            return GgufCompat::Unreadable;
        }
        for _ in 0..n_dims {
            if read_u64(&mut reader).is_none() {
                return GgufCompat::Incomplete;
            }
        }
        let Some(ggml_type) = read_u32(&mut reader) else {
            return GgufCompat::Incomplete;
        };
        if read_u64(&mut reader).is_none() {
            return GgufCompat::Incomplete;
        }
        if ggml_type >= ENGINE_GGML_TYPE_COUNT {
            return GgufCompat::UnsupportedTensors;
        }
    }
    GgufCompat::Ok
}

fn read_int_value<R: Read + Seek>(reader: &mut R, ty: u32) -> Option<u32> {
    match ty {
        TY_UINT32 | TY_INT32 => read_u32(reader),
        TY_UINT16 | TY_INT16 => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).ok()?;
            Some(u16::from_le_bytes(buf) as u32)
        }
        TY_UINT64 | TY_INT64 => {
            let n = read_u64(reader)?;
            u32::try_from(n).ok()
        }
        _ => None,
    }
}

fn skip_value<R: Read + Seek>(
    reader: &mut R,
    ty: u32,
    max_header: u64,
    max_array: u64,
) -> Option<()> {
    let skip = match ty {
        TY_UINT8 | TY_INT8 | TY_BOOL => 1,
        TY_UINT16 | TY_INT16 => 2,
        TY_UINT32 | TY_INT32 | TY_FLOAT32 => 4,
        TY_UINT64 | TY_INT64 | TY_FLOAT64 => 8,
        TY_STRING => {
            let n = read_u64(reader)?;
            if n > max_header {
                return None;
            }
            reader.seek(SeekFrom::Current(n as i64)).ok()?;
            return Some(());
        }
        TY_ARRAY => {
            let elem = read_u32(reader)?;
            let count = read_u64(reader)?;
            if count > max_array {
                return None;
            }
            for _ in 0..count {
                skip_value(reader, elem, max_header, max_array)?;
            }
            return Some(());
        }
        _ => return None,
    };
    reader.seek(SeekFrom::Current(skip)).ok()?;
    Some(())
}

fn read_string<R: Read>(reader: &mut R) -> Option<String> {
    let n = read_u64(reader)?;
    if n > MAX_KEY {
        return None;
    }
    let mut buf = vec![0u8; n as usize];
    reader.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn read_u32<R: Read>(reader: &mut R) -> Option<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf).ok()?;
    Some(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(reader: &mut R) -> Option<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf).ok()?;
    Some(u64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gguf_with_ctx(n: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(GGUF_MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        let key = b"llama.context_length";
        bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(key);
        bytes.extend_from_slice(&TY_UINT32.to_le_bytes());
        bytes.extend_from_slice(&n.to_le_bytes());
        bytes
    }

    #[test]
    fn reads_trained_context_from_a_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.gguf");
        std::fs::write(&path, gguf_with_ctx(32_768)).unwrap();
        assert_eq!(read_context_length(&path), Some(32_768));
    }

    #[test]
    fn rejects_a_non_gguf_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.bin");
        std::fs::write(&path, b"not a gguf").unwrap();
        assert_eq!(read_context_length(&path), None);
        assert_eq!(inspect_header(b"not a gguf"), GgufCompat::Unreadable);
    }

    fn push_string(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value);
    }

    #[test]
    fn rejects_custom_file_type() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(GGUF_MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        push_string(&mut bytes, b"general.file_type");
        bytes.extend_from_slice(&TY_STRING.to_le_bytes());
        push_string(&mut bytes, b"custom_1bit_packed");
        assert_eq!(inspect_header(&bytes), GgufCompat::CustomFormat);
    }

    #[test]
    fn rejects_tensor_types_the_engine_does_not_know() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(GGUF_MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        push_string(&mut bytes, b"weight");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&32u64.to_le_bytes());
        bytes.extend_from_slice(&51u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        assert_eq!(inspect_header(&bytes), GgufCompat::UnsupportedTensors);
    }

    #[test]
    fn accepts_a_plain_header() {
        assert_eq!(inspect_header(&gguf_with_ctx(4096)), GgufCompat::Ok);
    }
}
