//! `SegmentFormat` — the pluggable storage-format trait.
//!
//! A `SegmentFormat` encapsulates one on-disk format (telemetry, vector,
//! metrics, wide-table, …). The engine interacts with storage only through
//! these traits; swapping formats is a matter of providing a new impl.
//!
//! # Trait hierarchy
//!
//! - [`SegmentFormat`] — factory for readers and writers of this format.
//! - [`ExtentReader`] — an open extent, surfaces metadata and block reads.
//! - [`ExtentWriter`] — a not-yet-flushed extent under construction.
//! - [`BlockPredicate`] — a small predicate language the format can
//!   evaluate against block-level stats (for pruning before decode).

use crate::errors::Result;
use crate::types::{ExtentId, SchemaRef, TableId};
use arrow_array::RecordBatch;
use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;

/// Identifier of one block within an extent.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BlockId(pub u32);

/// Identifier of one column in a schema.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ColumnId(pub u32);

/// A pluggable on-disk format.
#[async_trait]
pub trait SegmentFormat: Send + Sync + 'static {
    /// Short human-readable name. Appears in format version headers.
    fn name(&self) -> &'static str;

    /// Magic bytes prefix written at the start of every extent of this format.
    /// Used to detect mis-addressed reads (wrong format in wrong bucket).
    fn magic(&self) -> &'static [u8];

    /// Current format version this implementation writes.
    fn current_version(&self) -> u32;

    /// Maximum format version this implementation can read.
    fn max_readable_version(&self) -> u32;

    /// Open an existing extent for reads.
    async fn open_extent(&self, input: OpenExtentInput) -> Result<Arc<dyn ExtentReader>>;

    /// Start writing a new extent.
    async fn start_extent(
        &self,
        schema: SchemaRef,
        target_bytes: u64,
    ) -> Result<Box<dyn ExtentWriter>>;

    /// Borrow the underlying object store, when the format is object-store
    /// backed.
    ///
    /// The ANN query path (`kyma-exec`'s `ann_topk`) needs the raw store to
    /// fetch per-extent index sidecars (which live next to extents under
    /// `{tenant}/indexes/…`, not addressed through the format). A format that
    /// is not object-store backed returns `None`, and callers fall back to the
    /// SQL/exact path. Default `None` so existing/mock formats need no change.
    fn object_store(&self) -> Option<Arc<dyn object_store::ObjectStore>> {
        None
    }
}

/// Parameters needed to open an extent for reads.
#[derive(Debug, Clone)]
pub struct OpenExtentInput {
    pub extent_id: ExtentId,
    pub table_id: TableId,
    pub schema: SchemaRef,
    pub object_path: String,
    pub byte_size: u64,
}

/// An open extent, readable in terms of blocks of Arrow `RecordBatch`es.
#[async_trait]
pub trait ExtentReader: Send + Sync {
    /// Metadata about this extent (row count, block count, byte size).
    fn metadata(&self) -> &ExtentMetadata;

    /// Decide which blocks survive the given predicate based on block-level
    /// stats and indices. Cheap metadata work; does not fetch block bytes.
    async fn pruned_blocks(&self, predicate: &BlockPredicate) -> Result<Vec<BlockId>>;

    /// Read one block, projecting to the requested columns only.
    async fn read_block(&self, block: BlockId, projection: &[ColumnId]) -> Result<RecordBatch>;
}

/// Extent-level metadata surfaced to planners.
#[derive(Debug, Clone)]
pub struct ExtentMetadata {
    pub row_count: u64,
    pub block_count: u32,
    pub byte_size: u64,
    pub schema: SchemaRef,
    pub format_version: u32,
}

/// A lowered, format-agnostic predicate evaluable at the block / extent level.
///
/// Richer expressions live in `kyma-plan`'s `LogicalPlan`; the exec layer
/// translates them to this shape before calling into the format.
#[derive(Debug, Clone)]
pub enum BlockPredicate {
    /// No predicate; every block survives.
    All,
    And(Vec<BlockPredicate>),
    Or(Vec<BlockPredicate>),
    Not(Box<BlockPredicate>),
    /// `col` ∈ [start, end).
    TimeRange {
        col: ColumnId,
        start_inclusive: i64,
        end_exclusive: i64,
    },
    Equals {
        col: ColumnId,
        value: ScalarValue,
    },
    InSet {
        col: ColumnId,
        values: Vec<ScalarValue>,
    },
    /// Full-text-search predicate on a string column with inverted index.
    StringHas {
        col: ColumnId,
        needle: String,
    },
    /// Dynamic-column path existence predicate.
    DynamicPathExists {
        col: ColumnId,
        path: String,
    },
    /// Dynamic-column path equality predicate.
    DynamicPathEquals {
        col: ColumnId,
        path: String,
        value: ScalarValue,
    },
}

/// Minimal scalar value used in block-level predicates.
#[derive(Debug, Clone)]
pub enum ScalarValue {
    Null,
    Bool(bool),
    Int64(i64),
    Float64(f64),
    String(String),
    Bytes(Bytes),
    Timestamp(i64), // nanoseconds since Unix epoch
}

/// A not-yet-flushed extent under construction.
///
/// Implementations buffer rows in memory, emit blocks to an object-store
/// multipart upload, and write a footer on [`finish`].
#[async_trait]
pub trait ExtentWriter: Send {
    /// Append a batch of rows.
    async fn append(&mut self, batch: RecordBatch) -> Result<()>;

    /// Seal the extent: flush the final block, write the footer, complete
    /// the upload. The returned result carries everything the catalog needs
    /// to register the new extent.
    async fn finish(self: Box<Self>) -> Result<ExtentWriteResult>;
}

/// Information captured after an extent is successfully written.
#[derive(Debug, Clone)]
pub struct ExtentWriteResult {
    pub extent_id: ExtentId,
    pub object_path: String,
    pub byte_size: u64,
    pub row_count: u64,
    pub block_count: u32,
    /// Min/max timestamp across the extent (if the schema has a timestamp).
    pub min_timestamp_nanos: Option<i64>,
    pub max_timestamp_nanos: Option<i64>,
    /// Set of dynamic-column paths present anywhere in the extent.
    pub present_paths: Vec<String>,
    /// Per-column statistics captured during write.
    ///
    /// Key is the column name. Value is a JSON object:
    /// ```json
    /// { "distinct": ["v1", "v2", ...] } // null if cardinality exceeds threshold
    /// ```
    /// Consumed by the catalog's `column_stats` for equality-pushdown
    /// pruning. Unknown keys in this field are ignored by the engine.
    pub column_stats: serde_json::Value,
}

/// Per-extent format dispatch over several [`SegmentFormat`] impls.
///
/// The engine threads a single `Arc<dyn SegmentFormat>` everywhere; wrapping the
/// formats in a `FormatRegistry` (which itself *is* a `SegmentFormat`) keeps that
/// surface unchanged while letting new extents use one format and reads pick the
/// right decoder per object:
///
/// - `start_extent` always delegates to the configured **write** format
///   (`KYMA_WRITE_FORMAT`), so writes flip wholesale.
/// - `open_extent` sniffs the object's first 4 magic bytes and delegates to
///   whichever registered reader claims them (`magic()[..4]`). TLM (`KYMA…`,
///   both v1/v2) and Parquet (`PAR1`) coexist, so old extents stay readable
///   forever and compaction migrates formats organically.
///
/// On a sniff failure (tiny/unreadable head) it falls back to the write format —
/// correctness is preserved because a wrong guess just fails the subsequent
/// decode loudly rather than silently mis-reading.
pub struct FormatRegistry {
    write: Arc<dyn SegmentFormat>,
    readers: Vec<Arc<dyn SegmentFormat>>,
}

impl FormatRegistry {
    /// `write` is used for new extents and is also a reader. `extra_readers` are
    /// additional decoders (e.g. the legacy TLM format when writing Parquet).
    pub fn new(write: Arc<dyn SegmentFormat>, extra_readers: Vec<Arc<dyn SegmentFormat>>) -> Self {
        let mut readers = Vec::with_capacity(1 + extra_readers.len());
        readers.push(write.clone());
        readers.extend(extra_readers);
        Self { write, readers }
    }

    /// The reader whose magic prefix matches `head` (first 4 bytes of an object).
    fn reader_for_magic(&self, head: &[u8]) -> Option<&Arc<dyn SegmentFormat>> {
        self.readers.iter().find(|f| {
            let m = f.magic();
            m.len() >= 4 && head.len() >= 4 && m[..4] == head[..4]
        })
    }
}

#[async_trait]
impl SegmentFormat for FormatRegistry {
    fn name(&self) -> &'static str {
        "format-registry"
    }
    fn magic(&self) -> &'static [u8] {
        self.write.magic()
    }
    fn current_version(&self) -> u32 {
        self.write.current_version()
    }
    fn max_readable_version(&self) -> u32 {
        self.write.max_readable_version()
    }

    async fn open_extent(&self, input: OpenExtentInput) -> Result<Arc<dyn ExtentReader>> {
        // Sniff the object's first 4 bytes to pick the decoder. Use any reader's
        // object store (they share one). Fall back to the write format if we
        // can't sniff (e.g. no store, tiny object).
        if self.readers.len() > 1 {
            if let Some(store) = self.write.object_store() {
                let path = object_store::path::Path::from(input.object_path.as_str());
                if let Ok(head) = store.get_range(&path, 0..4).await {
                    if let Some(fmt) = self.reader_for_magic(&head) {
                        return fmt.open_extent(input).await;
                    }
                }
            }
        }
        self.write.open_extent(input).await
    }

    async fn start_extent(
        &self,
        schema: SchemaRef,
        target_bytes: u64,
    ) -> Result<Box<dyn ExtentWriter>> {
        self.write.start_extent(schema, target_bytes).await
    }

    fn object_store(&self) -> Option<Arc<dyn object_store::ObjectStore>> {
        self.write.object_store()
    }
}
