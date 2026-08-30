use std::path::Path;
use std::sync::Arc;

use crate::cc_pipeline::{run_pass, CcPipelineOptions};
use crate::cc_sync::CcSyncOptions;
use crate::cc_writeback::WritebackConfig;
use crate::{open_engine, Engine, Paths};
use pensieve_embed::{EmbedError, EmbeddingBackend};
use pensieve_memory::{CreateMemory, MemoryType, MemoryWriter};
use pensieve_server::agent::cc_curate::{CurationConfig, LlmCurationConfig};

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

async fn engine_at(tmp: &Path) -> (Engine, MemoryWriter) {
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
    (engine, writer)
}

fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, content).expect("write");
}

#[tokio::test]
async fn full_pass_ingests_promotes_and_reindexes() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer) = engine_at(tmp.path()).await;

    // A Claude Code project with one memory file + index.
    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    write(
        &mem.join("MEMORY.md"),
        "# Memory index\n\n- [Auth model](auth-model.md) — tokens\n",
    );
    write(
        &mem.join("auth-model.md"),
        "---\nname: auth-model\ndescription: Auth model\nmetadata:\n  type: project\n---\n\nTokens.\n",
    );
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);

    // A high-value pensieve memory in the same realm — promotion candidate.
    let mut cm = CreateMemory::new("We always deploy through the staging gate first.");
    cm.title = Some("Staging Gate Rule".to_string());
    cm.memory_type = MemoryType::Decision;
    cm.realm = "proj".to_string();
    cm.importance = 0.9;
    writer.save(&cm).await.expect("seed");

    let opts = CcPipelineOptions {
        sync: CcSyncOptions {
            projects_dir: projects.clone(),
            claude_json: Some(claude_json),
            project: None,
        },
        curate: true,
        promote_cfg: CurationConfig::default(),
        llm_cfg: LlmCurationConfig::default(),
        writeback: WritebackConfig::default(),
    };

    let report = run_pass(&engine, &writer, None, &opts).await.expect("pass 1");
    assert_eq!(report.sync.projects.len(), 1);
    assert_eq!(report.sync.projects[0].upserted, 1);
    assert_eq!(report.curated.len(), 1);
    assert_eq!(report.curated[0].realm, "proj");
    assert_eq!(report.curated[0].outcome.promoted, 1);
    assert_eq!(report.curated[0].applied.written, 1);

    // The promoted file landed next to Claude Code's own memory file.
    let promoted = mem.join("pensieve-staging-gate-rule.md");
    let parsed = pensieve_ccmem::frontmatter::parse(
        &std::fs::read_to_string(&promoted).expect("promoted file"),
    )
    .expect("parses");
    assert!(parsed.is_pensieve_authored());
    assert!(parsed.body.contains("staging gate"));
    // MEMORY.md: user entry intact + managed region with the promotion.
    let idx = std::fs::read_to_string(mem.join("MEMORY.md")).expect("index");
    assert!(idx.contains("- [Auth model](auth-model.md) — tokens"));
    assert!(idx.contains("pensieve-staging-gate-rule.md"));

    // Second pass: fully idempotent (promoted file re-ingest is loop-guarded,
    // no rewrites, no index churn).
    let report = run_pass(&engine, &writer, None, &opts).await.expect("pass 2");
    assert_eq!(report.sync.projects[0].upserted, 0);
    assert_eq!(report.curated[0].outcome.promoted, 0);
    assert_eq!(report.curated[0].applied.written, 0);
    assert!(!report.curated[0].applied.index_updated);
}

#[tokio::test]
async fn quiet_deferral_replans_until_writeback_lands() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer) = engine_at(tmp.path()).await;
    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    write(
        &mem.join("note.md"),
        "---\nname: note\nmetadata:\n  type: project\n---\n\nA note.\n",
    );
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);

    let mut cm = CreateMemory::new("Promote me when the session ends.");
    cm.title = Some("Deferred Promotion".to_string());
    cm.memory_type = MemoryType::Decision;
    cm.realm = "proj".to_string();
    cm.importance = 0.9;
    writer.save(&cm).await.expect("seed");

    // An active Claude Code session for the project defers writeback.
    let sessions = tmp.path().join("sessions");
    #[allow(clippy::cast_possible_truncation)]
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_millis() as i64;
    write(
        &sessions.join("live.json"),
        &format!(r#"{{"pid":1,"cwd":"/tmp/proj","updatedAt":{now_ms},"status":"busy"}}"#),
    );

    let opts = CcPipelineOptions {
        sync: CcSyncOptions {
            projects_dir: projects.clone(),
            claude_json: Some(claude_json),
            project: None,
        },
        curate: true,
        promote_cfg: CurationConfig::default(),
        llm_cfg: LlmCurationConfig::default(),
        writeback: WritebackConfig {
            sessions_dir: Some(sessions.clone()),
            ..WritebackConfig::default()
        },
    };

    let report = run_pass(&engine, &writer, None, &opts).await.expect("pass 1");
    assert!(report.curated[0].applied.skipped_quiet);
    let promoted = mem.join("pensieve-deferred-promotion.md");
    assert!(!promoted.exists(), "deferred: nothing written mid-session");

    // Session ends → the next pass must still want to write the file
    // (a deferred apply must not have burned the promotion stamp).
    std::fs::remove_file(sessions.join("live.json")).expect("rm session");
    let report = run_pass(&engine, &writer, None, &opts).await.expect("pass 2");
    assert_eq!(report.curated[0].applied.written, 1);
    assert!(promoted.exists(), "promotion lands once the session is over");

    // And then it reaches steady state.
    let report = run_pass(&engine, &writer, None, &opts).await.expect("pass 3");
    assert_eq!(report.curated[0].applied.written, 0);
}

#[tokio::test]
async fn curate_disabled_only_ingests() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (engine, writer) = engine_at(tmp.path()).await;
    let projects = tmp.path().join("projects");
    let mem = projects.join("-tmp-proj").join("memory");
    write(
        &mem.join("note.md"),
        "---\nname: note\nmetadata:\n  type: project\n---\n\nA note.\n",
    );
    let claude_json = tmp.path().join("claude.json");
    write(&claude_json, r#"{"projects": {"/tmp/proj": {}}}"#);

    let mut cm = CreateMemory::new("Big decision.");
    cm.title = Some("Big".to_string());
    cm.memory_type = MemoryType::Decision;
    cm.realm = "proj".to_string();
    cm.importance = 0.9;
    writer.save(&cm).await.expect("seed");

    let opts = CcPipelineOptions {
        sync: CcSyncOptions {
            projects_dir: projects.clone(),
            claude_json: Some(claude_json),
            project: None,
        },
        curate: false,
        promote_cfg: CurationConfig::default(),
        llm_cfg: LlmCurationConfig::default(),
        writeback: WritebackConfig::default(),
    };
    let report = run_pass(&engine, &writer, None, &opts).await.expect("pass");
    assert_eq!(report.sync.projects[0].upserted, 1);
    assert!(report.curated.is_empty());
    assert!(!mem.join("pensieve-big.md").exists());
    assert!(!mem.join("MEMORY.md").exists(), "no index invented");
}
