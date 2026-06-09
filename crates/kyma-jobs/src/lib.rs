//! kyma-jobs — the worker side of the job fabric.
//!
//! The control plane (tables + HTTP API) lives in kyma-catalog::fabric and
//! kyma-server::fabric_handler. This crate is what a *worker* runs, embedded
//! in the server process or inside a remote `kyma worker run` daemon:
//!
//! - [`queue::JobQueue`] — how a worker talks to the control plane: claim,
//!   progress, complete, fail, lease. [`queue::PgQueue`] is the in-process
//!   front-end (direct Postgres); the remote HTTP front-end lives with the
//!   daemon.
//! - [`executor::JobExecutor`] — one implementation per job kind.
//! - [`runner::JobRunner`] — the claim → dispatch → report loop, with lease
//!   keep-alive while an executor runs.
//! - [`connector_sync`] — the `connector_sync` executor, re-hosting the
//!   connector tick body over the fabric.

#![forbid(unsafe_code)]

pub mod connector_sync;
pub mod executor;
pub mod queue;
pub mod runner;

pub use executor::{JobCtx, JobError, JobExecutor, ProgressSink};
pub use queue::{JobQueue, PgQueue};
pub use runner::{ExecutorRegistry, JobRunner};
