//! Structural code parser — re-exported from the standalone `pensieve-codeparse`
//! crate so the GitHub data source keeps `github::parse::*` working unchanged
//! while server/memory can use the same parser without this crate's `github`
//! feature. See `pensieve_codeparse`.

pub use pensieve_codeparse::*;
