use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::cc_sync::{run_once, CcSyncOptions};
use crate::{open_engine, Engine, Paths};
use kyma_embed::{EmbedError, EmbeddingBackend};
use kyma_memory::{CreateMemory, MemoryWriter};
use kyma_server::agent::{execute_sql, SharedToolCtx};

/// Deterministic in-process embedding stub (dim 8) — no model downloads.
#[derive(Debug)]
struct TestEmbed;

#[async_trait::async_trait]
impl EmbeddingBackend for TestEmbed {
    fn id(&self) -> &str {
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

async fn engine_at(tmp: &Path) -> (Engine, MemoryWriter, SharedToolCtx) {
    let paths = Paths {
        catalog_db: tmp.join("catalog.db").display().to_string(),
        data_root: tmp.join("data").display().to_string(),
    };
    let engine = open_engine(&paths).await.expect("engine");
    let writer = MemoryWriter::new(
        engine.catalog.clone(),
        engine.format.clone(),
        Arc::new(TestEmbed),
    );
    let shared = SharedToolCtx {
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
    };
    (engine, writer, shared)
}

fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, content).expect("write");
}

async fn rows(shared: &SharedToolCtx, sql: &str) -> Vec<Value> {
    let res = execute_sql(shared, kyma_memory::DEFAULT_DATABASE, sql, 10_000).await;
    res.get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

const LATEST: &str = "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM memory_nodes)";

#[tokio::test]
async fn ingests_upserts_and_skips() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;

    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    write(
        &mem.join("MEMORY.md"),
        "# Memory index\n\n- [Auth model](auth-model.md) — tokens\n",
    );
    write(
        &mem.join("auth-model.md"),
        "---\nname: auth-model\ndescription: Auth model decision\nmetadata:\n  type: project\n  originSessionId: 22222222-2222-2222-2222-222222222222\n---\n\nWe use session tokens. See [[build-notes]].\n",
    );
    write(
        &mem.join("build-notes.md"),
        "---\nname: build-notes\ndescription: Build preferences\nmetadata:\n  type: user\n---\n\nPrefer cargo nextest.\n",
    );
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);

    let opts = CcSyncOptions {
        projects_dir: projects.clone(),
        claude_json: Some(claude_json),
        project: None,
    };
    let report = run_once(&engine, &writer, &opts).await.expect("sync 1");
    assert_eq!(report.projects.len(), 1);
    assert_eq!(report.projects[0].slug, "-tmp-proj");
    assert_eq!(report.projects[0].realm, "proj");
    assert_eq!(report.projects[0].upserted, 2);
    assert_eq!(report.projects[0].skipped, 0);

    let got = rows(
        &shared,
        &format!("{LATEST} SELECT id, realm, memory_type, title, topic_key, provenance, source_session_id FROM latest WHERE rn = 1 AND topic_key LIKE 'claude-md:%' ORDER BY topic_key"),
    )
    .await;
    assert_eq!(got.len(), 2);
    assert_eq!(got[0]["topic_key"], "claude-md:-tmp-proj/auth-model");
    assert_eq!(got[0]["memory_type"], "fact"); // project → fact
    assert_eq!(got[0]["realm"], "proj");
    assert_eq!(got[0]["title"], "Auth model decision");
    assert_eq!(
        got[0]["source_session_id"],
        "22222222-2222-2222-2222-222222222222"
    );
    let prov: Value =
        serde_json::from_str(got[0]["provenance"].as_str().expect("prov str")).expect("prov json");
    assert_eq!(prov["source"], "claude-code-file");
    assert_eq!(prov["cc_name"], "auth-model");
    assert_eq!(got[1]["topic_key"], "claude-md:-tmp-proj/build-notes");
    assert_eq!(got[1]["memory_type"], "preference"); // user → preference

    // Re-run unchanged → everything skipped, nothing re-embedded.
    let report = run_once(&engine, &writer, &opts).await.expect("sync 2");
    assert_eq!(report.projects[0].upserted, 0);
    assert_eq!(report.projects[0].skipped, 2);

    // Edit one body → exactly one upsert, same node id (new version, no dup).
    write(
        &mem.join("auth-model.md"),
        "---\nname: auth-model\ndescription: Auth model decision\nmetadata:\n  type: project\n---\n\nWe use session tokens AND refresh tokens.\n",
    );
    let report = run_once(&engine, &writer, &opts).await.expect("sync 3");
    assert_eq!(report.projects[0].upserted, 1);
    assert_eq!(report.projects[0].skipped, 1);

    let ids = rows(
        &shared,
        "SELECT DISTINCT id FROM memory_nodes WHERE topic_key = 'claude-md:-tmp-proj/auth-model'",
    )
    .await;
    assert_eq!(ids.len(), 1, "edit must not mint a second node");
    let got = rows(
        &shared,
        &format!("{LATEST} SELECT content FROM latest WHERE rn = 1 AND topic_key = 'claude-md:-tmp-proj/auth-model'"),
    )
    .await;
    assert!(got[0]["content"]
        .as_str()
        .expect("content")
        .contains("refresh tokens"));
}

#[tokio::test]
async fn wikilinks_become_relates_to_edges() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;

    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    write(
        &mem.join("auth-model.md"),
        "---\nname: auth-model\ndescription: Auth\nmetadata:\n  type: project\n---\n\nTokens. See [[build-notes]] and [[ghost]] and [[auth-model]].\n",
    );
    write(
        &mem.join("build-notes.md"),
        "---\nname: build-notes\ndescription: Build\nmetadata:\n  type: user\n---\n\nNextest.\n",
    );
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);
    let opts = CcSyncOptions {
        projects_dir: projects.clone(),
        claude_json: Some(claude_json),
        project: None,
    };

    let report = run_once(&engine, &writer, &opts).await.expect("sync 1");
    assert_eq!(report.projects[0].edges_added, 1, "one resolved wikilink");

    let edges = rows(
        &shared,
        "SELECT DISTINCT id, src, dst, type FROM memory_edges WHERE type = 'RELATES_TO'",
    )
    .await;
    assert_eq!(edges.len(), 1, "ghost + self links must not create edges");
    let src_node = rows(
        &shared,
        "SELECT DISTINCT id FROM memory_nodes WHERE topic_key = 'claude-md:-tmp-proj/auth-model'",
    )
    .await;
    let dst_node = rows(
        &shared,
        "SELECT DISTINCT id FROM memory_nodes WHERE topic_key = 'claude-md:-tmp-proj/build-notes'",
    )
    .await;
    assert_eq!(edges[0]["src"], src_node[0]["id"]);
    assert_eq!(edges[0]["dst"], dst_node[0]["id"]);

    // Re-run unchanged: no new edge ids.
    let report = run_once(&engine, &writer, &opts).await.expect("sync 2");
    assert_eq!(report.projects[0].edges_added, 0);
    let edges = rows(
        &shared,
        "SELECT DISTINCT id FROM memory_edges WHERE type = 'RELATES_TO'",
    )
    .await;
    assert_eq!(edges.len(), 1);
}

#[tokio::test]
async fn deleted_file_archives_node_and_reappearance_restores_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;

    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    let auth = "---\nname: auth-model\ndescription: Auth\nmetadata:\n  type: project\n---\n\nTokens.\n";
    write(&mem.join("auth-model.md"), auth);
    write(
        &mem.join("build-notes.md"),
        "---\nname: build-notes\ndescription: Build\nmetadata:\n  type: user\n---\n\nNextest.\n",
    );
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);
    let opts = CcSyncOptions {
        projects_dir: projects.clone(),
        claude_json: Some(claude_json),
        project: None,
    };

    run_once(&engine, &writer, &opts).await.expect("sync 1");
    std::fs::remove_file(mem.join("auth-model.md")).expect("rm");
    let report = run_once(&engine, &writer, &opts).await.expect("sync 2");
    assert_eq!(report.projects[0].archived, 1);

    let got = rows(
        &shared,
        &format!("{LATEST} SELECT status, invalid_at FROM latest WHERE rn = 1 AND topic_key = 'claude-md:-tmp-proj/auth-model'"),
    )
    .await;
    assert_eq!(got[0]["status"], "archived");
    assert!(got[0]["invalid_at"].as_str().is_some_and(|s| !s.is_empty()));
    // The surviving file is untouched.
    let got = rows(
        &shared,
        &format!("{LATEST} SELECT status FROM latest WHERE rn = 1 AND topic_key = 'claude-md:-tmp-proj/build-notes'"),
    )
    .await;
    assert_eq!(got[0]["status"], "active");

    // Idempotent: nothing more to archive on the next pass.
    let report = run_once(&engine, &writer, &opts).await.expect("sync 3");
    assert_eq!(report.projects[0].archived, 0);

    // The file comes back → the same node is restored, not duplicated.
    write(&mem.join("auth-model.md"), auth);
    let report = run_once(&engine, &writer, &opts).await.expect("sync 4");
    assert_eq!(report.projects[0].upserted, 1);
    let ids = rows(
        &shared,
        "SELECT DISTINCT id FROM memory_nodes WHERE topic_key = 'claude-md:-tmp-proj/auth-model'",
    )
    .await;
    assert_eq!(ids.len(), 1, "reappearance must reuse the node");
    let got = rows(
        &shared,
        &format!("{LATEST} SELECT status FROM latest WHERE rn = 1 AND topic_key = 'claude-md:-tmp-proj/auth-model'"),
    )
    .await;
    assert_eq!(got[0]["status"], "active");
}

#[tokio::test]
async fn rename_with_stable_name_keeps_the_node() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;

    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    let auth = "---\nname: auth-model\ndescription: Auth\nmetadata:\n  type: project\n---\n\nTokens.\n";
    write(&mem.join("auth-model.md"), auth);
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);
    let opts = CcSyncOptions {
        projects_dir: projects.clone(),
        claude_json: Some(claude_json),
        project: None,
    };

    run_once(&engine, &writer, &opts).await.expect("sync 1");
    std::fs::rename(mem.join("auth-model.md"), mem.join("renamed.md")).expect("mv");
    let report = run_once(&engine, &writer, &opts).await.expect("sync 2");
    assert_eq!(report.projects[0].archived, 0, "rename is not a deletion");

    let ids = rows(
        &shared,
        "SELECT DISTINCT id FROM memory_nodes WHERE topic_key = 'claude-md:-tmp-proj/auth-model'",
    )
    .await;
    assert_eq!(ids.len(), 1, "rename must not mint a second node");
    let got = rows(
        &shared,
        &format!("{LATEST} SELECT status FROM latest WHERE rn = 1 AND topic_key = 'claude-md:-tmp-proj/auth-model'"),
    )
    .await;
    assert_eq!(got[0]["status"], "active");
}

#[tokio::test]
async fn kyma_authored_files_skip_then_update_on_user_edit() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;

    // The node kyma promoted earlier (writeback side).
    let mut cm = CreateMemory::new("Original promoted content");
    cm.title = Some("Promoted note".into());
    cm.realm = "proj".into();
    cm.topic_key = Some("promo/x".into());
    cm.importance = 0.9;
    let id = writer.save(&cm).await.expect("seed save");

    let body = "Original promoted content\n";
    let h = kyma_ccmem::hash::content_hash("promoted-note", Some("reference"), body);
    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    let front = format!(
        "---\nname: promoted-note\nmetadata:\n  type: reference\n  source: kyma\n  kyma_memory_id: memory:{id}\n  content_hash: {h}\n---\n\n"
    );
    write(&mem.join("kyma-promoted-note.md"), &format!("{front}{body}"));
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);
    let opts = CcSyncOptions {
        projects_dir: projects.clone(),
        claude_json: Some(claude_json),
        project: None,
    };

    // Untouched promoted file: recognized, skipped, no claude-md node minted.
    let report = run_once(&engine, &writer, &opts).await.expect("sync 1");
    assert_eq!(report.projects[0].upserted, 0);
    assert_eq!(report.projects[0].skipped, 1);
    let got = rows(
        &shared,
        "SELECT id FROM memory_nodes WHERE topic_key LIKE 'claude-md:%'",
    )
    .await;
    assert!(got.is_empty(), "promoted file must not re-ingest");

    // User edits the promoted file → pulled back as an update to the
    // original node, marked user-owned; identity fields preserved.
    write(
        &mem.join("kyma-promoted-note.md"),
        &format!("{front}User-improved content\n"),
    );
    let report = run_once(&engine, &writer, &opts).await.expect("sync 2");
    assert_eq!(report.projects[0].user_edited, 1);
    assert_eq!(report.projects[0].upserted, 0);

    let got = rows(
        &shared,
        &format!("{LATEST} SELECT content, title, topic_key, importance, provenance FROM latest WHERE rn = 1 AND id = 'memory:{id}'"),
    )
    .await;
    assert_eq!(got.len(), 1);
    assert!(got[0]["content"]
        .as_str()
        .expect("content")
        .contains("User-improved"));
    assert_eq!(got[0]["topic_key"], "promo/x", "topic_key preserved");
    assert_eq!(got[0]["title"], "Promoted note", "title preserved");
    let prov: Value =
        serde_json::from_str(got[0]["provenance"].as_str().expect("prov str")).expect("prov json");
    assert_eq!(prov["cc_user_owned"], true);

    // The pull-back happens exactly once: an unchanged user-owned file is
    // skipped on subsequent scans (per-path hash state), not re-ingested
    // forever (its in-file stamp will never match again by design).
    let versions_before = rows(
        &shared,
        &format!("SELECT updated_at FROM memory_nodes WHERE id = 'memory:{id}'"),
    )
    .await
    .len();
    let report = run_once(&engine, &writer, &opts).await.expect("sync 3");
    assert_eq!(report.projects[0].user_edited, 0, "no repeat pull-back");
    assert_eq!(report.projects[0].skipped, 1);
    let versions_after = rows(
        &shared,
        &format!("SELECT updated_at FROM memory_nodes WHERE id = 'memory:{id}'"),
    )
    .await
    .len();
    assert_eq!(versions_before, versions_after, "no new node version");
}
