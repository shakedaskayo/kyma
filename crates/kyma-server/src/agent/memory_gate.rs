//! The single enforcement chokepoint for memory mutations.
//!
//! Every gateable mutation — whether it comes from real-time consolidation
//! ([`super::memory_conflict`]) or a dreaming housekeeping tool
//! ([`super::memory_tools`]) — is expressed as an [`OpPayload`] and routed
//! through [`dispatch`]. `dispatch` consults the pure [`super::memory_policy`]
//! classifier and then either applies the op, applies it and records a
//! post-hoc review row, or defers it as a pending approval row. The actual
//! mutation lives in one place ([`apply_op`]) so approving a deferred op
//! re-runs exactly the same write.
//!
//! Rollback leans on the bi-temporal node model: every op has a deterministic
//! [`Inverse`] expressed in `status` / `invalid_at` / `superseded_by` / content
//! versioning, so an undo restores the recall-authoritative state. Annotation
//! edges (`MERGED_INTO`, `INVALIDATES`) are append-only and remain as history;
//! relationship-edge undo appends an audit tombstone.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use kyma_memory::types::{CreateMemory, MemoryType};
use kyma_memory::{rows, EDGE_INVALIDATES};

use super::memory_policy::{classify, Disposition, HitlPolicy, MemoryOp};
use super::memory_queue_store::{QueueRow, QueueStore};
use super::memory_tools::{build_writer, fetch_latest_node};
use super::tools::SharedToolCtx;

const EDGE_MERGED_INTO: &str = "MERGED_INTO";

/// A resolved gate: the tenant's policy + where queue rows go + who/where the
/// proposing mutation originated. Built per consolidation tick or dreaming run.
pub struct HitlGate {
    pub policy: HitlPolicy,
    pub store: Arc<QueueStore>,
    /// Who resolves (server identity / authed user); stamped on auto-applied rows.
    pub resolver: Option<String>,
    /// `realtime` | `dreaming` | `file_promote`.
    pub source: &'static str,
    pub source_run_id: Option<Uuid>,
}

/// Per-mutation context the classifier + queue row need.
pub struct GateCtx {
    pub op: MemoryOp,
    pub realm: String,
    pub mem_type: Option<String>,
    pub confidence: Option<f32>,
    pub reason: Option<String>,
}

/// What [`dispatch`] did.
#[derive(Debug, Clone, Copy)]
pub struct GateOutcome {
    /// True if the mutation was actually written (Apply or PostHoc).
    pub applied: bool,
    /// The queue row id, when one was written (PostHoc or Gate).
    pub queued_id: Option<Uuid>,
}

// ── op payloads ──────────────────────────────────────────────────────────────

/// A new memory to create. Mirrors the fields the conflict pipeline sets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddSpec {
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "default_kind")]
    pub memory_type: String,
    pub realm: String,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub valid_at: Option<String>,
    #[serde(default)]
    pub provenance: Value,
}

fn default_kind() -> String {
    "fact".into()
}
fn default_importance() -> f32 {
    0.5
}

/// The deferred/applied mutation, stored verbatim in the queue row's `payload`
/// so an approve re-runs exactly the same write [`apply_op`] would have.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpPayload {
    Add(AddSpec),
    Update {
        target_id: String,
        new_content: String,
    },
    Invalidate {
        target_id: String,
        replacement: AddSpec,
    },
    /// Invalidate `target_id`, superseded by an *existing* memory `by_id`
    /// (the `memory_judge` "supersedes" verdict — no new replacement created).
    Supersede {
        target_id: String,
        by_id: String,
    },
    Merge {
        into_id: String,
        from_ids: Vec<String>,
    },
    Archive {
        memory_id: String,
    },
    Link {
        src: String,
        dst: String,
        rel: String,
        realm: String,
        #[serde(default)]
        target_namespace: Option<String>,
    },
}

/// An edge touched by an op (for inverse + audit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeRef {
    pub src: String,
    pub dst: String,
    pub rel: String,
    pub realm: String,
    #[serde(default)]
    pub target_namespace: Option<String>,
}

/// What [`apply_op`] changed — the raw material for [`inverse_of`].
#[derive(Debug, Clone, Default)]
pub struct AppliedRef {
    /// Node created by this op (Add, or the Invalidate replacement).
    pub created_node_id: Option<String>,
    /// Prior latest rows of nodes this op modified (for restore-on-undo).
    pub prior_rows: Vec<Value>,
    /// Edges this op appended.
    pub edges: Vec<EdgeRef>,
}

/// The deterministic undo for an applied op.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "inv", rename_all = "snake_case")]
pub enum Inverse {
    /// Hide a created node (undo Add / Invalidate replacement).
    ArchiveNode { node_id: String },
    /// Re-append prior latest rows so latest-wins restores their state.
    RestoreRows { rows: Vec<Value> },
    /// Append audit tombstones for edges (append-only store: provenance kept).
    RemoveEdges { edges: Vec<EdgeRef> },
    Multi { steps: Vec<Inverse> },
}

// ── dispatch ─────────────────────────────────────────────────────────────────

/// Route one mutation through the policy. `apply` is the canonical writer for
/// this op (call sites pass `|| apply_op(shared, &payload)`), so the Apply and
/// PostHoc paths and a later approve all run identical code.
pub async fn dispatch<F, Fut>(
    gate: &HitlGate,
    ctx: GateCtx,
    payload: OpPayload,
    apply: F,
) -> anyhow::Result<GateOutcome>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<AppliedRef>>,
{
    let disp = classify(
        ctx.op,
        ctx.confidence,
        &ctx.realm,
        ctx.mem_type.as_deref(),
        &gate.policy,
    );
    match disp {
        Disposition::Apply => {
            apply().await?;
            Ok(GateOutcome {
                applied: true,
                queued_id: None,
            })
        }
        Disposition::PostHoc => {
            let applied = apply().await?;
            let inverse = inverse_of(ctx.op, &payload, &applied);
            let row = QueueRow::new(
                ctx.op,
                ctx.realm,
                "post_hoc",
                "auto_applied",
                ctx.confidence,
                ctx.reason,
                gate.source,
                gate.source_run_id,
                serde_json::to_value(&payload)?,
                inverse.map(|i| serde_json::to_value(i)).transpose()?,
            );
            let id = row.id;
            gate.store.insert(&row).await?;
            Ok(GateOutcome {
                applied: true,
                queued_id: Some(id),
            })
        }
        Disposition::Gate => {
            let row = QueueRow::new(
                ctx.op,
                ctx.realm,
                "gate",
                "pending",
                ctx.confidence,
                ctx.reason,
                gate.source,
                gate.source_run_id,
                serde_json::to_value(&payload)?,
                None,
            );
            let id = row.id;
            gate.store.insert(&row).await?;
            Ok(GateOutcome {
                applied: false,
                queued_id: Some(id),
            })
        }
    }
}

/// Convenience for call sites that may not have a gate (interactive agent): when
/// `gate` is `None`, just apply.
pub async fn gate_or_apply<F, Fut>(
    gate: Option<&HitlGate>,
    ctx: GateCtx,
    payload: OpPayload,
    apply: F,
) -> anyhow::Result<GateOutcome>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<AppliedRef>>,
{
    match gate {
        Some(g) => dispatch(g, ctx, payload, apply).await,
        None => {
            apply().await?;
            Ok(GateOutcome {
                applied: true,
                queued_id: None,
            })
        }
    }
}

/// Tool-side gate for the dreaming housekeeping tools. When the ctx carries a
/// HITL gate, route the op through it and return the tool-result JSON; `None`
/// means "no gate — the caller applies through its normal (ungated) path".
pub async fn gate_tool_op(
    shared: &SharedToolCtx,
    op: MemoryOp,
    realm: &str,
    reason: Option<String>,
    payload: OpPayload,
) -> Option<Value> {
    let gate = shared.hitl.as_deref()?;
    let ctx = GateCtx {
        op,
        realm: realm.to_string(),
        mem_type: None,
        confidence: None,
        reason,
    };
    let res = dispatch(gate, ctx, payload.clone(), || apply_op(shared, &payload)).await;
    Some(match res {
        Ok(o) if o.applied => json!({
            "ok": true,
            "applied": true,
            "queued_for_review": o.queued_id.is_some(),
            "queue_id": o.queued_id.map(|i| i.to_string()),
        }),
        Ok(o) => json!({
            "ok": true,
            "applied": false,
            "queued_for_review": true,
            "queue_id": o.queued_id.map(|i| i.to_string()),
            "note": "deferred to the memory approval queue (HITL policy)",
        }),
        Err(e) => json!({"error": format!("gate: {e}")}),
    })
}

// ── inverse (pure) ───────────────────────────────────────────────────────────

/// Compute the undo for an applied op. Pure — drives the unit tests.
pub fn inverse_of(op: MemoryOp, _payload: &OpPayload, applied: &AppliedRef) -> Option<Inverse> {
    match op {
        MemoryOp::Add | MemoryOp::PromoteFileCandidate => applied
            .created_node_id
            .clone()
            .map(|node_id| Inverse::ArchiveNode { node_id }),
        MemoryOp::Update | MemoryOp::Archive => {
            if applied.prior_rows.is_empty() {
                None
            } else {
                Some(Inverse::RestoreRows {
                    rows: applied.prior_rows.clone(),
                })
            }
        }
        MemoryOp::Invalidate => {
            let mut steps = vec![Inverse::RestoreRows {
                rows: applied.prior_rows.clone(),
            }];
            if let Some(node_id) = applied.created_node_id.clone() {
                steps.push(Inverse::ArchiveNode { node_id });
            }
            if !applied.edges.is_empty() {
                steps.push(Inverse::RemoveEdges {
                    edges: applied.edges.clone(),
                });
            }
            Some(Inverse::Multi { steps })
        }
        MemoryOp::Merge => Some(Inverse::Multi {
            steps: vec![
                Inverse::RestoreRows {
                    rows: applied.prior_rows.clone(),
                },
                Inverse::RemoveEdges {
                    edges: applied.edges.clone(),
                },
            ],
        }),
        MemoryOp::LinkEntityCrossRealm | MemoryOp::RelationshipWrite => {
            if applied.edges.is_empty() {
                None
            } else {
                Some(Inverse::RemoveEdges {
                    edges: applied.edges.clone(),
                })
            }
        }
    }
}

// ── apply (real mutations) ───────────────────────────────────────────────────

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The single applier for every gateable op. Uses the synchronous
/// [`MemoryWriter`] path (gated/approved ops are low-frequency, human-reviewed —
/// batching isn't needed and synchronous is simplest to reason about).
pub async fn apply_op(shared: &SharedToolCtx, payload: &OpPayload) -> anyhow::Result<AppliedRef> {
    let writer = build_writer(shared)
        .await
        .map_err(|e| anyhow::anyhow!("writer: {e}"))?;
    match payload {
        OpPayload::Add(spec) => {
            let id = add_spec(&writer, spec).await?;
            Ok(AppliedRef {
                created_node_id: Some(id),
                ..Default::default()
            })
        }
        OpPayload::Update {
            target_id,
            new_content,
        } => {
            let prior = fetch_latest_node(shared, target_id)
                .await
                .map_err(|e| anyhow::anyhow!("fetch {target_id}: {e}"))?;
            let mut row = prior.clone();
            let emb = writer.embed_one(new_content).await?;
            let now = now_rfc3339();
            row["content"] = json!(new_content);
            row["content_preview"] = json!(rows::preview(new_content));
            row["embedding"] = json!(emb);
            row["updated_at"] = json!(now);
            row["valid_at"] = json!(now);
            writer.append_node_rows(vec![row]).await?;
            Ok(AppliedRef {
                prior_rows: vec![prior],
                ..Default::default()
            })
        }
        OpPayload::Archive { memory_id } => {
            let prior = fetch_latest_node(shared, memory_id)
                .await
                .map_err(|e| anyhow::anyhow!("fetch {memory_id}: {e}"))?;
            let mut row = prior.clone();
            row["status"] = json!("archived");
            row["updated_at"] = json!(now_rfc3339());
            writer.append_node_rows(vec![row]).await?;
            Ok(AppliedRef {
                prior_rows: vec![prior],
                ..Default::default()
            })
        }
        OpPayload::Invalidate {
            target_id,
            replacement,
        } => {
            let new_id = add_spec(&writer, replacement).await?;
            let prior = fetch_latest_node(shared, target_id)
                .await
                .map_err(|e| anyhow::anyhow!("fetch {target_id}: {e}"))?;
            let mut row = prior.clone();
            let now = now_rfc3339();
            row["invalid_at"] = json!(now);
            row["superseded_by"] = json!(new_id);
            row["updated_at"] = json!(now);
            writer.append_node_rows(vec![row]).await?;
            let _ = writer
                .link(&new_id, target_id, EDGE_INVALIDATES, &replacement.realm, None)
                .await;
            Ok(AppliedRef {
                created_node_id: Some(new_id.clone()),
                prior_rows: vec![prior],
                edges: vec![EdgeRef {
                    src: new_id,
                    dst: target_id.clone(),
                    rel: EDGE_INVALIDATES.to_string(),
                    realm: replacement.realm.clone(),
                    target_namespace: None,
                }],
            })
        }
        OpPayload::Supersede { target_id, by_id } => {
            let prior = fetch_latest_node(shared, target_id)
                .await
                .map_err(|e| anyhow::anyhow!("fetch {target_id}: {e}"))?;
            let realm = prior
                .get("realm")
                .and_then(Value::as_str)
                .unwrap_or("default")
                .to_string();
            let mut row = prior.clone();
            let now = now_rfc3339();
            row["invalid_at"] = json!(now);
            row["superseded_by"] = json!(by_id);
            row["updated_at"] = json!(now);
            writer.append_node_rows(vec![row]).await?;
            let _ = writer
                .link(by_id, target_id, EDGE_INVALIDATES, &realm, None)
                .await;
            Ok(AppliedRef {
                prior_rows: vec![prior],
                edges: vec![EdgeRef {
                    src: by_id.clone(),
                    dst: target_id.clone(),
                    rel: EDGE_INVALIDATES.to_string(),
                    realm,
                    target_namespace: None,
                }],
                ..Default::default()
            })
        }
        OpPayload::Merge { into_id, from_ids } => {
            let mut prior_rows = Vec::new();
            let mut edges = Vec::new();
            let now = now_rfc3339();
            for from in from_ids {
                if from == into_id {
                    continue;
                }
                let prior = match fetch_latest_node(shared, from).await {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let realm = prior
                    .get("realm")
                    .and_then(Value::as_str)
                    .unwrap_or("default")
                    .to_string();
                let mut row = prior.clone();
                row["status"] = json!("archived");
                row["updated_at"] = json!(now);
                writer.append_node_rows(vec![row]).await?;
                let edge = rows::edge_row(from, into_id, EDGE_MERGED_INTO, &realm, None, None, &now);
                writer.append_edge_rows(vec![edge]).await?;
                prior_rows.push(prior);
                edges.push(EdgeRef {
                    src: from.clone(),
                    dst: into_id.clone(),
                    rel: EDGE_MERGED_INTO.to_string(),
                    realm,
                    target_namespace: None,
                });
            }
            Ok(AppliedRef {
                prior_rows,
                edges,
                ..Default::default()
            })
        }
        OpPayload::Link {
            src,
            dst,
            rel,
            realm,
            target_namespace,
        } => {
            writer
                .link(src, dst, rel, realm, target_namespace.as_deref())
                .await?;
            Ok(AppliedRef {
                edges: vec![EdgeRef {
                    src: src.clone(),
                    dst: dst.clone(),
                    rel: rel.clone(),
                    realm: realm.clone(),
                    target_namespace: target_namespace.clone(),
                }],
                ..Default::default()
            })
        }
    }
}

/// Persist an [`AddSpec`] as a new memory; returns its `memory:<uuid>` node id.
async fn add_spec(writer: &kyma_memory::MemoryWriter, spec: &AddSpec) -> anyhow::Result<String> {
    let mut cm = CreateMemory::new(spec.content.clone());
    cm.title = spec.title.clone();
    cm.memory_type = MemoryType::parse(&spec.memory_type);
    cm.realm = spec.realm.clone();
    cm.importance = spec.importance.clamp(0.0, 1.0);
    cm.references = spec.references.clone();
    cm.valid_at = spec.valid_at.clone();
    if !spec.provenance.is_null() {
        cm.provenance = Some(spec.provenance.clone());
    }
    cm.tags = vec!["source:extraction".to_string()];
    let id = writer.save(&cm).await?;
    Ok(rows::node_id(&id))
}

/// Run an inverse to roll back an applied op.
pub async fn apply_inverse(shared: &SharedToolCtx, inv: &Inverse) -> anyhow::Result<()> {
    let writer = build_writer(shared)
        .await
        .map_err(|e| anyhow::anyhow!("writer: {e}"))?;
    apply_inverse_inner(shared, &writer, inv).await
}

fn apply_inverse_inner<'a>(
    shared: &'a SharedToolCtx,
    writer: &'a kyma_memory::MemoryWriter,
    inv: &'a Inverse,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
    Box::pin(async move {
        match inv {
            Inverse::ArchiveNode { node_id } => {
                if let Ok(mut row) = fetch_latest_node(shared, node_id).await {
                    row["status"] = json!("archived");
                    row["updated_at"] = json!(now_rfc3339());
                    writer.append_node_rows(vec![row]).await?;
                }
                Ok(())
            }
            Inverse::RestoreRows { rows: restore } => {
                let now = now_rfc3339();
                for r in restore {
                    let mut row = r.clone();
                    // Bump updated_at so the restored version wins latest-wins.
                    row["updated_at"] = json!(now);
                    writer.append_node_rows(vec![row]).await?;
                }
                Ok(())
            }
            Inverse::RemoveEdges { edges } => {
                let now = now_rfc3339();
                for e in edges {
                    // Append-only store: write an audit tombstone for the edge.
                    let props = json!({ "deleted": true, "deleted_at": now });
                    let edge = rows::edge_row(
                        &e.src,
                        &e.dst,
                        &e.rel,
                        &e.realm,
                        e.target_namespace.as_deref(),
                        Some(&props),
                        &now,
                    );
                    writer.append_edge_rows(vec![edge]).await?;
                }
                Ok(())
            }
            Inverse::Multi { steps } => {
                for s in steps {
                    apply_inverse_inner(shared, writer, s).await?;
                }
                Ok(())
            }
        }
    })
}

// ── resolve (approve / reject / undo) ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveAction {
    Approve,
    Reject,
    Undo,
}

impl ResolveAction {
    pub fn parse(s: &str) -> Option<ResolveAction> {
        match s {
            "approve" => Some(ResolveAction::Approve),
            "reject" => Some(ResolveAction::Reject),
            "undo" => Some(ResolveAction::Undo),
            _ => None,
        }
    }
}

/// Resolve one queue row. Returns the updated row, or an error string the
/// route turns into a 4xx/5xx.
pub async fn resolve(
    gate: &HitlGate,
    shared: &SharedToolCtx,
    id: Uuid,
    action: ResolveAction,
    edited_payload: Option<OpPayload>,
    comment: Option<&str>,
) -> anyhow::Result<QueueRow> {
    let row = gate
        .store
        .get(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("queue row not found"))?;
    let by = gate.resolver.as_deref();
    match action {
        ResolveAction::Approve => {
            if row.status != "pending" {
                anyhow::bail!("only pending rows can be approved (status: {})", row.status);
            }
            let payload = match edited_payload {
                Some(p) => p,
                None => serde_json::from_value(row.payload.clone())?,
            };
            apply_op(shared, &payload).await?;
            gate.store
                .update_status(id, "approved", by, comment)
                .await?
                .ok_or_else(|| anyhow::anyhow!("row vanished during approve"))
        }
        ResolveAction::Reject => {
            if row.status != "pending" {
                anyhow::bail!("only pending rows can be rejected (status: {})", row.status);
            }
            gate.store
                .update_status(id, "rejected", by, comment)
                .await?
                .ok_or_else(|| anyhow::anyhow!("row vanished during reject"))
        }
        ResolveAction::Undo => {
            if row.status != "auto_applied" {
                anyhow::bail!(
                    "only auto-applied (post-hoc) rows can be undone (status: {})",
                    row.status
                );
            }
            let inv: Inverse = row
                .inverse
                .clone()
                .ok_or_else(|| anyhow::anyhow!("row has no inverse to undo"))
                .and_then(|v| serde_json::from_value(v).map_err(Into::into))?;
            apply_inverse(shared, &inv).await?;
            gate.store
                .update_status(id, "rolled_back", by, comment)
                .await?
                .ok_or_else(|| anyhow::anyhow!("row vanished during undo"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::memory_policy::OpMode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn enabled_policy() -> HitlPolicy {
        let mut p = HitlPolicy::default();
        p.enabled = true;
        p
    }

    fn gate_with(policy: HitlPolicy, store: Arc<QueueStore>) -> HitlGate {
        HitlGate {
            policy,
            store,
            resolver: Some("tester".into()),
            source: "realtime",
            source_run_id: None,
        }
    }

    fn local_store() -> (tempfile::TempDir, Arc<QueueStore>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(QueueStore::Local {
            path: dir.path().join("q.json"),
        });
        (dir, store)
    }

    fn fake_applied() -> AppliedRef {
        AppliedRef {
            created_node_id: Some("memory:new".into()),
            prior_rows: vec![json!({"id":"memory:old","status":"active"})],
            edges: vec![],
        }
    }

    #[tokio::test]
    async fn apply_disposition_runs_closure_no_row() {
        let (_d, store) = local_store();
        let gate = gate_with(HitlPolicy::default(), store.clone()); // disabled => Apply
        let calls = AtomicUsize::new(0);
        let out = dispatch(
            &gate,
            GateCtx {
                op: MemoryOp::Merge,
                realm: "r".into(),
                mem_type: None,
                confidence: None,
                reason: None,
            },
            OpPayload::Merge {
                into_id: "memory:a".into(),
                from_ids: vec!["memory:b".into()],
            },
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(fake_applied())
            },
        )
        .await
        .unwrap();
        assert!(out.applied);
        assert!(out.queued_id.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(store.list(&Default::default()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn gate_disposition_skips_closure_writes_pending() {
        let (_d, store) = local_store();
        let gate = gate_with(enabled_policy(), store.clone()); // Merge => Gate
        let calls = AtomicUsize::new(0);
        let out = dispatch(
            &gate,
            GateCtx {
                op: MemoryOp::Merge,
                realm: "r".into(),
                mem_type: None,
                confidence: None,
                reason: Some("dup".into()),
            },
            OpPayload::Merge {
                into_id: "memory:a".into(),
                from_ids: vec!["memory:b".into()],
            },
            || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(fake_applied())
            },
        )
        .await
        .unwrap();
        assert!(!out.applied);
        assert!(out.queued_id.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 0, "gate must not apply");
        let rows = store.list(&Default::default()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "pending");
        assert_eq!(rows[0].mode, "gate");
        assert!(rows[0].inverse.is_none());
    }

    #[tokio::test]
    async fn posthoc_disposition_applies_and_records_inverse() {
        let (_d, store) = local_store();
        let mut p = enabled_policy();
        p.ops.insert(MemoryOp::Update, OpMode::PostHoc);
        let gate = gate_with(p, store.clone());
        let out = dispatch(
            &gate,
            GateCtx {
                op: MemoryOp::Update,
                realm: "r".into(),
                mem_type: None,
                confidence: None,
                reason: None,
            },
            OpPayload::Update {
                target_id: "memory:old".into(),
                new_content: "new".into(),
            },
            || async { Ok(fake_applied()) },
        )
        .await
        .unwrap();
        assert!(out.applied);
        let rows = store.list(&Default::default()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "auto_applied");
        assert_eq!(rows[0].mode, "post_hoc");
        assert!(rows[0].inverse.is_some(), "post-hoc must carry an inverse");
    }

    #[test]
    fn inverse_add_archives_created() {
        let inv = inverse_of(
            MemoryOp::Add,
            &OpPayload::Add(AddSpec {
                content: "x".into(),
                title: None,
                memory_type: "fact".into(),
                realm: "r".into(),
                importance: 0.5,
                references: vec![],
                valid_at: None,
                provenance: Value::Null,
            }),
            &AppliedRef {
                created_node_id: Some("memory:new".into()),
                ..Default::default()
            },
        )
        .unwrap();
        match inv {
            Inverse::ArchiveNode { node_id } => assert_eq!(node_id, "memory:new"),
            other => panic!("expected ArchiveNode, got {other:?}"),
        }
    }

    #[test]
    fn inverse_update_restores_prior() {
        let prior = json!({"id":"memory:old","content":"orig"});
        let inv = inverse_of(
            MemoryOp::Update,
            &OpPayload::Update {
                target_id: "memory:old".into(),
                new_content: "new".into(),
            },
            &AppliedRef {
                prior_rows: vec![prior.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        match inv {
            Inverse::RestoreRows { rows } => assert_eq!(rows, vec![prior]),
            other => panic!("expected RestoreRows, got {other:?}"),
        }
    }

    #[test]
    fn inverse_invalidate_is_multi_restore_archive_removeedges() {
        let inv = inverse_of(
            MemoryOp::Invalidate,
            &OpPayload::Invalidate {
                target_id: "memory:t".into(),
                replacement: AddSpec {
                    content: "x".into(),
                    title: None,
                    memory_type: "fact".into(),
                    realm: "r".into(),
                    importance: 0.5,
                    references: vec![],
                    valid_at: None,
                    provenance: Value::Null,
                },
            },
            &AppliedRef {
                created_node_id: Some("memory:new".into()),
                prior_rows: vec![json!({"id":"memory:t"})],
                edges: vec![EdgeRef {
                    src: "memory:new".into(),
                    dst: "memory:t".into(),
                    rel: EDGE_INVALIDATES.into(),
                    realm: "r".into(),
                    target_namespace: None,
                }],
            },
        )
        .unwrap();
        match inv {
            Inverse::Multi { steps } => {
                assert_eq!(steps.len(), 3);
                assert!(matches!(steps[0], Inverse::RestoreRows { .. }));
                assert!(matches!(steps[1], Inverse::ArchiveNode { .. }));
                assert!(matches!(steps[2], Inverse::RemoveEdges { .. }));
            }
            other => panic!("expected Multi, got {other:?}"),
        }
    }

    #[test]
    fn op_payload_serde_roundtrips() {
        let p = OpPayload::Merge {
            into_id: "memory:a".into(),
            from_ids: vec!["memory:b".into(), "memory:c".into()],
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["op"], "merge");
        let back: OpPayload = serde_json::from_value(v).unwrap();
        match back {
            OpPayload::Merge { into_id, from_ids } => {
                assert_eq!(into_id, "memory:a");
                assert_eq!(from_ids.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn reject_transitions_pending_to_rejected() {
        let (_d, store) = local_store();
        let gate = gate_with(enabled_policy(), store.clone());
        // Seed a pending row directly.
        let row = QueueRow::new(
            MemoryOp::Merge,
            "r",
            "gate",
            "pending",
            None,
            None,
            "realtime",
            None,
            serde_json::to_value(OpPayload::Merge {
                into_id: "memory:a".into(),
                from_ids: vec!["memory:b".into()],
            })
            .unwrap(),
            None,
        );
        store.insert(&row).await.unwrap();
        // We can't build a real SharedToolCtx here, but reject never touches the
        // store's apply path — exercise it via the store directly.
        let updated = store
            .update_status(row.id, "rejected", Some("tester"), Some("nope"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "rejected");
        let _ = &gate; // gate constructed to ensure the type wiring compiles
    }
}
