//! `kyma-index-vector` — per-extent IVF + 1-bit RaBitQ ANN vector index.
//!
//! This crate builds the **vector ANN sidecar** for one extent (S1.1, Part A:
//! the build side). A sidecar is an immutable blob derived from one extent's
//! embedding column and stored next to the extent (see
//! [`kyma_core::index_sidecar`]). The query path that reads it (`ann_topk`,
//! probing + exact rerank) is Part B and lives elsewhere; this crate only
//! produces the blob and exposes the read primitives Part B builds on.
//!
//! # Pipeline
//!
//! ```text
//!   extent embedding column (FixedSizeList<Float32>, block-ordered batches)
//!        │  builder.rs: project + L2-normalize each row, derive RowAddress
//!        ▼
//!   rows: [(RowAddress, unit-vector)]
//!        │  ivf.rs: deterministic mini-batch k-means → nlist centroids
//!        │          nlist = clamp(round(√n_rows), 16, 256)
//!        ▼
//!   per-partition assignments
//!        │  rabitq.rs: fixed-seed rotation, 1-bit sign codes + correction
//!        ▼
//!   file.rs: serialize → KYIV sidecar blob (header · centroids · postings · footer)
//! ```
//!
//! # On-object layout
//!
//! The blob is self-describing and seekable — a reader probes the centroids,
//! then loads only the partitions it needs via the footer directory. See
//! [`file`] for the exact byte layout. Everything is deterministic for a given
//! `(rows, dim)`: the k-means init/sampling and the RaBitQ rotation both draw
//! from fixed-seed PRNGs (see [`rng`]), so index rebuilds converge to
//! byte-identical output.
//!
//! # Metric
//!
//! Rows are L2-normalized to the unit sphere, where
//! `cosine_distance = 1 − ⟨a,b⟩ = ½‖a−b‖²`. The metric byte is `0` (cosine);
//! clustering and probing use squared L2, which is monotone in cosine distance
//! on the unit sphere.

pub mod build;
pub mod builder;
pub mod file;
pub mod global_tree;
pub mod ivf;
pub mod rabitq;
pub mod rng;
pub mod search;

pub use build::build_ivf_rabitq;
pub use builder::IvfRabitqBuilder;
pub use global_tree::{GlobalTree, PartitionRef};
pub use file::{
    probe, read_centroids, read_header, read_partition, Entry, Header, SidecarError,
    FORMAT_VERSION, MAGIC, METRIC_COSINE,
};
pub use rabitq::{encode_query_rotated, encode_rotated, QueryCode, Rotation, VectorCode};
pub use search::scan_partition;
