use std::sync::Arc;

use serde_json::{json, Value};
use uuid::Uuid;

use super::cc_curate::{
    commit_guard_stamps, plan_curation, CurationConfig, CurationInput, FileAction, GuardStamp,
};
use super::{execute_sql, SharedToolCtx};
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_embed::{EmbedError, EmbeddingBackend};
use kyma_format_tlm::TelemetryFormat;
use kyma_memory::{CreateMemory, MemoryType, MemoryWriter};
use kyma_storage::{build_object_store, StorageConfig};

/// Deterministic in-process embedding stub (dim 8) — no model downloads.
#[derive(Debug)]
struct TestEmbed;

#[async_trait::async_trait]
impl EmbeddingBackend for TestEmbed {
    fn id(&self) -> &'static str {
        "test/stub-8"
    }
    fn dimension(&self) -> u16 {
        8
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; 8];
                for (i, b) in t.bytes().enumerate() {
                    v[i % 8] += f32::from(b) / 255.0;
                }
                v
            })
            .collect())
    }
}

async fn engine() -> (tempfile::TempDir, MemoryWriter, SharedToolCtx) {
    let tmp = tempfile::tempdir().expect("tmp");
    let sqlite = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect(
            &tmp.path().join("catalog.db").display().to_string(),
        )
        .await
        .expect("catalog"),
    );
    let catalog: Arc<dyn Catalog> = sqlite;
    let data_root = tmp.path().join("data");
    std::fs::create_dir_all(&data_root).expect("data dir");
    let store = build_object_store(&StorageConfig::Local {
        root: data_root.display().to_string(),
    })
    .expect("store");
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store, "kyma-test"));
    let writer = MemoryWriter::new(catalog.clone(), format.clone(), Arc::new(TestEmbed));
    let shared = SharedToolCtx { federation: None,
        catalog,
        format,
        pool: None,
        memory: None,
        hitl: None,
    };
    (tmp, writer, shared)
}

#[allow(clippy::too_many_arguments)]
async fn seed(
    writer: &MemoryWriter,
    realm: &str,
    title: &str,
    content: &str,
    mtype: MemoryType,
    importance: f32,
    topic_key: Option<&str>,
    provenance: Option<Value>,
) -> Uuid {
    let mut cm = CreateMemory::new(content);
    cm.title = Some(title.to_string());
    cm.memory_type = mtype;
    cm.realm = realm.to_string();
    cm.importance = importance;
    cm.topic_key = topic_key.map(str::to_string);
    cm.provenance = provenance;
    writer.save(&cm).await.expect("seed")
}

async fn rows(shared: &SharedToolCtx, sql: &str) -> Vec<Value> {
    let res = execute_sql(shared, kyma_memory::DEFAULT_DATABASE, sql, 10_000).await;
    res.get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

const LATEST: &str = "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM memory_nodes)";

/// Append a new version of a node with `invalid_at` set (bi-temporal
/// invalidation, as the conflict pipeline would do).
async fn invalidate(writer: &MemoryWriter, shared: &SharedToolCtx, id: Uuid) {
    let got = rows(
        &shared.clone(),
        &format!(
            "{LATEST} SELECT id, labels, realm, memory_type, title, content, content_preview, tags, importance, status, source_session_id, source_run_id, embedding, created_at, updated_at, valid_at, invalid_at, superseded_by, provenance, topic_key FROM latest WHERE rn = 1 AND id = 'memory:{id}'"
        ),
    )
    .await;
    let mut row = got.first().cloned().expect("node row");
    let now = chrono::Utc::now().to_rfc3339();
    row["invalid_at"] = json!(now);
    row["updated_at"] = json!(now);
    writer.append_node_rows(vec![row]).await.expect("invalidate");
}

fn input(now: &str) -> CurationInput<'_> {
    CurationInput {
        realm: "proj",
        path_slug: "-tmp-proj",
        now,
    }
}

/// Simulate a fully successful apply: commit every guard stamp.
async fn commit_all(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    actions: &[FileAction],
    stamps: &[GuardStamp],
    now: &str,
) {
    commit_guard_stamps(shared, writer, stamps, &vec![true; actions.len()], now)
        .await
        .expect("commit stamps");
}

fn writes(actions: &[FileAction]) -> Vec<&FileAction> {
    actions
        .iter()
        .filter(|a| matches!(a, FileAction::WriteMemoryFile { .. }))
        .collect()
}

fn archives(actions: &[FileAction]) -> Vec<&FileAction> {
    actions
        .iter()
        .filter(|a| matches!(a, FileAction::ArchiveFile { .. }))
        .collect()
}

fn index_entries(actions: &[FileAction]) -> Vec<super::cc_curate::IndexEntry> {
    actions
        .iter()
        .find_map(|a| match a {
            FileAction::SetIndex { entries } => Some(entries.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn parses_curation_decisions_tolerantly() {
    use super::cc_curate::{parse_curation_decision, CurationOp};

    let d = parse_curation_decision(
        r#"{"op": "ARCHIVE", "reason": "obsolete since the rewrite"}"#,
    );
    assert_eq!(d.op, CurationOp::Archive);
    assert_eq!(d.reason.as_deref(), Some("obsolete since the rewrite"));

    // Fenced + lowercase + refreshed description.
    let d = parse_curation_decision(
        "```json\n{\"op\": \"refresh\", \"refreshed_description\": \"clearer one-liner\"}\n```",
    );
    assert_eq!(d.op, CurationOp::Refresh);
    assert_eq!(d.refreshed_description.as_deref(), Some("clearer one-liner"));

    // Garbage degrades to the safe default: KEEP.
    let d = parse_curation_decision("I think we should probably keep it?");
    assert_eq!(d.op, CurationOp::Keep);
    let d = parse_curation_decision(r#"{"op": "DELETE EVERYTHING"}"#);
    assert_eq!(d.op, CurationOp::Keep);
}

#[test]
fn cosine_similarity_behaves() {
    use super::cc_curate::cosine;
    assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-9);
    assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-9);
    assert!(
        cosine(&[0.0, 0.0], &[1.0, 0.0]).abs() < 1e-12,
        "zero vector is not NaN"
    );
}

#[test]
fn staleness_respects_age_and_review_stamps() {
    use super::cc_curate::is_stale;
    let now = "2026-06-05T00:00:00+00:00";
    let old = "2026-01-01T00:00:00+00:00"; // ~155 days
    let fresh = "2026-06-01T00:00:00+00:00";
    assert!(is_stale(old, None, now, 90));
    assert!(!is_stale(fresh, None, now, 90));
    assert!(
        !is_stale(old, Some(fresh), now, 90),
        "recently reviewed → not re-questioned"
    );
    assert!(is_stale(old, Some(old), now, 90), "stale review re-questions");
}

#[tokio::test]
async fn llm_pass_without_engine_is_a_clean_noop() {
    use super::cc_curate::{llm_curation_pass, LlmCurationConfig};
    let (_tmp, writer, shared) = engine().await;
    seed(
        &writer,
        "proj",
        "Old note",
        "Quite old.",
        MemoryType::Fact,
        0.7,
        Some("claude-md:-tmp-proj/old-note"),
        Some(json!({"source": "claude-code-file", "cc_file": "old-note.md"})),
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let mut actions = Vec::new();
    let mut stamps = Vec::new();
    let mut outcome = super::cc_curate::CurationOutcome::default();
    llm_curation_pass(
        &shared,
        &writer,
        None,
        &input(&now),
        &LlmCurationConfig::default(),
        &mut actions,
        &mut stamps,
        &mut outcome,
    )
    .await
    .expect("noop");
    assert!(actions.is_empty());
    assert!(stamps.is_empty());
    assert_eq!(outcome.llm_reviewed, 0);
}

#[tokio::test]
async fn promotes_top_memories_capped_ordered_and_idempotent() {
    let (_tmp, writer, shared) = engine().await;
    let id_decision = seed(
        &writer,
        "proj",
        "Auth Model Decision",
        "We chose session tokens over JWTs because of revocation.",
        MemoryType::Decision,
        0.9,
        None,
        None,
    )
    .await;
    seed(
        &writer,
        "proj",
        "Prefer Nextest",
        "Always run tests with cargo nextest.",
        MemoryType::Preference,
        0.7,
        None,
        None,
    )
    .await;
    seed(
        &writer,
        "proj",
        "Low signal",
        "Probably noise.",
        MemoryType::Fact,
        0.3,
        None,
        None,
    )
    .await;
    seed(
        &writer,
        "proj",
        "Session summary",
        "Did some work.",
        MemoryType::Summary,
        0.95,
        None,
        None,
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let cfg = CurationConfig {
        promote_max: 2,
        ..CurationConfig::default()
    };
    let (actions, stamps, outcome) = plan_curation(&shared, &writer, &input(&now), &cfg)
        .await
        .expect("plan");

    let w = writes(&actions);
    assert_eq!(w.len(), 2, "cap 2: decision + preference, no summary/low");
    assert_eq!(outcome.promoted, 2);

    let FileAction::WriteMemoryFile {
        file,
        content,
        node_id,
        content_hash,
    } = w[0]
    else {
        unreachable!()
    };
    assert_eq!(file, "kyma-auth-model-decision.md");
    assert_eq!(node_id, &format!("memory:{id_decision}"));
    let parsed = kyma_ccmem::frontmatter::parse(content).expect("rendered file parses");
    assert!(parsed.is_kyma_authored());
    assert_eq!(parsed.front.name.as_deref(), Some("kyma-auth-model-decision"));
    assert_eq!(parsed.front.cc_type.as_deref(), Some("project")); // decision → project
    assert_eq!(parsed.front.kyma_memory_id.as_deref(), Some(node_id.as_str()));
    assert_eq!(parsed.front.content_hash.as_deref(), Some(content_hash.as_str()));
    assert!(parsed.body.contains("session tokens over JWTs"));
    // The stamped hash matches a recompute over the rendered file.
    let recomputed = kyma_ccmem::hash::content_hash(
        parsed.front.name.as_deref().unwrap_or_default(),
        parsed.front.cc_type.as_deref(),
        &parsed.body,
    );
    assert_eq!(&recomputed, content_hash);

    let idx = index_entries(&actions);
    assert_eq!(idx.len(), 2);
    assert_eq!(idx[0].file, "kyma-auth-model-decision.md", "score order");
    assert_eq!(idx[1].file, "kyma-prefer-nextest.md");
    assert_eq!(outcome.index_entries, 2);

    // Apply succeeds → stamps commit; second pass: no rewrites, no churn.
    commit_all(&shared, &writer, &actions, &stamps, &now).await;
    let now2 = chrono::Utc::now().to_rfc3339();
    let (actions2, _, outcome2) = plan_curation(&shared, &writer, &input(&now2), &cfg)
        .await
        .expect("plan 2");
    assert!(writes(&actions2).is_empty(), "idempotent: no rewrites");
    assert_eq!(outcome2.promoted, 0);
    assert_eq!(index_entries(&actions2), idx);
}

#[tokio::test]
async fn dry_run_plans_without_stamping_the_store() {
    let (_tmp, writer, shared) = engine().await;
    seed(
        &writer,
        "proj",
        "Big Decision",
        "We chose X.",
        MemoryType::Decision,
        0.9,
        None,
        None,
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let cfg = CurationConfig {
        dry_run: true,
        ..CurationConfig::default()
    };
    let (actions, _stamps, _) = plan_curation(&shared, &writer, &input(&now), &cfg)
        .await
        .expect("plan");
    assert_eq!(writes(&actions).len(), 1);

    // A second dry-run plan must emit the same writes — nothing was stamped.
    let now2 = chrono::Utc::now().to_rfc3339();
    let (actions2, _, _) = plan_curation(&shared, &writer, &input(&now2), &cfg)
        .await
        .expect("plan 2");
    assert_eq!(
        writes(&actions2).len(),
        1,
        "dry run must not persist promotion stamps"
    );
}

#[tokio::test]
async fn excludes_file_born_and_user_owned_from_promotion() {
    let (_tmp, writer, shared) = engine().await;
    // File-born memory (came from a Claude Code file) — never promoted back.
    seed(
        &writer,
        "proj",
        "From a file",
        "Born as a memory file.",
        MemoryType::Fact,
        0.9,
        Some("claude-md:-tmp-proj/from-a-file"),
        Some(json!({"source": "claude-code-file", "cc_file": "from-a-file.md"})),
    )
    .await;
    // Previously promoted, then user-edited → user-owned: indexed, never rewritten.
    seed(
        &writer,
        "proj",
        "User Owned Note",
        "The user edited this promoted file.",
        MemoryType::Decision,
        0.9,
        Some("promo/user-owned"),
        Some(json!({
            "cc_user_owned": true,
            "cc_promoted_file": "kyma-user-owned-note.md",
            "cc_content_hash": "stale",
        })),
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let (actions, _stamps, _) = plan_curation(
        &shared,
        &writer,
        &input(&now),
        &CurationConfig::default(),
    )
    .await
    .expect("plan");

    assert!(
        writes(&actions).is_empty(),
        "file-born and user-owned must not be (re)written"
    );
    let idx = index_entries(&actions);
    assert_eq!(idx.len(), 1, "user-owned file keeps its index entry");
    assert_eq!(idx[0].file, "kyma-user-owned-note.md");
}

#[tokio::test]
async fn superseded_file_born_archives_its_file_once() {
    let (_tmp, writer, shared) = engine().await;
    let id = seed(
        &writer,
        "proj",
        "Old note",
        "This was true once.",
        MemoryType::Fact,
        0.7,
        Some("claude-md:-tmp-proj/old-note"),
        Some(json!({"source": "claude-code-file", "cc_file": "old-note.md"})),
    )
    .await;
    invalidate(&writer, &shared, id).await;

    // A node archived because its file was deleted must NOT round-trip back
    // into an archive action.
    seed(
        &writer,
        "proj",
        "Deleted on disk",
        "Gone already.",
        MemoryType::Fact,
        0.7,
        Some("claude-md:-tmp-proj/deleted-on-disk"),
        Some(json!({
            "source": "claude-code-file",
            "cc_file": "deleted-on-disk.md",
            "cc_archived_reason": "file_deleted",
        })),
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let cfg = CurationConfig::default();
    let (actions, stamps, outcome) = plan_curation(&shared, &writer, &input(&now), &cfg)
        .await
        .expect("plan");
    let arch = archives(&actions);
    assert_eq!(arch.len(), 1);
    let FileAction::ArchiveFile { file, .. } = arch[0] else {
        unreachable!()
    };
    assert_eq!(file, "old-note.md");
    assert_eq!(outcome.archived_files, 1);

    // Once the archive lands (stamps commit), the next pass is quiet.
    commit_all(&shared, &writer, &actions, &stamps, &now).await;
    let now2 = chrono::Utc::now().to_rfc3339();
    let (actions2, _, _) = plan_curation(&shared, &writer, &input(&now2), &cfg)
        .await
        .expect("plan 2");
    assert!(archives(&actions2).is_empty(), "archive emitted exactly once");
}

#[tokio::test]
async fn exact_duplicate_file_born_memories_merge() {
    let (_tmp, writer, shared) = engine().await;
    let keep = seed(
        &writer,
        "proj",
        "Build tip",
        "Use cargo nextest for the suite.",
        MemoryType::Fact,
        0.8,
        Some("claude-md:-tmp-proj/build-tip"),
        Some(json!({"source": "claude-code-file", "cc_file": "build-tip.md"})),
    )
    .await;
    let lose = seed(
        &writer,
        "proj",
        "Build tip again",
        "Use cargo nextest for the suite.",
        MemoryType::Fact,
        0.6,
        Some("claude-md:-tmp-proj/build-tip-again"),
        Some(json!({"source": "claude-code-file", "cc_file": "build-tip-again.md"})),
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let (actions, _stamps, outcome) = plan_curation(
        &shared,
        &writer,
        &input(&now),
        &CurationConfig::default(),
    )
    .await
    .expect("plan");

    let arch = archives(&actions);
    assert_eq!(arch.len(), 1);
    let FileAction::ArchiveFile { file, reason, .. } = arch[0] else {
        unreachable!()
    };
    assert_eq!(file, "build-tip-again.md", "lower importance loses");
    assert!(reason.contains("duplicate"));
    assert_eq!(outcome.merged, 1);

    // DB mirrored in the same pass: loser archived + superseded_by + edge.
    let got = rows(
        &shared,
        &format!("{LATEST} SELECT status, superseded_by FROM latest WHERE rn = 1 AND id = 'memory:{lose}'"),
    )
    .await;
    assert_eq!(got[0]["status"], "archived");
    assert_eq!(got[0]["superseded_by"], format!("memory:{keep}"));
    let edges = rows(
        &shared,
        &format!("SELECT DISTINCT id FROM memory_edges WHERE type = 'MERGED_INTO' AND src = 'memory:{lose}' AND dst = 'memory:{keep}'"),
    )
    .await;
    assert_eq!(edges.len(), 1);
}

#[tokio::test]
async fn demoted_promotion_is_archived_and_unstamped() {
    let (_tmp, writer, shared) = engine().await;
    // Stamped as promoted earlier, but importance has since dropped far
    // below the floor (0.6 × 0.8 = 0.48 threshold).
    seed(
        &writer,
        "proj",
        "Faded Note",
        "Used to matter.",
        MemoryType::Decision,
        0.3,
        Some("promo/faded"),
        Some(json!({
            "cc_promoted_file": "kyma-faded-note.md",
            "cc_content_hash": "whatever",
        })),
    )
    .await;

    let now = chrono::Utc::now().to_rfc3339();
    let cfg = CurationConfig::default();
    let (actions, stamps, _) = plan_curation(&shared, &writer, &input(&now), &cfg)
        .await
        .expect("plan");
    let arch = archives(&actions);
    assert_eq!(arch.len(), 1);
    let FileAction::ArchiveFile { file, reason, .. } = arch[0] else {
        unreachable!()
    };
    assert_eq!(file, "kyma-faded-note.md");
    assert!(reason.contains("demoted"));
    assert!(index_entries(&actions).is_empty());

    // Once the archive lands (stamps commit, clearing the promotion stamp),
    // the next pass is quiet.
    commit_all(&shared, &writer, &actions, &stamps, &now).await;
    let now2 = chrono::Utc::now().to_rfc3339();
    let (actions2, _, _) = plan_curation(&shared, &writer, &input(&now2), &cfg)
        .await
        .expect("plan 2");
    assert!(archives(&actions2).is_empty());
}
