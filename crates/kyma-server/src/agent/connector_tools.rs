//! Read-only connector access for the dreaming agent.
//!
//! Two tools give the agent direct, authenticated READ access to configured
//! connectors so it can fill memory gaps with fresh context:
//!
//! - `list_connectors` — discover what sources exist (no secrets exposed).
//! - `connector_read(connector_id, operation, params)` — a narrow per-kind
//!   allowlist of read operations, resolved through the connector's stored
//!   credential. v1 kinds: `github` (readme/file/issues/pulls/repo) and
//!   `postgres` (SELECT-only query). Other kinds return "unsupported".
//!
//! Every call is budgeted per run ([`ConnectorReadBudget`]) and results are
//! truncated so a giant file can't blow the context window. There are no
//! write operations on this surface by construction.

use adk_rust::tool::FunctionTool;
use adk_rust::{Tool, ToolContext};
use kyma_core::credentials::{CredentialStore, CredentialValue};
use kyma_core::tenant::TenantId;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Per-run gap-fill budget shared by every `connector_read` closure of one
/// dreaming run. Over-budget calls return an error payload so the agent can
/// move on to housekeeping instead of aborting.
pub struct ConnectorReadBudget {
    max_calls: u64,
    max_bytes: u64,
    used_calls: AtomicU64,
    used_bytes: AtomicU64,
}

impl ConnectorReadBudget {
    pub fn new(max_calls: u32, max_bytes: u64) -> Self {
        Self {
            max_calls: max_calls as u64,
            max_bytes,
            used_calls: AtomicU64::new(0),
            used_bytes: AtomicU64::new(0),
        }
    }

    /// Reserve one call; `false` = budget exhausted.
    fn take_call(&self) -> bool {
        self.used_calls.fetch_add(1, Ordering::Relaxed) < self.max_calls
    }

    /// Record bytes fetched; `false` = byte budget exhausted (the result is
    /// still returned — the NEXT call gets refused).
    fn add_bytes(&self, n: u64) -> bool {
        self.used_bytes.fetch_add(n, Ordering::Relaxed) + n <= self.max_bytes
    }

    pub fn used(&self) -> (u64, u64) {
        (
            self.used_calls.load(Ordering::Relaxed),
            self.used_bytes.load(Ordering::Relaxed),
        )
    }
}

/// Everything the connector read tools need. Built per dreaming run.
#[derive(Clone)]
pub struct ConnectorToolCtx {
    pub pool: Option<PgPool>,
    pub credentials: Arc<dyn CredentialStore>,
    pub tenant: TenantId,
    pub budget: Arc<ConnectorReadBudget>,
}

/// Per-operation result cap — keeps one read from flooding the context.
const MAX_RESULT_CHARS: usize = 8 * 1024;
/// Max file size fetched from a source before truncation/skip.
const MAX_FETCH_BYTES: usize = 512 * 1024;

const LIST_CONNECTORS_DESC: &str = "List the configured data connectors (id, name, kind, \
enabled, target database). Use this to discover which external sources you can read from \
with `connector_read` when filling memory gaps.";

pub fn tool_list_connectors(ctx: ConnectorToolCtx) -> Arc<dyn Tool> {
    Arc::new(
        FunctionTool::new(
            "list_connectors",
            LIST_CONNECTORS_DESC,
            move |_tc: Arc<dyn ToolContext>, _args: Value| {
                let ctx = ctx.clone();
                async move {
                    let Some(pool) = &ctx.pool else {
                        return Ok(json!({"error": "no connector store in local mode"}));
                    };
                    let rows = sqlx::query(
                        "SELECT id, name, type, enabled, target_database \
                         FROM connectors WHERE tenant_id = $1 ORDER BY name",
                    )
                    .bind(ctx.tenant.as_uuid())
                    .fetch_all(pool)
                    .await;
                    match rows {
                        Ok(rows) => {
                            let items: Vec<Value> = rows
                                .iter()
                                .map(|r| {
                                    json!({
                                        "id": r.get::<Uuid, _>("id").to_string(),
                                        "name": r.get::<String, _>("name"),
                                        "kind": r.get::<String, _>("type"),
                                        "enabled": r.get::<bool, _>("enabled"),
                                        "target_database": r.get::<String, _>("target_database"),
                                        "read_ops": read_ops_for(&r.get::<String, _>("type")),
                                    })
                                })
                                .collect();
                            Ok(json!({ "connectors": items }))
                        }
                        Err(e) => Ok(json!({"error": format!("list connectors: {e}")})),
                    }
                }
            },
        )
        .with_read_only(true),
    )
}

fn read_ops_for(kind: &str) -> Vec<&'static str> {
    match kind {
        "github" => vec![
            "get_repo",
            "get_readme",
            "get_file",
            "list_issues",
            "get_issue",
            "list_pulls",
        ],
        "postgres" => vec!["query"],
        _ => vec![],
    }
}

const CONNECTOR_READ_DESC: &str = "READ-ONLY access to a configured connector's source, using \
its stored credential. Args: {connector_id, operation, params}. Operations by kind — \
github: get_repo {repo?}, get_readme {repo?}, get_file {repo?, path}, list_issues {repo?, limit?}, \
get_issue {repo?, number}, list_pulls {repo?, limit?} (repo defaults to the connector's first \
configured \"owner/name\"); postgres: query {sql} (SELECT-only, auto-limited). Budgeted per run; \
results truncated to 8KB. Use list_connectors first to discover ids and supported ops.";

pub fn tool_connector_read(ctx: ConnectorToolCtx) -> Arc<dyn Tool> {
    Arc::new(
        FunctionTool::new(
            "connector_read",
            CONNECTOR_READ_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let ctx = ctx.clone();
                async move { Ok(connector_read(&ctx, args).await) }
            },
        )
        .with_read_only(true),
    )
}

async fn connector_read(ctx: &ConnectorToolCtx, args: Value) -> Value {
    let Some(pool) = &ctx.pool else {
        return json!({"error": "no connector store in local mode"});
    };
    if !ctx.budget.take_call() {
        let (calls, bytes) = ctx.budget.used();
        return json!({"error": format!(
            "connector read budget exhausted for this run ({calls} calls, {bytes} bytes) — \
             continue with housekeeping using what you already have"
        )});
    }
    let connector_id = match args
        .get("connector_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(id) => id,
        None => return json!({"error": "connector_id (uuid) is required"}),
    };
    let operation = args
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let params = args.get("params").cloned().unwrap_or_else(|| json!({}));

    let row = match sqlx::query(
        "SELECT type, config_jsonb FROM connectors WHERE tenant_id = $1 AND id = $2",
    )
    .bind(ctx.tenant.as_uuid())
    .bind(connector_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return json!({"error": "no such connector"}),
        Err(e) => return json!({"error": format!("load connector: {e}")}),
    };
    let kind: String = row.get("type");
    let config: Value = row.get("config_jsonb");

    let result = match kind.as_str() {
        "github" => github_read(ctx, &config, &operation, &params).await,
        "postgres" => postgres_read(ctx, &config, &operation, &params).await,
        other => json!({"error": format!(
            "connector kind `{other}` has no read operations in this version \
             (supported: github, postgres)"
        )}),
    };

    // Count + truncate the payload so one read can't flood the context.
    let rendered = result.to_string();
    ctx.budget.add_bytes(rendered.len() as u64);
    if rendered.len() > MAX_RESULT_CHARS {
        match result {
            Value::Object(mut map) => {
                if let Some(Value::String(s)) = map.get_mut("content") {
                    s.truncate(MAX_RESULT_CHARS);
                    map.insert("truncated".into(), json!(true));
                    return Value::Object(map);
                }
                let mut s = Value::Object(map).to_string();
                s.truncate(MAX_RESULT_CHARS);
                json!({ "content": s, "truncated": true })
            }
            other => {
                let mut s = other.to_string();
                s.truncate(MAX_RESULT_CHARS);
                json!({ "content": s, "truncated": true })
            }
        }
    } else {
        result
    }
}

// ── github ───────────────────────────────────────────────────────────────────

async fn github_read(ctx: &ConnectorToolCtx, config: &Value, op: &str, params: &Value) -> Value {
    // Resolve the repo: explicit param or the connector's first configured one.
    let repo_param = params
        .get("repo")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            config
                .get("repos")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    let Some(repo) = repo_param else {
        return json!({"error": "no repo configured or supplied (expected \"owner/name\")"});
    };
    let Some((owner, name)) = repo.split_once('/') else {
        return json!({"error": format!("repo {repo:?} must be \"owner/name\"")});
    };

    // Credential: credential_id (preferred) → inline token. Mirrors the
    // github connector's own resolution; env refs are not resolved here.
    let token = match config
        .get("credential_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(cid) => match ctx.credentials.get(ctx.tenant, cid).await {
            Ok(cred) => match cred.value {
                CredentialValue::Pat { token } => token,
                other => {
                    return json!({"error": format!(
                        "credential has kind={}, github read requires `pat`",
                        other.kind()
                    )})
                }
            },
            Err(e) => return json!({"error": format!("resolve credential: {e}")}),
        },
        None => {
            let inline = config
                .get("token")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if inline.is_empty() || inline.starts_with('$') {
                return json!({"error": "connector has no directly readable credential"});
            }
            inline.to_string()
        }
    };

    let http = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => return json!({"error": format!("http client: {e}")}),
    };
    let client = kyma_datasources::github::client::GithubClient::new(http, token);

    match op {
        "get_repo" => match client.get_repo(owner, name).await {
            Ok(v) => slim_repo(v),
            Err(e) => json!({"error": e.to_string()}),
        },
        "get_readme" => match client.get_readme(owner, name, MAX_FETCH_BYTES).await {
            Ok(Some(content)) => json!({ "repo": repo, "content": content }),
            Ok(None) => json!({"error": "readme missing, binary, or too large"}),
            Err(e) => json!({"error": e.to_string()}),
        },
        "get_file" => {
            let Some(path) = params.get("path").and_then(|v| v.as_str()) else {
                return json!({"error": "params.path is required for get_file"});
            };
            match client.get_contents(owner, name, path, MAX_FETCH_BYTES).await {
                Ok(Some(content)) => json!({ "repo": repo, "path": path, "content": content }),
                Ok(None) => json!({"error": "file missing, binary, or too large"}),
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        "list_issues" | "list_pulls" => {
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20)
                .min(50) as usize;
            let fetched = if op == "list_issues" {
                client.list_issues(owner, name, None, 1).await
            } else {
                client.list_pulls(owner, name, None, 1).await
            };
            match fetched {
                Ok((items, _stop)) => {
                    let slim: Vec<Value> = items.into_iter().take(limit).map(slim_issue).collect();
                    json!({ "repo": repo, "items": slim })
                }
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        "get_issue" => {
            let Some(number) = params.get("number").and_then(|v| v.as_u64()) else {
                return json!({"error": "params.number is required for get_issue"});
            };
            match client.get_issue(owner, name, number).await {
                Ok(v) => slim_issue(v),
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        other => json!({"error": format!("unsupported github operation `{other}`")}),
    }
}

/// Keep only the fields the agent needs from a repo payload.
fn slim_repo(v: Value) -> Value {
    json!({
        "full_name": v.get("full_name"),
        "description": v.get("description"),
        "default_branch": v.get("default_branch"),
        "language": v.get("language"),
        "topics": v.get("topics"),
        "open_issues_count": v.get("open_issues_count"),
        "stargazers_count": v.get("stargazers_count"),
        "pushed_at": v.get("pushed_at"),
        "archived": v.get("archived"),
    })
}

/// Keep only the fields the agent needs from an issue/PR payload.
fn slim_issue(v: Value) -> Value {
    json!({
        "number": v.get("number"),
        "title": v.get("title"),
        "state": v.get("state"),
        "user": v.get("user").and_then(|u| u.get("login")).cloned(),
        "labels": v
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|a| a.iter().filter_map(|x| x.get("name").cloned()).collect::<Vec<_>>()),
        "created_at": v.get("created_at"),
        "updated_at": v.get("updated_at"),
        "body": v
            .get("body")
            .and_then(|b| b.as_str())
            .map(|s| s.chars().take(1000).collect::<String>()),
    })
}

// ── postgres ─────────────────────────────────────────────────────────────────

async fn postgres_read(ctx: &ConnectorToolCtx, config: &Value, op: &str, params: &Value) -> Value {
    if op != "query" {
        return json!({"error": format!("unsupported postgres operation `{op}` (only `query`)")});
    }
    let Some(sql) = params.get("sql").and_then(|v| v.as_str()) else {
        return json!({"error": "params.sql is required for query"});
    };
    // SELECT-only: single statement, must start with SELECT or WITH…SELECT.
    let normalized = sql.trim().trim_end_matches(';').trim();
    if normalized.contains(';') {
        return json!({"error": "exactly one statement allowed"});
    }
    let lowered = normalized.to_lowercase();
    let is_select = lowered.starts_with("select")
        || (lowered.starts_with("with") && lowered.contains("select"));
    let forbidden = [
        "insert", "update", "delete", "drop", "alter", "create", "truncate", "grant", "revoke",
        "copy", "vacuum", "call", "do ",
    ];
    if !is_select || forbidden.iter().any(|kw| lowered.starts_with(kw)) {
        return json!({"error": "postgres connector_read is SELECT-only"});
    }
    // Hard row cap regardless of the query's own LIMIT.
    let limited = format!("SELECT * FROM ({normalized}) AS _kyma_read LIMIT 100");

    // Connection URL: credential_id of kind `url` (preferred) → inline url.
    let url = match config
        .get("credential_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    {
        Some(cid) => match ctx.credentials.get(ctx.tenant, cid).await {
            Ok(cred) => match cred.value {
                CredentialValue::Url { connection_string } => connection_string,
                other => {
                    return json!({"error": format!(
                        "credential has kind={}, postgres read requires `url`",
                        other.kind()
                    )})
                }
            },
            Err(e) => return json!({"error": format!("resolve credential: {e}")}),
        },
        None => {
            let inline = config.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            if inline.is_empty() {
                return json!({"error": "connector has no connection url"});
            }
            inline.to_string()
        }
    };

    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(p) => p,
        Err(e) => return json!({"error": format!("connect: {e}")}),
    };
    let rows = sqlx::query(&limited).fetch_all(&pool).await;
    pool.close().await;
    match rows {
        Ok(rows) => {
            // Render rows generically: every column as text.
            let out: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let mut obj = serde_json::Map::new();
                    for col in r.columns() {
                        use sqlx::Column as _;
                        let name = col.name();
                        let val: Option<String> = r.try_get::<Option<String>, _>(name).ok().flatten()
                            .or_else(|| r.try_get::<Option<i64>, _>(name).ok().flatten().map(|v| v.to_string()))
                            .or_else(|| r.try_get::<Option<f64>, _>(name).ok().flatten().map(|v| v.to_string()))
                            .or_else(|| r.try_get::<Option<bool>, _>(name).ok().flatten().map(|v| v.to_string()));
                        obj.insert(name.to_string(), val.map(Value::String).unwrap_or(Value::Null));
                    }
                    Value::Object(obj)
                })
                .collect();
            json!({ "rows": out, "row_count": out.len() })
        }
        Err(e) => json!({"error": format!("query: {e}")}),
    }
}
