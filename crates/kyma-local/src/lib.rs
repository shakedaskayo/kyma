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

pub mod agent_sources;
pub mod brain_registry;
mod cc_pipeline;
mod cc_sync;
mod cc_writeback;
mod cli_config_heal;
mod cred_store;
mod datasource_catalog;
pub mod node;
pub mod server_service;
mod setup;
mod source_watchers;
pub mod sqlite_queue;
mod sync;
mod vault_sync;
mod watcher_settings;
pub mod watcher_status;
pub mod worker;

#[cfg(test)]
mod server_service_unit_tests;
#[cfg(test)]
mod worker_unit_tests;

#[cfg(test)]
mod cc_pipeline_unit_tests;

#[cfg(test)]
mod cc_sync_unit_tests;

#[cfg(test)]
mod vault_sync_unit_tests;

#[cfg(test)]
mod cc_writeback_unit_tests;

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::credentials::CredentialStore;
use kyma_core::crypto::Crypto;
use kyma_core::segment_format::SegmentFormat;
use kyma_datasources::admin::AdminState as DataSourceAdminState;
use kyma_datasources::catalog_trait::DataSourceCatalog;
use kyma_datasources::registry::DataSourceRegistry;
use kyma_datasources::runner::{DataSourceTickDeps, GraphRegisterFn, RowSink};
use kyma_datasources::scheduler::DataSourceScheduler;
use kyma_datasources::secrets::EnvSecretStore;
use kyma_format_tlm::TelemetryFormat;
use kyma_ingest_core::events::IngestEvents;
use kyma_ingest_core::WritePath;
use kyma_ingest_otlp::self_export::{SelfTraceCtx, SelfTraceExporter};
use kyma_ingest_rest::IngestState;
use kyma_mcp::{serve_stdio, McpState, ServerInfo, ToolDispatch};
use kyma_server::agent::local::{
    FileEnabledSkillsStore, FileEnginePreferenceStore, NullCredentialStore,
};
use kyma_server::agent::{
    AgentState, ConsumerPublisher, ConsumerSink, LocalConsumerPublisher, SharedToolCtx,
};
use kyma_server::auth::{
    require_role_middleware, AuthBackend, AuthLayerState, EnvAuthBackend, Role, SessionAuthBackend,
};
use kyma_server::catalog_handler::SchemaCache;
use kyma_server::QueryState;
use kyma_storage::{build_object_store, StorageConfig};
use tracing::{info, warn};

/// Set up the tracing subscriber for `kyma serve` — includes the fmt layer
/// AND a `tracing_opentelemetry` layer that routes `kyma_telemetry`-target
/// spans to the in-process self-trace exporter.
///
/// Returns the handle for wiring the exporter to storage once the catalog
/// is ready. Call this BEFORE `run_serve` and pass the returned handle to
/// `run_serve`.
///
/// If the global subscriber is already installed (e.g. in a test harness),
/// this is a no-op and returns an unwired handle.
pub fn setup_serve_tracing() -> Arc<OnceLock<SelfTraceCtx>> {
    let exporter = SelfTraceExporter::unwired();
    let handle = exporter.handle();
    use opentelemetry::trace::TracerProvider as _;
    let tp = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();
    let tracer = tp.tracer("kyma-local");
    let otel_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(
            tracing_subscriber::filter::Targets::new()
                .with_target("kyma_telemetry", tracing::Level::INFO),
        );
    use tracing_subscriber::prelude::*;
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(false)
                .with_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                        tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn")
                    }),
                ),
        )
        .with(otel_layer)
        .try_init();
    handle
}

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
    Paths {
        catalog_db,
        data_root,
    }
}

/// The shared local engine: embedded catalog + local-filesystem columnar store.
#[derive(Clone)]
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

    let store = build_object_store(&StorageConfig::Local {
        root: paths.data_root.clone(),
    })
    .context("building local object store")?;
    // S2.1: per-extent format dispatch (TLM + Parquet readers). Local writes use
    // KYMA_WRITE_FORMAT (default "tlm"); both formats stay readable so a local
    // store can mix them.
    let tlm_fmt: Arc<dyn SegmentFormat> =
        Arc::new(TelemetryFormat::new(store.clone(), "kyma-local"));
    let parquet_fmt: Arc<dyn SegmentFormat> =
        Arc::new(kyma_format_parquet::ParquetFormat::new(store, "kyma-local"));
    let format: Arc<dyn SegmentFormat> =
        if std::env::var("KYMA_WRITE_FORMAT").as_deref() == Ok("parquet") {
            Arc::new(kyma_core::segment_format::FormatRegistry::new(
                parquet_fmt,
                vec![tlm_fmt],
            ))
        } else {
            Arc::new(kyma_core::segment_format::FormatRegistry::new(
                tlm_fmt,
                vec![parquet_fmt],
            ))
        };
    info!(data = %paths.data_root, "local object store ready (TLM + Parquet readers)");

    Ok(Engine {
        sqlite,
        catalog,
        format,
    })
}

/// Forwards consumer activity to a running `kyma serve` over HTTP. Used by the
/// standalone `kyma mcp` (stdio) process, which has no access to the serve's
/// in-process bus, so the agent driving it still shows in the live overlay.
/// Best-effort + fire-and-forget; with no serve up the POST just fails silently.
struct RemoteConsumerPublisher {
    endpoint: String,
    token: String,
    client: reqwest::Client,
}

impl ConsumerPublisher for RemoteConsumerPublisher {
    fn tenant(&self) -> kyma_core::tenant::TenantId {
        kyma_core::tenant::DEFAULT_TENANT
    }
    fn publish(&self, activity: kyma_ingest_core::ConsumerActivity) {
        let url = format!("{}/v1/consumers/emit", self.endpoint.trim_end_matches('/'));
        let token = self.token.clone();
        let client = self.client.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = client
                    .post(url)
                    .bearer_auth(token)
                    .json(&activity)
                    .timeout(std::time::Duration::from_secs(2))
                    .send()
                    .await;
            });
        }
    }
}

/// Build a forwarder to a running serve from `${KYMA_HOME}/config.json`, if it
/// exists with an endpoint + token. `None` ⇒ `kyma mcp` runs standalone (no
/// overlay forwarding), exactly as before.
fn remote_consumer_sink() -> Option<ConsumerSink> {
    let home = std::env::var("KYMA_HOME")
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.kyma")))?;
    let raw = std::fs::read_to_string(format!("{home}/config.json")).ok()?;
    let cfg: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let endpoint = cfg.get("endpoint")?.as_str()?.trim().to_string();
    let token = cfg.get("token")?.as_str()?.trim().to_string();
    if endpoint.is_empty() || token.is_empty() {
        return None;
    }
    Some(std::sync::Arc::new(RemoteConsumerPublisher {
        endpoint,
        token,
        client: reqwest::Client::new(),
    }))
}

fn mcp_state(engine: &Engine, memory: Option<kyma_memory::MemoryQueue>) -> McpState {
    // No Postgres pool in local mode — recall/save run over the engine.
    let shared = SharedToolCtx {
        // Forward to a running serve so stdio agents appear in the live overlay.
        consumer_sink: remote_consumer_sink(),
        federation: None,
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
        memory,
        hitl: None,
        memory_settings_path: None,
    };
    McpState {
        dispatch: ToolDispatch::new(shared),
        server_info: ServerInfo {
            name: "kyma".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    }
}

/// A running async memory ingest queue: the submit/barrier handle, the worker
/// task, and the trigger that tells the worker to drain + stop.
struct LocalMemoryQueue {
    queue: kyma_memory::MemoryQueue,
    worker: tokio::task::JoinHandle<()>,
    stop: tokio::sync::oneshot::Sender<()>,
}

impl LocalMemoryQueue {
    /// Flush queued memories and stop the worker. Called when the transport
    /// is done (stdin EOF / HTTP shutdown) so no queued memory is lost.
    async fn shutdown(self) {
        if !self.queue.drain(std::time::Duration::from_secs(15)).await {
            warn!("memory queue drain timed out; the worker's shutdown pass retries");
        }
        let _ = self.stop.send(());
        // The worker's shutdown arm drains anything still buffered.
        let _ = self.worker.await;
    }
}

/// Spawn the async memory ingest worker over the local engine. Returns `None`
/// (synchronous memory writes) when `KYMA_MEMORY_ASYNC=0` or the embedding
/// backend cannot be built. Local default: in-memory queue tier — crash loss
/// window is bounded by the flush linger; durable opt-in via
/// `KYMA_MEMORY_QUEUE_DURABLE=1` (persists pending saves to the catalog).
async fn spawn_local_memory_queue(engine: &Engine) -> Option<LocalMemoryQueue> {
    let disabled = std::env::var("KYMA_MEMORY_ASYNC")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false);
    if disabled {
        info!("KYMA_MEMORY_ASYNC=0 — memory writes are synchronous");
        return None;
    }
    let embed = match kyma_memory::shared_embedding().await {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "embedding backend unavailable; memory writes stay synchronous");
            return None;
        }
    };
    let cfg = kyma_memory::MemoryIngestConfig::from_env(false);
    let (stop, stop_rx) = tokio::sync::oneshot::channel::<()>();
    // Local/embedded mode has no cluster node registration (no `nodes` row to
    // reuse) — a fresh id is fine here: the embedded SQLite catalog's
    // `background_tasks.claimed_by` carries no foreign key to `nodes`, unlike
    // the Postgres catalog used by the server binary.
    let (queue, worker) = kyma_memory::spawn_memory_queue(
        engine.catalog.clone(),
        engine.format.clone(),
        embed,
        cfg,
        kyma_core::types::NodeId::new(),
        async move {
            let _ = stop_rx.await;
        },
    );
    info!("async memory ingest queue started (batched embeds + group commits)");
    Some(LocalMemoryQueue {
        queue,
        worker,
        stop,
    })
}

/// `kyma mcp` — serve the Model Context Protocol over stdio.
///
/// The caller (the `kyma` binary) must route tracing to **stderr** — stdout is
/// the JSON-RPC protocol channel.
pub async fn run_mcp() -> Result<()> {
    kyma_server::agent::identity::set_source("mcp-stdio");
    let engine = open_engine(&resolve_paths()).await?;
    // Opportunistic Claude Code file-memory sync: a session is starting, so
    // pick up any memory files that changed since the last one. Detached —
    // never delays the protocol handshake. Kill switch: KYMA_CC_SYNC_ON_MCP=0.
    if env_flag("KYMA_CC_SYNC_ON_MCP", true) && env_flag("KYMA_CC_FILE_SYNC", true) {
        let eng = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = run_cc_phase(&eng, None, &SyncOptions::default()).await {
                tracing::debug!("mcp-startup cc-sync: {e}");
            }
        });
    }
    let memq = spawn_local_memory_queue(&engine).await;
    let state = mcp_state(&engine, memq.as_ref().map(|m| m.queue.clone()));
    info!("serving MCP over stdio (memory + data + graph); stdin/stdout is the protocol channel");
    let served = serve_stdio(state).await;
    // stdin EOF (client disconnected): land queued memories before exiting —
    // this is what makes async saves safe in a short-lived stdio process.
    if let Some(memq) = memq {
        memq.shutdown().await;
    }
    served.context("stdio MCP loop")?;
    Ok(())
}

/// Assemble the complete local-mode axum router from already-opened engine
/// components.
///
/// Exposed for integration tests and for embedding the local server in other
/// contexts — the returned `Router` is the exact same application that
/// `run_serve` binds and serves. `run_serve` calls this function and then
/// performs the socket-binding, signal handling, and background-worker
/// lifecycle that are not part of router construction.
///
/// Also returns the `AgentState` so the caller can start optional background
/// workers (e.g. the `KYMA_CC_WATCH` watcher) that hold a reference to it.
/// `watcher_status` is passed in by the caller (created before `SqliteDataSourceCatalog`
/// so it can be injected into the catalog for `list_watchers()`).
#[allow(clippy::too_many_arguments)]
pub fn build_local_app(
    catalog: Arc<dyn kyma_core::catalog::Catalog>,
    format: Arc<dyn kyma_core::segment_format::SegmentFormat>,
    backend: Arc<dyn AuthBackend>,
    memory: Option<kyma_memory::MemoryQueue>,
    local_dreaming: Option<Arc<kyma_server::agent::dreaming_local::LocalDreamingStore>>,
    mcp_url: Option<String>,
    // Optional credential store — if None, falls back to NullCredentialStore.
    cred_store: Option<Arc<dyn CredentialStore>>,
    // Optional data source admin state (catalog + registry). When provided the
    // /v1/data-sources routes are mounted behind Role::Write.
    ds_admin: Option<(Arc<dyn DataSourceCatalog>, Arc<DataSourceRegistry>)>,
    // In-process watcher status — created before SqliteDataSourceCatalog and
    // injected into it so list_watchers() returns cc-sync heartbeats.
    watcher_status: watcher_status::LocalWatcherStatus,
    // Located `git` binary — mounts the brain repos surface (`/v1/brain` +
    // `/git/<name>.git`). `None` ⇒ management API reports git_available:false
    // and repo-touching endpoints answer 503.
    brain_git: Option<Arc<kyma_brain::gitbin::GitBin>>,
) -> (axum::Router, AgentState, kyma_server::brain::BrainState) {
    let schema_cache = Arc::new(SchemaCache::from_env());
    let query_state = QueryState {
        federation: None,
        catalog: catalog.clone(),
        format: format.clone(),
        schema_cache: schema_cache.clone(),
        node_id: None,
        pg_pool: None, // local: no Postgres — pool-only surfaces degrade gracefully
        layout_cache: std::sync::Arc::new(kyma_server::graph_layout_cache::LayoutCache::new()),
    };
    // Engine preference + enabled skills persist to JSON under ~/.kyma so
    // Settings → Agent engine works locally and survives restarts. Engine
    // auth auto-detects env vars / ~/.claude/.credentials.json — the Postgres
    // credential store stays a control-plane feature.
    let kyma_home = std::env::var("KYMA_HOME").unwrap_or_else(|_| {
        let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{base}/.kyma")
    });
    let resolved_cred_store: Arc<dyn CredentialStore> = cred_store
        .clone()
        .unwrap_or_else(|| Arc::new(NullCredentialStore));
    // Live consumer-activity bus — fed by the memory tool paths, subscribed by
    // the /v1/consumers/live WebSocket that drives the graph explorer overlay.
    let consumer_events = kyma_ingest_core::ConsumerEvents::new(256);
    let agent_state = AgentState {
        catalog: catalog.clone(),
        format: format.clone(),
        pool: None, // local: run/session history not persisted; memory runs over the engine
        engines: Arc::new(FileEnginePreferenceStore::new(std::format!(
            "{kyma_home}/agent-engine.json"
        ))),
        credentials: resolved_cred_store,
        tenant: kyma_core::tenant::DEFAULT_TENANT,
        skills: Arc::new(FileEnabledSkillsStore::new(std::format!(
            "{kyma_home}/agent-skills.json"
        ))),
        // Loopback to this serve's own MCP endpoint so the ClaudeCli engine can
        // reach the local memory + data tools during dreaming/ask. `None` keeps
        // MCP wiring disabled (adk engines query the engine directly).
        mcp_url,
        memory: memory.clone(),
        // Degraded local-mode dreaming: inline execution + in-memory ring + SQLite.
        local_dreaming,
        // Local memory settings persist to a JSON file under ${KYMA_HOME}.
        memory_settings_path: Some(std::path::PathBuf::from(std::format!(
            "{kyma_home}/memory-settings.json"
        ))),
        consumer_events: Some(consumer_events.clone()),
    };
    let ingest_events = IngestEvents::new(256);
    let write_path =
        WritePath::new(catalog.clone(), format.clone()).with_events(ingest_events.clone());
    let ingest_state = IngestState {
        catalog: catalog.clone(),
        write_path,
    };
    let read_mw = || {
        axum::middleware::from_fn_with_state(
            AuthLayerState {
                backend: backend.clone(),
                required: Role::Read,
            },
            require_role_middleware,
        )
    };
    let write_mw = || {
        axum::middleware::from_fn_with_state(
            AuthLayerState {
                backend: backend.clone(),
                required: Role::Write,
            },
            require_role_middleware,
        )
    };

    let admin_mw = || {
        axum::middleware::from_fn_with_state(
            AuthLayerState {
                backend: backend.clone(),
                required: Role::Admin,
            },
            require_role_middleware,
        )
    };

    // The same web interface + full API the hosted server serves, over the
    // embedded catalog. Read surfaces (query/catalog/graph/agent/memory/MCP)
    // require Role::Read; ingest + dashboard/cleanup writes require
    // Role::Write; user admin requires Role::Admin; login/web/health are open.
    // Control-plane-only surfaces (data sources, credentials, OAuth, saved-view
    // writes) are NOT mounted — /v1/capabilities tells clients so, and the
    // SPA fallback 404s unknown /v1/* paths instead of serving HTML.
    // (agent_state is cloned: the KYMA_CC_WATCH watcher in run_serve keeps one.)
    // Build a second QueryState for the live router sharing the same schema_cache.
    let query_state_for_live = QueryState {
        federation: None,
        catalog: catalog.clone(),
        format: format.clone(),
        schema_cache,
        node_id: None,
        pg_pool: None,
        layout_cache: std::sync::Arc::new(kyma_server::graph_layout_cache::LayoutCache::new()),
    };
    // Build McpState from the same catalog + format the rest of the app uses.
    let mcp = McpState {
        dispatch: ToolDispatch::new(SharedToolCtx {
            consumer_sink: Some(std::sync::Arc::new(LocalConsumerPublisher {
                events: consumer_events.clone(),
                tenant: kyma_core::tenant::DEFAULT_TENANT,
            })),
            federation: None,
            catalog: catalog.clone(),
            format: format.clone(),
            pool: None,
            memory: memory.clone(),
            hitl: None,
            memory_settings_path: agent_state.memory_settings_path.clone(),
        }),
        server_info: ServerInfo {
            name: "kyma".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    let read_router = kyma_server::router_with_agent(query_state, agent_state.clone())
        .merge(kyma_mcp::router(mcp))
        .merge(kyma_server::capabilities::router(
            kyma_server::capabilities::Capabilities::LOCAL,
        ))
        // POST /v1/consumers/emit — separate kyma mcp (stdio) processes forward
        // their consumer activity here so they show in the live overlay.
        .merge(
            kyma_server::discover::consumers_live::consumers_emit_router(Some(
                consumer_events.clone(),
            )),
        )
        .layer(read_mw());
    let ingest_router = kyma_ingest_rest::router(ingest_state).layer(write_mw());
    // Dashboards + table cleanup write over the Catalog trait — fully
    // supported by the embedded SQLite catalog (the web UI needs them).
    let local_write_router = kyma_server::dashboards_write_router(catalog.clone())
        .merge(kyma_server::cleanup_write_router(catalog.clone()))
        .merge(kyma_server::compact_write_router(catalog.clone()))
        .layer(write_mw());
    // Keep the per-tenant quota cache (S2.6) fresh so the admission limiter and
    // the /v1/admin/tenant-quotas endpoint work in local mode too. No-op when the
    // tenant_quotas table is empty (the single-tenant default).
    let _quota_refresh_handle = kyma_server::quota_cache::spawn_refresh(catalog.clone());
    let admin_users_router =
        kyma_server::admin_handler::admin_users_router(catalog.clone()).layer(admin_mw());
    let session_router =
        kyma_server::auth_handler::auth_session_router(catalog.clone()).layer(read_mw());

    // Live-tail WebSocket — mounted WITHOUT auth middleware; the session
    // authenticates via its first message (browsers can't send WS headers).
    // Live consumers WebSocket — same auth-by-first-message pattern as the
    // live-tail router. Backfills recent memory spans from the local `otel`
    // self-trace DB (matches the frontend's OPS_DB).
    let consumers_router = kyma_server::discover::consumers_live::consumers_live_router(
        query_state_for_live.clone(),
        backend.clone(),
        Some(consumer_events),
        "otel".to_string(),
    );
    let live_router = kyma_server::discover::live::explore_live_router(
        query_state_for_live,
        backend.clone(),
        Some(ingest_events),
    );
    // /v1/workers stub (empty registry) so the dreaming UI's NodesStrip renders
    // its empty state. Behind read auth like the rest of the API surface.
    let workers_router = kyma_server::local_workers_router().layer(read_mw());
    // In-process watcher registry serving /v1/data-sources/watchers.
    // Omit this when the full datasource admin router is mounted — that router
    // already exposes /v1/data-sources/watchers (via SqliteDataSourceCatalog
    // which has watcher_status injected) and the two would conflict.
    let watchers_router = if ds_admin.is_none() {
        Some(watcher_status.router().layer(read_mw()))
    } else {
        None
    };
    // Settings route always mounted — lets the UI toggle cc-sync regardless of
    // whether the full ds_admin router is present.
    let watcher_settings_router = watcher_status.settings_router().layer(read_mw());

    // Credentials CRUD — only when a real credential store was supplied.
    let creds_router = cred_store.map(|store| {
        kyma_server::credentials_handler::router(
            kyma_server::credentials_handler::CredentialsState { store },
        )
        .layer(write_mw())
    });

    // Data source admin CRUD — only when a catalog + registry were supplied.
    let ds_router = ds_admin.map(|(ds_catalog, ds_registry)| {
        kyma_server::datasource_admin_router(DataSourceAdminState {
            catalog: ds_catalog,
            registry: ds_registry,
        })
        .layer(write_mw())
    });

    // Brain repos: /v1/brain management (read-mounted; mutating handlers gate
    // Write/Admin in-handler) + /git/<name>.git smart HTTP with Basic auth.
    let brain_state = kyma_server::brain::BrainState::new(
        Arc::new(brain_registry::LocalBrainRegistry::new(std::format!(
            "{kyma_home}/brains.json"
        ))),
        brain_git,
        std::path::PathBuf::from(std::format!("{kyma_home}/brain")),
        agent_state.clone(),
    );
    let brain_mgmt_router =
        kyma_server::brain::routes::brain_router(brain_state.clone()).layer(read_mw());
    let brain_git_router = kyma_server::brain::git_http::git_http_router(brain_state.clone()).layer(
        axum::middleware::from_fn_with_state(
            AuthLayerState {
                backend: backend.clone(),
                required: Role::Read,
            },
            kyma_server::auth::require_git_auth_middleware,
        ),
    );

    let mut app = read_router
        .merge(ingest_router)
        .merge(local_write_router)
        .merge(admin_users_router)
        .merge(session_router)
        .merge(workers_router)
        .merge(kyma_server::auth_handler::auth_login_router(
            catalog.clone(),
        ))
        .merge(kyma_server::health_router())
        .merge(live_router)
        .merge(consumers_router)
        .merge(kyma_server::web_ui::router())
        .merge(watcher_settings_router)
        .merge(brain_mgmt_router)
        .merge(brain_git_router);
    if let Some(r) = watchers_router {
        app = app.merge(r);
    }
    if let Some(r) = creds_router {
        app = app.merge(r);
    }
    if let Some(r) = ds_router {
        app = app.merge(r);
    }
    let app = kyma_server::with_permissive_cors(app);
    (app, agent_state, brain_state)
}

/// `kyma serve` — serve the web UI + full HTTP API on `addr`, over the embedded
/// catalog (zero infra).
///
/// `self_trace_handle` is the value returned by `setup_serve_tracing()`. When
/// `Some`, the self-trace exporter is wired to the catalog write path so that
/// internal `kyma_telemetry` spans land in the `otel_traces` table.
pub async fn run_serve(
    addr: SocketAddr,
    self_trace_handle: Option<Arc<OnceLock<SelfTraceCtx>>>,
) -> Result<()> {
    kyma_server::agent::identity::set_source("local-serve");
    let engine = open_engine(&resolve_paths()).await?;

    // Wire self-trace exporter now that the catalog + write path are ready.
    // This makes internal kyma_telemetry spans land in otel_traces immediately.
    // Pre-create the table so the Traces page never 404s on an empty database.
    if let Some(handle) = &self_trace_handle {
        let wp = WritePath::new(engine.catalog.clone(), engine.format.clone());
        let _ = handle.set(SelfTraceCtx {
            catalog: engine.catalog.clone(),
            write_path: wp,
            database: "otel".into(),
        });
    }
    kyma_ingest_otlp::ensure_traces_table(&engine.catalog, "otel").await;

    let memq = spawn_local_memory_queue(&engine).await;
    let memory = memq.as_ref().map(|m| m.queue.clone());

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

    // Self-heal the CLI connection: the CLI, MCP, and every coding-agent
    // capture hook read ~/.kyma/config.json, and a stale token there (e.g. an
    // expired browser session token from `kyma connect`) silently 401s them
    // all. Validate it against this serve's auth backend and mint a durable
    // replacement when needed. Best-effort — never blocks startup.
    if env_flag("KYMA_LOCAL_HEAL_CONFIG", true) {
        let cfg_path = std::path::PathBuf::from(home_dir()).join(".kyma/config.json");
        match cli_config_heal::heal_cli_config(&cfg_path, addr, &backend, &engine.catalog).await {
            Ok(cli_config_heal::HealOutcome::TokenMinted) => {
                info!(config = %cfg_path.display(), "minted durable CLI token (stored token was missing or stale)");
            }
            Ok(cli_config_heal::HealOutcome::EndpointRepaired) => {
                info!(config = %cfg_path.display(), "repaired CLI endpoint");
            }
            Ok(cli_config_heal::HealOutcome::ForeignEndpoint) => {
                info!(config = %cfg_path.display(), "CLI config points at another server — left untouched");
            }
            Ok(cli_config_heal::HealOutcome::TokenValid) => {}
            Err(e) => warn!(error = %e, "couldn't self-heal CLI config"),
        }
    }

    // Degraded local-mode dreaming state: in-memory ring hydrated from the
    // embedded SQLite catalog. Inline runs + the dreaming HTTP handlers read it.
    let local_dreaming = Some(
        kyma_server::agent::dreaming_local::LocalDreamingStore::new(engine.catalog.clone()).await,
    );
    // Loopback URL for this serve's own MCP endpoint, so the ClaudeCli engine
    // can reach the local memory/data tools during a dreaming run. When bound
    // to all interfaces (0.0.0.0 / ::), dial 127.0.0.1 instead — the unspecified
    // address isn't connectable as a destination.
    let mcp_host = if addr.ip().is_unspecified() {
        format!("127.0.0.1:{}", addr.port())
    } else {
        addr.to_string()
    };
    let mcp_url = Some(format!("http://{mcp_host}/mcp/v1"));

    // Local serve always seeds a user, so the auth backend is "enabled" and the
    // MCP endpoint requires a bearer. The dreaming ClaudeCli path reads
    // KYMA_INTERNAL_BEARER for that header. If a static admin token is
    // configured via KYMA_AUTH_TOKENS and KYMA_INTERNAL_BEARER isn't already
    // set, derive it so dreaming-over-ClaudeCli works out of the box.
    if std::env::var("KYMA_INTERNAL_BEARER").is_err() {
        if let Some(tok) = first_admin_token() {
            // SAFETY: set before any worker thread reads it; single-threaded here.
            std::env::set_var("KYMA_INTERNAL_BEARER", tok);
        }
    }

    // ── Credentials: AES-256-GCM key from env or auto-generated file ──────────
    let kyma_home = std::env::var("KYMA_HOME").unwrap_or_else(|_| {
        let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{base}/.kyma")
    });
    let secret_key_path = format!("{kyma_home}/secret.key");
    let crypto = Arc::new(
        Crypto::from_env_or_file(&secret_key_path)
            .context("loading or generating local secret key")?,
    );
    let pool = engine.sqlite.pool().clone();
    let cred_store: Arc<dyn CredentialStore> =
        Arc::new(cred_store::SqliteCredentialStore::new(pool.clone(), crypto));

    // ── In-process watcher status (created before ds_catalog so it can be
    // injected for list_watchers()) ─────────────────────────────────────────
    let watcher_status = {
        let ws = watcher_status::LocalWatcherStatus::default();
        let settings = watcher_settings::WatcherSettings::load().await;
        ws.set_cc_sync_enabled(settings.cc_sync_enabled);
        ws
    };

    // ── Data Sources: catalog + connector registry ──────────────────────────
    let ds_catalog: Arc<dyn DataSourceCatalog> = Arc::new(
        datasource_catalog::SqliteDataSourceCatalog::new(pool.clone())
            .with_watcher_status(watcher_status.clone()),
    );
    let ds_control = Arc::new(datasource_catalog::SqliteDataSourceControl::new(
        pool.clone(),
    ));

    let mut conn_reg = DataSourceRegistry::new();
    // Register all supported connectors (same set as the hosted server).
    use kyma_datasources::prometheus::PromDataSource;
    conn_reg.register(Arc::new(PromDataSource));
    conn_reg.register(Arc::new(kyma_datasources::postgres::PgIntrospectDataSource));
    conn_reg.register(Arc::new(kyma_datasources::s3::S3DataSource));
    conn_reg.register(Arc::new(kyma_datasources::gitlab::GitlabDataSource));
    conn_reg.register(Arc::new(kyma_datasources::bitbucket::BitbucketDataSource));
    conn_reg.register(Arc::new(kyma_datasources::github::GithubDataSource));
    conn_reg.register(Arc::new(kyma_datasources::notion::NotionDataSource));
    conn_reg.register(Arc::new(kyma_datasources::googledrive::GdriveDataSource));
    conn_reg.register(Arc::new(kyma_datasources::gmail::GmailDataSource));
    conn_reg.register(Arc::new(kyma_datasources::slack::SlackDataSource));
    conn_reg.register(Arc::new(kyma_datasources::jira::JiraDataSource));
    conn_reg.register(Arc::new(kyma_datasources::confluence::ConfluenceDataSource));
    conn_reg.register(Arc::new(kyma_datasources::msfabric::MsFabricDataSource));
    conn_reg.register(Arc::new(kyma_datasources::obsidian::ObsidianDataSource));
    let conn_registry = Arc::new(conn_reg);

    // ── RowSink: auto-create + evolve schema, then ingest ───────────────────
    let catalog_for_sink = engine.catalog.clone();
    let format_for_sink = engine.format.clone();
    let conn_sink: RowSink = Arc::new(
        move |db: String, tbl: String, rows: Vec<serde_json::Value>, idem: Option<String>| {
            let catalog = catalog_for_sink.clone();
            let write_path =
                kyma_ingest_core::WritePath::new(catalog.clone(), format_for_sink.clone());
            Box::pin(async move {
                let table = kyma_ingest_core::ensure_table(catalog.as_ref(), &db, &tbl)
                    .await
                    .map_err(|e| anyhow::anyhow!("ensure_table: {e}"))?;
                let table = kyma_ingest_core::evolve_schema_for_records(
                    catalog.as_ref(),
                    &db,
                    table,
                    &rows,
                )
                .await
                .map_err(|e| anyhow::anyhow!("evolve_schema: {e}"))?;
                let batches = kyma_datasources::arrow_coerce::rows_to_batches(&table.schema, rows)
                    .map_err(|e| anyhow::anyhow!("arrow coerce: {e}"))?;
                write_path
                    .ingest_with_idempotency(&db, &table, batches, idem.as_deref())
                    .await
                    .map_err(|e| anyhow::anyhow!("ingest: {e}"))?;
                Ok(())
            })
        },
    );

    // ── GraphRegisterFn: create property graph binding after ingest ─────────
    let catalog_for_graph = engine.catalog.clone();
    let graph_register: GraphRegisterFn =
        Arc::new(move |db: String, hint: kyma_datasources::GraphHint| {
            let catalog = catalog_for_graph.clone();
            Box::pin(async move {
                let spec = kyma_core::catalog::GraphSpec::with_defaults(
                    hint.node_table.clone(),
                    hint.edge_table.clone(),
                );
                match catalog.create_graph(&db, &hint.graph_name, spec).await {
                    Ok(_) => {}
                    Err(e) => {
                        let msg = e.to_string();
                        if !msg.contains("already exists")
                            && !msg.contains("duplicate")
                            && !msg.contains("23505")
                        {
                            tracing::warn!(db=%db, graph=%hint.graph_name, "graph register: {e}");
                        }
                    }
                }
                Ok(())
            })
        });

    let tick_deps = DataSourceTickDeps {
        control: ds_control.clone(),
        registry: conn_registry.clone(),
        sink: conn_sink,
        graph_register,
        secrets: Arc::new(EnvSecretStore),
        credentials: cred_store.clone(),
        oauth: None, // OAuth requires Postgres
        artifacts: None,
        catalog: Some(engine.catalog.clone()),
    };

    // Brain repos need the git binary; absent ⇒ the surface degrades to 503s.
    let brain_git = kyma_brain::gitbin::GitBin::detect().await.map(Arc::new);

    let (app, agent_state, brain_state) = build_local_app(
        engine.catalog.clone(),
        engine.format.clone(),
        backend,
        memory,
        local_dreaming.clone(),
        mcp_url,
        Some(cred_store),
        Some((ds_catalog.clone(), conn_registry.clone())),
        watcher_status.clone(),
        brain_git,
    );

    // ── Data source scheduler: periodic tick + trigger enqueue ──────────────
    {
        let sched = DataSourceScheduler::new(ds_catalog.clone());
        tokio::spawn(async move {
            sched
                .run(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await;
        });
        info!("local data source scheduler running");
    }

    // ── Fabric worker: claims + executes data_source_sync, plus (S1.5) the
    //    index_build + embed_backfill jobs that activate local-mode ANN + BM25
    //    retrieval — every server retrieval capability except the global
    //    centroid tree, which is server-only by design. ────────────────────────
    {
        let worker_id = uuid::Uuid::new_v4();
        let queue = Arc::new(sqlite_queue::SqliteQueue::new(
            pool.clone(),
            worker_id,
            vec!["data_source".to_string()],
            120, // 2-minute lease
        ));
        let mut exec_reg = kyma_jobs::ExecutorRegistry::new();
        exec_reg.register(Arc::new(
            kyma_jobs::datasource_sync::DataSourceSyncExecutor::new(tick_deps),
        ));

        // Register the sidecar-building executors + start the activation
        // scheduler. Gated on an object store (always present for the local TLM
        // format); the scheduler enqueues into the same SQLite `jobs` table this
        // worker drains.
        let mut indexing_kinds = "data_source_sync";
        if let Some(store) = engine.format.object_store() {
            let mut builders: std::collections::HashMap<
                kyma_core::index_sidecar::SidecarKind,
                Arc<dyn kyma_core::index_sidecar::SidecarBuilder>,
            > = std::collections::HashMap::new();
            builders.insert(
                kyma_core::index_sidecar::SidecarKind::IvfRabitq,
                Arc::new(kyma_index_vector::IvfRabitqBuilder::new()),
            );
            builders.insert(
                kyma_core::index_sidecar::SidecarKind::TantivyFts,
                Arc::new(kyma_index_fts::TantivyFtsBuilder::new()),
            );
            exec_reg.register(Arc::new(kyma_jobs::index_build::IndexBuildExecutor::new(
                engine.catalog.clone(),
                engine.format.clone(),
                store,
                builders,
            )));
            // The embed_backfill executor needs the process embedding backend.
            // If unavailable (provider feature off) only ANN/FTS over already-
            // embedded data activate; embeddings stay as ingested.
            match kyma_memory::shared_embedding().await {
                Ok(embedder) => {
                    exec_reg.register(Arc::new(
                        kyma_jobs::embed_backfill::EmbedBackfillExecutor::new(
                            engine.catalog.clone(),
                            engine.format.clone(),
                            embedder,
                        ),
                    ));
                    indexing_kinds = "data_source_sync + index_build + embed_backfill";
                }
                Err(e) => {
                    warn!(error = %e, "embedding backend unavailable; local embed_backfill disabled");
                    indexing_kinds = "data_source_sync + index_build";
                }
            }

            // Activation scheduler: enqueue missing index_build / embed_backfill
            // jobs over the SQLite jobs table. Same scan logic the server runs;
            // stops on Ctrl-C like the other local loops.
            let scheduler = kyma_compaction::IndexScheduler::with_enqueuer(
                engine.catalog.clone(),
                Arc::new(sqlite_queue::SqliteEnqueuer::new(pool.clone())),
            );
            let (sched_shutdown, sched_rx) = tokio::sync::broadcast::channel::<()>(1);
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = sched_shutdown.send(());
            });
            tokio::spawn(scheduler.run(sched_rx));

            // Graph-snapshot activation scheduler (S3.2): the same refresh loop
            // the server runs, so local-mode deep-subgraph queries also serve
            // from a persistent CSR snapshot once a graph's edges change.
            let graph_snap = kyma_server::graph_snapshot_sched::GraphSnapshotScheduler::new(
                engine.catalog.clone(),
                engine.format.clone(),
            );
            let (gsnap_shutdown, gsnap_rx) = tokio::sync::broadcast::channel::<()>(1);
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = gsnap_shutdown.send(());
            });
            tokio::spawn(graph_snap.run(gsnap_rx));
        }

        let runner = kyma_jobs::JobRunner::new(queue, exec_reg, 120);
        tokio::spawn(async move {
            runner
                .run(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await;
        });
        info!(%worker_id, kinds = indexing_kinds, "local fabric worker running");
    }

    // Optional in-process dreaming scheduler — only fires when dreaming is
    // enabled in ${KYMA_HOME}/memory-settings.json (OFF by default). Runs inline
    // in this process; no worker fabric.
    if let Some(store) = local_dreaming {
        let scheduler =
            kyma_server::agent::dreaming::LocalDreamingScheduler::new(agent_state.clone(), store);
        tokio::spawn(async move {
            scheduler
                .run(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await;
        });
    }

    // In-process brain-export scheduler — exports each brain on its own
    // interval (per-brain `export_interval_secs`; 0 = manual only). Shares
    // the app's BrainState so exports serialize against pushes.
    {
        let scheduler = kyma_server::brain::scheduler::LocalBrainScheduler::new(brain_state);
        tokio::spawn(async move {
            scheduler
                .run(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await;
        });
    }

    // In-process compaction. Local mode previously never compacted (the
    // scheduler was hardcoded to Postgres), so the `extents` table grew
    // unbounded and every scan paid to enumerate thousands of tiny segments.
    // The worker + scheduler now cooperate via the catalog task queue over
    // SQLite; each stops on Ctrl-C like the dreaming scheduler above. Thresholds
    // are tunable via KYMA_COMPACTION_* (same knobs as server mode).
    {
        use kyma_compaction::{CompactionScheduler, CompactionWorker};
        let mut worker = CompactionWorker::new(
            engine.catalog.clone(),
            engine.format.clone(),
            kyma_core::types::NodeId::new(),
        );
        let mut scheduler = CompactionScheduler::new(engine.catalog.clone());
        if let Some(ms) = std::env::var("KYMA_COMPACTION_IDLE_SLEEP_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            worker.idle_sleep = std::time::Duration::from_millis(ms);
        }
        if let Some(s) = std::env::var("KYMA_COMPACTION_POLL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            scheduler.poll_interval = std::time::Duration::from_secs(s);
        }
        if let Some(n) = std::env::var("KYMA_COMPACTION_MIN_EXTENTS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
        {
            scheduler.min_extents_to_compact = n;
        }
        tokio::spawn(worker.run(async {
            let _ = tokio::signal::ctrl_c().await;
        }));
        tokio::spawn(scheduler.run(async {
            let _ = tokio::signal::ctrl_c().await;
        }));
        info!("local compaction worker + scheduler running");
    }

    // Vault watchers: continuous sync loops for continuous-drive data sources
    // (obsidian). Reconciles enabled rows ↔ running notify loops every 15s,
    // so create/pause/resume/delete in the UI take effect without a restart.
    {
        let eng = engine.clone();
        let status = watcher_status.clone();
        let control = ds_control.clone();
        let pool_w = pool.clone();
        tokio::spawn(async move {
            source_watchers::run_manager(eng, status, control, pool_w).await;
        });
        info!("vault watcher manager running (continuous data sources)");
    }

    // Resident watcher: keep Claude Code file memory synced while the local
    // server runs. On by default — disable via the UI (Settings) or by setting
    // KYMA_CC_WATCH=0 as a host-level kill switch.
    if env_flag("KYMA_CC_WATCH", true) {
        let eng = engine.clone();
        let agent = agent_state.clone();
        let poll = env_secs("KYMA_CC_SYNC_POLL_SECS", 30);
        let status = watcher_status.clone();
        tokio::spawn(async move {
            let opts = SyncOptions::default();
            let (node_host, node_id, identity) = watcher_status::node_identity();
            let config = serde_json::json!({
                "poll_secs": poll.as_secs(),
                "root": "~/.claude/projects",
            });
            let started_at = chrono::Utc::now().to_rfc3339();
            let mut last_scan: Option<serde_json::Value> = None;
            loop {
                if status.cc_sync_enabled() {
                    match run_cc_phase(&eng, Some(&agent), &opts).await {
                        Ok(scan) => last_scan = Some(scan),
                        Err(e) => tracing::warn!("cc-sync watcher: {e}"),
                    }
                    status.upsert(watcher_status::LocalWatcher {
                        id: "cc-sync".into(),
                        kind: "cc_sync".into(),
                        node_host: node_host.clone(),
                        node_id: node_id.clone(),
                        identity: identity.clone(),
                        config: config.clone(),
                        started_at: started_at.clone(),
                        last_heartbeat_at: chrono::Utc::now().to_rfc3339(),
                        last_scan: last_scan.clone(),
                    });
                } else {
                    // Disabled — remove from the visible list so the UI shows
                    // the empty state with the Enable button.
                    status.remove("cc-sync");
                    last_scan = None;
                }
                tokio::time::sleep(poll).await;
            }
        });
        let enabled = watcher_status.cc_sync_enabled();
        if enabled {
            info!(
                "cc-sync watcher running (every {:?}; toggle in UI or KYMA_CC_WATCH=0 to disable)",
                poll
            );
        } else {
            info!("cc-sync watcher spawned but disabled via settings (enable in UI)");
        }
    }

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
    // `into_make_service_with_connect_info` so handlers can read the peer addr
    // (the live-consumers overlay records the connecting agent's ip/pid).
    let served = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
        info!("shutdown signal received");
    })
    .await
    .context("http server");
    // Land queued memories before exiting (Ctrl-C path).
    if let Some(memq) = memq {
        memq.shutdown().await;
    }
    served?;
    Ok(())
}

/// Options for `kyma sync`.
#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    /// Keep running, re-syncing on an interval (`KYMA_CC_SYNC_POLL_SECS`,
    /// default 30s).
    pub watch: bool,
    /// Plan + audit-log Claude Code file changes without writing them
    /// (memory ingestion into the local engine still runs).
    pub dry_run: bool,
    /// Only the local Claude Code file phase.
    pub cc_only: bool,
    /// Only the control-plane push/pull.
    pub cloud_only: bool,
    /// Limit the file phase to one project path.
    pub project: Option<std::path::PathBuf>,
}

fn env_flag(key: &str, default: bool) -> bool {
    std::env::var(key).map_or(default, |v| v != "0")
}

/// First `admin`-role token from `KYMA_AUTH_TOKENS` (`tok:role,…`), if any.
/// Used to derive `KYMA_INTERNAL_BEARER` so the dreaming ClaudeCli path can
/// reach the auth-protected loopback MCP endpoint.
fn first_admin_token() -> Option<String> {
    let raw = std::env::var("KYMA_AUTH_TOKENS").ok()?;
    raw.split(',').find_map(|pair| {
        let (tok, role) = pair.trim().split_once(':')?;
        (role.trim() == "admin" && !tok.trim().is_empty()).then(|| tok.trim().to_string())
    })
}

fn env_secs(key: &str, default: u64) -> std::time::Duration {
    std::time::Duration::from_secs(
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default),
    )
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
}

/// `~/.claude` (override: `KYMA_CC_HOME`).
fn claude_home() -> std::path::PathBuf {
    std::env::var("KYMA_CC_HOME")
        .unwrap_or_else(|_| format!("{}/.claude", home_dir()))
        .into()
}

/// Assemble the file-phase options from env + CLI options.
fn cc_pipeline_options(opts: &SyncOptions) -> cc_pipeline::CcPipelineOptions {
    let cc = claude_home();
    let kyma_home = std::env::var("KYMA_HOME").unwrap_or_else(|_| format!("{}/.kyma", home_dir()));
    cc_pipeline::CcPipelineOptions {
        sync: cc_sync::CcSyncOptions {
            projects_dir: cc.join("projects"),
            claude_json: Some(format!("{}/.claude.json", home_dir()).into()),
            project: opts.project.clone(),
        },
        curate: env_flag("KYMA_CC_CURATE", true),
        promote_cfg: kyma_server::agent::cc_curate::CurationConfig::from_env(),
        llm_cfg: kyma_server::agent::cc_curate::LlmCurationConfig::from_env(),
        writeback: cc_writeback::WritebackConfig {
            dry_run: opts.dry_run,
            quiet_window: env_secs("KYMA_CC_QUIET_WINDOW", 300),
            lock_ttl: env_secs("KYMA_CC_LOCK_TTL", 300),
            sessions_dir: Some(cc.join("sessions")),
            audit_log: Some(format!("{kyma_home}/cc-curation.log").into()),
        },
    }
}

/// One Claude Code file-phase pass: ingest memory files → curate → apply.
///
/// Returns the sync-phase rollup in the watcher `last_scan` shape
/// (`CcSyncReport::last_scan_value`) so the `KYMA_CC_WATCH` loop can feed
/// `/v1/data-sources/watchers`; other callers ignore it.
async fn run_cc_phase(
    engine: &Engine,
    agent: Option<&AgentState>,
    opts: &SyncOptions,
) -> Result<serde_json::Value> {
    let pass_start = std::time::Instant::now();
    let wall_start = chrono::Utc::now();
    let embed = kyma_memory::shared_embedding()
        .await
        .map_err(|e| anyhow::anyhow!("embedding backend: {e}"))?;
    let writer =
        kyma_memory::MemoryWriter::new(engine.catalog.clone(), engine.format.clone(), embed);
    let popts = cc_pipeline_options(opts);
    let report = cc_pipeline::run_pass(engine, &writer, agent, &popts).await?;
    let (mut up, mut skip, mut arch) = (0, 0, 0);
    for p in &report.sync.projects {
        up += p.upserted + p.user_edited;
        skip += p.skipped;
        arch += p.archived;
    }
    let (mut written, mut farch) = (0, 0);
    for c in &report.curated {
        written += c.applied.written;
        farch += c.applied.archived;
        tracing::debug!(
            realm = %c.realm,
            promoted = c.outcome.promoted,
            refreshed = c.outcome.refreshed,
            merged = c.outcome.merged,
            index_entries = c.outcome.index_entries,
            written = c.applied.written,
            archived = c.applied.archived,
            "cc-sync curated project"
        );
    }
    eprintln!(
        "cc-sync ok — {} project(s): {up} ingested, {skip} unchanged, {arch} archived in store; \
         {written} file(s) written, {farch} file(s) archived{}",
        report.sync.projects.len(),
        if opts.dry_run { " (dry run)" } else { "" },
    );
    write_cc_sync_health(true, None);
    Ok(report.sync.last_scan_value(
        u64::try_from(pass_start.elapsed().as_millis()).unwrap_or(u64::MAX),
        wall_start,
    ))
}

/// Path to the cc-sync health marker `kyma status` reads. Fixed under
/// `$HOME/.kyma`, not `KYMA_HOME`-relative, matching the hook-side
/// `capture-health.json` convention so both processes agree on the path.
fn cc_sync_health_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".kyma").join("cc-sync-health.json"))
}

/// Record the outcome of a cc-sync pass so `kyma status` can report sync
/// freshness instead of it being invisible between session-boundary hook
/// runs (which run this detached, fire-and-forget). Best-effort: a failure
/// to write the marker never fails the sync itself.
fn write_cc_sync_health(ok: bool, detail: Option<&str>) {
    let Some(path) = cc_sync_health_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "status": if ok { "ok" } else { "error" },
        "detail": detail,
    });
    let _ = std::fs::write(&path, body.to_string());
}

/// `kyma sync` — sync memory with Claude Code's file memory (always, when
/// `~/.claude` exists) and with a control plane (when `KYMA_CLOUD_URL` is
/// set). `--watch` keeps the loop running.
pub async fn run_sync(opts: SyncOptions) -> Result<()> {
    let engine = open_engine(&resolve_paths()).await?;
    let poll = env_secs("KYMA_CC_SYNC_POLL_SECS", 30);
    loop {
        // ── Phase 1: Claude Code memory files (fails open). ──
        if !opts.cloud_only && env_flag("KYMA_CC_FILE_SYNC", true) {
            if let Err(e) = run_cc_phase(&engine, None, &opts).await {
                eprintln!("cc-sync failed (continuing): {e}");
                write_cc_sync_health(false, Some(&e.to_string()));
            }
        }
        // ── Phase 2: control plane (only when configured). ──
        if !opts.cc_only {
            match std::env::var("KYMA_CLOUD_URL") {
                Ok(cloud_url) if !cloud_url.is_empty() => {
                    let cfg = sync::SyncConfig {
                        cloud_url,
                        token: std::env::var("KYMA_CLOUD_TOKEN")
                            .ok()
                            .filter(|s| !s.is_empty()),
                        realm: std::env::var("KYMA_SYNC_REALM")
                            .ok()
                            .filter(|s| !s.is_empty()),
                        now: chrono::Utc::now().to_rfc3339(),
                    };
                    sync::run(&engine, cfg).await?;
                }
                _ if opts.cloud_only => {
                    return Err(anyhow::anyhow!(
                        "set KYMA_CLOUD_URL (and usually KYMA_CLOUD_TOKEN) to sync to a control plane"
                    ));
                }
                _ => {}
            }
        }
        if !opts.watch {
            return Ok(());
        }
        tokio::select! {
            () = tokio::time::sleep(poll) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
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
    eprintln!(
        "  sync    : kyma sync          (Claude Code file memory + KYMA_CLOUD_URL push/pull)"
    );
    eprintln!("            --watch --dry-run --cc-only --cloud-only --project <path>");
    eprintln!(
        "  worker  : kyma worker install|uninstall|status (background sync as an OS user service)"
    );
}
