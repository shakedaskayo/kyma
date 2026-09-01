//! Build side: turn an extent's text rows into a packed tantivy index sidecar.
//!
//! tantivy writes a segment as a handful of files (`meta.json`, `*.idx`,
//! `*.pos`, `*.term`, `*.store`, …) through a [`Directory`]. We build into a
//! throwaway temp directory, commit, then read every produced file back and
//! pack them all into one sidecar blob via [`crate::file::write`]. The read
//! side ([`crate::file::open_index`]) reconstructs a `RamDirectory` from the
//! same files. Building in a temp dir (rather than a `RamDirectory`) is the
//! robust path: tantivy's `RamDirectory` exposes no way to enumerate the files
//! it produced, but a real directory does.

use std::io::Read;

use bytes::Bytes;
use tantivy::schema::{
    IndexRecordOption, Schema, SchemaBuilder, TextFieldIndexing, TextOptions, INDEXED, STORED,
};
use tantivy::{doc, Index, TantivyDocument};

use pensieve_core::index_sidecar::RowAddress;

use crate::file::FtsError;
use crate::tokenizer::{PensieveWordTokenizer, MIN_TOKEN_LEN, TOKENIZER_NAME};

type Result<T> = std::result::Result<T, FtsError>;

/// Field name of the indexed document body.
pub const BODY_FIELD: &str = "body";
/// Field name of the packed `RowAddress` (`block << 32 | row`), stored only.
pub const ADDR_FIELD: &str = "addr";

/// Pack a [`RowAddress`] into one `u64` for a stored fast field.
#[inline]
pub fn pack_addr(addr: RowAddress) -> u64 {
    (u64::from(addr.block.0) << 32) | u64::from(addr.row)
}

/// Inverse of [`pack_addr`].
#[inline]
pub fn unpack_addr(v: u64) -> RowAddress {
    RowAddress {
        block: pensieve_core::segment_format::BlockId((v >> 32) as u32),
        row: (v & 0xFFFF_FFFF) as u32,
    }
}

/// The tantivy schema for an FTS sidecar: a tokenized, position-indexed body
/// field (BM25 + phrase support) and a stored `addr` field carrying the row's
/// `RowAddress`. Shared by the build and search paths so field handles line up.
pub fn fts_schema() -> Schema {
    let mut b: SchemaBuilder = Schema::builder();
    // Body: tokenized with `pensieve-word-v1` (set explicitly so it matches the
    // writer's token rule), indexed WithFreqsAndPositions for BM25 + phrase
    // queries. The same tokenizer name is re-registered by file::open_index.
    let body_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER_NAME)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    b.add_text_field(BODY_FIELD, body_opts);
    b.add_u64_field(ADDR_FIELD, STORED | INDEXED);
    b.build()
}

/// Build a tantivy BM25 index over `(RowAddress, text)` rows and serialize it
/// into a single sidecar blob. Returns `(bytes, params_json)`.
///
/// Deterministic for a given row set: one segment, single-threaded writer, no
/// timestamps in the packed files that vary across identical builds.
pub fn build_fts<I>(rows: I) -> Result<(Bytes, serde_json::Value)>
where
    I: IntoIterator<Item = (RowAddress, String)>,
{
    let schema = fts_schema();
    let body = schema.get_field(BODY_FIELD).expect("body field");
    let addr = schema.get_field(ADDR_FIELD).expect("addr field");

    let tmp = tempfile::tempdir().map_err(|e| FtsError::Tantivy(format!("tempdir: {e}")))?;
    let index = Index::create_in_dir(tmp.path(), schema.clone())
        .map_err(|e| FtsError::Tantivy(format!("create index: {e}")))?;
    index
        .tokenizers()
        .register(TOKENIZER_NAME, PensieveWordTokenizer);

    let mut writer = index
        .writer::<TantivyDocument>(15_000_000)
        .map_err(|e| FtsError::Tantivy(format!("writer: {e}")))?;
    let mut n_docs: u64 = 0;
    for (row_addr, text) in rows {
        writer
            .add_document(doc!(body => text, addr => pack_addr(row_addr)))
            .map_err(|e| FtsError::Tantivy(format!("add_document: {e}")))?;
        n_docs += 1;
    }
    writer
        .commit()
        .map_err(|e| FtsError::Tantivy(format!("commit: {e}")))?;
    // Release file handles before reading the directory back.
    drop(writer);
    drop(index);

    // Read every file tantivy produced (skip lock files — runtime-only).
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in
        std::fs::read_dir(tmp.path()).map_err(|e| FtsError::Tantivy(format!("readdir: {e}")))?
    {
        let entry = entry.map_err(|e| FtsError::Tantivy(format!("dirent: {e}")))?;
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".lock") {
            continue;
        }
        let mut f = std::fs::File::open(entry.path())
            .map_err(|e| FtsError::Tantivy(format!("open {name}: {e}")))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .map_err(|e| FtsError::Tantivy(format!("read {name}: {e}")))?;
        files.push((name, buf));
    }
    // Deterministic order so identical row sets yield identical blobs.
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let bytes = crate::file::write(&files);
    let params = serde_json::json!({
        "tokenizer": TOKENIZER_NAME,
        "min_token_len": MIN_TOKEN_LEN,
        "field": BODY_FIELD,
        "docs": n_docs,
        "version": 1,
    });
    Ok((bytes, params))
}
