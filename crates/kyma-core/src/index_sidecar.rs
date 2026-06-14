//! Index sidecars — per-extent auxiliary index objects (S0.4 plumbing).
//!
//! A *sidecar* is an immutable object-store blob built **from** one extent's
//! data but stored **next to** the extent (under `{tenant}/indexes/…`), with a
//! catalog row (`extent_indexes`) describing it. S1's ANN (IVF+RaBitQ) and FTS
//! (tantivy) builders produce real sidecars; S0 ships only this vocabulary,
//! the catalog surface ([`crate::catalog::Catalog::register_index_sidecar`]
//! et al.), the `index_build` job executor in `kyma-jobs`, and a disk cache in
//! `kyma-storage`.
//!
//! Lifecycle invariants:
//! - Sidecars are immutable and derived: losing one only costs a rebuild.
//! - Upload-then-register: a crash mid-build leaves at worst an orphan object,
//!   never a catalog row pointing at a missing object.
//! - `extent_indexes` rows cascade with their extent row; the physical-delete
//!   GC removes sidecar objects under the same grace period as extent data.

use crate::catalog::ExtentManifest;
use crate::errors::{Error, Result};
use crate::segment_format::BlockId;
use crate::types::{ExtentId, TableId};
use arrow_array::RecordBatch;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use uuid::Uuid;

/// The kind of index a sidecar holds. String codes are **stable** — they
/// appear in the catalog (`extent_indexes.kind`), in job payloads, and in
/// object paths; never repurpose one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarKind {
    /// IVF-partitioned `RaBitQ` vector index (ANN). Built by S1.
    IvfRabitq,
    /// Tantivy full-text-search index. Built by S1.
    TantivyFts,
}

impl SidecarKind {
    /// The stable string code stored in the catalog and used in object paths.
    pub const fn as_str(self) -> &'static str {
        match self {
            SidecarKind::IvfRabitq => "ivf_rabitq",
            SidecarKind::TantivyFts => "tantivy_fts",
        }
    }

    /// Every known kind (for registries / conformance tests).
    pub const ALL: [SidecarKind; 2] = [SidecarKind::IvfRabitq, SidecarKind::TantivyFts];
}

impl std::fmt::Display for SidecarKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SidecarKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "ivf_rabitq" => Ok(SidecarKind::IvfRabitq),
            "tantivy_fts" => Ok(SidecarKind::TantivyFts),
            other => Err(Error::Config(format!("unknown sidecar kind: {other}"))),
        }
    }
}

/// Address of one row inside an extent: which block, and the row offset
/// within that block. Sidecars store these so a postings/ANN hit can be
/// resolved back to extent data without a full scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RowAddress {
    pub block: BlockId,
    pub row: u32,
}

/// Catalog row describing one sidecar object (`extent_indexes` table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexSidecarDescriptor {
    pub id: Uuid,
    pub extent_id: ExtentId,
    pub table_id: TableId,
    /// Source column the index was built over.
    pub column: String,
    pub kind: SidecarKind,
    /// Object-store path of the sidecar blob
    /// (convention: `{tenant}/indexes/{extent_id}/{column}.{kind}`).
    pub object_path: String,
    pub byte_size: u64,
    /// Builder-specific parameters (nlist, quantization bits, tokenizer, …).
    pub params: Json,
    /// For ANN sidecars: the embedding model the vectors came from. Part of
    /// the idempotency key — one column may carry sidecars for several models.
    pub embedding_model_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A finished sidecar, ready to upload + register.
#[derive(Debug, Clone)]
pub struct BuiltSidecar {
    /// The serialized index blob.
    pub bytes: Bytes,
    /// Effective build parameters, persisted to `extent_indexes.params`.
    pub params: Json,
    /// Embedding model the index was built against (ANN builders); `None`
    /// for model-free kinds (FTS).
    pub embedding_model_id: Option<String>,
}

/// Stream of record batches read from one extent, block by block — the shape
/// [`crate::segment_format::ExtentReader::read_block`] produces. Batches
/// arrive in block order so builders can derive [`RowAddress`]es by counting.
pub type BatchStream<'a> = BoxStream<'a, Result<RecordBatch>>;

/// Builds one sidecar from one extent's data.
///
/// S0 ships **no production implementations** — this trait is the seam S1's
/// IVF+RaBitQ and tantivy builders plug into. The `index_build` job executor
/// (`kyma-jobs::index_build`) drives it: it opens the extent through the
/// [`SegmentFormat`][crate::segment_format::SegmentFormat], streams every
/// block (all columns; the builder projects by name), and uploads + registers
/// the result.
#[async_trait]
pub trait SidecarBuilder: Send + Sync {
    /// The kind of sidecar this builder produces.
    fn kind(&self) -> SidecarKind;

    /// Consume the extent's batches and produce the serialized sidecar.
    ///
    /// `column` is the source column to index; batches carry the extent's
    /// full schema, in block order. Implementations should be deterministic
    /// for a given (extent, column, params) so rebuilds converge.
    async fn build(
        &self,
        extent: &ExtentManifest,
        column: &str,
        batches: BatchStream<'_>,
    ) -> Result<BuiltSidecar>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn kind_codes_round_trip() {
        for kind in SidecarKind::ALL {
            let code = kind.to_string();
            assert_eq!(SidecarKind::from_str(&code).unwrap(), kind);
            // Serde uses the same stable codes as Display/FromStr.
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::String(code)
            );
        }
        assert_eq!(SidecarKind::IvfRabitq.as_str(), "ivf_rabitq");
        assert_eq!(SidecarKind::TantivyFts.as_str(), "tantivy_fts");
        assert!(SidecarKind::from_str("bogus").is_err());
    }

    #[test]
    fn descriptor_serde_round_trips() {
        let desc = IndexSidecarDescriptor {
            id: Uuid::new_v4(),
            extent_id: ExtentId::new(),
            table_id: TableId::new(),
            column: "embedding".into(),
            kind: SidecarKind::IvfRabitq,
            object_path: "t/indexes/e/embedding.ivf_rabitq".into(),
            byte_size: 1234,
            params: serde_json::json!({ "nlist": 64 }),
            embedding_model_id: Some("fastembed/bge-small-en-v1.5".into()),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&desc).unwrap();
        let back: IndexSidecarDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, desc);
        // The kind field serializes as its stable code.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["kind"], "ivf_rabitq");
    }

    #[test]
    fn row_address_serde_round_trips() {
        let addr = RowAddress {
            block: BlockId(7),
            row: 42,
        };
        let json = serde_json::to_string(&addr).unwrap();
        let back: RowAddress = serde_json::from_str(&json).unwrap();
        assert_eq!(back, addr);
    }
}
