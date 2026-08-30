//! The unified logical-plan IR.
//!
//! TODO(M2): flesh out the full enum per architecture plan §6. The shape
//! below is a minimal placeholder that lets the crate compile so dependents
//! (kyma-kql, kyma-exec) can reference the type.

use kyma_core::types::TableId;

/// Frontend-neutral logical plan. First-class nodes exist for KQL idioms
/// that have no natural SQL analog (`TimeSeries`, `MvExpand`).
#[derive(Debug, Clone)]
pub enum LogicalPlan {
    /// A reference to a table, with later filters / projections applied
    /// as the plan is built. Placeholder node; real variants land in M2.
    TableScan {
        /// The table being scanned.
        table_id: TableId,
    },
    /// Placeholder so the enum compiles before real variants exist.
    Placeholder,
}
