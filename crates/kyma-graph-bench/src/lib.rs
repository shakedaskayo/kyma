//! Graph benchmarks (S0.5 scaffold).
//!
//! `benches/oracle.rs` baselines the **petgraph oracle** from
//! `kyma-graph-testkit` (k-hop, shortest-path, synth-generation throughput)
//! on power-law graphs at 10k / 100k edges — and 1M edges when
//! `KYMA_BENCH_LARGE=1` (kept off by default so `cargo bench` stays fast).
//!
//! These numbers exist so S3's engine-side vectorized path operator has a
//! reference curve to be plotted against: S3 adds `benches/engine.rs` (same
//! topologies, same parameters, driven through the engine) alongside, and
//! the oracle baseline tells us how much of the gap is "graph is hard"
//! versus "engine overhead".
//!
//! Run: `cargo bench -p kyma-graph-bench` (smoke: `cargo bench -p
//! kyma-graph-bench -- --test`).
