//! kyma-core — the architectural contract of the kyma engine.
//!
//! Every other crate in the workspace depends on this one (and this one
//! depends on no sibling crates). The trait surfaces declared here are the
//! non-negotiable boundaries between subsystems: storage formats, indices,
//! catalog, and query frontends. Any change to these traits is a major
//! architectural decision — review carefully.
//!
//! # Module map
//!
//! - [`types`] — opaque id types, schema reference alias.
//! - [`errors`] — typed errors; all fallible functions return `Result<T>`.
//! - [`segment_format`] — `SegmentFormat` + reader/writer traits.
//! - [`index`] — `IndexProvider`, `PostingList`, `TermDict`.
//! - [`catalog`] — `Catalog`, `SnapshotTxn`, `ExtentManifest`, `PrunePredicate`.
//! - [`query_frontend`] — `QueryFrontend`, `QueryContext`, `QueryBudget`.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod credentials;
pub mod crypto;
pub mod errors;
pub mod index;
pub mod query_frontend;
pub mod segment_format;
pub mod tenant;
pub mod types;

pub use errors::{Error, Result};
pub use tenant::{TenantId, DEFAULT_TENANT};
