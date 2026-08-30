//! End-to-end brain-repo test over the REAL app assembly and the REAL `git`
//! binary: create a brain via `/v1/brain`, `git clone` it over smart HTTP,
//! edit a note → push → the same memory gets a new version, add an inbox
//! note → push → a new `brain:`-keyed memory appears, delete a note → push
//! → the memory is archived. Skipped when no `git` binary is present.
//!
//! Own test binary ⇒ the process-global embedding OnceCell is seeded only here.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use kyma_local::build_local_app;
use kyma_memory::{CreateMemory, MemoryWriter};
use kyma_server::agent::tools::{execute_sql, SharedToolCtx};
use serde_json::Value;

#[derive(Debug)]
struct MockEmbed;

#[async_trait::async_trait]
impl kyma_embed::EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/brain-it"
    }
    fn dimension(&self) -> u16 {
        4
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, kyma_embed::EmbedError> {
        Ok(texts.iter().map(|t| vec![0.5, 0.25, 0.125, t.len() as f32 / 100.0]).collect())
    }
}

type CatalogArc = Arc<dyn kyma_core::catalog::Catalog>;
type FormatArc = Arc<dyn kyma_core::segment_format::SegmentFormat>;

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir) // no user gitconfig
        .args(args)
        .output()
        .expect("spawn git")
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn shared_ctx(catalog: &CatalogArc, format: &FormatArc) -> SharedToolCtx {
    SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: catalog.clone(),
        format: format.clone(),
        pool: None,
        memory: None,
        hitl: None,
        memory_settings_path: None,
    }
}

async fn latest_node(shared: &SharedToolCtx, id_pred: &str) -> Option<Value> {
    let q = format!(
        "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn \
         FROM memory_nodes) SELECT * FROM latest WHERE __rn = 1 AND {id_pred} LIMIT 1"
    );
    execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 1)
        .await
        .get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
}

#[tokio::test(flavor = "multi_thread")]
async fn brain_clone_push_ingest_round_trip() {
    if Command::new("git").arg("version").output().is_err() {
        eprintln!("skipping: git binary not found");
        return;
    }
    let _ = kyma_memory::try_set_shared_embedding(Arc::new(MockEmbed));

    // Isolated KYMA_HOME so brains.json + repos land in the tempdir.
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("KYMA_HOME", tmp.path().join("kyma-home"));

    let catalog: CatalogArc = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect_in_memory().await.expect("in-memory catalog"),
    );
    let data_root = tmp.path().join("data");
    std::fs::create_dir_all(&data_root).unwrap();
    let store = kyma_storage::build_object_store(&kyma_storage::StorageConfig::Local {
        root: data_root.to_string_lossy().to_string(),
    })
    .expect("local store");
    let format: FormatArc = Arc::new(kyma_format_tlm::TelemetryFormat::new(store, "test"));

    // Seed one memory in realm `kyma`.
    let writer = MemoryWriter::new(catalog.clone(), format.clone(), Arc::new(MockEmbed));
    let seeded_id = {
        let mut m = CreateMemory::new("Sessions are stateless JWTs signed with the server keypair.");
        m.title = Some("Auth model uses stateless JWT".into());
        m.realm = "kyma".into();
        m.memory_type = kyma_memory::MemoryType::Decision;
        m.importance = 0.8;
        writer.save(&m).await.expect("seed memory")
    };

    let backend: Arc<dyn kyma_server::auth::AuthBackend> = Arc::new(
        kyma_server::auth::EnvAuthBackend::from_str("admin-tok:admin,read-tok:read"),
    );
    let brain_git = kyma_brain::gitbin::GitBin::detect().await.map(Arc::new);
    assert!(brain_git.is_some(), "git detected above, GitBin::detect must succeed");
    let (app, _agent_state, _brain_state) = build_local_app(
        catalog.clone(),
        format.clone(),
        backend,
        None,
        None,
        None,
        None,
        None,
        kyma_local::watcher_status::LocalWatcherStatus::default(),
        brain_git,
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http = reqwest::Client::new();
    let base = format!("http://{addr}");

    // ── Create the brain (admin) — runs the first export inline. ──────────
    let resp = http
        .post(format!("{base}/v1/brain"))
        .bearer_auth("admin-tok")
        .json(&serde_json::json!({ "name": "team", "realms": ["kyma"] }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let created: Value = resp.json().await.unwrap();
    assert_eq!(status.as_u16(), 201, "create failed: {created}");
    assert_eq!(created["first_export"]["notes"], 1, "first export: {created}");

    // Read-role token can read the registry.
    let list: Value = http
        .get(format!("{base}/v1/brain"))
        .bearer_auth("read-tok")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["brains"][0]["config"]["name"], "team");

    // ── Clone over smart HTTP (token as Basic password). ───────────────────
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let clone_url = format!("http://kyma:admin-tok@{addr}/git/team.git");
    git_ok(&work, &["clone", &clone_url, "clone"]);
    let repo = work.join("clone");

    let note_rel = {
        // Find the exported decision note.
        let dir = repo.join("notes/decisions");
        let entry = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap();
        format!("notes/decisions/{}", entry.file_name().to_string_lossy())
    };
    let note_text = std::fs::read_to_string(repo.join(&note_rel)).unwrap();
    assert!(note_text.contains("Sessions are stateless JWTs"), "{note_text}");
    assert!(note_text.contains(&format!("kyma_memory_id: {seeded_id}")), "{note_text}");
    assert!(repo.join("index.md").exists());
    assert!(repo.join(".kyma/manifest.json").exists());

    // Unauthenticated clone must fail when auth is enabled.
    let out = git(&work, &["clone", &format!("http://{addr}/git/team.git"), "noauth"]);
    assert!(!out.status.success(), "unauthenticated clone must fail");

    git_ok(&repo, &["config", "user.email", "t@example.com"]);
    git_ok(&repo, &["config", "user.name", "Test"]);

    // ── Edit the note body and push → same memory id gets a new version. ──
    let edited = note_text.replace(
        "Sessions are stateless JWTs signed with the server keypair.",
        "Sessions are stateless JWTs. Refresh flow lives in the token service.",
    );
    std::fs::write(repo.join(&note_rel), edited).unwrap();
    // New inbox note in the same push.
    std::fs::write(
        repo.join("inbox/new-idea.md"),
        "---\ntitle: Cache the auth keys\ntype: learning\ntags: [auth]\n---\n\n# Cache the auth keys\n\nKey lookups dominate token verification; cache them.\n",
    )
    .unwrap();
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "-q", "-m", "edit note + add idea"]);
    git_ok(&repo, &["push", "-q", "origin", "main"]);

    let shared = shared_ctx(&catalog, &format);
    let row = latest_node(&shared, &format!("id = 'memory:{seeded_id}'")).await.expect("row");
    assert!(
        row["content"].as_str().unwrap().contains("Refresh flow lives in the token service"),
        "pushed edit must update the same memory: {row}"
    );
    assert_eq!(row["status"], "active");

    let created_row = latest_node(&shared, "topic_key = 'brain:team:inbox/new-idea.md'")
        .await
        .expect("inbox note ingested");
    assert_eq!(created_row["title"], "Cache the auth keys");
    assert_eq!(created_row["memory_type"], "learning");
    assert_eq!(created_row["realm"], "kyma");

    // ── Delete the note and push → memory archived, never destroyed. ──────
    git_ok(&repo, &["rm", "-q", &note_rel]);
    git_ok(&repo, &["commit", "-q", "-m", "remove note"]);
    git_ok(&repo, &["push", "-q", "origin", "main"]);

    let row = latest_node(&shared, &format!("id = 'memory:{seeded_id}'")).await.expect("row");
    assert_eq!(row["status"], "archived", "deleted file must archive the memory: {row}");

    // ── Force-push must be rejected (denyNonFastForwards). ────────────────
    git_ok(&repo, &["commit", "-q", "--allow-empty", "-m", "published"]);
    git_ok(&repo, &["push", "-q", "origin", "main"]);
    git_ok(&repo, &["reset", "-q", "--hard", "HEAD~1"]);
    git_ok(&repo, &["commit", "-q", "--allow-empty", "-m", "divergent"]);
    let out = git(&repo, &["push", "-q", "--force", "origin", "main"]);
    assert!(!out.status.success(), "force-push must be rejected");
    // Re-sync the clone for the assertions below.
    git_ok(&repo, &["reset", "-q", "--hard", "origin/main"]);

    // ── Re-export now reflects the push: archived note gone from tree. ────
    let resp = http
        .post(format!("{base}/v1/brain/team/export"))
        .bearer_auth("admin-tok")
        .send()
        .await
        .unwrap();
    let export: Value = resp.json().await.unwrap();
    assert!(export.get("error").is_none(), "{export}");
    git_ok(&repo, &["pull", "-q", "--rebase=false", "origin", "main"]);
    assert!(!repo.join(&note_rel).exists(), "archived note must leave the tree");
    // The inbox note got re-filed under notes/learnings/ by the export.
    assert!(!repo.join("inbox/new-idea.md").exists(), "inbox note re-filed on export");
    let refiled = std::fs::read_dir(repo.join("notes/learnings"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert!(refiled >= 1, "expected the inbox note re-filed under notes/learnings");
}
