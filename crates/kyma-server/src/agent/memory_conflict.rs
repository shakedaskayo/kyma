//! Conflict resolution + bi-temporal apply (Mem0 A.U.D.N. + Zep/Graphiti).
//!
//! Each extracted candidate memory is reconciled against the most
//! semantically-similar existing memories: the model picks
//! ADD / UPDATE / NOOP / INVALIDATE and we apply it append-only. Contradicted
//! memories are **invalidated** (their `invalid_at` set + an `INVALIDATES`
//! edge to the replacement), never deleted — so history and audit survive and
//! point-in-time recall stays correct.

use serde_json::{json, Value};

use kyma_memory::types::{MemoryType, RecallFilter};
use kyma_memory::{MemoryWriter, DEFAULT_DATABASE, NODE_TABLE};

use super::memory_extract::{decide_conflict, ConflictOp, ExtractedMemory};
use super::memory_gate::{self, AddSpec, GateCtx, HitlGate, OpPayload};
use super::memory_policy::MemoryOp;
use super::state::AgentState;
use super::tools::{execute_sql, SharedToolCtx};

/// Per-run counts of the A.U.D.N. decisions applied.
#[derive(Debug, Default, Clone)]
pub struct ConflictTally {
    pub added: i64,
    pub updated: i64,
    pub noop: i64,
    pub invalidated: i64,
    /// Decisions deferred to the HITL approval queue (not applied).
    pub gated: i64,
    /// Rejected by the validity gate (M8.3b) before ever reaching a conflict
    /// decision — trivial/filler content or below the confidence floor.
    pub rejected_trivial: i64,
}

impl ConflictTally {
    pub fn written(&self) -> i64 {
        self.added + self.updated
    }
    pub fn merge(&mut self, other: &ConflictTally) {
        self.added += other.added;
        self.updated += other.updated;
        self.noop += other.noop;
        self.invalidated += other.invalidated;
        self.gated += other.gated;
        self.rejected_trivial += other.rejected_trivial;
    }
    pub fn to_json(&self) -> Value {
        json!({
            "added": self.added,
            "updated": self.updated,
            "noop": self.noop,
            "invalidated": self.invalidated,
            "gated": self.gated,
            "rejected_trivial": self.rejected_trivial,
        })
    }
}

/// Number of nearest neighbours shown to the conflict-decision model.
const NEIGHBOURS: usize = 5;

/// Reconcile one candidate memory into the store. `references` are entity node
/// ids the memory is about (become `REFERENCES` edges); `provenance` records
/// how it was formed. `gate` is the optional HITL chokepoint — when its policy
/// gates/flags an op the mutation is deferred/recorded instead of (or in
/// addition to) being applied. `activity_id` (M8.2), when set, is the raw
/// input's Activity node id — a successfully-applied Add/Update/Invalidate
/// gets a `DERIVED_FROM` edge back to it, so recall can later surface "we saw
/// this input before" as a worked-example precedent. Never errors out the
/// pipeline — failures degrade to a no-op for that candidate and are logged.
pub async fn consolidate_memory(
    state: &AgentState,
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    realm: &str,
    m: &ExtractedMemory,
    references: Vec<String>,
    provenance: Value,
    gate: Option<&HitlGate>,
    activity_id: Option<&str>,
) -> ConflictTally {
    let mut tally = ConflictTally::default();
    let content = m.content.trim();
    if content.is_empty() {
        return tally;
    }
    let kind = MemoryType::parse(&m.kind);

    // 1. Find nearest existing neighbours (same realm + kind) for the decision.
    let similar = nearest(shared, writer, realm, kind, content).await;

    // 2. Decide. On any model/parse failure, default to ADD so we never drop
    //    genuinely new information.
    let decision = match decide_conflict(state, content, &similar).await {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!(error = %e, "conflict decision failed; defaulting to ADD");
            super::memory_extract::ConflictDecision {
                op: ConflictOp::Add,
                target_id: None,
                merged_content: None,
                reason: Some("decision error".into()),
            }
        }
    };

    // Build the candidate's "create" spec once — reused for ADD, an UPDATE that
    // falls back to ADD, and the INVALIDATE replacement.
    let add_spec = || AddSpec {
        content: content.to_string(),
        title: m.title.clone(),
        memory_type: m.kind.clone(),
        realm: realm.to_string(),
        importance: m.importance.clamp(0.0, 1.0),
        references: references.clone(),
        valid_at: m.valid_at.clone(),
        provenance: provenance.clone(),
    };

    // 3. Map the A.U.D.N. decision onto a gateable (op, payload).
    let (op, payload) = match decision.op {
        ConflictOp::Noop => {
            tally.noop += 1;
            return tally;
        }
        ConflictOp::Add => (MemoryOp::Add, OpPayload::Add(add_spec())),
        ConflictOp::Update => match decision.target_id.as_deref() {
            Some(target) => {
                let new_content = decision.merged_content.as_deref().unwrap_or(content);
                (
                    MemoryOp::Update,
                    OpPayload::Update {
                        target_id: target.to_string(),
                        new_content: new_content.to_string(),
                    },
                )
            }
            // No target → genuinely new; degrade to ADD.
            None => (MemoryOp::Add, OpPayload::Add(add_spec())),
        },
        ConflictOp::Invalidate => match decision.target_id.as_deref() {
            Some(target) => (
                MemoryOp::Invalidate,
                OpPayload::Invalidate {
                    target_id: target.to_string(),
                    replacement: add_spec(),
                },
            ),
            // Nothing to invalidate → just add the new memory.
            None => (MemoryOp::Add, OpPayload::Add(add_spec())),
        },
    };

    // 4. Route through the gate (or apply directly when there is none / policy
    //    is off). `payload` is cloned into dispatch; the closure re-applies the
    //    very same op via the single canonical applier.
    let ctx = GateCtx {
        op,
        realm: realm.to_string(),
        mem_type: Some(m.kind.clone()),
        confidence: m.confidence,
        reason: decision.reason.clone(),
    };
    let outcome = memory_gate::gate_or_apply(gate, ctx, payload.clone(), || {
        memory_gate::apply_op(shared, &payload)
    })
    .await;

    match outcome {
        Ok(o) => {
            if o.applied {
                match op {
                    MemoryOp::Add => tally.added += 1,
                    MemoryOp::Update => tally.updated += 1,
                    MemoryOp::Invalidate => {
                        tally.added += 1; // replacement created
                        tally.invalidated += 1; // target superseded
                    }
                    _ => {}
                }
                // M8.2: link the written memory back to its source Activity
                // (worked-example precedent retrieval). Best-effort — a failed
                // link never fails consolidation.
                if let (Some(activity), Some(new_id)) = (activity_id, o.created_node_id.as_deref())
                {
                    let _ = writer
                        .link(
                            new_id,
                            activity,
                            kyma_memory::EDGE_DERIVED_FROM,
                            realm,
                            Some(kyma_memory::activities::ACTIVITIES_NAMESPACE),
                        )
                        .await;
                }
            } else {
                tally.gated += 1; // deferred to the approval queue
            }
        }
        Err(e) => tracing::debug!(error = %e, op = ?op, "consolidate apply failed"),
    }
    tally
}

/// Embed the candidate and recall the nearest same-kind memories in realm.
async fn nearest(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    realm: &str,
    kind: MemoryType,
    content: &str,
) -> Vec<(String, String)> {
    let qvec = match writer.embed_one(content).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let filter = RecallFilter {
        realms: vec![realm.to_string()],
        memory_type: Some(kind),
        ..Default::default()
    };
    let sql = kyma_memory::sql::recall_sql(NODE_TABLE, &qvec, &filter, NEIGHBOURS, None);
    let res = execute_sql(shared, DEFAULT_DATABASE, &sql, NEIGHBOURS).await;
    res.get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let id = r.get("id").and_then(Value::as_str)?;
                    let c = r
                        .get("content")
                        .or_else(|| r.get("content_preview"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    Some((id.to_string(), c.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

// The actual mutations (add / update / invalidate / merge / archive / link) now
// live in one place — `memory_gate::apply_op` — so a deferred op approved later
// re-runs exactly the same write. `consolidate_memory` just builds the payload
// and routes it through the gate.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_sums_every_field_including_rejected_trivial() {
        let mut a = ConflictTally {
            added: 1,
            updated: 2,
            noop: 3,
            invalidated: 4,
            gated: 5,
            rejected_trivial: 6,
        };
        let b = ConflictTally {
            added: 10,
            updated: 20,
            noop: 30,
            invalidated: 40,
            gated: 50,
            rejected_trivial: 60,
        };
        a.merge(&b);
        assert_eq!(a.added, 11);
        assert_eq!(a.updated, 22);
        assert_eq!(a.noop, 33);
        assert_eq!(a.invalidated, 44);
        assert_eq!(a.gated, 55);
        assert_eq!(a.rejected_trivial, 66);
    }

    #[test]
    fn written_excludes_rejected_trivial() {
        let t = ConflictTally {
            added: 2,
            updated: 3,
            rejected_trivial: 100,
            ..Default::default()
        };
        assert_eq!(t.written(), 5, "rejected_trivial never counts as written");
    }

    #[test]
    fn to_json_includes_rejected_trivial_key() {
        let t = ConflictTally {
            rejected_trivial: 7,
            ..Default::default()
        };
        assert_eq!(t.to_json()["rejected_trivial"], json!(7));
    }
}
