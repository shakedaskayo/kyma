//! HTTP handlers for `POST /v1/agent/ask` (SSE), `GET /v1/agent/runs/:run_id`,
//! and the engine-management surface (`GET/PUT /v1/agent/engine`, etc.).
//!
//! The ask handler drives an ADK-Rust `Runner` and emits a stream of SSE
//! frames. Once the stream completes (naturally, on error, or on budget
//! overrun) we insert one row into `agent_runs` with the full trace as JSONB.

use std::convert::Infallible;
use std::time::{Duration, Instant};

use adk_rust::futures::StreamExt;
use adk_rust::identity::{SessionId, UserId};
use adk_rust::{Content, Part};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use chrono::Utc;
use futures::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::types::Json as SqlxJson;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{debug, error, info, warn};

use super::engine::{
    build_engine, claude_cli, engine_catalogue, CredentialResolver, EngineConfig, EngineKind,
};
use super::runner::{make_runner, model_id, ANON_USER};
use super::sessions;
use super::state::AgentState;

/// Hard cap on tool-call count per run. Above this, the turn is aborted
/// with `run_error{code="tool_loop"}` and persisted as `budget_exceeded`.
const MAX_TOOL_CALLS: u32 = 12;

/// Wall-clock deadline for the whole SSE exchange.
const RUN_WALL_CLOCK: Duration = Duration::from_secs(60);

/// Default number of new turns after which a session's rolling summary is
/// refreshed. Overridable via `KYMA_SESSION_SUMMARY_EVERY`.
const DEFAULT_SUMMARY_EVERY: i32 = 12;

fn summary_every() -> i32 {
    std::env::var("KYMA_SESSION_SUMMARY_EVERY")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SUMMARY_EVERY)
}

pub fn router(state: AgentState) -> axum::Router {
    axum::Router::new()
        .route("/ask", post(ask_handler))
        .route("/runs/:run_id", get(run_lookup_handler))
        .route("/sessions", get(list_sessions_handler))
        .route(
            "/sessions/:session_id",
            get(get_session_handler).delete(delete_session_handler),
        )
        .route("/sessions/:session_id/turns", get(get_session_turns_handler))
        .route("/engines", get(list_engines))
        .route("/engine", get(get_engine).put(put_engine))
        .route("/engine/test", post(test_engine))
        .route("/skills", get(list_skills))
        .route("/skills/enabled", get(get_enabled_skills).put(put_enabled_skills))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct AskRequest {
    question: String,
    #[serde(default)]
    #[allow(dead_code)]
    database: Option<String>,
    #[serde(default)]
    include_thinking: bool,
    /// Optional conversation session. When present, prior turns are replayed
    /// for context and this turn is appended. A new id is minted when absent.
    #[serde(default)]
    session_id: Option<String>,
    /// Marks the session source (e.g. `"claude_code"` for hook-driven asks).
    #[serde(default)]
    source: Option<String>,
}

/// One recorded SSE frame — we stash both the user-facing event name and
/// the JSON body so we can persist the full trace into `agent_runs`
/// after the stream completes.
#[derive(Debug, Clone)]
struct TraceFrame {
    event: &'static str,
    data: Value,
}

/// Emit a trace frame both into the SSE sender **and** into the trace vec
/// that will later be persisted. Parallel tee isolates the two concerns.
fn emit(
    tx: &mpsc::UnboundedSender<Result<SseEvent, Infallible>>,
    trace: &mut Vec<TraceFrame>,
    event: &'static str,
    data: Value,
) {
    trace.push(TraceFrame {
        event,
        data: data.clone(),
    });
    let body = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());
    // Best-effort: if the client has hung up we just keep walking the
    // stream until the budget fires — the cancellation is cooperative.
    let _ = tx.send(Ok(SseEvent::default().event(event).data(body)));
}

/// POST /v1/agent/ask — run one agent turn and stream SSE frames.
async fn ask_handler(State(state): State<AgentState>, Json(body): Json<AskRequest>) -> Response {
    let question = body.question.trim().to_string();
    if question.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "question must be non-empty"})),
        )
            .into_response();
    }

    // Engine-kind branch: the Claude CLI engine owns its own tool loop, OAuth,
    // skills, and MCPs — adk-rust can't wrap it. Divert here before building
    // the runner.
    if let Ok(cfg) = state.engines.get().await {
        if cfg.kind == EngineKind::ClaudeCli {
            return ask_via_claude_cli(state.pool.clone(), &question, &cfg).await;
        }
    }

    let run_id = uuid::Uuid::new_v4();
    let include_thinking = body.include_thinking;
    let model = model_id(&state).await;
    let started_at = Utc::now();
    let start = Instant::now();

    // Resolve (or create) the conversation session (A5). Prior turns are
    // replayed into the runner; this turn is appended below.
    let source = body.source.as_deref().unwrap_or("kyma");
    let tenant_uuid = state.tenant.as_uuid();
    let sctx = sessions::load_or_create(
        &state.pool,
        body.session_id.as_deref(),
        tenant_uuid,
        ANON_USER,
        source,
    )
    .await;
    let session_uuid = sctx.session_id;
    let session_id_str = session_uuid.to_string();
    let user_turn_index = sctx.next_turn_index;
    let assistant_turn_index = user_turn_index + 1;

    info!(run_id = %run_id, session_id = %session_uuid, question = %question, "agent run starting");

    // Record the user turn up-front so it survives even if the run errors.
    sessions::persist_turn(
        &state.pool,
        session_uuid,
        tenant_uuid,
        user_turn_index,
        "user",
        &question,
        None,
    )
    .await;

    // Build runner up-front so we can surface init errors via an
    // inline-but-single-event SSE stream (rather than an HTTP 500).
    let runner = match make_runner(&state, &session_id_str, &sctx.history, sctx.summary.as_deref())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(run_id = %run_id, error = %e, "failed to build agent runner");
            let payload = json!({
                "run_id": run_id.to_string(),
                "code": "init_error",
                "message": e.to_string(),
            });
            let events = vec![
                Ok::<_, Infallible>(
                    SseEvent::default().event("session").data(
                        serde_json::to_string(&json!({ "session_id": session_id_str.clone() }))
                            .unwrap_or_default(),
                    ),
                ),
                Ok(SseEvent::default()
                    .event("run_error")
                    .data(serde_json::to_string(&payload).unwrap_or_default())),
                Ok(SseEvent::default().event("run_finished").data(
                    serde_json::to_string(&json!({
                        "run_id": run_id.to_string(),
                        "elapsed_ms": start.elapsed().as_millis() as u64,
                        "tool_calls": 0,
                    }))
                    .unwrap_or_default(),
                )),
            ];
            // Best-effort persistence — ignore errors here since the
            // primary surface (SSE) already told the client.
            let _ = persist_run(
                &state.pool,
                run_id,
                &question,
                &model,
                tenant_uuid,
                Some(session_uuid),
                started_at,
                Utc::now(),
                "error",
                &Value::Null,
                &json!([{ "event": "run_error", "data": payload }]),
            )
            .await;
            return Sse::new(stream::iter(events))
                .keep_alive(KeepAlive::default())
                .into_response();
        }
    };

    // Spawn the driver task. The task owns the runner and the event loop;
    // the response stream is a `tokio_stream::UnboundedReceiverStream` over
    // the SSE events the task produces.
    let (tx, rx) = mpsc::unbounded_channel::<Result<SseEvent, Infallible>>();
    let pool = state.pool.clone();
    let summary_state = state.clone();
    let summary_every = summary_every();

    tokio::spawn(async move {
        let mut trace: Vec<TraceFrame> = Vec::new();
        let mut tool_calls: u32 = 0;
        let mut last_run_sql: Option<String> = None;
        let mut final_text: String = String::new();

        // Surface the session id first so the client can capture it even if
        // the run errors mid-stream.
        emit(
            &tx,
            &mut trace,
            "session",
            json!({ "session_id": session_id_str.clone() }),
        );

        emit(
            &tx,
            &mut trace,
            "run_started",
            json!({
                "run_id": run_id.to_string(),
                "model": model,
                "question": question,
            }),
        );

        let content = Content::new("user").with_text(&question);

        let user_id = match UserId::new(ANON_USER) {
            Ok(u) => u,
            Err(e) => {
                emit(
                    &tx,
                    &mut trace,
                    "run_error",
                    json!({
                        "code": "internal",
                        "message": format!("user_id: {e}"),
                    }),
                );
                finish_and_persist(
                    &pool, &tx, run_id, &question, &model, tenant_uuid, Some(session_uuid),
                    started_at, start, tool_calls, "error", &trace,
                )
                .await;
                return;
            }
        };
        let session_id = match SessionId::new(&session_id_str) {
            Ok(s) => s,
            Err(e) => {
                emit(
                    &tx,
                    &mut trace,
                    "run_error",
                    json!({
                        "code": "internal",
                        "message": format!("session_id: {e}"),
                    }),
                );
                finish_and_persist(
                    &pool, &tx, run_id, &question, &model, tenant_uuid, Some(session_uuid),
                    started_at, start, tool_calls, "error", &trace,
                )
                .await;
                return;
            }
        };

        let run_future = async {
            let mut stream = match runner.run(user_id, session_id, content).await {
                Ok(s) => s,
                Err(e) => {
                    return Err(format!("runner.run: {e}"));
                }
            };

            while let Some(ev_result) = stream.next().await {
                let ev = match ev_result {
                    Ok(e) => e,
                    Err(e) => return Err(format!("event: {e}")),
                };

                let partial = ev.llm_response.partial;
                let parts_iter = ev
                    .llm_response
                    .content
                    .iter()
                    .flat_map(|c| c.parts.iter().cloned())
                    .collect::<Vec<Part>>();

                for part in parts_iter {
                    match part {
                        Part::Text { text } => {
                            if partial {
                                emit(&tx, &mut trace, "answer_delta", json!({"text": text}));
                            } else {
                                // Non-partial Text means we have the
                                // complete (possibly aggregated) turn
                                // answer. Stash for `answer_final`.
                                final_text.push_str(&text);
                                emit(&tx, &mut trace, "answer_delta", json!({"text": text}));
                            }
                        }
                        Part::Thinking { thinking, .. } => {
                            if include_thinking {
                                emit(&tx, &mut trace, "thinking_delta", json!({"text": thinking}));
                            }
                        }
                        Part::FunctionCall { name, args, .. } => {
                            tool_calls += 1;
                            if name == "run_sql" {
                                if let Some(s) = args.get("sql").and_then(|v| v.as_str()) {
                                    last_run_sql = Some(s.to_string());
                                }
                            }
                            emit(
                                &tx,
                                &mut trace,
                                "tool_call",
                                json!({
                                    "tool": name,
                                    "args": args,
                                    "call_index": tool_calls,
                                }),
                            );
                            if tool_calls > MAX_TOOL_CALLS {
                                return Err(format!("tool_loop:{}", tool_calls));
                            }
                        }
                        Part::FunctionResponse {
                            function_response, ..
                        } => {
                            emit(
                                &tx,
                                &mut trace,
                                "tool_result",
                                json!({
                                    "tool": function_response.name,
                                    "result": function_response.response,
                                }),
                            );
                        }
                        _ => {}
                    }
                }

                if ev.is_final_response() {
                    // Fold in any final text even if it arrived as a
                    // non-partial full text part above. Nothing to do
                    // here — we've already pushed it to `final_text`.
                    debug!(run_id = %run_id, "agent emitted is_final_response");
                }
            }
            Ok::<(), String>(())
        };

        let outcome = tokio::time::timeout(RUN_WALL_CLOCK, run_future).await;

        let (status, status_str): (&str, &str) = match outcome {
            Ok(Ok(())) => ("success", "success"),
            Ok(Err(msg)) if msg.starts_with("tool_loop:") => {
                emit(
                    &tx,
                    &mut trace,
                    "run_error",
                    json!({
                        "code": "tool_loop",
                        "message": msg,
                    }),
                );
                ("budget_exceeded", "budget_exceeded")
            }
            Ok(Err(msg)) => {
                emit(
                    &tx,
                    &mut trace,
                    "run_error",
                    json!({
                        "code": "runner_error",
                        "message": msg,
                    }),
                );
                ("error", "error")
            }
            Err(_elapsed) => {
                warn!(run_id = %run_id, "agent run exceeded 60s wall clock");
                emit(
                    &tx,
                    &mut trace,
                    "run_error",
                    json!({
                        "code": "timeout",
                        "message": format!(
                            "agent run exceeded {}s wall clock budget",
                            RUN_WALL_CLOCK.as_secs()
                        ),
                    }),
                );
                ("budget_exceeded", "budget_exceeded")
            }
        };

        if status == "success" {
            emit(
                &tx,
                &mut trace,
                "answer_final",
                json!({
                    "text": final_text,
                    "kql_used": Value::Null,
                    "sql_used": last_run_sql,
                }),
            );
            // Record the assistant turn, then refresh the rolling summary.
            sessions::persist_turn(
                &pool,
                session_uuid,
                tenant_uuid,
                assistant_turn_index,
                "assistant",
                &final_text,
                None,
            )
            .await;
            sessions::maybe_summarize_detached(summary_state, session_uuid, summary_every);
        }

        finish_and_persist(
            &pool, &tx, run_id, &question, &model, tenant_uuid, Some(session_uuid), started_at,
            start, tool_calls, status_str, &trace,
        )
        .await;
    });

    let stream = UnboundedReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[allow(clippy::too_many_arguments)]
async fn finish_and_persist(
    pool: &PgPool,
    tx: &mpsc::UnboundedSender<Result<SseEvent, Infallible>>,
    run_id: uuid::Uuid,
    question: &str,
    model: &str,
    tenant: uuid::Uuid,
    session_id: Option<uuid::Uuid>,
    started_at: chrono::DateTime<Utc>,
    start: Instant,
    tool_calls: u32,
    status: &str,
    trace: &[TraceFrame],
) {
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let finished = json!({
        "run_id": run_id.to_string(),
        "usage": {},
        "elapsed_ms": elapsed_ms,
        "tool_calls": tool_calls,
    });
    let body = serde_json::to_string(&finished).unwrap_or_default();
    let _ = tx.send(Ok(SseEvent::default().event("run_finished").data(body)));

    let trace_json = Value::Array(
        trace
            .iter()
            .map(|f| {
                json!({
                    "event": f.event,
                    "data": f.data,
                })
            })
            .collect(),
    );
    let usage_json = json!({
        "tool_calls": tool_calls,
        "elapsed_ms": elapsed_ms,
    });

    if let Err(e) = persist_run(
        pool,
        run_id,
        question,
        model,
        tenant,
        session_id,
        started_at,
        Utc::now(),
        status,
        &usage_json,
        &trace_json,
    )
    .await
    {
        error!(run_id = %run_id, error = %e, "failed to persist agent_runs row");
    }
}

#[allow(clippy::too_many_arguments)]
async fn persist_run(
    pool: &PgPool,
    run_id: uuid::Uuid,
    question: &str,
    model_id: &str,
    tenant: uuid::Uuid,
    session_id: Option<uuid::Uuid>,
    started_at: chrono::DateTime<Utc>,
    finished_at: chrono::DateTime<Utc>,
    status: &str,
    usage_json: &Value,
    trace_json: &Value,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO agent_runs (
            run_id, tenant_id, question, model_id, auth_subject,
            session_id, started_at, finished_at, status,
            usage_json, trace_json, replay_cache_hit
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
    )
    .bind(run_id)
    .bind(tenant)
    .bind(question)
    .bind(model_id)
    .bind(ANON_USER)
    .bind(session_id)
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(SqlxJson(usage_json.clone()))
    .bind(SqlxJson(trace_json.clone()))
    .bind(false)
    .execute(pool)
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// GET/DELETE /v1/agent/sessions* — multi-turn session surface (A5)
// ---------------------------------------------------------------------------

fn parse_session_id(raw: &str) -> Result<uuid::Uuid, Response> {
    uuid::Uuid::parse_str(raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid session_id (expected uuid)"})),
        )
            .into_response()
    })
}

async fn list_sessions_handler(State(state): State<AgentState>) -> Response {
    let rows: Vec<(
        uuid::Uuid,
        Option<String>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        String,
        i64,
    )> = match sqlx::query_as(
        r#"
        SELECT s.session_id, s.title, s.created_at, s.last_active, s.source,
               COUNT(t.turn_index) AS turn_count
        FROM agent_sessions s
        LEFT JOIN agent_session_turns t ON t.session_id = s.session_id
        GROUP BY s.session_id
        ORDER BY s.last_active DESC
        LIMIT 200
        "#,
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "list sessions failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let sessions: Vec<Value> = rows
        .into_iter()
        .map(|(id, title, created, last_active, source, turn_count)| {
            json!({
                "session_id": id.to_string(),
                "title": title,
                "created_at": created,
                "last_active": last_active,
                "source": source,
                "turn_count": turn_count,
            })
        })
        .collect();
    Json(json!({ "sessions": sessions })).into_response()
}

async fn get_session_handler(
    State(state): State<AgentState>,
    Path(session_id): Path<String>,
) -> Response {
    let sid = match parse_session_id(&session_id) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let row: Option<(
        Option<String>,
        Option<String>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        String,
        i32,
    )> = sqlx::query_as(
        "SELECT title, rolling_summary, created_at, last_active, source, summary_turn_index \
         FROM agent_sessions WHERE session_id = $1",
    )
    .bind(sid)
    .fetch_optional(&state.pool)
    .await
    .unwrap_or(None);
    match row {
        Some((title, summary, created, last_active, source, summary_idx)) => Json(json!({
            "session_id": sid.to_string(),
            "title": title,
            "rolling_summary": summary,
            "created_at": created,
            "last_active": last_active,
            "source": source,
            "summary_turn_index": summary_idx,
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response(),
    }
}

async fn get_session_turns_handler(
    State(state): State<AgentState>,
    Path(session_id): Path<String>,
) -> Response {
    let sid = match parse_session_id(&session_id) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let rows: Vec<(
        i32,
        String,
        SqlxJson<Value>,
        Option<uuid::Uuid>,
        chrono::DateTime<Utc>,
    )> = sqlx::query_as(
        "SELECT turn_index, role, content_json, run_id, created_at \
         FROM agent_session_turns WHERE session_id = $1 ORDER BY turn_index ASC",
    )
    .bind(sid)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let turns: Vec<Value> = rows
        .into_iter()
        .map(|(idx, role, content, run_id, created)| {
            json!({
                "turn_index": idx,
                "role": role,
                "content": content.0,
                "run_id": run_id.map(|r| r.to_string()),
                "created_at": created,
            })
        })
        .collect();
    Json(json!({ "session_id": sid.to_string(), "turns": turns })).into_response()
}

async fn delete_session_handler(
    State(state): State<AgentState>,
    Path(session_id): Path<String>,
) -> Response {
    let sid = match parse_session_id(&session_id) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    // Turns cascade via FK ON DELETE CASCADE. `agent_runs.session_id` is a plain
    // (non-FK) column, so detach it to avoid dangling references.
    let _ = sqlx::query("UPDATE agent_runs SET session_id = NULL WHERE session_id = $1")
        .bind(sid)
        .execute(&state.pool)
        .await;
    match sqlx::query("DELETE FROM agent_sessions WHERE session_id = $1")
        .bind(sid)
        .execute(&state.pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            Json(json!({"deleted": true, "session_id": sid.to_string()})).into_response()
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/agent/runs/:run_id
// ---------------------------------------------------------------------------

async fn run_lookup_handler(
    State(state): State<AgentState>,
    Path(run_id): Path<String>,
) -> Response {
    let uid = match uuid::Uuid::parse_str(&run_id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid run_id (expected uuid)"})),
            )
                .into_response();
        }
    };

    let row: Option<(
        String,
        String,
        String,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        SqlxJson<Value>,
        SqlxJson<Value>,
    )> = match sqlx::query_as(
        r#"
        SELECT question, model_id, status, started_at, finished_at,
               usage_json, trace_json
        FROM agent_runs
        WHERE run_id = $1
        "#,
    )
    .bind(uid)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(run_id = %run_id, error = %e, "agent_runs lookup failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let Some((question, model, status, started_at, finished_at, usage, trace)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "run not found", "run_id": run_id})),
        )
            .into_response();
    };
    Json(json!({
        "run_id": run_id,
        "question": question,
        "model_id": model,
        "status": status,
        "started_at": started_at,
        "finished_at": finished_at,
        "usage": usage.0,
        "trace": trace.0,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Engine management routes
// ---------------------------------------------------------------------------

/// `GET /engines` — list available providers + their default models + the active config.
async fn list_engines(
    State(state): State<AgentState>,
) -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let active = state
        .engines
        .get()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Pass the active Ollama host (if configured) so the catalogue's live
    // `/api/tags` fetch hits the user's actual instance, not the default.
    let ollama_hint = active.host.as_deref();
    let catalogue = engine_catalogue(ollama_hint).await;
    Ok(axum::Json(serde_json::json!({
        "available": catalogue,
        "active": active,
    })))
}

/// `GET /engine` — the persisted EngineConfig (one global row, v1).
async fn get_engine(
    State(state): State<AgentState>,
) -> Result<axum::Json<EngineConfig>, (axum::http::StatusCode, String)> {
    state
        .engines
        .get()
        .await
        .map(axum::Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `PUT /engine` — save a new EngineConfig.
async fn put_engine(
    State(state): State<AgentState>,
    axum::Json(cfg): axum::Json<EngineConfig>,
) -> Result<axum::Json<EngineConfig>, (axum::http::StatusCode, String)> {
    state
        .engines
        .put(&cfg)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(cfg))
}

/// `POST /engine/test` — dry-run probe against a candidate config without
/// persisting it. Confirms creds resolve + the provider responds.
async fn test_engine(
    State(state): State<AgentState>,
    axum::Json(cfg): axum::Json<EngineConfig>,
) -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    use adk_rust::futures::StreamExt;
    use adk_rust::{GenerateContentConfig, LlmRequest};

    // Claude CLI engine: spawn the binary with a 1-token prompt and confirm
    // it produces *any* stdout + exits 0. Bypasses build_engine entirely
    // because the CLI doesn't go through adk-rust.
    if cfg.kind == EngineKind::ClaudeCli {
        let probe = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let (mut chunks, wait) = claude_cli::run("ping", Some(&cfg.model), None)
                .map_err(|e| format!("spawn: {e}"))?;
            let mut got_output = false;
            while let Some(chunk) = chunks.recv().await {
                if matches!(chunk, claude_cli::Chunk::Stdout(_)) {
                    got_output = true;
                }
            }
            let code = match wait.await {
                Ok(Ok(c)) => c,
                Ok(Err(e)) => return Err(format!("io: {e}")),
                Err(e) => return Err(format!("join: {e}")),
            };
            if code != 0 {
                return Err(format!("claude exited with {code}"));
            }
            if !got_output {
                return Err("claude produced no output".to_string());
            }
            Ok::<(), String>(())
        })
        .await;

        return match probe {
            Ok(Ok(())) => Ok(axum::Json(serde_json::json!({
                "ok": true,
                "kind": cfg.kind,
                "model": cfg.model,
            }))),
            Ok(Err(msg)) => Err((axum::http::StatusCode::BAD_GATEWAY, msg)),
            Err(_elapsed) => Err((
                axum::http::StatusCode::GATEWAY_TIMEOUT,
                "claude_cli probe timed out after 30s".into(),
            )),
        };
    }

    // adk-rust path for Anthropic / OpenAI / Ollama.
    let resolver = CredentialResolver::new(state.credentials.clone(), state.tenant);
    let key = resolver
        .resolve(&cfg)
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, format!("credential: {e}")))?;
    let llm = build_engine(&cfg, key)
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, format!("init: {e}")))?;

    let req = LlmRequest {
        model: cfg.model.clone(),
        contents: vec![Content::new("user").with_text("ping")],
        config: Some(GenerateContentConfig {
            max_output_tokens: Some(1),
            ..Default::default()
        }),
        tools: Default::default(),
    };

    let probe = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut stream = llm
            .generate_content(req, false)
            .await
            .map_err(|e| format!("provider: {e:?}"))?;
        while let Some(item) = stream.next().await {
            item.map_err(|e| format!("stream: {e:?}"))?;
            break;
        }
        Ok::<(), String>(())
    })
    .await;

    match probe {
        Ok(Ok(())) => Ok(axum::Json(serde_json::json!({
            "ok": true,
            "kind": cfg.kind,
            "model": cfg.model,
        }))),
        Ok(Err(msg)) => Err((axum::http::StatusCode::BAD_GATEWAY, msg)),
        Err(_elapsed) => Err((
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            "probe timed out after 30s".into(),
        )),
    }
}

// Satisfy dead-code lint: Stream is only used implicitly via Sse::new.
#[allow(dead_code)]
fn _type_check<S>(_: S)
where
    S: Stream<Item = Result<SseEvent, Infallible>>,
{
}

// ---------------------------------------------------------------------------
// Skill management routes
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct SkillRow {
    name: String,
    description: String,
    source: super::skills::SkillSource,
    path: String,
    enabled: bool,
    /// Truncated body preview for the UI tooltip.
    preview: String,
}

/// `GET /skills` — list every discovered skill on the host, plus its enabled
/// state. Discovery is cheap (filesystem walk), so we run it on every request
/// rather than caching.
async fn list_skills(
    State(state): State<AgentState>,
) -> Result<axum::Json<Vec<SkillRow>>, (axum::http::StatusCode, String)> {
    let enabled = state
        .skills
        .get()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let enabled_set: std::collections::HashSet<&str> =
        enabled.iter().map(String::as_str).collect();

    let mut rows: Vec<SkillRow> = super::skills::discover_all()
        .into_iter()
        .map(|s| {
            let preview = preview_body(&s.body);
            SkillRow {
                enabled: enabled_set.contains(s.name.as_str()),
                name: s.name,
                description: s.description,
                source: s.source,
                path: s.path,
                preview,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(axum::Json(rows))
}

/// `GET /skills/enabled` — just the toggled set, for callers that don't need
/// the full discovery payload.
async fn get_enabled_skills(
    State(state): State<AgentState>,
) -> Result<axum::Json<Vec<String>>, (axum::http::StatusCode, String)> {
    state
        .skills
        .get()
        .await
        .map(axum::Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Debug, serde::Deserialize)]
struct PutEnabledSkillsBody {
    skills: Vec<String>,
}

/// `PUT /skills/enabled` — replace the toggled set wholesale.
async fn put_enabled_skills(
    State(state): State<AgentState>,
    axum::Json(body): axum::Json<PutEnabledSkillsBody>,
) -> Result<axum::Json<Vec<String>>, (axum::http::StatusCode, String)> {
    state
        .skills
        .put(&body.skills)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(body.skills))
}

fn preview_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= 200 {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(200).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Claude CLI engine — spawns `claude --print` and pipes its stdout through
// the existing SSE event shape. Bypasses adk-rust entirely.
// ---------------------------------------------------------------------------

async fn ask_via_claude_cli(pool: PgPool, question: &str, cfg: &EngineConfig) -> Response {
    let run_id = uuid::Uuid::new_v4();
    let started_at = Utc::now();
    let start = Instant::now();
    let model = format!("claude_cli/{}", cfg.model);

    info!(run_id = %run_id, model = %model, "claude_cli ask starting");

    let (tx, rx) = mpsc::unbounded_channel::<Result<SseEvent, Infallible>>();
    let _ = tx.send(Ok(SseEvent::default().event("run_started").data(
        serde_json::to_string(&json!({
            "run_id": run_id.to_string(),
            "model": model,
            "question": question,
        }))
        .unwrap_or_default(),
    )));

    let question_owned = question.to_string();
    let model_label = cfg.model.clone();
    tokio::spawn(async move {
        let mut answer = String::new();
        let mut errored = false;

        let (mut chunks, wait) = match claude_cli::run(&question_owned, Some(&model_label), None) {
            Ok(pair) => pair,
            Err(e) => {
                let _ = tx.send(Ok(SseEvent::default().event("run_error").data(
                    serde_json::to_string(&json!({
                        "code": "spawn_failed",
                        "message": e.to_string(),
                    }))
                    .unwrap_or_default(),
                )));
                emit_finished(&tx, run_id, start.elapsed().as_millis() as u64);
                let _ = persist_run(
                    &pool,
                    run_id,
                    &question_owned,
                    &model,
                    kyma_core::tenant::DEFAULT_TENANT.as_uuid(),
                    None,
                    started_at,
                    Utc::now(),
                    "error",
                    &Value::Null,
                    &json!([{ "event": "run_error", "data": { "message": e.to_string() } }]),
                )
                .await;
                return;
            }
        };

        while let Some(chunk) = chunks.recv().await {
            match chunk {
                claude_cli::Chunk::Stdout(line) => {
                    if !answer.is_empty() {
                        answer.push('\n');
                    }
                    answer.push_str(&line);
                    let _ = tx.send(Ok(SseEvent::default().event("answer_delta").data(
                        serde_json::to_string(&json!({"text": format!("{}\n", line)}))
                            .unwrap_or_default(),
                    )));
                }
                claude_cli::Chunk::Stderr(line) => {
                    if !line.is_empty() {
                        warn!(run_id = %run_id, line = %line, "claude_cli stderr");
                    }
                }
            }
        }

        let exit_code = match wait.await {
            Ok(Ok(code)) => code,
            Ok(Err(e)) => {
                let _ = tx.send(Ok(SseEvent::default().event("run_error").data(
                    serde_json::to_string(&json!({
                        "code": "io",
                        "message": e.to_string(),
                    }))
                    .unwrap_or_default(),
                )));
                errored = true;
                -1
            }
            Err(_join_err) => -1,
        };

        if exit_code != 0 && !errored {
            let _ = tx.send(Ok(SseEvent::default().event("run_error").data(
                serde_json::to_string(&json!({
                    "code": "nonzero_exit",
                    "exit_code": exit_code,
                }))
                .unwrap_or_default(),
            )));
            errored = true;
        }

        if !errored {
            let _ = tx.send(Ok(SseEvent::default().event("answer_final").data(
                serde_json::to_string(&json!({"text": answer.clone()}))
                    .unwrap_or_default(),
            )));
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;
        emit_finished(&tx, run_id, elapsed_ms);
        let _ = persist_run(
            &pool,
            run_id,
            &question_owned,
            &model,
            kyma_core::tenant::DEFAULT_TENANT.as_uuid(),
            None,
            started_at,
            Utc::now(),
            if errored { "error" } else { "success" },
            &Value::String(answer.clone()),
            &json!([{ "event": "answer_final", "data": { "text": answer } }]),
        )
        .await;
    });

    Sse::new(UnboundedReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn emit_finished(
    tx: &mpsc::UnboundedSender<Result<SseEvent, Infallible>>,
    run_id: uuid::Uuid,
    elapsed_ms: u64,
) {
    let _ = tx.send(Ok(SseEvent::default().event("run_finished").data(
        serde_json::to_string(&json!({
            "run_id": run_id.to_string(),
            "elapsed_ms": elapsed_ms,
            "tool_calls": 0,
        }))
        .unwrap_or_default(),
    )));
}
