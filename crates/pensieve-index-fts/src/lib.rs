//! Per-extent tantivy BM25 full-text-search index, stored as a format-agnostic
//! sidecar (one immutable blob per extent under
//! `{tenant}/indexes/{extent}/{column}.tantivy_fts`).
//!
//! The index replaces the catalog's coarse `LIKE`/token-set prune for the
//! lexical leg of search and memory with real BM25 ranking. Like the
//! [`pensieve_index_vector`] ANN sidecar it is built by the async
//! [`pensieve_core::index_sidecar::SidecarBuilder`] path: this crate provides
//! [`TantivyFtsBuilder`].
//!
//! ## Tokenizer parity
//!
//! The body field is analyzed with [`tokenizer::PensieveWordTokenizer`]
//! (`pensieve-word-v1`), which applies the extent writer's exact token rule
//! (lowercase, ASCII-alphanumeric runs, length ≥ 2). This guarantees the
//! catalog's existing token-set prune is never a false-negative for an
//! FTS-indexed extent: every term the writer records, tantivy also indexes.
//!
//! ## On-object layout
//!
//! A tantivy segment is several files; [`build::build_fts`] packs them into one
//! blob ([`file::write`]) and [`file::open_index`] reconstructs a `RamDirectory`
//! to search. Loading the whole blob is the simple, correct baseline; a
//! Quickwit-style hotcache with range reads is a later optimization.

pub mod build;
pub mod builder;
pub mod file;
pub mod search;
pub mod tokenizer;

pub use build::{build_fts, fts_schema, pack_addr, unpack_addr};
pub use builder::TantivyFtsBuilder;
pub use file::{open_index, read_header, FtsError};
pub use search::bm25_search;
pub use tokenizer::{tokenize, PensieveWordTokenizer, TOKENIZER_NAME};
