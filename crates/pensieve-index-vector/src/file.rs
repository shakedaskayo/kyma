//! On-object serialization for the IVF + 1-bit RaBitQ sidecar.
//!
//! Layout (all integers little-endian):
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ HEADER                                                             │
//! │   magic        : "KYIV" (4 bytes)                                  │
//! │   version      : u32                                               │
//! │   dim          : u32                                               │
//! │   metric       : u8   (0 = cosine)                                 │
//! │   nlist        : u32                                               │
//! │   model_id_len : u32                                               │
//! │   model_id     : model_id_len bytes of UTF-8                       │
//! ├──────────────────────────────────────────────────────────────────┤
//! │ CENTROID BLOCK : nlist × dim × f32                                 │
//! ├──────────────────────────────────────────────────────────────────┤
//! │ PARTITION 0 .. PARTITION nlist-1, each:                            │
//! │   count        : u32                                               │
//! │   entries[count] each:                                             │
//! │     block      : u32                                               │
//! │     row        : u32                                               │
//! │     code        : ceil(dim/8) bytes                                │
//! │     correction : f32                                               │
//! ├──────────────────────────────────────────────────────────────────┤
//! │ FOOTER                                                             │
//! │   directory : nlist × (offset u64, byte_len u64)                   │
//! │               offset is absolute from the start of the blob, to    │
//! │               the partition's `count` field.                       │
//! │   footer_offset : u64  (absolute offset of `directory` start)      │
//! │   magic         : "KYIV" (4 bytes, trailing)                       │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The footer lets a reader seek to any single partition without parsing the
//! whole file — the Part B query path probes the centroids, then loads only the
//! `nprobe` partitions it needs. Reads validate every offset/length against the
//! buffer bounds and return [`SidecarError`] rather than panicking on
//! truncated or corrupt input.

use crate::rabitq::{code_len, VectorCode};
use bytes::Bytes;
use kyma_core::index_sidecar::RowAddress;
use kyma_core::segment_format::BlockId;
use thiserror::Error;

/// 4-byte magic at the head and tail of the sidecar blob.
pub const MAGIC: &[u8; 4] = b"KYIV";

/// On-object format version.
pub const FORMAT_VERSION: u32 = 1;

/// Metric code for cosine (the only supported metric today).
pub const METRIC_COSINE: u8 = 0;

/// Errors reading or interpreting a sidecar blob.
#[derive(Debug, Error)]
pub enum SidecarError {
    #[error("sidecar too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },
    #[error("bad magic bytes")]
    BadMagic,
    #[error("unsupported sidecar version {0}")]
    UnsupportedVersion(u32),
    #[error("corrupt sidecar: {0}")]
    Corrupt(String),
    #[error("partition index {idx} out of range (nlist={nlist})")]
    PartitionOutOfRange { idx: usize, nlist: usize },
}

type Result<T> = std::result::Result<T, SidecarError>;

/// One posting-list entry: the row's extent address + its RaBitQ code.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub addr: RowAddress,
    pub code: VectorCode,
}

/// Fixed-size header fields parsed from the front of a sidecar.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub version: u32,
    pub dim: u32,
    pub metric: u8,
    pub nlist: u32,
    pub model_id: Option<String>,
    /// Byte offset of the centroid block (just past the header).
    pub centroids_offset: usize,
}

// ---- little-endian read helpers (bounds-checked) ----

fn rd_u32(buf: &[u8], off: usize) -> Result<u32> {
    let end = off
        .checked_add(4)
        .ok_or_else(|| SidecarError::Corrupt("offset overflow".into()))?;
    let slice = buf.get(off..end).ok_or(SidecarError::TooShort {
        need: end,
        have: buf.len(),
    })?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

fn rd_u64(buf: &[u8], off: usize) -> Result<u64> {
    let end = off
        .checked_add(8)
        .ok_or_else(|| SidecarError::Corrupt("offset overflow".into()))?;
    let slice = buf.get(off..end).ok_or(SidecarError::TooShort {
        need: end,
        have: buf.len(),
    })?;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

fn rd_f32(buf: &[u8], off: usize) -> Result<f32> {
    let end = off
        .checked_add(4)
        .ok_or_else(|| SidecarError::Corrupt("offset overflow".into()))?;
    let slice = buf.get(off..end).ok_or(SidecarError::TooShort {
        need: end,
        have: buf.len(),
    })?;
    Ok(f32::from_le_bytes(slice.try_into().unwrap()))
}

/// Serialize an IVF+RaBitQ index into a sidecar blob.
///
/// `centroids` is `nlist` vectors of length `dim`. `partitions[i]` holds the
/// posting-list entries assigned to centroid `i`.
pub fn write(
    dim: usize,
    metric: u8,
    centroids: &[Vec<f32>],
    partitions: &[Vec<Entry>],
    model_id: Option<&str>,
) -> Bytes {
    let nlist = centroids.len();
    assert_eq!(
        nlist,
        partitions.len(),
        "centroids/partitions length mismatch"
    );
    let clen = code_len(dim);

    let mut buf: Vec<u8> = Vec::new();

    // --- Header ---
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.extend_from_slice(&(dim as u32).to_le_bytes());
    buf.push(metric);
    buf.extend_from_slice(&(nlist as u32).to_le_bytes());
    let model_bytes = model_id.unwrap_or("").as_bytes();
    buf.extend_from_slice(&(model_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(model_bytes);

    // --- Centroid block ---
    for c in centroids {
        debug_assert_eq!(c.len(), dim);
        for &x in c {
            buf.extend_from_slice(&x.to_le_bytes());
        }
    }

    // --- Partitions, recording the directory as we go ---
    let mut directory: Vec<(u64, u64)> = Vec::with_capacity(nlist);
    for part in partitions {
        let start = buf.len() as u64;
        buf.extend_from_slice(&(part.len() as u32).to_le_bytes());
        for e in part {
            buf.extend_from_slice(&e.addr.block.0.to_le_bytes());
            buf.extend_from_slice(&e.addr.row.to_le_bytes());
            debug_assert_eq!(e.code.code.len(), clen);
            buf.extend_from_slice(&e.code.code);
            buf.extend_from_slice(&e.code.correction.to_le_bytes());
        }
        let len = buf.len() as u64 - start;
        directory.push((start, len));
    }

    // --- Footer ---
    let footer_offset = buf.len() as u64;
    for (off, len) in &directory {
        buf.extend_from_slice(&off.to_le_bytes());
        buf.extend_from_slice(&len.to_le_bytes());
    }
    buf.extend_from_slice(&footer_offset.to_le_bytes());
    buf.extend_from_slice(MAGIC);

    Bytes::from(buf)
}

/// Parse the header (and centroid offset) of a sidecar blob.
pub fn read_header(buf: &[u8]) -> Result<Header> {
    if buf.len() < 4 {
        return Err(SidecarError::TooShort {
            need: 4,
            have: buf.len(),
        });
    }
    if &buf[0..4] != MAGIC {
        return Err(SidecarError::BadMagic);
    }
    let version = rd_u32(buf, 4)?;
    if version != FORMAT_VERSION {
        return Err(SidecarError::UnsupportedVersion(version));
    }
    let dim = rd_u32(buf, 8)?;
    let metric = *buf.get(12).ok_or(SidecarError::TooShort {
        need: 13,
        have: buf.len(),
    })?;
    let nlist = rd_u32(buf, 13)?;
    let model_len = rd_u32(buf, 17)? as usize;
    let model_start = 21usize;
    let model_end = model_start
        .checked_add(model_len)
        .ok_or_else(|| SidecarError::Corrupt("model_id length overflow".into()))?;
    let model_slice = buf
        .get(model_start..model_end)
        .ok_or(SidecarError::TooShort {
            need: model_end,
            have: buf.len(),
        })?;
    let model_id = if model_len == 0 {
        None
    } else {
        Some(
            std::str::from_utf8(model_slice)
                .map_err(|e| SidecarError::Corrupt(format!("model_id not utf-8: {e}")))?
                .to_string(),
        )
    };
    Ok(Header {
        version,
        dim,
        metric,
        nlist,
        model_id,
        centroids_offset: model_end,
    })
}

/// Read the centroid block (`nlist × dim` f32) given a parsed header.
pub fn read_centroids(buf: &[u8], header: &Header) -> Result<Vec<Vec<f32>>> {
    let dim = header.dim as usize;
    let nlist = header.nlist as usize;
    let mut off = header.centroids_offset;
    let mut centroids = Vec::with_capacity(nlist);
    for _ in 0..nlist {
        let mut c = Vec::with_capacity(dim);
        for _ in 0..dim {
            c.push(rd_f32(buf, off)?);
            off += 4;
        }
        centroids.push(c);
    }
    Ok(centroids)
}

/// Locate the footer directory: returns `(footer_offset, nlist)` after
/// validating the trailing magic.
fn read_footer_directory(buf: &[u8], nlist: usize) -> Result<u64> {
    // trailing: directory(nlist×16) || footer_offset u64 || magic(4)
    if buf.len() < 12 {
        return Err(SidecarError::TooShort {
            need: 12,
            have: buf.len(),
        });
    }
    let tail = buf.len();
    if &buf[tail - 4..tail] != MAGIC {
        return Err(SidecarError::BadMagic);
    }
    let footer_offset = rd_u64(buf, tail - 12)?;
    // The directory must fit between footer_offset and the footer_offset field.
    let dir_bytes = (nlist as u64)
        .checked_mul(16)
        .ok_or_else(|| SidecarError::Corrupt("directory size overflow".into()))?;
    let dir_end = footer_offset
        .checked_add(dir_bytes)
        .ok_or_else(|| SidecarError::Corrupt("directory end overflow".into()))?;
    if dir_end as usize != tail - 12 {
        return Err(SidecarError::Corrupt(format!(
            "footer directory bounds mismatch: dir_end={dir_end}, expected {}",
            tail - 12
        )));
    }
    Ok(footer_offset)
}

/// Read one partition's posting list by index, resolving its byte range
/// through the footer directory.
pub fn read_partition(buf: &[u8], header: &Header, idx: usize) -> Result<Vec<Entry>> {
    let nlist = header.nlist as usize;
    if idx >= nlist {
        return Err(SidecarError::PartitionOutOfRange { idx, nlist });
    }
    let footer_offset = read_footer_directory(buf, nlist)?;
    let dir_entry_off = footer_offset as usize + idx * 16;
    let part_off = rd_u64(buf, dir_entry_off)? as usize;
    let part_len = rd_u64(buf, dir_entry_off + 8)? as usize;
    let part_end = part_off
        .checked_add(part_len)
        .ok_or_else(|| SidecarError::Corrupt("partition end overflow".into()))?;
    if part_end > buf.len() {
        return Err(SidecarError::TooShort {
            need: part_end,
            have: buf.len(),
        });
    }
    decode_partition(buf, part_off, part_end, header.dim as usize)
}

fn decode_partition(buf: &[u8], start: usize, end: usize, dim: usize) -> Result<Vec<Entry>> {
    let clen = code_len(dim);
    let entry_size = 4 + 4 + clen + 4; // block + row + code + correction
    let count = rd_u32(buf, start)? as usize;
    let mut off = start + 4;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let needed = off
            .checked_add(entry_size)
            .ok_or_else(|| SidecarError::Corrupt("entry offset overflow".into()))?;
        if needed > end {
            return Err(SidecarError::Corrupt(format!(
                "partition entry runs past its declared length (need {needed}, end {end})"
            )));
        }
        let block = rd_u32(buf, off)?;
        let row = rd_u32(buf, off + 4)?;
        let code = buf[off + 8..off + 8 + clen].to_vec();
        let correction = rd_f32(buf, off + 8 + clen)?;
        entries.push(Entry {
            addr: RowAddress {
                block: BlockId(block),
                row,
            },
            code: VectorCode { code, correction },
        });
        off = needed;
    }
    Ok(entries)
}

/// The `nprobe` nearest centroids to a normalized query, by the sidecar's
/// metric. Returns partition indices, nearest first.
///
/// For cosine on the unit sphere, nearest by squared L2 = nearest by cosine.
pub fn probe(
    buf: &[u8],
    header: &Header,
    query_normalized: &[f32],
    nprobe: usize,
) -> Result<Vec<usize>> {
    let centroids = read_centroids(buf, header)?;
    if query_normalized.len() != header.dim as usize {
        return Err(SidecarError::Corrupt(format!(
            "query dim {} != index dim {}",
            query_normalized.len(),
            header.dim
        )));
    }
    let mut scored: Vec<(usize, f32)> = centroids
        .iter()
        .enumerate()
        .map(|(i, c)| (i, crate::ivf::sq_l2(query_normalized, c)))
        .collect();
    // Stable order: distance asc, then index asc (deterministic ties).
    scored.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let k = nprobe.min(scored.len());
    Ok(scored.into_iter().take(k).map(|(i, _)| i).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rabitq::{encode_rotated, Rotation};
    use crate::rng::SplitMix64;

    fn norm(mut v: Vec<f32>) -> Vec<f32> {
        let n = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
        if n > 1e-12 {
            for x in &mut v {
                *x /= n;
            }
        }
        v
    }

    fn sample_index() -> (Bytes, usize, Vec<Vec<f32>>) {
        let dim = 16;
        let rot = Rotation::new(dim);
        let mut rng = SplitMix64::new(5);
        let centroids: Vec<Vec<f32>> = (0..4)
            .map(|_| norm((0..dim).map(|_| rng.next_gaussian() as f32).collect()))
            .collect();
        let mut partitions: Vec<Vec<Entry>> = vec![Vec::new(); 4];
        for blk in 0..3u32 {
            for row in 0..5u32 {
                let v = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
                let code = encode_rotated(&rot.apply(&v));
                let p = (blk * 5 + row) as usize % 4;
                partitions[p].push(Entry {
                    addr: RowAddress {
                        block: BlockId(blk),
                        row,
                    },
                    code,
                });
            }
        }
        let bytes = write(
            dim,
            METRIC_COSINE,
            &centroids,
            &partitions,
            Some("test/model"),
        );
        (bytes, dim, centroids)
    }

    #[test]
    fn header_round_trip() {
        let (bytes, dim, _c) = sample_index();
        let h = read_header(&bytes).unwrap();
        assert_eq!(h.version, FORMAT_VERSION);
        assert_eq!(h.dim as usize, dim);
        assert_eq!(h.metric, METRIC_COSINE);
        assert_eq!(h.nlist, 4);
        assert_eq!(h.model_id.as_deref(), Some("test/model"));
    }

    #[test]
    fn centroids_round_trip() {
        let (bytes, _dim, centroids) = sample_index();
        let h = read_header(&bytes).unwrap();
        let back = read_centroids(&bytes, &h).unwrap();
        assert_eq!(back, centroids);
    }

    #[test]
    fn partitions_round_trip() {
        let (bytes, dim, _c) = sample_index();
        let h = read_header(&bytes).unwrap();
        let rot = Rotation::new(dim);
        let mut rng = SplitMix64::new(5);
        // Re-derive expected partitions exactly as sample_index did.
        let _centroids: Vec<Vec<f32>> = (0..4)
            .map(|_| norm((0..dim).map(|_| rng.next_gaussian() as f32).collect()))
            .collect();
        let mut expected: Vec<Vec<Entry>> = vec![Vec::new(); 4];
        for blk in 0..3u32 {
            for row in 0..5u32 {
                let v = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
                let code = encode_rotated(&rot.apply(&v));
                let p = (blk * 5 + row) as usize % 4;
                expected[p].push(Entry {
                    addr: RowAddress {
                        block: BlockId(blk),
                        row,
                    },
                    code,
                });
            }
        }
        for i in 0..4 {
            let got = read_partition(&bytes, &h, i).unwrap();
            assert_eq!(got, expected[i], "partition {i} mismatch");
        }
    }

    #[test]
    fn partition_out_of_range_errs() {
        let (bytes, _dim, _c) = sample_index();
        let h = read_header(&bytes).unwrap();
        assert!(matches!(
            read_partition(&bytes, &h, 99),
            Err(SidecarError::PartitionOutOfRange { .. })
        ));
    }

    #[test]
    fn truncated_footer_errs_not_panics() {
        let (bytes, _dim, _c) = sample_index();
        let h = read_header(&bytes).unwrap();
        // Lop off the trailing magic + footer offset.
        let truncated = &bytes[..bytes.len() - 8];
        let r = read_partition(truncated, &h, 0);
        assert!(r.is_err(), "expected error on truncated footer");
    }

    #[test]
    fn corrupt_magic_errs() {
        let (bytes, _dim, _c) = sample_index();
        let mut b = bytes.to_vec();
        b[0] = b'X';
        assert!(matches!(read_header(&b), Err(SidecarError::BadMagic)));
    }

    #[test]
    fn empty_buffer_errs() {
        assert!(read_header(&[]).is_err());
        assert!(read_header(&[1, 2]).is_err());
    }

    #[test]
    fn probe_matches_bruteforce() {
        let (bytes, dim, centroids) = sample_index();
        let h = read_header(&bytes).unwrap();
        let mut rng = SplitMix64::new(123);
        for _ in 0..20 {
            let q = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
            let probed = probe(&bytes, &h, &q, 2).unwrap();
            // Brute-force nearest 2 centroids.
            let mut bf: Vec<(usize, f32)> = centroids
                .iter()
                .enumerate()
                .map(|(i, c)| (i, crate::ivf::sq_l2(&q, c)))
                .collect();
            bf.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.cmp(&b.0)));
            let expected: Vec<usize> = bf.into_iter().take(2).map(|(i, _)| i).collect();
            assert_eq!(probed, expected);
        }
    }
}
