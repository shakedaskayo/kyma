//! Structural code parser — re-exported from the standalone `kyma-codeparse`
//! crate so the GitHub data source keeps `github::parse::*` working unchanged
//! while server/memory can use the same parser without this crate's `github`
//! feature. See `kyma_codeparse`.

pub use kyma_codeparse::*;
