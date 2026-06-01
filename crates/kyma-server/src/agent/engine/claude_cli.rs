//! Claude CLI engine — drives the local Claude Code agent loop and streams
//! its answer back **token-by-token**.
//!
//! Unlike the other engines this one is *not* an `adk_rust::Llm` — Claude
//! Code already owns its own tool loop, MCPs, skills, OAuth, and CLAUDE.md
//! discovery. Asking adk-rust to wrap it would defeat the point. Instead
//! the agent route handler checks `EngineKind::ClaudeCli` first and routes
//! the request through [`run_stream`] below; only for the other kinds does
//! it fall back to the adk-rust runner.
//!
//! We spawn the local `claude` binary with `--print --output-format
//! stream-json --verbose --include-partial-messages` and parse the
//! newline-delimited JSON event stream. The `--include-partial-messages` flag
//! is what gives us real-time token deltas (`stream_event`) instead of one
//! buffered blob at the end — that buffering was the root cause of the "no
//! streaming" bug.
//!
//! We deserialize each line into the [`claude-code-agent-sdk`] crate's typed
//! [`Message`] model (its `StreamEvent` / `ContentBlock` types map the CLI
//! schema exactly), but we drive the subprocess ourselves rather than via the
//! crate's `query_stream`: that helper aborts the whole stream on the first
//! line it can't deserialize, and the real CLI interleaves frames the crate's
//! `Message` enum doesn't know (`rate_limit_event`, some `system` subtypes).
//! Parsing line-by-line lets us simply skip unknown frames and keep going.
//!
//! Authentication: the CLI handles itself. On macOS it reads the
//! `Claude Code-credentials` Keychain entry that the user already populated
//! by running `claude` interactively at least once. We pass nothing.

use std::collections::HashMap;
use std::process::Stdio;

use claude_code_agent_sdk::{ContentBlock, Message, ToolResultBlock, ToolResultContent};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{trace, warn};

/// Discover the CLI binary on `$PATH`. Returns `None` if `claude` isn't
/// installed — the catalogue uses this to decide whether to advertise the
/// engine in the picker.
pub fn locate_binary() -> Option<std::path::PathBuf> {
    which::which("claude").ok()
}

pub fn default_models() -> Vec<String> {
    // Same shortlist Claude Code itself accepts via --model. The string is
    // forwarded verbatim, so adding a new alias here is purely cosmetic.
    vec![
        "claude-opus-4-8".into(),
        "claude-sonnet-4-6".into(),
        "claude-haiku-4-5".into(),
        "default".into(),
    ]
}

/// A high-level event produced by [`run_stream`]. The route handler maps each
/// of these onto an AI-SDK UI Message Stream part. Text/thinking blocks carry
/// a `block_id` that is unique across the whole turn (a monotonic counter), so
/// downstream parts never collide even when the CLI reuses content-block
/// indices across multiple assistant messages.
#[derive(Debug, Clone)]
pub enum ClaudeEvent {
    /// First frame carrying Claude's own session id, which the client echoes
    /// back on the next turn to resume the conversation.
    Init { session_id: String },
    TextStart { block_id: String },
    TextDelta { block_id: String, text: String },
    TextEnd { block_id: String },
    ThinkingStart { block_id: String },
    ThinkingDelta { block_id: String, text: String },
    ThinkingEnd { block_id: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { id: String, output: Value, is_error: bool },
    /// Terminal success frame with run metadata.
    Result {
        session_id: String,
        total_cost_usd: Option<f64>,
        duration_ms: u64,
        num_turns: u32,
        is_error: bool,
    },
    /// Terminal error frame (spawn failure, transport error, CLI non-zero, …).
    Error { message: String },
}

/// How to reach Kyma's own MCP server so the agent can query the user's data.
///
/// When supplied to [`run_stream`], the `kyma` MCP server is registered with
/// the CLI (`--mcp-config`), its tools are auto-approved (`--allowedTools
/// mcp__kyma`), and a system-prompt hint nudges the model to use them.
#[derive(Clone, Debug)]
pub struct McpConfig {
    /// HTTP URL of the Kyma MCP endpoint, e.g. `http://127.0.0.1:8080/mcp/v1`.
    pub url: String,
    /// Full `Authorization` header value to present (e.g. `"Bearer …"`), or
    /// `None` when the server runs with auth disabled.
    pub auth_header: Option<String>,
}

/// Appended to the system prompt when the Kyma MCP server is wired, so the
/// model knows the data tools exist and prefers them over guessing.
const MCP_SYSTEM_HINT: &str = "You are connected to the user's Kyma data warehouse through the `kyma` MCP server. \
To answer questions about their data, use its tools — list_databases, describe_table, explore_schema, \
sample_rows, run_sql, run_kql, find_references_to, and graph_traverse — rather than guessing. \
Call `memory_search` FIRST when a question may depend on prior context, decisions, or how entities \
relate — it does graph-aware hybrid recall and returns connected resources you can follow with \
graph_traverse. Query the real data, then answer concisely based on what you find.";

#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    Text,
    Thinking,
}

struct BlockState {
    id: String,
    kind: BlockKind,
}

/// Spawn the Claude Code agent loop for one turn and stream [`ClaudeEvent`]s.
///
/// * `question` — the user's prompt for this turn.
/// * `model` — a `--model` alias, or `"default"`/`None` to let the CLI decide.
/// * `resume_session_id` — when `Some`, the turn resumes that Claude session
///   (`--resume`), preserving multi-turn context.
/// * `cwd` — working directory for the agent (`None` = inherit the server's).
/// * `mcp` — when `Some`, register Kyma's MCP server so the agent can query the
///   user's data; when `None`, the agent runs with no Kyma tools.
///
/// The returned receiver closes when the turn completes; the final event is
/// always either [`ClaudeEvent::Result`] or [`ClaudeEvent::Error`].
pub fn run_stream(
    question: &str,
    model: Option<&str>,
    resume_session_id: Option<&str>,
    cwd: Option<&std::path::Path>,
    mcp: Option<&McpConfig>,
) -> anyhow::Result<mpsc::UnboundedReceiver<ClaudeEvent>> {
    let binary = locate_binary()
        .ok_or_else(|| anyhow::anyhow!("`claude` not found on PATH — install Claude Code"))?;

    let mut cmd = Command::new(&binary);
    cmd.arg("--print")
        .arg("--output-format")
        .arg("stream-json")
        // Required alongside stream-json; emits the token-level deltas.
        .arg("--include-partial-messages")
        // stream-json requires --verbose for the full event stream.
        .arg("--verbose");
    if let Some(m) = model {
        if m != "default" {
            cmd.arg("--model").arg(m);
        }
    }
    if let Some(sid) = resume_session_id {
        cmd.arg("--resume").arg(sid);
    }

    // Register Kyma's MCP server (data tools) for this turn. The config is
    // written to a 0600 temp file rather than passed inline so the bearer token
    // never lands in the process arg list (visible via `ps`). The file is
    // removed once the child exits.
    let mcp_config_path = match mcp {
        Some(cfg) => match write_mcp_config(cfg) {
            Ok(path) => {
                cmd.arg("--mcp-config").arg(&path);
                // Auto-approve every `kyma` MCP tool (no interactive prompt in
                // --print mode); other tools stay default-denied.
                cmd.arg("--allowedTools").arg("mcp__kyma");
                cmd.arg("--append-system-prompt").arg(MCP_SYSTEM_HINT);
                Some(path)
            }
            Err(e) => {
                warn!(error = %e, "claude_cli: failed to write MCP config; running without Kyma tools");
                None
            }
        },
        None => None,
    };

    cmd.arg(question);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("spawn `claude`: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("no stderr"))?;

    let (tx, rx) = mpsc::unbounded_channel::<ClaudeEvent>();

    // Drain stderr to the log so transient CLI errors aren't lost.
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.trim().is_empty() {
                warn!(line = %line, "claude_cli stderr");
            }
        }
    });

    tokio::spawn(async move {
        // Per-turn state: a monotonic block counter and the active content
        // blocks keyed by their CLI content-block index.
        let mut next_block = 0u64;
        let mut blocks: HashMap<u64, BlockState> = HashMap::new();
        let mut session_id = String::new();
        let mut saw_result = false;

        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break, // EOF
                Err(e) => {
                    let _ = tx.send(ClaudeEvent::Error {
                        message: format!("claude stdout read error: {e}"),
                    });
                    saw_result = true;
                    break;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Parse as JSON, then into the crate's typed Message. Frames the
            // Message enum doesn't model (rate_limit_event, etc.) simply don't
            // deserialize — skip them instead of aborting the stream.
            let value: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    trace!(error = %e, "claude_cli: non-JSON line skipped");
                    continue;
                }
            };
            let msg: Message = match serde_json::from_value(value) {
                Ok(m) => m,
                Err(e) => {
                    trace!(error = %e, "claude_cli: unmodeled frame skipped");
                    continue;
                }
            };

            match msg {
                Message::System(sys) => {
                    if let Some(sid) = sys.session_id {
                        if session_id.is_empty() {
                            session_id = sid.clone();
                            let _ = tx.send(ClaudeEvent::Init { session_id: sid });
                        }
                    }
                }
                // Partial token deltas — the real-time path.
                Message::StreamEvent(se) => {
                    if session_id.is_empty() && !se.session_id.is_empty() {
                        session_id = se.session_id.clone();
                    }
                    handle_stream_event(&se.event, &tx, &mut next_block, &mut blocks);
                }
                // Complete assistant message: we've already streamed its text
                // and thinking via StreamEvent deltas, so we only mine it for
                // tool-use blocks (which don't arrive as text deltas).
                Message::Assistant(a) => {
                    for block in a.message.content {
                        if let ContentBlock::ToolUse(tu) = block {
                            let _ = tx.send(ClaudeEvent::ToolUse {
                                id: tu.id,
                                name: tu.name,
                                input: tu.input,
                            });
                        }
                    }
                }
                // Tool results come back as user messages with tool_result blocks.
                Message::User(u) => {
                    if let Some(content) = u.content {
                        for block in content {
                            if let ContentBlock::ToolResult(tr) = block {
                                let (output, is_err) = tool_result_payload(&tr);
                                let _ = tx.send(ClaudeEvent::ToolResult {
                                    id: tr.tool_use_id,
                                    output,
                                    is_error: is_err,
                                });
                            }
                        }
                    }
                }
                Message::Result(r) => {
                    if session_id.is_empty() {
                        session_id = r.session_id.clone();
                    }
                    let _ = tx.send(ClaudeEvent::Result {
                        session_id: r.session_id,
                        total_cost_usd: r.total_cost_usd,
                        duration_ms: r.duration_ms,
                        num_turns: r.num_turns,
                        is_error: r.is_error,
                    });
                    saw_result = true;
                }
                // Internal control protocol — ignore.
                Message::ControlCancelRequest(_) => {}
            }
        }

        let status = child.wait().await;

        // The CLI has read the MCP config by now; drop the temp file.
        if let Some(path) = &mcp_config_path {
            let _ = std::fs::remove_file(path);
        }

        // Defensive: if the stream ended without a terminal frame, synthesize
        // one so the route handler always closes the SSE cleanly. Surface a
        // non-zero exit as an error only when we never saw a result.
        if !saw_result {
            match status {
                Ok(s) if !s.success() => {
                    let _ = tx.send(ClaudeEvent::Error {
                        message: format!(
                            "claude exited with {}",
                            s.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into())
                        ),
                    });
                }
                _ => {
                    let _ = tx.send(ClaudeEvent::Result {
                        session_id,
                        total_cost_usd: None,
                        duration_ms: 0,
                        num_turns: 0,
                        is_error: false,
                    });
                }
            }
        }
    });

    Ok(rx)
}

/// Translate one raw Anthropic streaming event (the `event` field of a
/// `Message::StreamEvent`) into [`ClaudeEvent`] text/thinking deltas.
fn handle_stream_event(
    event: &Value,
    tx: &mpsc::UnboundedSender<ClaudeEvent>,
    next_block: &mut u64,
    blocks: &mut HashMap<u64, BlockState>,
) {
    let Some(kind) = event.get("type").and_then(Value::as_str) else {
        return;
    };
    match kind {
        "content_block_start" => {
            let idx = event.get("index").and_then(Value::as_u64).unwrap_or(0);
            let block_type = event
                .get("content_block")
                .and_then(|b| b.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match block_type {
                "text" => {
                    let id = register_block(idx, BlockKind::Text, next_block, blocks);
                    let _ = tx.send(ClaudeEvent::TextStart { block_id: id });
                }
                "thinking" => {
                    let id = register_block(idx, BlockKind::Thinking, next_block, blocks);
                    let _ = tx.send(ClaudeEvent::ThinkingStart { block_id: id });
                }
                // tool_use blocks are emitted from the complete Assistant message.
                _ => {}
            }
        }
        "content_block_delta" => {
            let idx = event.get("index").and_then(Value::as_u64).unwrap_or(0);
            let delta = event.get("delta");
            let delta_type = delta
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match delta_type {
                "text_delta" => {
                    if let Some(text) = delta.and_then(|d| d.get("text")).and_then(Value::as_str) {
                        let id = ensure_block(idx, BlockKind::Text, next_block, blocks, tx);
                        let _ = tx.send(ClaudeEvent::TextDelta {
                            block_id: id,
                            text: text.to_string(),
                        });
                    }
                }
                "thinking_delta" => {
                    if let Some(text) =
                        delta.and_then(|d| d.get("thinking")).and_then(Value::as_str)
                    {
                        if text.is_empty() {
                            return;
                        }
                        let id = ensure_block(idx, BlockKind::Thinking, next_block, blocks, tx);
                        let _ = tx.send(ClaudeEvent::ThinkingDelta {
                            block_id: id,
                            text: text.to_string(),
                        });
                    }
                }
                // input_json_delta (streaming tool args) and signature_delta
                // are ignored — tool input is taken whole from the Assistant
                // message, and signatures aren't user-visible.
                _ => {}
            }
        }
        "content_block_stop" => {
            let idx = event.get("index").and_then(Value::as_u64).unwrap_or(0);
            if let Some(state) = blocks.remove(&idx) {
                match state.kind {
                    BlockKind::Text => {
                        let _ = tx.send(ClaudeEvent::TextEnd { block_id: state.id });
                    }
                    BlockKind::Thinking => {
                        let _ = tx.send(ClaudeEvent::ThinkingEnd { block_id: state.id });
                    }
                }
            }
        }
        _ => {}
    }
}

fn register_block(
    idx: u64,
    kind: BlockKind,
    next_block: &mut u64,
    blocks: &mut HashMap<u64, BlockState>,
) -> String {
    let id = format!("b{next_block}");
    *next_block += 1;
    blocks.insert(idx, BlockState { id: id.clone(), kind });
    id
}

/// Return the id for an open block at `idx`, opening one (and emitting the
/// matching start event) if none exists yet.
fn ensure_block(
    idx: u64,
    kind: BlockKind,
    next_block: &mut u64,
    blocks: &mut HashMap<u64, BlockState>,
    tx: &mpsc::UnboundedSender<ClaudeEvent>,
) -> String {
    if let Some(state) = blocks.get(&idx) {
        return state.id.clone();
    }
    let id = register_block(idx, kind, next_block, blocks);
    match kind {
        BlockKind::Text => {
            let _ = tx.send(ClaudeEvent::TextStart {
                block_id: id.clone(),
            });
        }
        BlockKind::Thinking => {
            let _ = tx.send(ClaudeEvent::ThinkingStart {
                block_id: id.clone(),
            });
        }
    }
    id
}

/// Write the Claude `--mcp-config` JSON for the Kyma MCP server to a private
/// (0600) temp file and return its path. Using a file keeps the bearer token
/// out of the process arg list.
fn write_mcp_config(cfg: &McpConfig) -> anyhow::Result<std::path::PathBuf> {
    use std::io::Write;

    let mut server = json!({ "type": "http", "url": cfg.url });
    if let Some(auth) = &cfg.auth_header {
        server["headers"] = json!({ "Authorization": auth });
    }
    let config = json!({ "mcpServers": { "kyma": server } });

    let path = std::env::temp_dir().join(format!("kyma-mcp-{}.json", uuid::Uuid::new_v4()));
    let mut file = std::fs::File::create(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(serde_json::to_string(&config)?.as_bytes())?;
    Ok(path)
}

/// Normalize a tool-result block into a JSON output value + error flag.
fn tool_result_payload(tr: &ToolResultBlock) -> (Value, bool) {
    let is_err = tr.is_error.unwrap_or(false);
    let output = match &tr.content {
        Some(ToolResultContent::Text(s)) => Value::String(s.clone()),
        Some(ToolResultContent::Blocks(blocks)) => Value::Array(blocks.clone()),
        None => json!(null),
    };
    (output, is_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect the events produced by feeding a sequence of raw streaming
    /// events through [`handle_stream_event`].
    fn run(events: &[Value]) -> Vec<ClaudeEvent> {
        let (tx, mut rx) = mpsc::unbounded_channel::<ClaudeEvent>();
        let mut next_block = 0u64;
        let mut blocks: HashMap<u64, BlockState> = HashMap::new();
        for ev in events {
            handle_stream_event(ev, &tx, &mut next_block, &mut blocks);
        }
        drop(tx);
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    #[test]
    fn text_block_streams_start_delta_end_with_stable_id() {
        let evs = run(&[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}),
            json!({"type":"content_block_stop","index":0}),
        ]);
        match evs.as_slice() {
            [
                ClaudeEvent::TextStart { block_id: a },
                ClaudeEvent::TextDelta { block_id: b, text: t1 },
                ClaudeEvent::TextDelta { block_id: c, text: t2 },
                ClaudeEvent::TextEnd { block_id: d },
            ] => {
                assert_eq!(a, b);
                assert_eq!(a, c);
                assert_eq!(a, d);
                assert_eq!(t1, "Hel");
                assert_eq!(t2, "lo");
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn thinking_deltas_map_to_thinking_events_and_skip_empty_and_signature() {
        let evs = run(&[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reasoning"}}),
            json!({"type":"content_block_stop","index":0}),
        ]);
        match evs.as_slice() {
            [
                ClaudeEvent::ThinkingStart { .. },
                ClaudeEvent::ThinkingDelta { text, .. },
                ClaudeEvent::ThinkingEnd { .. },
            ] => assert_eq!(text, "reasoning"),
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn reused_block_index_across_messages_gets_distinct_ids() {
        let evs = run(&[
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}),
            json!({"type":"content_block_stop","index":0}),
            // Second assistant message reuses index 0.
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"b"}}),
            json!({"type":"content_block_stop","index":0}),
        ]);
        let ids: Vec<&str> = evs
            .iter()
            .filter_map(|e| match e {
                ClaudeEvent::TextStart { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "reused index must not collide");
    }

    #[test]
    fn mcp_config_file_has_http_server_with_auth_and_is_private() {
        let cfg = McpConfig {
            url: "http://127.0.0.1:8080/mcp/v1".into(),
            auth_header: Some("Bearer secret-token".into()),
        };
        let path = write_mcp_config(&cfg).expect("write config");
        let body = std::fs::read_to_string(&path).expect("read config");
        let v: Value = serde_json::from_str(&body).unwrap();

        let server = &v["mcpServers"]["kyma"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], "http://127.0.0.1:8080/mcp/v1");
        assert_eq!(server["headers"]["Authorization"], "Bearer secret-token");

        // The token must not be world-readable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "config file must be 0600");
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mcp_config_omits_auth_header_when_absent() {
        let cfg = McpConfig {
            url: "http://127.0.0.1:8080/mcp/v1".into(),
            auth_header: None,
        };
        let path = write_mcp_config(&cfg).expect("write config");
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v["mcpServers"]["kyma"].get("headers").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unmodeled_frames_do_not_deserialize_into_message() {
        // These are the frames the real CLI interleaves that the crate's
        // Message enum doesn't model — they must be skippable (Err), which is
        // exactly why we parse line-by-line instead of via query_stream.
        for frame in [
            json!({"type":"rate_limit_event","rate_limit_info":{}}),
            json!({"type":"totally_unknown"}),
        ] {
            assert!(
                serde_json::from_value::<Message>(frame.clone()).is_err(),
                "expected {frame} to be unmodeled"
            );
        }
        // A normal assistant frame still parses.
        let ok = json!({"type":"assistant","message":{"content":[]},"session_id":"s"});
        assert!(serde_json::from_value::<Message>(ok).is_ok());
    }
}
