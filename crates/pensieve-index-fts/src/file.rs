//! On-object serialization for the tantivy FTS sidecar.
//!
//! A tantivy index is itself a small directory of files (`meta.json`, segment
//! `.store`/`.idx`/`.term`/… files, `.managed.json`). We pack that whole
//! directory into one self-describing sidecar blob and, on the read side,
//! reconstruct a [`tantivy::directory::RamDirectory`] from it so the index can
//! be opened and searched.
//!
//! **Simple-but-correct baseline.** Part A loads the *entire* blob into a
//! `RamDirectory` before searching. That is correct for any extent (the files
//! are immutable and self-contained) but reads the whole sidecar even for a
//! one-term query. Quickwit-style hotcache / range-read loading (open only the
//! term dictionary + the posting lists a query touches) is a Part-B /
//! optimization concern and is intentionally not done here.
//!
//! Layout (all integers little-endian):
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ HEADER                                                             │
//! │   magic        : "KYFT" (4 bytes)                                  │
//! │   version      : u32                                               │
//! │   file_count   : u32                                               │
//! ├──────────────────────────────────────────────────────────────────┤
//! │ MANIFEST : file_count entries, each:                              │
//! │   name_len   : u32                                                 │
//! │   name       : name_len bytes UTF-8 (relative path in the index)   │
//! │   data_off   : u64  (absolute offset of this file's bytes)         │
//! │   data_len   : u64                                                 │
//! ├──────────────────────────────────────────────────────────────────┤
//! │ DATA : the concatenated file bytes, in manifest order             │
//! ├──────────────────────────────────────────────────────────────────┤
//! │ TRAILER                                                            │
//! │   magic : "KYFT" (4 bytes, trailing)                              │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Every offset/length is validated against the buffer bounds on read, so a
//! truncated or corrupt blob yields [`FtsError`] rather than a panic.

use bytes::Bytes;
use tantivy::directory::RamDirectory;
use tantivy::Directory;
use thiserror::Error;

/// 4-byte magic at the head and tail of the sidecar blob.
pub const MAGIC: &[u8; 4] = b"KYFT";

/// On-object format version.
pub const FORMAT_VERSION: u32 = 1;

/// Errors reading, interpreting, or reconstructing a sidecar blob.
#[derive(Debug, Error)]
pub enum FtsError {
    #[error("sidecar too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },
    #[error("bad magic bytes")]
    BadMagic,
    #[error("unsupported sidecar version {0}")]
    UnsupportedVersion(u32),
    #[error("corrupt sidecar: {0}")]
    Corrupt(String),
    #[error("tantivy error: {0}")]
    Tantivy(String),
}

type Result<T> = std::result::Result<T, FtsError>;

/// Parsed header of a sidecar blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub file_count: u32,
    /// Byte offset of the first manifest entry (just past the fixed header).
    pub manifest_offset: usize,
}

/// One manifest entry: a file name and the byte range of its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub name: String,
    pub data_off: usize,
    pub data_len: usize,
}

// ---- little-endian read helpers (bounds-checked) ----

fn rd_u32(buf: &[u8], off: usize) -> Result<u32> {
    let end = off
        .checked_add(4)
        .ok_or_else(|| FtsError::Corrupt("offset overflow".into()))?;
    let slice = buf.get(off..end).ok_or(FtsError::TooShort {
        need: end,
        have: buf.len(),
    })?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

fn rd_u64(buf: &[u8], off: usize) -> Result<u64> {
    let end = off
        .checked_add(8)
        .ok_or_else(|| FtsError::Corrupt("offset overflow".into()))?;
    let slice = buf.get(off..end).ok_or(FtsError::TooShort {
        need: end,
        have: buf.len(),
    })?;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

/// Serialize a list of `(name, bytes)` index files into a sidecar blob.
pub fn write(files: &[(String, Vec<u8>)]) -> Bytes {
    let mut buf: Vec<u8> = Vec::new();

    // --- Header ---
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&(files.len() as u32).to_le_bytes());

    // --- Manifest (offsets filled after we know the data layout) ---
    // First compute where DATA begins: header(12) + Σ manifest-entry sizes.
    let manifest_bytes: usize = files.iter().map(|(name, _)| 4 + name.len() + 8 + 8).sum();
    let data_start = buf.len() + manifest_bytes;

    let mut cursor = data_start;
    for (name, bytes) in files {
        buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.extend_from_slice(&(cursor as u64).to_le_bytes());
        buf.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        cursor += bytes.len();
    }
    debug_assert_eq!(buf.len(), data_start);

    // --- Data ---
    for (_name, bytes) in files {
        buf.extend_from_slice(bytes);
    }

    // --- Trailer ---
    buf.extend_from_slice(MAGIC);

    Bytes::from(buf)
}

/// Parse the fixed header of a sidecar blob, validating both magics.
pub fn read_header(buf: &[u8]) -> Result<Header> {
    if buf.len() < 12 {
        return Err(FtsError::TooShort {
            need: 12,
            have: buf.len(),
        });
    }
    if &buf[0..4] != MAGIC {
        return Err(FtsError::BadMagic);
    }
    // Trailing magic must also be present and correct.
    if buf.len() < 4 || &buf[buf.len() - 4..] != MAGIC {
        return Err(FtsError::BadMagic);
    }
    let version = rd_u32(buf, 4)?;
    if version != FORMAT_VERSION {
        return Err(FtsError::UnsupportedVersion(version));
    }
    let file_count = rd_u32(buf, 8)?;
    Ok(Header {
        version,
        file_count,
        manifest_offset: 12,
    })
}

/// Parse the manifest, validating every file's byte range against the buffer.
pub fn read_manifest(buf: &[u8], header: &Header) -> Result<Vec<ManifestEntry>> {
    let mut off = header.manifest_offset;
    let mut entries = Vec::with_capacity(header.file_count as usize);
    let data_floor = buf.len().saturating_sub(4); // bytes must end before trailing magic
    for _ in 0..header.file_count {
        let name_len = rd_u32(buf, off)? as usize;
        off += 4;
        let name_end = off
            .checked_add(name_len)
            .ok_or_else(|| FtsError::Corrupt("name length overflow".into()))?;
        let name_slice = buf.get(off..name_end).ok_or(FtsError::TooShort {
            need: name_end,
            have: buf.len(),
        })?;
        let name = std::str::from_utf8(name_slice)
            .map_err(|e| FtsError::Corrupt(format!("file name not utf-8: {e}")))?
            .to_string();
        off = name_end;
        let data_off = rd_u64(buf, off)? as usize;
        off += 8;
        let data_len = rd_u64(buf, off)? as usize;
        off += 8;
        let data_end = data_off
            .checked_add(data_len)
            .ok_or_else(|| FtsError::Corrupt("file data range overflow".into()))?;
        if data_end > data_floor {
            return Err(FtsError::Corrupt(format!(
                "file '{name}' data range [{data_off},{data_end}) runs past data region (floor {data_floor})"
            )));
        }
        entries.push(ManifestEntry {
            name,
            data_off,
            data_len,
        });
    }
    Ok(entries)
}

/// Reconstruct a [`RamDirectory`] from a sidecar blob: every packed file is
/// written back at its original relative path.
///
/// Returns an error (never panics) on a truncated or corrupt blob.
pub fn open_directory(buf: &[u8]) -> Result<RamDirectory> {
    let header = read_header(buf)?;
    let manifest = read_manifest(buf, &header)?;
    let dir = RamDirectory::create();
    for e in &manifest {
        let slice = buf
            .get(e.data_off..e.data_off + e.data_len)
            .ok_or(FtsError::TooShort {
                need: e.data_off + e.data_len,
                have: buf.len(),
            })?;
        // `atomic_write` populates the RamDirectory's in-memory file map; the
        // same map backs `open_read`, so files written via tantivy's
        // `open_write` during the build are readable after this restore.
        dir.atomic_write(std::path::Path::new(&e.name), slice)
            .map_err(|err| FtsError::Tantivy(format!("restore '{}': {err}", e.name)))?;
    }
    Ok(dir)
}

/// Open a searchable [`tantivy::Index`] from a sidecar blob (the read-side
/// entry point). Loads the whole blob into a `RamDirectory` (see module docs).
pub fn open_index(buf: &[u8]) -> Result<tantivy::Index> {
    let dir = open_directory(buf)?;
    let mut index =
        tantivy::Index::open(dir).map_err(|e| FtsError::Tantivy(format!("open index: {e}")))?;
    // Re-register our custom tokenizer on the opened index (tokenizer managers
    // are per-Index runtime state, not persisted in the segment files).
    index.tokenizers().register(
        crate::tokenizer::TOKENIZER_NAME,
        crate::tokenizer::PensieveWordTokenizer,
    );
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_files() -> Vec<(String, Vec<u8>)> {
        vec![
            ("meta.json".to_string(), b"{\"a\":1}".to_vec()),
            ("seg.store".to_string(), vec![0u8, 1, 2, 3, 4, 5]),
            (".managed.json".to_string(), b"[]".to_vec()),
        ]
    }

    #[test]
    fn header_and_manifest_round_trip() {
        let files = sample_files();
        let blob = write(&files);
        let h = read_header(&blob).unwrap();
        assert_eq!(h.version, FORMAT_VERSION);
        assert_eq!(h.file_count as usize, files.len());
        let m = read_manifest(&blob, &h).unwrap();
        assert_eq!(m.len(), files.len());
        for (entry, (name, bytes)) in m.iter().zip(&files) {
            assert_eq!(&entry.name, name);
            assert_eq!(entry.data_len, bytes.len());
            assert_eq!(
                &blob[entry.data_off..entry.data_off + entry.data_len],
                &bytes[..]
            );
        }
    }

    #[test]
    fn empty_file_list_round_trips() {
        let blob = write(&[]);
        let h = read_header(&blob).unwrap();
        assert_eq!(h.file_count, 0);
        assert!(read_manifest(&blob, &h).unwrap().is_empty());
    }

    #[test]
    fn bad_leading_magic_errs() {
        let mut b = write(&sample_files()).to_vec();
        b[0] = b'X';
        assert!(matches!(read_header(&b), Err(FtsError::BadMagic)));
    }

    #[test]
    fn bad_trailing_magic_errs() {
        let mut b = write(&sample_files()).to_vec();
        let n = b.len();
        b[n - 1] = b'X';
        assert!(matches!(read_header(&b), Err(FtsError::BadMagic)));
    }

    #[test]
    fn truncated_blob_errs_not_panics() {
        let blob = write(&sample_files());
        // Drop the trailing magic + part of the data — manifest offsets now
        // overrun the buffer.
        let truncated = &blob[..blob.len() - 6];
        // Header parse fails on missing trailing magic …
        assert!(read_header(truncated).is_err());
    }

    #[test]
    fn corrupt_manifest_range_errs() {
        let files = sample_files();
        let blob = write(&files);
        let h = read_header(&blob).unwrap();
        // Corrupt the first file's data_len to overflow the data region.
        let mut b = blob.to_vec();
        // First manifest entry: name_len(4) + name + data_off(8) + data_len(8).
        let name_len = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let data_len_off = 16 + name_len + 8;
        b[data_len_off..data_len_off + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(read_manifest(&b, &h), Err(FtsError::Corrupt(_))));
    }

    #[test]
    fn empty_buffer_errs() {
        assert!(read_header(&[]).is_err());
        assert!(read_header(&[1, 2, 3]).is_err());
    }
}
