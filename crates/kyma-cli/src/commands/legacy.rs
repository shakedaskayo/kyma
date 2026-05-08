//! Legacy direct-Postgres admin commands (preserved for self-hosted users
//! and for the `kyma-bootstrap.sh` startup script that ships in the runtime
//! Docker image).
//!
//! These commands talk straight to Postgres via `kyma-catalog::PostgresCatalog`
//! — they do NOT go through the cloud control-plane HTTPS API. They are kept
//! here so the bootstrap flow (`kyma-cli db create-database`, `db create-table`,
//! `db list-tables`) keeps working unchanged after Slice 2.

use anyhow::{anyhow, Context, Result};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use clap::Subcommand;
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, TableConfig};
use std::sync::Arc;

#[derive(Debug, Subcommand)]
pub enum DbCmd {
    /// Create a new database (namespace).
    CreateDatabase { name: String },
    /// Create a new table.
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
    /// List tables in a database.
    ListTables {
        #[arg(long)]
        db: String,
    },
    /// Add a nullable column to an existing table.
    AlterTable {
        #[arg(long)]
        db: String,
        #[arg(long)]
        table: String,
        /// Spec: `name:type`. Types: bool, int, long, real, string, timestamp, dynamic.
        #[arg(long)]
        add_column: String,
    },
}

pub async fn run(cmd: DbCmd, catalog_url: &str) -> Result<()> {
    match cmd {
        DbCmd::CreateDatabase { name } => {
            let catalog = connect(catalog_url).await?;
            let id = catalog
                .create_database(&name)
                .await
                .with_context(|| format!("creating database {name}"))?;
            println!("created database {name} ({id})");
        }
        DbCmd::CreateTable {
            db,
            name,
            schema,
            retention_days,
        } => {
            let catalog = connect(catalog_url).await?;
            let db_id = find_database_id(&catalog, catalog_url, &db).await?;
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
        }
        DbCmd::AlterTable {
            db,
            table,
            add_column,
        } => {
            let catalog = connect(catalog_url).await?;
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
        }
        DbCmd::ListTables { db } => {
            let catalog = connect(catalog_url).await?;
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
        }
    }
    Ok(())
}

async fn connect(url: &str) -> Result<Arc<dyn Catalog>> {
    let c = PostgresCatalog::connect(url)
        .await
        .with_context(|| format!("connecting to catalog {url}"))?;
    Ok(Arc::new(c))
}

async fn find_database_id(
    catalog: &Arc<dyn Catalog>,
    catalog_url: &str,
    name: &str,
) -> Result<kyma_core::types::DatabaseId> {
    // Phase-A expedient: query Postgres directly to resolve the database id.
    // Proper fix = add `lookup_database` to the Catalog trait (tracked as
    // follow-up).
    let _ = catalog;
    let pool = sqlx::PgPool::connect(catalog_url).await?;
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
