//! Retrieval-quality evaluation harness.
//!
//! The ground-truth + metrics layer every S1 retrieval change (ANN indexes,
//! BM25, rerankers) is gated on:
//!
//! - [`metrics`] — recall@k, NDCG@k (graded relevance), MRR, latency
//!   percentiles. Pure functions.
//! - [`oracle`] — exact brute-force top-k over an in-memory dataset, with
//!   distance semantics that mirror `kyma-exec`'s vector UDFs bit-for-bit
//!   (f32 inputs, f64 accumulation) so engine-vs-oracle comparisons are
//!   apples-to-apples. ANN recall is measured against THIS.
//! - [`datasets`] — seeded, reproducible synthetic vector datasets
//!   (clustered gaussian on the unit sphere + uniform random) and query
//!   sampling. Same seed → identical bytes, on every platform.
//! - [`golden`] — golden-set fixtures (`eval/golden/*.json`) + runner:
//!   NDCG@10 / MRR per judged query, aggregate, compare against a committed
//!   baseline with a tolerance.
//!
//! Two binaries ride on top:
//!
//! - `retrieval_bench` — end-to-end harness against a running engine over
//!   HTTP (ingest → SQL `cosine_distance` queries → compare vs oracle →
//!   JSON report). Deterministic; safe for CI.
//! - `judge_calibrate` — local-only LLM-judge labeling scaffold for minting
//!   golden fixtures. NEVER run in CI (the `--dry-run` keyword-overlap mode
//!   is the only deterministic path).

pub mod datasets;
pub mod golden;
pub mod metrics;
pub mod oracle;
