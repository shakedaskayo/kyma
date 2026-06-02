//! Local-engine library backing the **`kyma`** CLI's `mcp` / `serve` / `setup` /
//! `sync` commands — the single-binary context engine for local machines.
//!
//! Zero infra: an embedded **SQLite catalog** + a **local-filesystem object
//! store** + the in-process columnar engine. The `kyma` CLI exposes:
//!
//!   - `kyma mcp`   — serve the Model Context Protocol over **stdio** (what a
//!     coding agent spawns): durable graph-aware **memory** *and* live data/graph.
//!   - `kyma serve` — serve the **same web interface** + HTTP API the hosted
//!     server runs (query/KQL/SQL, catalog, graph, ingest, MCP over HTTP) on a
//!     local port, zero-auth.
//!   - `kyma setup <agent>` — wire a coding agent to `kyma mcp` in one command.
//!   - `kyma sync` — sync memory bidirectionally with a control plane.
//!
//! Data lives under `~/.kyma` (override with `KYMA_HOME` / `KYMA_LOCAL_DB` /
//! `KYMA_LOCAL_DATA`): `catalog.db` (metadata + memory graph) and `data/`
//! (columnar extents). For `mcp`, **stdout is the protocol channel** — logs go
//! to stderr.

#![forbid(unsafe_code)]

mod setup;
mod sync;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_format_tlm::TelemetryFormat;
use kyma_ingest_core::WritePath;
use kyma_ingest_rest::IngestState;
use kyma_mcp::{serve_stdio, McpState, ServerInfo, ToolDispatch};
use kyma_server::agent::local::{
    NullCredentialStore, NullEnabledSkillsStore, NullEnginePreferenceStore,
};
use kyma_server::agent::{AgentState, SharedToolCtx};
use kyma_server::auth::{
    require_role_middleware, AuthBackend, AuthLayerState, EnvAuthBackend, Role, SessionAuthBackend,
};
use kyma_server::catalog_handler::SchemaCache;
use kyma_server::QueryState;
use kyma_storage::{build_object_store, StorageConfig};
use tracing::{info, warn};

/// Resolved on-disk locations for the local engine.
struct Paths {
    catalog_db: String,
    data_root: String,
}

fn resolve_paths() -> Paths {
    let home = std::env::var("KYMA_HOME").unwrap_or_else(|_| {
        let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{base}/.kyma")
    });
    let catalog_db =
        std::env::var("KYMA_LOCAL_DB").unwrap_or_else(|_| format!("{home}/catalog.db"));
    let data_root = std::env::var("KYMA_LOCAL_DATA").unwrap_or_else(|_| format!("{home}/data"));
    Paths { catalog_db, data_root }
}

/// The shared local engine: embedded catalog + local-filesystem columnar store.
struct Engine {
    /// Concrete handle — for sync watermarks (`sync_state`) beyond the trait.
    sqlite: Arc<SqliteCatalog>,
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
}

async fn open_engine(paths: &Paths) -> Result<Engine> {
    if let Some(parent) = std::path::Path::new(&paths.catalog_db).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all(&paths.data_root)
        .with_context(|| format!("creating data root {}", paths.data_root))?;

    let sqlite = Arc::new(
        SqliteCatalog::connect(&paths.catalog_db)
            .await
            .map_err(|e| anyhow::anyhow!("opening catalog at {}: {e}", paths.catalog_db))?,
    );
    let catalog: Arc<dyn Catalog> = sqlite.clone();
    info!(catalog = %paths.catalog_db, "embedded catalog ready");

    let store = build_object_store(&StorageConfig::Local { root: paths.data_root.clone() })
        .context("building local object store")?;
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store, "kyma-local"));
    info!(data = %paths.data_root, "local object store ready");

    Ok(Engine { sqlite, catalog, format })
}

fn mcp_state(engine: &Engine) -> McpState {
    // No Postgres pool in local mode — recall/save run over the engine.
    let shared = SharedToolCtx {
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
    };
    McpState {
        dispatch: ToolDispatch::new(shared),
        server_info: ServerInfo {
            name: "kyma".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    }
}

/// `kyma mcp` — serve the Model Context Protocol over stdio.
///
/// The caller (the `kyma` binary) must route tracing to **stderr** — stdout is
/// the JSON-RPC protocol channel.
pub async fn run_mcp() -> Result<()> {
    let engine = open_engine(&resolve_paths()).await?;
    let state = mcp_state(&engine);
    info!("serving MCP over stdio (memory + data + graph); stdin/stdout is the protocol channel");
    serve_stdio(state).await.context("stdio MCP loop")?;
    Ok(())
}

/// `kyma serve` — serve the web UI + full HTTP API on `addr`, over the embedded
/// catalog (zero infra).
pub async fn run_serve(addr: SocketAddr) -> Result<()> {
    let engine = open_engine(&resolve_paths()).await?;

    // The web UI requires a sign-in. Seed a local user (default `admin`/`admin`,
    // override with KYMA_LOCAL_USER / KYMA_LOCAL_PASSWORD) and authenticate via
    // session tokens stored in the embedded catalog — same machinery as the
    // server, over SQLite.
    let user = std::env::var("KYMA_LOCAL_USER").unwrap_or_else(|_| "admin".into());
    let password = std::env::var("KYMA_LOCAL_PASSWORD").unwrap_or_else(|_| "admin".into());
    if engine.catalog.count_users().await.unwrap_or(0) == 0 {
        let phc = kyma_server::auth::passwords::hash_password(&password)
            .map_err(|e| anyhow::anyhow!("hashing local password: {e}"))?;
        engine
            .catalog
            .create_user(&user, &phc, "admin")
            .await
            .context("seeding local user")?;
        info!(username = %user, "seeded local web-UI user");
    }
    let backend: Arc<dyn AuthBackend> = Arc::new(SessionAuthBackend::new(
        engine.catalog.clone(),
        EnvAuthBackend::from_env(),
        true,
    ));

    let query_state = QueryState {
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        schema_cache: Arc::new(SchemaCache::from_env()),
        node_id: None,
        pg_pool: None, // local: no Postgres — pool-only surfaces degrade gracefully
    };
    let agent_state = AgentState {
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None, // local: run/session history not persisted; memory runs over the engine
        engines: Arc::new(NullEnginePreferenceStore),
        credentials: Arc::new(NullCredentialStore),
        tenant: kyma_core::tenant::DEFAULT_TENANT,
        skills: Arc::new(NullEnabledSkillsStore),
        mcp_url: None,
    };
    let write_path = WritePath::new(engine.catalog.clone(), engine.format.clone());
    let ingest_state = IngestState {
        catalog: engine.catalog.clone(),
        write_path,
    };

    let read_mw = || {
        axum::middleware::from_fn_with_state(
            AuthLayerState { backend: backend.clone(), required: Role::Read },
            require_role_middleware,
        )
    };
    let write_mw = || {
        axum::middleware::from_fn_with_state(
            AuthLayerState { backend: backend.clone(), required: Role::Write },
            require_role_middleware,
        )
    };

    // The same web interface + full API the hosted server serves, over the
    // embedded catalog. Read surfaces (query/catalog/graph/agent/memory/MCP)
    // require Role::Read; ingest requires Role::Write; login/web/health are open.
    let read_router = kyma_server::router_with_agent(query_state, agent_state)
        .merge(kyma_mcp::router(mcp_state(&engine)))
        .layer(read_mw());
    let ingest_router = kyma_ingest_rest::router(ingest_state).layer(write_mw());
    let session_router = kyma_server::auth_handler::auth_session_router(engine.catalog.clone())
        .layer(read_mw());

    let app = read_router
        .merge(ingest_router)
        .merge(session_router)
        .merge(kyma_server::auth_handler::auth_login_router(engine.catalog.clone()))
        .merge(kyma_server::health_router())
        .merge(kyma_server::web_ui::router());
    let app = kyma_server::with_permissive_cors(app);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    info!(%addr, "kyma serving the web UI + full HTTP API (local, SQLite)");
    info!("  web UI:   http://{addr}/   (sign in: {user} / {password})");
    info!("  ingest:   POST http://{addr}/v1/ingest   (X-Database / X-Table headers)");
    info!("  MCP:      http://{addr}/mcp/v1");
    if password == "admin" {
        warn!("using the default local password 'admin' — set KYMA_LOCAL_PASSWORD to change it");
    }
    axum::serve(listener, app).await.context("http server")?;
    Ok(())
}

/// `kyma sync` — push local memory changes to a control plane and pull remote
/// ones. Reads `KYMA_CLOUD_URL` / `KYMA_CLOUD_TOKEN` / `KYMA_SYNC_REALM`.
pub async fn run_sync() -> Result<()> {
    let cloud_url = std::env::var("KYMA_CLOUD_URL").map_err(|_| {
        anyhow::anyhow!("set KYMA_CLOUD_URL (and usually KYMA_CLOUD_TOKEN) to sync to a control plane")
    })?;
    let engine = open_engine(&resolve_paths()).await?;
    let cfg = sync::SyncConfig {
        cloud_url,
        token: std::env::var("KYMA_CLOUD_TOKEN").ok().filter(|s| !s.is_empty()),
        realm: std::env::var("KYMA_SYNC_REALM").ok().filter(|s| !s.is_empty()),
        now: chrono::Utc::now().to_rfc3339(),
    };
    sync::run(&engine, cfg).await
}

/// `kyma setup <agent>` — wire a coding agent to `kyma mcp` over stdio.
pub fn run_setup(agent: &str, print: bool) -> Result<()> {
    setup::run(agent, print)
}

/// Print the resolved local paths (diagnostics).
pub fn print_info() {
    let paths = resolve_paths();
    eprintln!("kyma — local single-binary context engine");
    eprintln!("  catalog : {}", paths.catalog_db);
    eprintln!("  data    : {}", paths.data_root);
    eprintln!("  mcp     : kyma mcp           (stdio MCP; memory + data + graph)");
    eprintln!("  serve   : kyma serve         (web UI + HTTP API + ingest, zero-auth)");
    eprintln!("  setup   : kyma setup <agent> (wire claude-code/cursor/windsurf to mcp)");
    eprintln!("  sync    : kyma sync          (push/pull memory to KYMA_CLOUD_URL)");
}
