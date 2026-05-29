//! Kyma CLI — admin + client mode.
//!
//! Client subcommands (talk to a running kyma server):
//!   connect  <url> [--token TOKEN]   save server URL + bearer token
//!   status                            show config + probe /health
//!   query    "<question>" [--json]    stream /v1/agent/ask to stdout
//!   install-skill [--target DIR]      write SKILL.md for coding agents
//!
//! Admin subcommands (talk directly to Postgres):
//!   create-database <name>
//!   create-table    --db <name> --name <name> --schema <spec>
//!   list-tables     --db <name>
//!   alter-table     --db <name> --table <name> --add-column <spec>
//!   create-graph    --db <name> --name <name> --nodes <tbl> --edges <tbl>
//!   list-graphs     --db <name>
//!   drop-graph      --db <name> --name <name>
//!   version

mod client;
use client::{
    effective_config, install_target, load_config, probe_health, save_config, stream_agent_ask,
    write_skill_file, ClientConfig,
};

use anyhow::{anyhow, Context, Result};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use clap::{Parser, Subcommand};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, TableConfig};
use std::sync::Arc;

const SKILL_TEMPLATE: &str = include_str!("skill_template.md");

#[derive(Debug, Parser)]
#[command(name = "kyma", about = "Kyma CLI — client queries + admin operations")]
struct Cli {
    /// Postgres connection URL (admin subcommands only).
    #[arg(
        long,
        env = "KYMA_CATALOG_URL",
        default_value = "postgres://kyma:kyma_dev@localhost:5433/kyma"
    )]
    catalog_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    // ── client subcommands ────────────────────────────────────────────

    /// Save a connection to a kyma server.
    Connect {
        /// Server base URL, e.g. http://localhost:8080
        url: String,
        /// Bearer token (omit to authenticate via /v1/auth/login interactively later).
        #[arg(long)]
        token: Option<String>,
    },
    /// Show the saved connection + probe the server's /health.
    Status,
    /// Stream an agent answer to stdout.
    Query {
        /// The question, e.g. "How many 500s in the last hour?"
        question: String,
        /// Emit the raw SSE event stream as JSONL instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Install the Kyma skill so coding agents (Claude Code, Cursor, …)
    /// can discover and use this CLI.
    InstallSkill {
        /// Target directory. Default: `$HOME/.kyma/skills/kyma`.
        #[arg(long)]
        target: Option<std::path::PathBuf>,
        /// Also symlink into `$HOME/.claude/skills/kyma` if that dir exists.
        #[arg(long)]
        also_link_claude: bool,
    },

    // ── admin subcommands ─────────────────────────────────────────────

    /// Create a new database (namespace) — admin, talks to Postgres directly.
    CreateDatabase { name: String },
    /// Create a new table — admin.
    CreateTable {
        #[arg(long)]
        db: String,
        #[arg(long)]
        name: String,
        /// Schema spec: "col:type,col:type,...". Types: int, long, real, bool, string, timestamp, dynamic.
        #[arg(long)]
        schema: String,
        /// Optional retention in days.
        #[arg(long)]
        retention_days: Option<u32>,
    },
    /// List tables in a database — admin.
    ListTables {
        #[arg(long)]
        db: String,
    },
    /// Add a nullable column to an existing table — admin.
    AlterTable {
        #[arg(long)]
        db: String,
        #[arg(long)]
        table: String,
        /// Spec: `name:type`. Types: bool, int, long, real, string, timestamp, dynamic.
        #[arg(long)]
        add_column: String,
    },
    /// Print the CLI version.
    Version,
    /// Register a property-graph — admin.
    CreateGraph {
        #[arg(long)]
        db: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        nodes: String,
        #[arg(long)]
        edges: String,
        #[arg(long, default_value = "id")]
        id_col: String,
        #[arg(long, default_value = "labels")]
        label_col: String,
        #[arg(long, default_value = "src")]
        src_col: String,
        #[arg(long, default_value = "dst")]
        dst_col: String,
        #[arg(long, default_value = "type")]
        type_col: String,
        #[arg(long)]
        realm_col: Option<String>,
    },
    /// List registered graphs in a database — admin.
    ListGraphs {
        #[arg(long)]
        db: String,
    },
    /// Drop a graph registration — admin.
    DropGraph {
        #[arg(long)]
        db: String,
        #[arg(long)]
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::try_init().ok();

    let cli = Cli::parse();

    match cli.command {
        // ── client subcommands ────────────────────────────────────────
        Command::Connect { url, token } => cmd_connect(url, token).await,
        Command::Status => cmd_status().await,
        Command::Query { question, json } => cmd_query(question, json).await,
        Command::InstallSkill {
            target,
            also_link_claude,
        } => cmd_install_skill(target, also_link_claude).await,

        // ── admin subcommands ─────────────────────────────────────────
        Command::Version => {
            println!("kyma {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::CreateDatabase { name } => {
            let catalog = connect_catalog(&cli.catalog_url).await?;
            let id = catalog
                .create_database(&name)
                .await
                .with_context(|| format!("creating database {name}"))?;
            println!("created database {name} ({id})");
            Ok(())
        }
        Command::CreateTable {
            db,
            name,
            schema,
            retention_days,
        } => {
            let catalog = connect_catalog(&cli.catalog_url).await?;
            let db_id = find_database_id(&catalog, &db).await?;
            let parsed_schema = parse_schema_spec(&schema)
                .with_context(|| format!("parsing schema spec: {schema}"))?;
            let config = TableConfig {
                retention_days,
                ..Default::default()
            };
            let id = catalog
                .create_table(db_id, &name, Arc::new(parsed_schema), config)
                .await
                .with_context(|| format!("creating table {db}.{name}"))?;
            println!("created table {db}.{name} ({id})");
            Ok(())
        }
        Command::AlterTable {
            db,
            table,
            add_column,
        } => {
            let catalog = connect_catalog(&cli.catalog_url).await?;
            let t = catalog.lookup_table(&db, &table).await?;
            let (name, ty) = add_column
                .split_once(':')
                .ok_or_else(|| anyhow!("--add-column must be name:type; got '{add_column}'"))?;
            let new_schema = catalog
                .alter_table_add_column(t.id, name.trim(), ty.trim())
                .await?;
            println!(
                "altered {db}.{table}: added column {name}:{ty} (schema_snapshot={new_schema})"
            );
            Ok(())
        }
        Command::ListTables { db } => {
            let catalog = connect_catalog(&cli.catalog_url).await?;
            let tables = catalog.list_tables_in_database(&db).await?;
            if tables.is_empty() {
                println!("(no tables in database {db})");
            } else {
                for t in tables {
                    let cols: Vec<String> = t
                        .schema
                        .fields()
                        .iter()
                        .map(|f| format!("{}:{:?}", f.name(), f.data_type()))
                        .collect();
                    println!("{}  [{}]", t.name, cols.join(", "));
                }
            }
            Ok(())
        }
        Command::CreateGraph {
            db,
            name,
            nodes,
            edges,
            id_col,
            label_col,
            src_col,
            dst_col,
            type_col,
            realm_col,
        } => {
            if name == "schema" {
                anyhow::bail!(
                    "'schema' is reserved for the synthetic schema-graph; choose another name"
                );
            }
            let cat = connect_catalog(&cli.catalog_url).await?;
            let spec = kyma_core::catalog::GraphSpec {
                node_table: nodes,
                edge_table: edges,
                id_col,
                label_col,
                src_col,
                dst_col,
                type_col,
                realm_col,
            };
            let reg = cat.create_graph(&db, &name, spec).await?;
            println!(
                "registered graph '{}' in db '{}' (nodes={}, edges={})",
                reg.name, db, reg.node_table, reg.edge_table
            );
            Ok(())
        }
        Command::ListGraphs { db } => {
            let cat = connect_catalog(&cli.catalog_url).await?;
            let graphs = cat.list_graphs(&db).await?;
            if graphs.is_empty() {
                println!("(no graphs registered in '{db}')");
            } else {
                for g in graphs {
                    println!(
                        "{}\tnodes={}\tedges={}\trealm={}",
                        g.name,
                        g.node_table,
                        g.edge_table,
                        g.realm_col.as_deref().unwrap_or("-")
                    );
                }
            }
            Ok(())
        }
        Command::DropGraph { db, name } => {
            let cat = connect_catalog(&cli.catalog_url).await?;
            if cat.drop_graph(&db, &name).await? {
                println!("dropped graph '{name}' from '{db}'");
            } else {
                println!("no graph '{name}' in '{db}'");
            }
            Ok(())
        }
    }
}

// ── client subcommand implementations ────────────────────────────────────────

async fn cmd_connect(url: String, token: Option<String>) -> Result<()> {
    let url = url.trim_end_matches('/').to_string();
    let cfg = ClientConfig {
        endpoint: url.clone(),
        token,
    };
    save_config(&cfg)?;
    println!("Saved connection to {url}");
    let path = client::config_path()?;
    println!("Config: {}", path.display());
    Ok(())
}

async fn cmd_status() -> Result<()> {
    match load_config() {
        Ok(cfg) => {
            println!("Endpoint:  {}", cfg.endpoint);
            println!(
                "Token:     {}",
                if cfg.token.is_some() {
                    "configured"
                } else {
                    "not set"
                }
            );
            match probe_health(&cfg).await {
                Ok(body) => println!("Health:    {}", body.trim()),
                Err(e) => println!("Health:    error — {e}"),
            }
        }
        Err(_) => {
            println!("No config found. Run `kyma connect <url>` first.");
        }
    }
    Ok(())
}

async fn cmd_query(question: String, json: bool) -> Result<()> {
    let cfg = effective_config()?;
    let mut had_error = false;
    stream_agent_ask(&cfg, &question, |event, data| {
        if json {
            println!(
                "{}",
                serde_json::json!({ "event": event, "data": data })
            );
            return;
        }
        match event {
            "answer_delta" | "answer_final" => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
                        print!("{}", t);
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    }
                }
            }
            "run_error" => {
                had_error = true;
                eprintln!("\n[error] {}", data);
            }
            "run_finished" => {
                println!();
            }
            _ => {}
        }
    })
    .await?;
    if had_error {
        std::process::exit(1);
    }
    Ok(())
}

async fn cmd_install_skill(
    target: Option<std::path::PathBuf>,
    also_link_claude: bool,
) -> Result<()> {
    let dir = install_target(target)?;
    let path = write_skill_file(&dir, SKILL_TEMPLATE)?;
    println!("Wrote {}", path.display());

    if also_link_claude {
        #[cfg(unix)]
        {
            if let Some(home) = std::env::var_os("HOME") {
                let claude_skills = std::path::PathBuf::from(home)
                    .join(".claude")
                    .join("skills");
                if claude_skills.is_dir() {
                    let link = claude_skills.join("kyma");
                    let _ = std::fs::remove_file(&link);
                    let _ = std::fs::remove_dir_all(&link);
                    std::os::unix::fs::symlink(&dir, &link).with_context(|| {
                        format!("symlink {} -> {}", link.display(), dir.display())
                    })?;
                    println!("Linked {} -> {}", link.display(), dir.display());
                } else {
                    println!(
                        "(skipped) {} doesn't exist; pass --also-link-claude later",
                        claude_skills.display()
                    );
                }
            }
        }
        #[cfg(not(unix))]
        {
            eprintln!(
                "note: --also-link-claude only works on Unix; copy the file manually on Windows"
            );
        }
    }
    Ok(())
}

// ── admin helpers ─────────────────────────────────────────────────────────────

async fn connect_catalog(url: &str) -> Result<Arc<dyn Catalog>> {
    let c = PostgresCatalog::connect(url)
        .await
        .with_context(|| format!("connecting to catalog {url}"))?;
    Ok(Arc::new(c))
}

async fn find_database_id(
    catalog: &Arc<dyn Catalog>,
    name: &str,
) -> Result<kyma_core::types::DatabaseId> {
    // We don't have a `lookup_database` method yet. Workaround: try to
    // create-then-read by creating; if duplicate, look it up via a direct
    // pg query. For phase A we just try to create the database and if it
    // already exists, we query an arbitrary existing table to resolve the
    // database id — which won't work if there are no tables yet.
    //
    // Clean fix: add a `lookup_database` to the trait. For now, cheat via
    // the pool on PostgresCatalog. We downcast the Arc via Any trick.
    //
    // Simpler still: the CLI always runs against Postgres in phase A, so we
    // just run a direct query here. Accept the layering violation as a
    // phase-A expedient; proper fix = add `lookup_database` to the Catalog
    // trait (tracked as follow-up).
    let _ = catalog;
    let pool = sqlx::PgPool::connect(
        std::env::var("KYMA_CATALOG_URL")
            .ok()
            .as_deref()
            .unwrap_or("postgres://kyma:kyma_dev@localhost:5433/kyma"),
    )
    .await?;
    let row: Option<(uuid::Uuid,)> = sqlx::query_as("SELECT id FROM databases WHERE name = $1")
        .bind(name)
        .fetch_optional(&pool)
        .await?;
    let id = row
        .ok_or_else(|| anyhow!("database '{}' not found — create it first", name))?
        .0;
    Ok(kyma_core::types::DatabaseId::from_uuid(id))
}

fn parse_schema_spec(spec: &str) -> Result<Schema> {
    let mut fields = Vec::new();
    for col in spec.split(',') {
        let col = col.trim();
        if col.is_empty() {
            continue;
        }
        let (name, ty) = col
            .split_once(':')
            .ok_or_else(|| anyhow!("column spec missing ':' — got '{col}'"))?;
        let name = name.trim();
        let ty = ty.trim();
        if name.is_empty() {
            return Err(anyhow!("empty column name in '{col}'"));
        }
        // Vector columns are non-nullable because null-vector ingest isn't
        // supported yet (the coercion path rejects serde_json::Value::Null).
        // All other columns default to nullable=true to match existing seed
        // scripts and the catalog's historical behaviour.
        let (data_type, nullable) = match ty {
            "bool" => (DataType::Boolean, true),
            "int" => (DataType::Int32, true),
            "long" => (DataType::Int64, true),
            "real" => (DataType::Float64, true),
            "string" => (DataType::Utf8, true),
            "timestamp" => (DataType::Timestamp(TimeUnit::Nanosecond, None), true),
            "dynamic" => (DataType::Binary, true),
            other if other.starts_with("vector(") && other.ends_with(')') => {
                let inner = &other[7..other.len() - 1];
                let dim: i32 = inner.trim().parse().map_err(|_| {
                    anyhow!("vector(N): N must be a positive integer, got '{inner}'")
                })?;
                if dim <= 0 {
                    return Err(anyhow!("vector(N): N must be > 0, got {dim}"));
                }
                (
                    DataType::FixedSizeList(
                        Arc::new(Field::new("item", DataType::Float32, false)),
                        dim,
                    ),
                    false,
                )
            }
            other => return Err(anyhow!("unsupported column type: {other}")),
        };
        fields.push(Field::new(name, data_type, nullable));
    }
    if fields.is_empty() {
        return Err(anyhow!("schema spec produced no fields"));
    }
    Ok(Schema::new(fields))
}
