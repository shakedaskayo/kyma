//! pensieve-jobs — the worker side of the job fabric.
//!
//! The control plane (tables + HTTP API) lives in pensieve-catalog::fabric and
//! pensieve-server::fabric_handler. This crate is what a *worker* runs, embedded
//! in the server process or inside a remote `pensieve worker run` daemon:
//!
//! - [`queue::JobQueue`] — how a worker talks to the control plane: claim,
//!   progress, complete, fail, lease. [`queue::PgQueue`] is the in-process
//!   front-end (direct Postgres); the remote HTTP front-end lives with the
//!   daemon.
//! - [`executor::JobExecutor`] — one implementation per job kind.
//! - [`runner::JobRunner`] — the claim → dispatch → report loop, with lease
//!   keep-alive while an executor runs.
//! - [`datasource_sync`] — the `data_source_sync`-kind executor, re-hosting the
//!   data source tick body over the fabric.
//! - [`index_build`] — the `index_build`-kind executor: builds index sidecars
//!   (ANN/FTS) per extent via pluggable `SidecarBuilder`s (S1 provides them).
//! - [`embed_backfill`] — the `embed_backfill`-kind executor: computes
//!   embeddings for a configured text column and fills them into a vector
//!   column by rewriting the extent (extents are immutable), with a
//!   content-hash cache so duplicate text isn't re-embedded.

#![forbid(unsafe_code)]

pub mod ann_maintain;
pub mod datasource_sync;
pub mod embed_backfill;
pub mod executor;
pub mod index_build;
pub mod queue;
pub mod runner;

pub use executor::{JobCtx, JobError, JobExecutor, ProgressSink};
pub use queue::{JobQueue, PgQueue};
pub use runner::{ExecutorRegistry, JobRunner};
