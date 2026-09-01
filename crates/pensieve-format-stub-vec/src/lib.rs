//! Stub vector / ANN storage-format implementation.
//!
//! This crate intentionally has no real implementation. Its _sole_ purpose
//! is to be a second consumer of the [`pensieve_core::segment_format::SegmentFormat`]
//! trait, so that abstractions baked into the trait during slice 1 are
//! exercised against two concrete shapes rather than one. Abstraction leaks
//! surface here first.
//!
//! TODO(M5): flesh out just enough to plug into the engine for a
//! smoke-level demo. No production-grade HNSW / IVF.

#![forbid(unsafe_code)]
