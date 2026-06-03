//! `kyma ingest push` · `kyma recall` · `kyma distill` · `kyma install-plugin`.
//!
//! These four verbs back the **kyma-memory Claude Code plugin** (see
//! `integrations/claude-code/kyma-memory/`). They all reuse the saved client
//! connection (`~/.kyma/config.json` or `KYMA_SERVER_URL`/`KYMA_TOKEN`) and the
//! shared reqwest client, so the plugin holds no credentials of its own.
//!
//!   - `ingest push`     firehose: stdin NDJSON → `POST /v1/ingest`
//!   - `recall`          fast semantic recall via MCP `tools/call recall_memory`
//!   - `distill`         stdin transcript → `/v1/agent/ask` (agent owns save_memory)
//!   - `install-plugin`  materialize the bundled plugin into ~/.claude/skills/

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::client::{self, ClientConfig};

// ── ingest push ──────────────────────────────────────────────────────────────

/// Read NDJSON from stdin and POST it to `/v1/ingest`. Auto-create +
/// schema-evolve are on so the firehose table appears on first write.
pub(crate) async fn ingest_push(
    table: String,
    db: Option<String>,
    idempotency_key: Option<String>,
) -> Result<()> {
    let cfg = client::effective_config()?;
    let mut body = String::new();
    std::io::stdin()
        .read_to_string(&mut body)
        .context("reading NDJSON from stdin")?;
    if body.trim().is_empty() {
        return Ok(()); // nothing to send — stay quiet for hook use
    }
    let db = db.unwrap_or_else(|| "default".to_string());
    let url = format!("{}/v1/ingest", cfg.endpoint.trim_end_matches('/'));
    let mut req = client::http_client()
        .post(url)
        .header("X-Table", table.as_str())
        .header("X-Database", db.as_str())
        .header("X-Schema-Evolve", "true")
        .header("content-type", "application/x-ndjson")
        .body(body);
    if let Some(k) = idempotency_key {
        req = req.header("X-Idempotency-Key", k);
    }
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.context("POST /v1/ingest")?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("ingest returned {status}: {text}"));
    }
    // Quiet on success unless asked for detail (hooks pipe this to /dev/null).
    if std::env::var_os("KYMA_INGEST_VERBOSE").is_some() {
        println!("{text}");
    }
    Ok(())
}

// ── recall ───────────────────────────────────────────────────────────────────

/// Semantic recall via the MCP `recall_memory` tool. Prints a compact ranked
/// list (for hook context injection) or raw JSON with `--json`.
pub(crate) async fn recall(
    query: String,
    realm: Option<String>,
    limit: usize,
    as_json: bool,
) -> Result<()> {
    let cfg = client::effective_config()?;
    let mut realms: Vec<String> = Vec::new();
    if let Some(r) = realm {
        if !r.is_empty() {
            realms.push(r);
            realms.push("global".to_string());
        }
    }
    let arguments = json!({
        "query": query,
        "limit": limit,
        "realms": realms,
    });
    let result = mcp_call(&cfg, "recall_memory", arguments).await?;

    if as_json {
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    // The graph-aware recall returns a ready-to-inject `context` block (memories
    // + connected resources, citation-rich). Prefer it for hook injection.
    if let Some(ctx) = result.get("context").and_then(Value::as_str) {
        if !ctx.trim().is_empty() {
            println!("{}", ctx.trim());
            return Ok(());
        }
    }
    // Fall back to a compact ranked list (`memories` from the orchestrator, or
    // legacy `rows`).
    let rows = result
        .get("memories")
        .and_then(Value::as_array)
        .or_else(|| result.get("rows").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    for row in rows {
        println!("{}", render_memory_line(&row));
    }
    Ok(())
}

// ── remember ───────────────────────────────────────────────────────────────

/// Save a durable memory via the MCP `save_memory` tool. With `topic_key`, a
/// re-save updates the memory in place instead of duplicating.
pub(crate) async fn remember(
    content: String,
    memory_type: Option<String>,
    realm: Option<String>,
    importance: Option<f32>,
    topic_key: Option<String>,
) -> Result<()> {
    let cfg = client::effective_config()?;
    let mut args = json!({ "content": content });
    if let Some(t) = memory_type { args["memory_type"] = json!(t); }
    if let Some(r) = realm { args["realm"] = json!(r); }
    if let Some(i) = importance { args["importance"] = json!(i); }
    if let Some(tk) = topic_key { args["topic_key"] = json!(tk); }
    let result = mcp_call(&cfg, "save_memory", args).await?;
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return Err(anyhow!("{err}"));
    }
    let id = result.get("id").and_then(Value::as_str).unwrap_or("?");
    let verb = if result.get("upserted").and_then(Value::as_bool).unwrap_or(false) {
        "updated"
    } else {
        "saved"
    };
    println!("{verb} memory {id}");
    Ok(())
}

// ── entity ─────────────────────────────────────────────────────────────────

/// Create or update a virtual graph entity via the MCP `ingest_entity` tool,
/// wiring it to existing graph nodes / memories. Idempotent per (realm, kind, name).
pub(crate) async fn entity(
    name: String,
    kind: Option<String>,
    realm: Option<String>,
    props: Vec<String>,
    links: Vec<String>,
    icon: Option<String>,
    entity_type: Option<String>,
) -> Result<()> {
    let cfg = client::effective_config()?;
    let mut args = json!({ "name": name });
    if let Some(k) = kind { args["kind"] = json!(k); }
    if let Some(r) = realm { args["realm"] = json!(r); }
    if let Some(i) = icon { args["icon"] = json!(i); }
    if let Some(t) = entity_type { args["type"] = json!(t); }
    if !props.is_empty() {
        let mut obj = serde_json::Map::new();
        for p in &props {
            if let Some((k, v)) = p.split_once('=') {
                obj.insert(k.trim().to_string(), json!(v.trim()));
            }
        }
        if !obj.is_empty() {
            args["properties"] = Value::Object(obj);
        }
    }
    // Each `--link` is `node_id[|namespace[|rel]]`:
    //   repo:owner/name|github|LIVES_IN   ·   memory:<uuid>||DOCUMENTED_BY
    if !links.is_empty() {
        let parsed: Vec<Value> = links
            .iter()
            .map(|l| {
                let mut parts = l.splitn(3, '|');
                let node = parts.next().unwrap_or("").trim().to_string();
                let mut o = json!({ "target_node_id": node });
                if let Some(ns) = parts.next().map(str::trim).filter(|s| !s.is_empty()) {
                    o["target_namespace"] = json!(ns);
                }
                if let Some(rel) = parts.next().map(str::trim).filter(|s| !s.is_empty()) {
                    o["relationship_type"] = json!(rel);
                }
                o
            })
            .collect();
        args["links"] = json!(parsed);
    }
    let result = mcp_call(&cfg, "ingest_entity", args).await?;
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return Err(anyhow!("{err}"));
    }
    let id = result.get("id").and_then(Value::as_str).unwrap_or("?");
    let n = result.get("links").and_then(Value::as_u64).unwrap_or(0);
    let verb = if result.get("upserted").and_then(Value::as_bool).unwrap_or(false) {
        "updated"
    } else {
        "created"
    };
    println!("{verb} entity {id} ({n} links)");
    Ok(())
}

/// Render one recalled memory row as a single compact line. Tolerant of which
/// columns the recall SQL projected.
fn render_memory_line(row: &Value) -> String {
    let mtype = row
        .get("memory_type")
        .and_then(Value::as_str)
        .unwrap_or("memory");
    let score = row
        .get("score")
        .and_then(Value::as_f64)
        .or_else(|| row.get("distance").and_then(Value::as_f64).map(|d| 1.0 - d));
    let body = row
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| row.get("content_preview").and_then(Value::as_str))
        .or_else(|| row.get("content").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    let body: String = body.chars().take(280).collect();
    match score {
        Some(s) => format!("- [{mtype} {:.2}] {body}", s),
        None => format!("- [{mtype}] {body}"),
    }
}

/// Issue a single stateless MCP `tools/call` and return its `structuredContent`.
async fn mcp_call(cfg: &ClientConfig, tool: &str, arguments: Value) -> Result<Value> {
    let url = format!("{}/mcp/v1", cfg.endpoint.trim_end_matches('/'));
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    let mut req = client::http_client()
        .post(url)
        .header("accept", "application/json")
        .json(&body);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.context("POST /mcp/v1")?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("mcp returned {status}: {text}"));
    }
    let v: Value = serde_json::from_str(&text).with_context(|| format!("parse mcp response: {text}"))?;
    if let Some(err) = v.get("error") {
        return Err(anyhow!("mcp error: {err}"));
    }
    Ok(v.get("result")
        .and_then(|r| r.get("structuredContent"))
        .cloned()
        .unwrap_or(Value::Null))
}

// ── distill ──────────────────────────────────────────────────────────────────

const DISTILL_MAX_CHARS: usize = 60_000;

/// Read a session transcript from stdin and hand it to the kyma agent (which
/// owns `save_memory`) with an extraction instruction. The agent persists the
/// durable memories; we stay quiet unless `KYMA_DISTILL_VERBOSE` is set.
pub(crate) async fn distill(_session: Option<String>, realm: Option<String>) -> Result<()> {
    let cfg = client::effective_config()?;
    let mut transcript = String::new();
    std::io::stdin()
        .read_to_string(&mut transcript)
        .context("reading transcript from stdin")?;
    if transcript.trim().is_empty() {
        return Ok(());
    }
    // Keep the tail if it's large (most recent context matters most).
    if transcript.len() > DISTILL_MAX_CHARS {
        let start = transcript.len() - DISTILL_MAX_CHARS;
        transcript = transcript[start..].to_string();
    }
    let realm = realm.unwrap_or_else(|| "default".to_string());
    let instruction = format!(
        "You are curating durable memory from a Claude Code coding session. Below is the \
session transcript (JSONL or text). Extract the SMALL set of genuinely durable, reusable \
memories: decisions (with rationale), preferences, conventions, non-obvious facts, \
learnings, and open threads worth resuming. For each, call save_memory with realm \
\"{realm}\", an appropriate memory_type (fact/decision/preference/learning/summary) and \
importance 0.3-0.9. Recall first to avoid duplicates; skip transient details and anything \
secret. Quality over quantity (typically 0-6). Then reply with a one-line summary of what \
you saved.\n\n--- SESSION TRANSCRIPT ---\n{transcript}"
    );

    let verbose = std::env::var_os("KYMA_DISTILL_VERBOSE").is_some();
    client::stream_agent_ask(&cfg, &instruction, None, |event, data| {
        if !verbose {
            return;
        }
        if event == "answer_delta" || event == "answer_final" {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                if let Some(t) = v.get("text").and_then(Value::as_str) {
                    print!("{t}");
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
            }
        }
    })
    .await?;
    Ok(())
}

// ── install-plugin ───────────────────────────────────────────────────────────

/// (relative path, file contents, is_executable). `.mcp.json` is generated
/// (templated with the live server URL + token) rather than embedded verbatim.
const PLUGIN_FILES: &[(&str, &str, bool)] = &[
    (
        ".claude-plugin/plugin.json",
        include_str!("../../../integrations/claude-code/kyma-memory/.claude-plugin/plugin.json"),
        false,
    ),
    (
        "hooks/hooks.json",
        include_str!("../../../integrations/claude-code/kyma-memory/hooks/hooks.json"),
        false,
    ),
    (
        "scripts/lib.sh",
        include_str!("../../../integrations/claude-code/kyma-memory/scripts/lib.sh"),
        true,
    ),
    (
        "scripts/on-session-start.sh",
        include_str!("../../../integrations/claude-code/kyma-memory/scripts/on-session-start.sh"),
        true,
    ),
    (
        "scripts/on-user-prompt.sh",
        include_str!("../../../integrations/claude-code/kyma-memory/scripts/on-user-prompt.sh"),
        true,
    ),
    (
        "scripts/on-post-tool.sh",
        include_str!("../../../integrations/claude-code/kyma-memory/scripts/on-post-tool.sh"),
        true,
    ),
    (
        "scripts/on-stop.sh",
        include_str!("../../../integrations/claude-code/kyma-memory/scripts/on-stop.sh"),
        true,
    ),
    (
        "scripts/on-session-end.sh",
        include_str!("../../../integrations/claude-code/kyma-memory/scripts/on-session-end.sh"),
        true,
    ),
    (
        "commands/kyma-recall.md",
        include_str!("../../../integrations/claude-code/kyma-memory/commands/kyma-recall.md"),
        false,
    ),
    (
        "commands/kyma-remember.md",
        include_str!("../../../integrations/claude-code/kyma-memory/commands/kyma-remember.md"),
        false,
    ),
    (
        "commands/kyma-ask.md",
        include_str!("../../../integrations/claude-code/kyma-memory/commands/kyma-ask.md"),
        false,
    ),
    (
        "commands/kyma-status.md",
        include_str!("../../../integrations/claude-code/kyma-memory/commands/kyma-status.md"),
        false,
    ),
    (
        "skills/kyma-memory/SKILL.md",
        include_str!("../../../integrations/claude-code/kyma-memory/skills/kyma-memory/SKILL.md"),
        false,
    ),
    (
        "agents/memory-curator.md",
        include_str!("../../../integrations/claude-code/kyma-memory/agents/memory-curator.md"),
        false,
    ),
    (
        "README.md",
        include_str!("../../../integrations/claude-code/kyma-memory/README.md"),
        false,
    ),
];

/// Materialize the bundled plugin into a Claude Code skills-dir plugin
/// (`~/.claude/skills/kyma-memory/` by default), templating the live Kyma URL +
/// token into `.mcp.json` so both hooks and the MCP server work immediately.
pub(crate) async fn install_plugin(target: Option<PathBuf>, force: bool) -> Result<()> {
    let dir = match target {
        Some(p) => p,
        None => claude_skills_dir()?.join("kyma-memory"),
    };

    if dir.exists() && !force {
        // Re-installing over an existing dir is fine (we overwrite files), but
        // warn so an accidental run is visible.
        eprintln!(
            "note: {} already exists — overwriting plugin files (use --force to silence)",
            dir.display()
        );
    }

    for (rel, contents, exec) in PLUGIN_FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        std::fs::write(&path, contents).with_context(|| format!("write {}", path.display()))?;
        if *exec {
            set_executable(&path);
        }
    }

    // Resolve the live connection (best-effort) for the MCP server config.
    let cfg = client::effective_config().ok();
    let url = cfg
        .as_ref()
        .map(|c| c.endpoint.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    let token = cfg.as_ref().and_then(|c| c.token.clone());

    let mcp_path = dir.join(".mcp.json");
    std::fs::write(&mcp_path, mcp_json(&url, token.as_deref())?)
        .with_context(|| format!("write {}", mcp_path.display()))?;
    // The MCP config may embed a bearer token — keep it private.
    set_private(&mcp_path);

    println!("Installed kyma-memory plugin → {}", dir.display());
    println!("  MCP server → {}/mcp/v1", url.trim_end_matches('/'));
    if cfg.is_none() {
        println!(
            "  ⚠  no Kyma connection found — run `kyma connect <url> --token <token>` then \
re-run `kyma install-plugin` so the MCP server is authenticated."
        );
    } else if token.is_none() {
        println!("  ⚠  no token saved — the MCP server will connect unauthenticated.");
    }
    println!("\nRestart Claude Code, then run /kyma-status to verify.");
    Ok(())
}

/// `~/.claude/skills` — where skills-dir plugins auto-load from.
fn claude_skills_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("$HOME is not set"))?;
    Ok(PathBuf::from(home).join(".claude").join("skills"))
}

/// Build the `.mcp.json` for a Streamable-HTTP connection to kyma's MCP server.
fn mcp_json(url: &str, token: Option<&str>) -> Result<String> {
    let url = url.trim_end_matches('/');
    let mut server = json!({
        "type": "http",
        "url": format!("{url}/mcp/v1"),
    });
    if let Some(t) = token {
        if !t.is_empty() {
            server["headers"] = json!({ "Authorization": format!("Bearer {t}") });
        }
    }
    let doc = json!({ "mcpServers": { "kyma": server } });
    Ok(serde_json::to_string_pretty(&doc)?)
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

#[cfg(unix)]
fn set_private(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn set_private(_path: &Path) {}
