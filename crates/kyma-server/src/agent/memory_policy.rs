//! HITL policy model + the single pure classifier.
//!
//! This module has **no I/O**: it is the one place memory-mutation policy is
//! evaluated, and it is fully unit-tested. Everything else (the queue store,
//! the gate dispatch, the routes) builds on the [`Disposition`] this returns.
//!
//! The policy answers, per mutation: run it automatically (`Auto`), apply it
//! but log it for after-the-fact review/undo (`PostHoc`), or hold it for human
//! approval before it touches the live graph (`Gate`). A low model-confidence
//! score escalates an op one severity level — so "candidates we're not sure
//! about" surface even when their op class would otherwise be automatic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The classes of memory mutation the policy can govern. Serialized
/// `snake_case` so it is stable in JSON settings + queue rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOp {
    /// A brand-new memory.
    Add,
    /// Rewrite an existing memory in place.
    Update,
    /// Mark an existing memory contradicted (bi-temporal supersede).
    Invalidate,
    /// Fold duplicate/overlapping memories together.
    Merge,
    /// Retire a memory (status → archived).
    Archive,
    /// Link a memory/entity to a node in a *different* realm/namespace.
    LinkEntityCrossRealm,
    /// Any other relationship/edge write (same-realm links, judge verdicts).
    RelationshipWrite,
    /// Promote a file/symbol candidate node into the live graph.
    PromoteFileCandidate,
}

impl MemoryOp {
    /// Every op class, in a stable order — used to seed default settings and
    /// to render the per-op controls in the UI.
    pub const ALL: [MemoryOp; 8] = [
        MemoryOp::Add,
        MemoryOp::Update,
        MemoryOp::Invalidate,
        MemoryOp::Merge,
        MemoryOp::Archive,
        MemoryOp::LinkEntityCrossRealm,
        MemoryOp::RelationshipWrite,
        MemoryOp::PromoteFileCandidate,
    ];

    /// Stable `snake_case` wire string (matches the serde representation) —
    /// used as the `operation` column value in the approval queue.
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryOp::Add => "add",
            MemoryOp::Update => "update",
            MemoryOp::Invalidate => "invalidate",
            MemoryOp::Merge => "merge",
            MemoryOp::Archive => "archive",
            MemoryOp::LinkEntityCrossRealm => "link_entity_cross_realm",
            MemoryOp::RelationshipWrite => "relationship_write",
            MemoryOp::PromoteFileCandidate => "promote_file_candidate",
        }
    }

    /// Parse the wire string back into an op. Unknown ⇒ `None`.
    pub fn parse(s: &str) -> Option<MemoryOp> {
        MemoryOp::ALL.into_iter().find(|o| o.as_str() == s)
    }
}

/// The configured base behavior for an op class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpMode {
    /// Apply silently, no queue row.
    Auto,
    /// Apply, but record a queue row so a human can review / undo afterward.
    PostHoc,
    /// Do not apply; hold a pending queue row for human approval.
    Gate,
}

/// The decision [`classify`] returns for a single mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Apply,
    PostHoc,
    Gate,
}

/// Operator-configurable human-in-the-loop policy. Persisted inside the
/// tenant's `memory_settings` JSONB row (see [`super::memory_settings`]).
///
/// `#[serde(default)]` means a settings row written before this field existed
/// loads with [`HitlPolicy::default`] rather than failing — so the feature is
/// invisible until an operator turns it on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HitlPolicy {
    /// Master switch. When `false`, [`classify`] always returns
    /// [`Disposition::Apply`] — i.e. the system behaves exactly as it did
    /// before this feature shipped.
    pub enabled: bool,
    /// Base mode per op class. A missing entry defaults to [`OpMode::Auto`].
    pub ops: BTreeMap<MemoryOp, OpMode>,
    /// Confidence below this escalates an op one severity level. `0.6` default.
    pub confidence_threshold: f32,
    /// Realms the policy applies to. Empty = all realms.
    pub realm_scope: Vec<String>,
    /// Memory types the policy applies to. Empty = all types.
    pub type_scope: Vec<String>,
}

impl Default for HitlPolicy {
    fn default() -> Self {
        use MemoryOp::*;
        use OpMode::*;
        // Hybrid by risk: gate the destructive/irreversible-feeling ops, review
        // the editing ops after the fact, let additive ops through. File
        // candidates are high-volume + structural, so they default to Auto.
        let ops = BTreeMap::from([
            (Add, Auto),
            (Update, PostHoc),
            (Invalidate, Gate),
            (Merge, Gate),
            (Archive, Gate),
            (LinkEntityCrossRealm, Gate),
            (RelationshipWrite, PostHoc),
            (PromoteFileCandidate, Auto),
        ]);
        Self {
            enabled: false,
            ops,
            confidence_threshold: 0.6,
            realm_scope: vec![],
            type_scope: vec![],
        }
    }
}

/// Escalate one severity level when the model is unsure. `Gate` is the ceiling.
fn bump(m: OpMode) -> OpMode {
    match m {
        OpMode::Auto => OpMode::PostHoc,
        OpMode::PostHoc => OpMode::Gate,
        OpMode::Gate => OpMode::Gate,
    }
}

/// The single, pure policy evaluation. Returns how a mutation should be handled.
///
/// Order: disabled → pass through; out-of-scope realm/type → pass through;
/// else look up the op's base mode and escalate it if `confidence` is below the
/// threshold. A `None` confidence never escalates (we don't punish ops the
/// extractor didn't score).
pub fn classify(
    op: MemoryOp,
    confidence: Option<f32>,
    realm: &str,
    mem_type: Option<&str>,
    p: &HitlPolicy,
) -> Disposition {
    if !p.enabled {
        return Disposition::Apply;
    }
    if !p.realm_scope.is_empty() && !p.realm_scope.iter().any(|r| r == realm) {
        return Disposition::Apply;
    }
    if !p.type_scope.is_empty() {
        match mem_type {
            Some(t) if p.type_scope.iter().any(|x| x == t) => {}
            _ => return Disposition::Apply,
        }
    }
    let base = p.ops.get(&op).copied().unwrap_or(OpMode::Auto);
    let eff = if confidence.is_some_and(|c| c < p.confidence_threshold) {
        bump(base)
    } else {
        base
    };
    match eff {
        OpMode::Auto => Disposition::Apply,
        OpMode::PostHoc => Disposition::PostHoc,
        OpMode::Gate => Disposition::Gate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pol() -> HitlPolicy {
        let mut p = HitlPolicy::default();
        p.enabled = true;
        p
    }

    #[test]
    fn disabled_is_passthrough() {
        let p = HitlPolicy::default(); // enabled: false
        assert_eq!(
            classify(MemoryOp::Merge, Some(0.1), "r", None, &p),
            Disposition::Apply
        );
    }

    #[test]
    fn gate_op_is_gated() {
        assert_eq!(
            classify(MemoryOp::Merge, None, "r", None, &pol()),
            Disposition::Gate
        );
    }

    #[test]
    fn auto_add_high_conf_applies() {
        assert_eq!(
            classify(MemoryOp::Add, Some(0.9), "r", None, &pol()),
            Disposition::Apply
        );
    }

    #[test]
    fn low_conf_escalates_add_to_posthoc() {
        assert_eq!(
            classify(MemoryOp::Add, Some(0.3), "r", None, &pol()),
            Disposition::PostHoc
        );
    }

    #[test]
    fn low_conf_escalates_posthoc_to_gate() {
        assert_eq!(
            classify(MemoryOp::Update, Some(0.3), "r", None, &pol()),
            Disposition::Gate
        );
    }

    #[test]
    fn low_conf_keeps_gate_at_gate() {
        assert_eq!(
            classify(MemoryOp::Merge, Some(0.0), "r", None, &pol()),
            Disposition::Gate
        );
    }

    #[test]
    fn missing_conf_does_not_escalate() {
        assert_eq!(
            classify(MemoryOp::Add, None, "r", None, &pol()),
            Disposition::Apply
        );
    }

    #[test]
    fn realm_scope_excludes_other_realms() {
        let mut p = pol();
        p.realm_scope = vec!["only".into()];
        assert_eq!(
            classify(MemoryOp::Merge, None, "other", None, &p),
            Disposition::Apply
        );
        assert_eq!(
            classify(MemoryOp::Merge, None, "only", None, &p),
            Disposition::Gate
        );
    }

    #[test]
    fn type_scope_excludes_other_types() {
        let mut p = pol();
        p.type_scope = vec!["decision".into()];
        assert_eq!(
            classify(MemoryOp::Merge, None, "r", Some("fact"), &p),
            Disposition::Apply
        );
        assert_eq!(
            classify(MemoryOp::Merge, None, "r", Some("decision"), &p),
            Disposition::Gate
        );
    }

    #[test]
    fn unset_op_defaults_to_auto() {
        let mut p = pol();
        p.ops.clear();
        assert_eq!(
            classify(MemoryOp::Merge, None, "r", None, &p),
            Disposition::Apply
        );
    }

    #[test]
    fn policy_serde_roundtrips() {
        let p = HitlPolicy::default();
        let v = serde_json::to_value(&p).unwrap();
        let back: HitlPolicy = serde_json::from_value(v).unwrap();
        assert_eq!(back.confidence_threshold, p.confidence_threshold);
        assert_eq!(back.ops.get(&MemoryOp::Merge), Some(&OpMode::Gate));
    }

    #[test]
    fn op_as_str_matches_serde_and_roundtrips() {
        for op in MemoryOp::ALL {
            // as_str() must equal the serde wire string so the DB column and
            // JSON payloads never disagree.
            let serde_str = serde_json::to_value(op).unwrap();
            assert_eq!(serde_str, serde_json::json!(op.as_str()));
            assert_eq!(MemoryOp::parse(op.as_str()), Some(op));
        }
        assert_eq!(MemoryOp::parse("nope"), None);
    }

    #[test]
    fn legacy_row_without_fields_loads_default() {
        // A settings fragment written before this field existed.
        let back: HitlPolicy = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!back.enabled);
        assert_eq!(back.ops.get(&MemoryOp::Add), Some(&OpMode::Auto));
    }
}
