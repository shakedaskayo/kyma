//! Cross-source search engine that powers `POST /v1/explore/search`.
//!
//! See `docs/superpowers/specs/2026-05-28-explore-discover-refactor-design.md`
//! sections 4 + 5 for the full contract.

pub mod compile;
pub mod fanout;
pub mod frames;
pub mod grammar;
pub mod handler;
pub mod live;
pub mod saved_views_handler;
pub mod saved_views_lookup;
pub mod scope;
