//! Microsoft Fabric SQL analytics endpoint — the first federated platform.
//!
//! Every Fabric Lakehouse / Warehouse / SQL DB exposes a **SQL analytics
//! endpoint**: a TDS (SQL Server wire protocol) host of the form
//! `<id>.<region>.fabric.microsoft.com`, queryable with T-SQL. Auth is an
//! Entra ID service principal (client-credentials flow) whose token is scoped
//! to `https://database.windows.net/`. Fabric executes the pushed-down SQL on
//! its own compute; pensieve only streams back the (already filtered/joined)
//! result rows.
//!
//! Two entry points:
//! - [`MsFabricExecutor`] — the per-source `SQLExecutor` used by
//!   `datafusion-federation` at query time;
//! - [`introspect`] — `INFORMATION_SCHEMA` walk used by the `msfabric`
//!   data source's metadata-sync tick to discover tables + Arrow schemas.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use arrow_array::RecordBatch;
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use datafusion::error::{DataFusionError, Result as DfResult};
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::SendableRecordBatchStream;
use datafusion::sql::unparser::dialect::Dialect;
use datafusion_federation::sql::{AstAnalyzer, SQLExecutor};
use futures::StreamExt;
use pensieve_core::credentials::CredentialValue;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, FromSql};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::warn;
use uuid::Uuid;

use crate::FederationGuardrails;

/// Stable platform id stored in [`pensieve_core::catalog::FederatedTableSpec`].
pub const PLATFORM_ID: &str = "msfabric";

/// Entra token audience for TDS endpoints (Fabric SQL, Azure SQL).
const TDS_SCOPE: &str = "https://database.windows.net/.default";

/// Refresh tokens this long before their reported expiry.
const TOKEN_SLACK: chrono::Duration = chrono::Duration::seconds(300);

// ---------------------------------------------------------------------------
// Entra ID client-credentials token source (process-wide cache)
// ---------------------------------------------------------------------------

/// Acquires + caches an Entra access token for one service principal.
pub struct EntraTokenSource {
    sp_tenant_id: String,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    cached: tokio::sync::Mutex<Option<CachedToken>>,
}

#[derive(Clone)]
struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

impl EntraTokenSource {
    fn new(sp_tenant_id: &str, client_id: &str, client_secret: &str) -> Self {
        Self {
            sp_tenant_id: sp_tenant_id.to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            http: reqwest::Client::new(),
            cached: tokio::sync::Mutex::new(None),
        }
    }

    /// Current access token, refreshed via client-credentials when missing or
    /// within [`TOKEN_SLACK`] of expiry.
    pub async fn token(&self) -> anyhow::Result<String> {
        let mut guard = self.cached.lock().await;
        if let Some(c) = guard.as_ref() {
            if c.expires_at - TOKEN_SLACK > Utc::now() {
                return Ok(c.token.clone());
            }
        }
        let url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.sp_tenant_id
        );
        let resp = self
            .http
            .post(&url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("scope", TDS_SCOPE),
            ])
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Entra token request failed ({status}): {body}");
        }
        let tok: TokenResponse = resp.json().await?;
        let cached = CachedToken {
            token: tok.access_token.clone(),
            expires_at: Utc::now() + chrono::Duration::seconds(tok.expires_in.max(60)),
        };
        *guard = Some(cached);
        Ok(tok.access_token)
    }
}

/// Process-wide token-source cache keyed by `(sp_tenant_id, client_id)` so
/// per-query executor construction doesn't re-run the OAuth flow. Entries are
/// only ever added; a rotated `client_secret` lands under the same key with a
/// fresh source because the secret participates in the comparison below.
fn token_source(value: &CredentialValue) -> anyhow::Result<Arc<EntraTokenSource>> {
    static SOURCES: OnceLock<Mutex<HashMap<(String, String, String), Arc<EntraTokenSource>>>> =
        OnceLock::new();
    let CredentialValue::ServicePrincipal {
        tenant_id,
        client_id,
        client_secret,
    } = value
    else {
        anyhow::bail!("msfabric requires a service_principal credential");
    };
    let key = (
        tenant_id.clone(),
        client_id.clone(),
        client_secret.clone(),
    );
    let map = SOURCES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map.lock().expect("token source cache poisoned");
    Ok(map
        .entry(key)
        .or_insert_with(|| {
            Arc::new(EntraTokenSource::new(tenant_id, client_id, client_secret))
        })
        .clone())
}

// ---------------------------------------------------------------------------
// TDS connection plumbing
// ---------------------------------------------------------------------------

type TdsClient = Client<Compat<TcpStream>>;

fn split_host_port(endpoint: &str) -> (String, u16) {
    match endpoint.rsplit_once(':') {
        Some((host, port)) => match port.parse::<u16>() {
            Ok(p) => (host.to_owned(), p),
            Err(_) => (endpoint.to_owned(), 1433),
        },
        None => (endpoint.to_owned(), 1433),
    }
}

async fn connect(
    endpoint: &str,
    database: &str,
    tokens: &EntraTokenSource,
) -> anyhow::Result<TdsClient> {
    let (host, port) = split_host_port(endpoint);
    let mut config = Config::new();
    config.host(&host);
    config.port(port);
    config.database(database);
    config.encryption(EncryptionLevel::Required);
    config.authentication(AuthMethod::aad_token(tokens.token().await?));

    let tcp = TcpStream::connect((host.as_str(), port)).await?;
    tcp.set_nodelay(true)?;
    let client = Client::connect(config, tcp.compat_write()).await?;
    Ok(client)
}

// ---------------------------------------------------------------------------
// The SQLExecutor
// ---------------------------------------------------------------------------

/// One Fabric SQL endpoint + database + credential = one remote compute
/// context. Stateless across queries apart from the token cache; each
/// `execute` opens a fresh TDS connection (Fabric endpoints are fronted by
/// gateways that make connection reuse across queries unattractive at this
/// stage — revisit with pooling if connect latency shows up in traces).
pub struct MsFabricExecutor {
    endpoint: String,
    database: String,
    tokens: Arc<EntraTokenSource>,
    /// Distinguishes this source from other executors in the federation
    /// optimizer — same string ⇒ same remote engine ⇒ joins may merge.
    context: String,
    permits: Arc<Semaphore>,
    timeout: Duration,
    max_rows: usize,
}

impl MsFabricExecutor {
    pub fn new(
        endpoint: &str,
        database: &str,
        credential: &CredentialValue,
        credential_id: Uuid,
        guardrails: &FederationGuardrails,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            endpoint: endpoint.to_owned(),
            database: database.to_owned(),
            tokens: token_source(credential)?,
            context: format!("{PLATFORM_ID}:{endpoint}/{database}#{credential_id}"),
            permits: Arc::new(Semaphore::new(guardrails.max_concurrent_per_source.max(1))),
            timeout: guardrails.remote_timeout,
            max_rows: guardrails.max_rows,
        })
    }

    async fn run_query(
        endpoint: String,
        database: String,
        tokens: Arc<EntraTokenSource>,
        permits: Arc<Semaphore>,
        timeout: Duration,
        max_rows: usize,
        sql: String,
        schema: SchemaRef,
    ) -> anyhow::Result<RecordBatch> {
        let _permit = permits.acquire().await?;
        let work = async {
            let mut client = connect(&endpoint, &database, &tokens).await?;
            let stream = client.simple_query(&sql).await?;
            let rows = stream.into_first_result().await?;

            let mut json_rows: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
            let mut truncated = false;
            for row in &rows {
                if json_rows.len() >= max_rows {
                    truncated = true;
                    break;
                }
                json_rows.push(row_to_json(row, &schema)?);
            }
            if truncated {
                warn!(
                    endpoint = %endpoint,
                    database = %database,
                    max_rows,
                    "federated result truncated at row cap"
                );
            }
            json_to_batch(&json_rows, schema)
        };
        tokio::time::timeout(timeout, work)
            .await
            .map_err(|_| anyhow::anyhow!("remote query exceeded {}ms timeout", timeout.as_millis()))?
    }
}

#[async_trait]
impl SQLExecutor for MsFabricExecutor {
    fn name(&self) -> &str {
        PLATFORM_ID
    }

    fn compute_context(&self) -> Option<String> {
        Some(self.context.clone())
    }

    fn dialect(&self) -> Arc<dyn Dialect> {
        crate::dialect::tsql_dialect()
    }

    fn ast_analyzer(&self) -> Option<AstAnalyzer> {
        Some(crate::dialect::tsql_ast_analyzer())
    }

    fn execute(&self, query: &str, schema: SchemaRef) -> DfResult<SendableRecordBatchStream> {
        let fut = Self::run_query(
            self.endpoint.clone(),
            self.database.clone(),
            self.tokens.clone(),
            self.permits.clone(),
            self.timeout,
            self.max_rows,
            query.to_owned(),
            schema.clone(),
        );
        let stream = futures::stream::once(fut)
            .map(|r| r.map_err(|e| DataFusionError::External(e.into())));
        Ok(Box::pin(RecordBatchStreamAdapter::new(schema, stream)))
    }

    async fn table_names(&self) -> DfResult<Vec<String>> {
        Err(DataFusionError::NotImplemented(
            "msfabric: table discovery happens via the data source's metadata sync".into(),
        ))
    }

    async fn get_table_schema(&self, _table_name: &str) -> DfResult<SchemaRef> {
        Err(DataFusionError::NotImplemented(
            "msfabric: schemas are cached in the pensieve catalog".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Row → JSON → Arrow
// ---------------------------------------------------------------------------

/// Convert one TDS row to a JSON object keyed by the *expected* (Arrow) column
/// names. Values are rendered so `arrow-json` coerces them into the schema's
/// types: timestamps as RFC 3339 strings, decimals as f64, GUIDs/binary as
/// strings.
fn row_to_json(row: &tiberius::Row, schema: &Schema) -> anyhow::Result<serde_json::Value> {
    let mut obj = serde_json::Map::with_capacity(row.columns().len());
    for (i, (col, data)) in row.cells().enumerate() {
        // Aggregates can come back with empty/synthesized names; fall back to
        // the schema's column at the same position.
        let name = if col.name().is_empty() {
            schema
                .fields()
                .get(i)
                .map(|f| f.name().clone())
                .unwrap_or_else(|| format!("col{i}"))
        } else {
            col.name().to_owned()
        };
        obj.insert(name, column_data_to_json(data)?);
    }
    Ok(serde_json::Value::Object(obj))
}

fn column_data_to_json(data: &ColumnData<'static>) -> anyhow::Result<serde_json::Value> {
    use serde_json::{json, Value as J};
    Ok(match data {
        ColumnData::Bit(v) => v.map_or(J::Null, |b| json!(b)),
        ColumnData::U8(v) => v.map_or(J::Null, |n| json!(n)),
        ColumnData::I16(v) => v.map_or(J::Null, |n| json!(n)),
        ColumnData::I32(v) => v.map_or(J::Null, |n| json!(n)),
        ColumnData::I64(v) => v.map_or(J::Null, |n| json!(n)),
        ColumnData::F32(v) => v.map_or(J::Null, |n| json!(n)),
        ColumnData::F64(v) => v.map_or(J::Null, |n| json!(n)),
        ColumnData::Numeric(v) => v.map_or(J::Null, |n| json!(f64::from(n))),
        ColumnData::String(v) => v.as_ref().map_or(J::Null, |s| json!(s.as_ref())),
        ColumnData::Guid(v) => v.map_or(J::Null, |g| json!(g.to_string())),
        ColumnData::Binary(v) => v.as_ref().map_or(J::Null, |b| {
            json!(b.iter().map(|x| format!("{x:02x}")).collect::<String>())
        }),
        ColumnData::Xml(v) => v.as_ref().map_or(J::Null, |x| json!(x.as_ref().to_string())),
        d @ (ColumnData::DateTime(_)
        | ColumnData::SmallDateTime(_)
        | ColumnData::DateTime2(_)) => {
            match chrono::NaiveDateTime::from_sql(d)? {
                Some(ts) => json!(ts.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()),
                None => J::Null,
            }
        }
        d @ ColumnData::Date(_) => match chrono::NaiveDate::from_sql(d)? {
            Some(dt) => json!(dt.format("%Y-%m-%d").to_string()),
            None => J::Null,
        },
        d @ ColumnData::Time(_) => match chrono::NaiveTime::from_sql(d)? {
            Some(t) => json!(t.format("%H:%M:%S%.6f").to_string()),
            None => J::Null,
        },
        d @ ColumnData::DateTimeOffset(_) => {
            match chrono::DateTime::<Utc>::from_sql(d)? {
                Some(ts) => json!(ts.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)),
                None => J::Null,
            }
        }
    })
}

/// Decode JSON rows into a single `RecordBatch` matching `schema` (arrow-json
/// handles string→timestamp/date parsing and numeric widening).
fn json_to_batch(rows: &[serde_json::Value], schema: SchemaRef) -> anyhow::Result<RecordBatch> {
    let mut decoder = arrow_json::ReaderBuilder::new(schema.clone()).build_decoder()?;
    decoder.serialize(rows)?;
    Ok(decoder
        .flush()?
        .unwrap_or_else(|| RecordBatch::new_empty(schema)))
}

// ---------------------------------------------------------------------------
// Introspection (used by the msfabric data source's metadata-sync tick)
// ---------------------------------------------------------------------------

/// One remote table discovered by [`introspect`].
pub struct RemoteTableMeta {
    pub remote_schema: String,
    pub remote_table: String,
    pub schema: SchemaRef,
}

/// Walk `INFORMATION_SCHEMA.COLUMNS` on a Fabric SQL endpoint and return every
/// user table/view with its Arrow schema (per [`map_tsql_type`]).
pub async fn introspect(
    endpoint: &str,
    database: &str,
    credential: &CredentialValue,
    timeout: Duration,
) -> anyhow::Result<Vec<RemoteTableMeta>> {
    let tokens = token_source(credential)?;
    let work = async {
        let mut client = connect(endpoint, database, &tokens).await?;
        let sql = "SELECT TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME, DATA_TYPE, IS_NULLABLE \
                   FROM INFORMATION_SCHEMA.COLUMNS \
                   WHERE TABLE_SCHEMA NOT IN ('sys', 'INFORMATION_SCHEMA', 'queryinsights') \
                   ORDER BY TABLE_SCHEMA, TABLE_NAME, ORDINAL_POSITION";
        let rows = client.simple_query(sql).await?.into_first_result().await?;

        // Ordered walk: rows arrive grouped per table.
        let mut out: Vec<RemoteTableMeta> = Vec::new();
        let mut current: Option<(String, String, Vec<Field>)> = None;
        for row in &rows {
            let schema_name: &str = row.get(0).unwrap_or_default();
            let table_name: &str = row.get(1).unwrap_or_default();
            let column_name: &str = row.get(2).unwrap_or_default();
            let data_type: &str = row.get(3).unwrap_or_default();
            let is_nullable: &str = row.get(4).unwrap_or("YES");

            let switch = match &current {
                Some((s, t, _)) => s != schema_name || t != table_name,
                None => true,
            };
            if switch {
                if let Some((s, t, fields)) = current.take() {
                    out.push(RemoteTableMeta {
                        remote_schema: s,
                        remote_table: t,
                        schema: Arc::new(Schema::new(fields)),
                    });
                }
                current = Some((schema_name.to_owned(), table_name.to_owned(), Vec::new()));
            }
            if let Some((_, _, fields)) = current.as_mut() {
                fields.push(Field::new(
                    column_name,
                    map_tsql_type(data_type),
                    !is_nullable.eq_ignore_ascii_case("NO"),
                ));
            }
        }
        if let Some((s, t, fields)) = current.take() {
            out.push(RemoteTableMeta {
                remote_schema: s,
                remote_table: t,
                schema: Arc::new(Schema::new(fields)),
            });
        }
        Ok(out)
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| anyhow::anyhow!("introspection exceeded {}ms timeout", timeout.as_millis()))?
}

/// T-SQL → Arrow type mapping. Conservative: anything unknown (or lossy to
/// represent) maps to Utf8 so values always round-trip as strings.
/// Decimals map to Float64 — precision loss past 2^53 is documented; revisit
/// with Decimal128 once end-to-end decimal handling is proven.
pub fn map_tsql_type(data_type: &str) -> DataType {
    match data_type.to_ascii_lowercase().as_str() {
        "bit" => DataType::Boolean,
        "tinyint" | "smallint" | "int" => DataType::Int32,
        "bigint" => DataType::Int64,
        "real" => DataType::Float32,
        "float" => DataType::Float64,
        "decimal" | "numeric" | "money" | "smallmoney" => DataType::Float64,
        "date" => DataType::Date32,
        "datetime" | "datetime2" | "smalldatetime" => {
            DataType::Timestamp(TimeUnit::Microsecond, None)
        }
        "datetimeoffset" => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        // time, [n]char/[n]varchar/[n]text, uniqueidentifier, xml, json,
        // binary/varbinary/image (hex), and anything else → strings.
        _ => DataType::Utf8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_map_covers_core_types() {
        assert_eq!(map_tsql_type("BIT"), DataType::Boolean);
        assert_eq!(map_tsql_type("int"), DataType::Int32);
        assert_eq!(map_tsql_type("bigint"), DataType::Int64);
        assert_eq!(map_tsql_type("decimal"), DataType::Float64);
        assert_eq!(map_tsql_type("varchar"), DataType::Utf8);
        assert_eq!(map_tsql_type("nvarchar"), DataType::Utf8);
        assert_eq!(map_tsql_type("date"), DataType::Date32);
        assert_eq!(
            map_tsql_type("datetime2"),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(map_tsql_type("geography"), DataType::Utf8);
    }

    #[test]
    fn endpoint_parsing_defaults_port() {
        assert_eq!(
            split_host_port("abc.datawarehouse.fabric.microsoft.com"),
            ("abc.datawarehouse.fabric.microsoft.com".to_owned(), 1433)
        );
        assert_eq!(
            split_host_port("host:14330"),
            ("host".to_owned(), 14330)
        );
    }

    #[test]
    fn json_decode_respects_schema() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, true),
            Field::new("ts", DataType::Timestamp(TimeUnit::Microsecond, None), true),
        ]));
        let rows = vec![
            serde_json::json!({"a": 1, "ts": "2026-01-02T03:04:05.000006"}),
            serde_json::json!({"a": null, "ts": null}),
        ];
        let batch = json_to_batch(&rows, schema).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2);
    }
}
