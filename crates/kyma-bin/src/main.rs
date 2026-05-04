//! The main kyma binary.
//!
//! Phase A wiring: loads config from env, connects to Postgres + MinIO,
//! registers this node in the catalog, starts the HTTP server with ingest +
//! query routes mounted.
//!
//! Future slices add: gRPC server, background compaction worker, TTL GC,
//! Kafka consumers, file-drop watchers, OTLP receivers, WAL + staging
//! buffers, proper config file support, graceful shutdown.

use anyhow::{Context, Result};
use clap::Parser;
use kyma_catalog::PostgresCatalog;
use kyma_compaction::{
    CompactionScheduler, CompactionWorker, PhysicalDeleteWorker, RetentionSweeper,
};
use kyma_connectors::prometheus::PromConnector;
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::runner::ConnectorRunner;
use kyma_connectors::scheduler::ConnectorScheduler;
use kyma_connectors::secrets::EnvSecretStore;
use kyma_core::catalog::{Catalog, NodeInfo, NodeRole};
use kyma_core::segment_format::SegmentFormat;
use kyma_format_tlm::TelemetryFormat;
use kyma_ingest_core::{
    spawn_idempotency_cleanup, CommitCoordinator, CoordinatorConfig, StagingBuffer, StagingConfig,
    WritePath,
};
use kyma_ingest_filedrop::{FiledropConfig, FiledropWatcher};
use kyma_ingest_kafka::{KafkaConsumerConfig, KafkaConsumerWorker};
use kyma_ingest_otlp::OtlpLogsService;
use kyma_ingest_rest::IngestState;
use kyma_server::auth::{
    require_role_middleware, AuthBackend, AuthLayerState, EnvAuthBackend, Role,
};
use kyma_server::{ConnectorAdminState, QueryState};
use kyma_storage::{build_object_store, config_from_env};
use opentelemetry_proto::tonic::collector::logs::v1::logs_service_server::LogsServiceServer as OtlpLogsServer;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

#[derive(Debug, Parser)]
#[command(
    name = "kyma",
    about = "kyma engine — unified data platform (pre-alpha)"
)]
struct Cli {
    /// Postgres catalog URL. Falls back to KYMA_CATALOG_URL env var.
    #[arg(
        long,
        env = "KYMA_CATALOG_URL",
        default_value = "postgres://kyma:kyma_dev@localhost:5433/kyma"
    )]
    catalog_url: String,

    /// HTTP listen address.
    #[arg(long, env = "KYMA_HTTP_ADDR", default_value = "0.0.0.0:8080")]
    http_addr: SocketAddr,

    /// gRPC (Arrow Flight) listen address. Set to "off" to disable.
    #[arg(long, env = "KYMA_GRPC_ADDR", default_value = "0.0.0.0:9090")]
    grpc_addr: String,

    /// OTLP gRPC listen address (standard port 4317). Set to "off" to disable.
    #[arg(long, env = "KYMA_OTLP_ADDR", default_value = "off")]
    otlp_addr: String,

    /// Target database for OTLP-received logs.
    #[arg(long, env = "KYMA_OTLP_DATABASE", default_value = "default")]
    otlp_database: String,

    /// Object-store path prefix.
    #[arg(long, env = "KYMA_PATH_PREFIX", default_value = "kyma")]
    path_prefix: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn,h2=warn")
            }),
        )
        .with_target(true)
        .init();

    // Install the Prometheus recorder. Must happen before any metrics macro
    // fires — so, very first thing in main.
    let _metrics_handle = kyma_server::metrics::install();

    let cli = Cli::parse();
    info!(catalog_url = %cli.catalog_url, http_addr = %cli.http_addr, "kyma starting");

    // 1. Catalog.
    let pg_catalog = Arc::new(
        PostgresCatalog::connect(&cli.catalog_url)
            .await
            .with_context(|| format!("connecting to catalog at {}", cli.catalog_url))?,
    );
    // Hang on to the raw pg pool — the agent surface persists `agent_runs`
    // rows via it — before we erase the concrete type behind `dyn Catalog`.
    let pg_pool = pg_catalog.pool().clone();
    let catalog: Arc<dyn Catalog> = pg_catalog.clone();
    info!("catalog connected; migrations applied");

    // 2. Object store + format.
    let storage_config = config_from_env();
    let store = build_object_store(&storage_config).context("building object store")?;
    info!("object store ready");
    let format: Arc<dyn SegmentFormat> =
        Arc::new(TelemetryFormat::new(store.clone(), cli.path_prefix.clone()));

    // 3. Register this node in the catalog.
    let lease = catalog
        .register_node(NodeInfo {
            role: NodeRole::AllInOne,
            endpoint: cli.http_addr.to_string(),
            capabilities: serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "features": ["http", "sql", "ingest"],
            }),
        })
        .await
        .context("registering node in catalog")?;
    info!(node_id = %lease.node_id, "node registered");

    // 4. Set up the shutdown broadcast channel now so background tasks
    //    (ingest staging, compaction, retention, gc) can subscribe.
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(8);

    //    Build the HTTP router by merging ingest + query + health + metrics.
    //    Health + metrics stay unauthenticated; ingest + query get the auth
    //    middleware (bypassed at runtime when KYMA_AUTH_TOKENS is empty).
    let backend: Arc<dyn AuthBackend> = match std::env::var("KYMA_AUTH_BACKEND")
        .ok()
        .as_deref()
    {
        #[cfg(feature = "cloud-auth")]
        Some("db") => {
            use kyma_server::auth::DbAuthBackend;
            info!("auth: using db backend (api_tokens table)");
            Arc::new(DbAuthBackend::new(pg_pool.clone()))
        }
        #[cfg(not(feature = "cloud-auth"))]
        Some("db") => {
            warn!(
                "KYMA_AUTH_BACKEND=db requested but the binary was compiled without \
                 the `cloud-auth` feature; falling back to EnvAuthBackend."
            );
            Arc::new(EnvAuthBackend::from_env())
        }
        Some(other) if !other.is_empty() => {
            warn!(
                "KYMA_AUTH_BACKEND={other:?} unrecognized; using EnvAuthBackend."
            );
            Arc::new(EnvAuthBackend::from_env())
        }
        _ => Arc::new(EnvAuthBackend::from_env()),
    };
    if backend.enabled() {
        info!("auth: bearer-token protection enabled on /v1/ingest (write) + /v1/query (read)");
    } else {
        info!("auth: disabled (set KYMA_AUTH_TOKENS to enable)");
    }

    // Staging buffer (group-commit) drives ingest throughput. Can be
    // disabled by setting KYMA_STAGING_DISABLED=1 for tests that want
    // one-extent-per-request semantics.
    let use_staging = std::env::var("KYMA_STAGING_DISABLED")
        .map(|v| v != "1" && v != "true")
        .unwrap_or(true);
    let write_path: WritePath = if use_staging {
        // Start the commit coordinator so flushes get group-commit squared.
        let coordinator = CommitCoordinator::spawn(catalog.clone(), CoordinatorConfig::from_env());
        let staging =
            StagingBuffer::new(catalog.clone(), format.clone(), StagingConfig::from_env())
                .with_coordinator(coordinator);
        // Background timer flushes age-expired buffers.
        let staging_rx = shutdown_tx.subscribe();
        let staging_for_timer = staging.clone();
        tokio::spawn(staging_for_timer.run_timer(async move {
            let mut rx = staging_rx;
            let _ = rx.recv().await;
        }));
        info!("ingest staging: group-commit enabled");
        WritePath::with_staging(catalog.clone(), format.clone(), staging)
    } else {
        info!("ingest staging: disabled (KYMA_STAGING_DISABLED=1)");
        WritePath::new(catalog.clone(), format.clone())
    };
    let ingest_router = kyma_ingest_rest::router(IngestState {
        catalog: catalog.clone(),
        write_path: write_path.clone(),
    })
    .layer(axum::middleware::from_fn_with_state(
        AuthLayerState {
            backend: backend.clone(),
            required: Role::Write,
        },
        require_role_middleware,
    ));
    let agent_state = kyma_server::agent::AgentState {
        catalog: catalog.clone(),
        format: format.clone(),
        pool: pg_pool.clone(),
    };
    let query_router =
        kyma_server::router_with_agent(
            QueryState {
                catalog: catalog.clone(),
                format: format.clone(),
                schema_cache: std::sync::Arc::new(
                    kyma_server::catalog_handler::SchemaCache::from_env(),
                ),
                node_id: Some(lease.node_id),
            },
            agent_state,
        )
        .layer(axum::middleware::from_fn_with_state(
            AuthLayerState {
                backend: backend.clone(),
                required: Role::Read,
            },
            require_role_middleware,
        ));

    // Build MCP state from the same SharedToolCtx the inline /v1/agent endpoint uses.
    let mcp_shared = kyma_server::agent::SharedToolCtx {
        catalog: catalog.clone(),
        format: format.clone(),
        pool: pg_pool.clone(),
    };
    let mcp_state = kyma_mcp::McpState {
        dispatch: kyma_mcp::ToolDispatch::new(mcp_shared),
        server_info: kyma_mcp::ServerInfo {
            name: "kyma".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    let mcp_router = kyma_mcp::router(mcp_state).layer(
        axum::middleware::from_fn_with_state(
            AuthLayerState { backend: backend.clone(), required: Role::Read },
            require_role_middleware,
        ),
    );
    // Connector registry + row-sink.
    let mut conn_reg = ConnectorRegistry::new();
    conn_reg.register(Arc::new(PromConnector));
    let conn_registry = Arc::new(conn_reg);

    // RowSink: bridges connector JSON rows → arrow coercion → WritePath.
    let conn_sink: kyma_connectors::runner::RowSink = {
        let catalog_for_sink = catalog.clone();
        let write_path_for_sink = write_path.clone();
        Arc::new(
            move |db: String, tbl: String, rows: Vec<serde_json::Value>, idem: Option<String>| {
                let catalog = catalog_for_sink.clone();
                let write_path = write_path_for_sink.clone();
                Box::pin(async move {
                    let table = catalog
                        .lookup_table(&db, &tbl)
                        .await
                        .map_err(|e| anyhow::anyhow!("lookup_table: {e}"))?;
                    let batches =
                        kyma_connectors::arrow_coerce::rows_to_batches(&table.schema, rows)
                            .map_err(|e| anyhow::anyhow!("arrow coerce: {e}"))?;
                    write_path
                        .ingest_with_idempotency(&table, batches, idem.as_deref())
                        .await
                        .map_err(|e| anyhow::anyhow!("ingest: {e}"))?;
                    Ok(())
                })
            },
        )
    };

    let health_router = kyma_server::health_router();
    let metrics_router = kyma_server::metrics::router();
    let connector_admin_router = kyma_server::connector_admin_router(ConnectorAdminState {
        catalog: pg_catalog.clone(),
        registry: conn_registry.clone(),
    })
    .layer(axum::middleware::from_fn_with_state(
        AuthLayerState {
            backend: backend.clone(),
            required: Role::Write,
        },
        require_role_middleware,
    ));
    let dashboards_write_router =
        kyma_server::dashboards_write_router(catalog.clone()).layer(
            axum::middleware::from_fn_with_state(
                AuthLayerState {
                    backend: backend.clone(),
                    required: Role::Write,
                },
                require_role_middleware,
            ),
        );
    let cleanup_write_router = kyma_server::cleanup_write_router(catalog.clone()).layer(
        axum::middleware::from_fn_with_state(
            AuthLayerState {
                backend: backend.clone(),
                required: Role::Write,
            },
            require_role_middleware,
        ),
    );
    let app = ingest_router
        .merge(query_router)
        .merge(mcp_router)
        .merge(dashboards_write_router)
        .merge(cleanup_write_router)
        .merge(health_router)
        .merge(metrics_router)
        .merge(connector_admin_router);
    #[cfg(feature = "web-ui")]
    let app = app.merge(kyma_server::web_ui::router());

    // Expose Flight over gRPC-web at /flight/* so browsers can query via Arrow Flight.
    // Auth is enforced the same way as /v1/* (Bearer token, Role::Read required).
    #[cfg(feature = "web-ui")]
    let app = {
        let flight_router =
            kyma_server::flight_web_router(kyma_server::QueryState {
                catalog: catalog.clone(),
                format: format.clone(),
                schema_cache: std::sync::Arc::new(
                    kyma_server::catalog_handler::SchemaCache::from_env(),
                ),
                node_id: Some(lease.node_id),
            })
            .layer(axum::middleware::from_fn_with_state(
                AuthLayerState {
                    backend: backend.clone(),
                    required: kyma_server::auth::Role::Read,
                },
                kyma_server::auth::require_role_middleware,
            ));
        app.merge(flight_router)
    };

    // 5. Spawn background workers. Each has an independent shutdown watch so
    //    a panic in one worker doesn't starve the others or the HTTP server.
    let mut worker = CompactionWorker::new(catalog.clone(), format.clone(), lease.node_id);
    let mut scheduler = CompactionScheduler::new(catalog.clone());
    // Allow env overrides for aggressive testing.
    if let Ok(ms) = std::env::var("KYMA_COMPACTION_IDLE_SLEEP_MS")
        .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
    {
        worker.idle_sleep = std::time::Duration::from_millis(ms);
    }
    if let Ok(s) = std::env::var("KYMA_COMPACTION_POLL_SECS")
        .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
    {
        scheduler.poll_interval = std::time::Duration::from_secs(s);
    }
    if let Ok(n) = std::env::var("KYMA_COMPACTION_MIN_EXTENTS")
        .and_then(|v| v.parse::<i64>().map_err(|_| std::env::VarError::NotPresent))
    {
        scheduler.min_extents_to_compact = n;
    }
    let worker_rx = shutdown_tx.subscribe();
    let scheduler_rx = shutdown_tx.subscribe();
    let worker_handle = tokio::spawn(worker.run(async move {
        let mut rx = worker_rx;
        let _ = rx.recv().await;
    }));
    let scheduler_handle = tokio::spawn(scheduler.run(async move {
        let mut rx = scheduler_rx;
        let _ = rx.recv().await;
    }));

    // Retention sweeper (soft-delete expired).
    let mut retention = RetentionSweeper::new(catalog.clone());
    if let Ok(s) = std::env::var("KYMA_RETENTION_POLL_SECS")
        .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
    {
        retention.poll_interval = std::time::Duration::from_secs(s);
    }
    let retention_rx = shutdown_tx.subscribe();
    let retention_handle = tokio::spawn(retention.run(async move {
        let mut rx = retention_rx;
        let _ = rx.recv().await;
    }));

    // Physical-delete worker (remove bytes after grace).
    let mut gc = PhysicalDeleteWorker::new(catalog.clone(), store.clone());
    if let Ok(s) = std::env::var("KYMA_PHYSICAL_GC_POLL_SECS")
        .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
    {
        gc.poll_interval = std::time::Duration::from_secs(s);
    }
    if let Ok(s) = std::env::var("KYMA_PHYSICAL_GC_GRACE_SECS")
        .and_then(|v| v.parse::<i64>().map_err(|_| std::env::VarError::NotPresent))
    {
        gc.grace_period = chrono::Duration::seconds(s);
    }
    let gc_rx = shutdown_tx.subscribe();
    let gc_handle = tokio::spawn(gc.run(async move {
        let mut rx = gc_rx;
        let _ = rx.recv().await;
    }));

    // File-drop watcher — polls an object-store prefix for NDJSON files.
    // Disabled by default; set KYMA_FILEDROP_ENABLED=1 to turn on.
    let filedrop_handle = if std::env::var("KYMA_FILEDROP_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        let watcher = FiledropWatcher::new(
            catalog.clone(),
            store.clone(),
            write_path.clone(),
            FiledropConfig::from_env(),
        );
        let filedrop_rx = shutdown_tx.subscribe();
        info!("file-drop watcher enabled");
        Some(tokio::spawn(watcher.run(async move {
            let mut rx = filedrop_rx;
            let _ = rx.recv().await;
        })))
    } else {
        None
    };

    // Kafka consumer — on when KYMA_KAFKA_ENABLED=1 and KYMA_KAFKA_TOPICS is non-empty.
    let kafka_handle = match KafkaConsumerConfig::from_env() {
        Some(config) => {
            let worker = KafkaConsumerWorker::new(catalog.clone(), write_path.clone(), config);
            let kafka_rx = shutdown_tx.subscribe();
            info!("kafka consumer enabled");
            Some(tokio::spawn(worker.run(async move {
                let mut rx = kafka_rx;
                let _ = rx.recv().await;
            })))
        }
        None => None,
    };

    // Connector scheduler.
    let conn_sched = ConnectorScheduler::new(pg_catalog.clone());
    let conn_sched_rx = shutdown_tx.subscribe();
    let conn_sched_handle = tokio::spawn(conn_sched.run(async move {
        let mut rx = conn_sched_rx;
        let _ = rx.recv().await;
    }));

    // Connector runners (N workers, default 4).
    let n_conn_workers = std::env::var("KYMA_CONNECTOR_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4);
    let mut conn_runner_handles = Vec::with_capacity(n_conn_workers);
    for _ in 0..n_conn_workers {
        let runner = ConnectorRunner::new(
            pg_catalog.clone(),
            conn_registry.clone(),
            conn_sink.clone(),
            EnvSecretStore,
            lease.node_id,
        );
        let runner_rx = shutdown_tx.subscribe();
        conn_runner_handles.push(tokio::spawn(async move {
            let mut rx = runner_rx;
            runner
                .run(async move {
                    let _ = rx.recv().await;
                })
                .await;
        }));
    }
    info!(
        workers = n_conn_workers,
        "connector scheduler + runners started"
    );

    // Idempotency ledger cleanup — runs every hour, deletes entries older
    // than 25 hours (1-hour grace beyond the 24-hour TTL).
    let idem_rx = shutdown_tx.subscribe();
    let idem_cleanup_handle = spawn_idempotency_cleanup(catalog.clone(), async move {
        let mut rx = idem_rx;
        let _ = rx.recv().await;
    });

    info!("background workers started (compaction, retention, physical-gc, idempotency-cleanup)");

    // 6. Arrow Flight gRPC server (optional — set KYMA_GRPC_ADDR=off).
    let grpc_handle = if cli.grpc_addr.eq_ignore_ascii_case("off") {
        info!("grpc: disabled (KYMA_GRPC_ADDR=off)");
        None
    } else {
        let grpc_addr: SocketAddr = cli
            .grpc_addr
            .parse()
            .with_context(|| format!("parsing grpc_addr {}", cli.grpc_addr))?;
        let flight_svc = kyma_server::flight::flight_server(kyma_server::flight::FlightState {
            catalog: catalog.clone(),
            format: format.clone(),
            node_id: Some(lease.node_id),
        });
        let mut grpc_rx = shutdown_tx.subscribe();
        info!(addr = %grpc_addr, "grpc/flight server listening");
        Some(tokio::spawn(async move {
            let res = tonic::transport::Server::builder()
                .add_service(flight_svc)
                .serve_with_shutdown(grpc_addr, async move {
                    let _ = grpc_rx.recv().await;
                })
                .await;
            if let Err(e) = res {
                error!(error = %e, "grpc server terminated");
            }
        }))
    };

    // 6.5 OTLP gRPC server (port 4317 conventionally).
    let otlp_handle = if cli.otlp_addr.eq_ignore_ascii_case("off") {
        info!("otlp: disabled (KYMA_OTLP_ADDR=off)");
        None
    } else {
        let otlp_addr: SocketAddr = cli
            .otlp_addr
            .parse()
            .with_context(|| format!("parsing otlp_addr {}", cli.otlp_addr))?;
        let otlp_svc = OtlpLogsServer::new(OtlpLogsService::new(
            catalog.clone(),
            write_path.clone(),
            cli.otlp_database.clone(),
        ));
        let mut otlp_rx = shutdown_tx.subscribe();
        info!(addr = %otlp_addr, database = %cli.otlp_database, "otlp gRPC server listening");
        Some(tokio::spawn(async move {
            let res = tonic::transport::Server::builder()
                .add_service(otlp_svc)
                .serve_with_shutdown(otlp_addr, async move {
                    let _ = otlp_rx.recv().await;
                })
                .await;
            if let Err(e) = res {
                error!(error = %e, "otlp server terminated");
            }
        }))
    };

    // 7. HTTP server.
    //    Apply dev CORS to the outermost router so browsers on a separate
    //    origin (e.g. `localhost:5173`) can reach the API. Production
    //    deploys should replace this with a config-driven allow-list.
    let app = kyma_server::with_permissive_cors(app);
    let listener = tokio::net::TcpListener::bind(cli.http_addr)
        .await
        .with_context(|| format!("binding {}", cli.http_addr))?;
    info!(addr = %cli.http_addr, "http server listening");
    let shutdown = shutdown_signal();
    let serve = axum::serve(listener, app).with_graceful_shutdown(shutdown);
    if let Err(e) = serve.await {
        error!(error = %e, "http server terminated with error");
    }

    // Explicitly drain the staging buffer now that HTTP is no longer accepting
    // new ingest requests. This ensures partially-flushed batches are committed
    // before we wind down background workers. The staging timer also drains on
    // shutdown_tx, but that fires later — this is the earlier, safer hook.
    info!("draining ingest staging buffer");
    write_path.drain_staging().await;
    info!("staging buffer drained");

    if let Some(h) = grpc_handle {
        let _ = h.await;
    }
    if let Some(h) = otlp_handle {
        let _ = h.await;
    }
    if let Some(h) = filedrop_handle {
        let _ = h.await;
    }
    if let Some(h) = kafka_handle {
        let _ = h.await;
    }

    // Tell workers to wind down.
    let _ = shutdown_tx.send(());
    let _ = worker_handle.await;
    let _ = scheduler_handle.await;
    let _ = retention_handle.await;
    let _ = gc_handle.await;
    let _ = conn_sched_handle.await;
    let _ = idem_cleanup_handle.await;
    for h in conn_runner_handles {
        let _ = h.await;
    }

    // 6. Best-effort cleanup — deregister the node.
    if let Err(e) = catalog.deregister_node(lease).await {
        error!(error = %e, "failed to deregister node on shutdown");
    } else {
        info!("node deregistered");
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("ctrl-c received; shutting down"),
        _ = terminate => info!("SIGTERM received; shutting down"),
    }
}
