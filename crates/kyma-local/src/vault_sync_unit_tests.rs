use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::vault_sync::{run_once, VaultSyncOptions};
use crate::{open_engine, Engine, Paths};
use kyma_embed::{EmbedError, EmbeddingBackend};
use kyma_memory::MemoryWriter;
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
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
        memory: None,
        hitl: None,
        memory_settings_path: None,
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

fn opts(vault: &Path) -> VaultSyncOptions {
    VaultSyncOptions {
        source_id: "11111111-1111-1111-1111-111111111111".into(),
        vault_path: vault.to_path_buf(),
        realm: "myvault".into(),
    }
}

#[tokio::test]
async fn ingests_plain_and_frontmatter_notes() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;
    let vault = tmp.path().join("vault");

    write(&vault.join("Alpha.md"), "Plain note about auth.\n");
    write(
        &vault.join("sub/Beta.md"),
        "---\ntitle: Beta Note\ntags: [design, api]\n---\n\nBody of beta.\n",
    );

    let o = opts(&vault);
    let report = run_once(&engine, &writer, &o).await.expect("sync");
    assert_eq!(report.seen, 2);
    assert_eq!(report.upserted, 2);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.errors, 0);

    let got = rows(
        &shared,
        &format!("{LATEST} SELECT realm, memory_type, title, tags, importance, topic_key, provenance FROM latest WHERE rn = 1 AND topic_key LIKE 'obsidian:%' ORDER BY topic_key"),
    )
    .await;
    assert_eq!(got.len(), 2);
    assert_eq!(
        got[0]["topic_key"],
        "obsidian:11111111-1111-1111-1111-111111111111/Alpha.md"
    );
    assert_eq!(got[0]["realm"], "myvault");
    assert_eq!(got[0]["memory_type"], "fact");
    assert_eq!(got[0]["title"], "Alpha");
    let tags0 = got[0]["tags"].as_str().unwrap_or("");
    assert!(tags0.contains("obsidian"), "tags: {tags0}");
    assert_eq!(
        got[1]["topic_key"],
        "obsidian:11111111-1111-1111-1111-111111111111/sub/Beta.md"
    );
    assert_eq!(got[1]["title"], "Beta Note");
    let tags1 = got[1]["tags"].as_str().unwrap_or("");
    assert!(
        tags1.contains("obsidian") && tags1.contains("design") && tags1.contains("api"),
        "tags: {tags1}"
    );
    let prov: Value =
        serde_json::from_str(got[0]["provenance"].as_str().expect("prov")).expect("json");
    assert_eq!(prov["source"], "obsidian");
    assert_eq!(prov["rel_path"], "Alpha.md");
}

#[tokio::test]
async fn rescan_skips_unchanged_and_edit_updates_same_node() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;
    let vault = tmp.path().join("vault");
    write(&vault.join("Alpha.md"), "Version one.\n");
    let o = opts(&vault);

    let r1 = run_once(&engine, &writer, &o).await.expect("sync 1");
    assert_eq!(r1.upserted, 1);

    let r2 = run_once(&engine, &writer, &o).await.expect("sync 2");
    assert_eq!(r2.upserted, 0);
    assert_eq!(r2.skipped, 1);

    write(&vault.join("Alpha.md"), "Version two, edited.\n");
    let r3 = run_once(&engine, &writer, &o).await.expect("sync 3");
    assert_eq!(r3.upserted, 1);

    let ids = rows(
        &shared,
        "SELECT DISTINCT id FROM memory_nodes WHERE topic_key = 'obsidian:11111111-1111-1111-1111-111111111111/Alpha.md'",
    )
    .await;
    assert_eq!(ids.len(), 1, "edit must not mint a second node");
    let got = rows(
        &shared,
        &format!("{LATEST} SELECT content FROM latest WHERE rn = 1 AND topic_key = 'obsidian:11111111-1111-1111-1111-111111111111/Alpha.md'"),
    )
    .await;
    assert!(got[0]["content"]
        .as_str()
        .expect("content")
        .contains("Version two"));
}

#[tokio::test]
async fn wikilinks_become_edges_with_obsidian_flavors() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;
    let vault = tmp.path().join("vault");
    // Alias link to beta (case-insensitive), embed of Gamma, ghost + self dropped.
    write(
        &vault.join("Alpha.md"),
        "Links: [[beta|the beta doc]] and ![[Gamma]] and [[Ghost]] and [[Alpha]].\n",
    );
    write(&vault.join("Beta.md"), "Beta body.\n");
    write(&vault.join("notes/Gamma.md"), "Gamma body.\n");
    let o = opts(&vault);

    let report = run_once(&engine, &writer, &o).await.expect("sync");
    assert_eq!(
        report.edges_added, 2,
        "beta + gamma resolve; ghost/self drop"
    );

    let edges = rows(
        &shared,
        "SELECT DISTINCT id, src, dst, props FROM memory_edges WHERE type = 'RELATES_TO'",
    )
    .await;
    assert_eq!(edges.len(), 2);
    for e in &edges {
        let props: Value =
            serde_json::from_str(e["props"].as_str().unwrap_or("{}")).unwrap_or_default();
        assert_eq!(props["via"], "obsidian-wikilink");
    }
}

#[tokio::test]
async fn deleted_note_archives_and_reappearing_unarchives() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, shared) = engine_at(tmp.path()).await;
    let vault = tmp.path().join("vault");
    write(&vault.join("Alpha.md"), "Alpha body.\n");
    write(&vault.join("Beta.md"), "Beta body.\n");
    let o = opts(&vault);

    run_once(&engine, &writer, &o).await.expect("sync 1");

    std::fs::remove_file(vault.join("Beta.md")).expect("rm");
    let r2 = run_once(&engine, &writer, &o).await.expect("sync 2");
    assert_eq!(r2.archived, 1);

    let got = rows(
        &shared,
        &format!("{LATEST} SELECT status FROM latest WHERE rn = 1 AND topic_key = 'obsidian:11111111-1111-1111-1111-111111111111/Beta.md'"),
    )
    .await;
    assert_eq!(got[0]["status"], "archived");

    // Reappear → re-ingest, back to live.
    write(&vault.join("Beta.md"), "Beta body.\n");
    let r3 = run_once(&engine, &writer, &o).await.expect("sync 3");
    assert_eq!(r3.upserted, 1);
    let got = rows(
        &shared,
        &format!("{LATEST} SELECT status FROM latest WHERE rn = 1 AND topic_key = 'obsidian:11111111-1111-1111-1111-111111111111/Beta.md'"),
    )
    .await;
    assert_ne!(got[0]["status"], "archived");
}

#[tokio::test]
async fn dot_dirs_excluded() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, _shared) = engine_at(tmp.path()).await;
    let vault = tmp.path().join("vault");
    write(&vault.join("Alpha.md"), "Real note.\n");
    write(&vault.join(".obsidian/templates/T.md"), "Template.\n");
    write(&vault.join(".trash/Old.md"), "Deleted note.\n");
    let o = opts(&vault);

    let report = run_once(&engine, &writer, &o).await.expect("sync");
    assert_eq!(report.seen, 1, "only the real note is seen");
    assert_eq!(report.upserted, 1);
}

#[tokio::test]
async fn missing_vault_is_an_error() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer, _shared) = engine_at(tmp.path()).await;
    let o = opts(&tmp.path().join("nope"));
    assert!(run_once(&engine, &writer, &o).await.is_err());
}
