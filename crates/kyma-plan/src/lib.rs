//! Unified logical-plan IR, optimizer, and parser-plugin registry.
//!
//! `kyma-plan`'s [`LogicalPlan`][logical_plan::LogicalPlan] is the
//! frontend-neutral IR that every [`kyma_core::query_frontend::QueryFrontend`]
//! produces and that `kyma-exec` lowers to DataFusion.
//!
//! # Module map
//!
//! - [`logical_plan`] — the unified plan enum (frontend-neutral).
//! - [`optimizer`] — rewrite rules (telemetry-specific + generic).
//! - [`query_frontend_registry`] — routes queries by Content-Type to the
//!   right registered frontend and unpacks its opaque plan.

#![forbid(unsafe_code)]

pub mod logical_plan;
pub mod optimizer;
pub mod query_frontend_registry;
