//! Multi-turn conversation sessions for the agent (the "A5" surface).
//!
//! Activates the dormant `agent_sessions` / `agent_session_turns` tables: a
//! turn is persisted on each ask, and prior turns are replayed into the ADK
//! session (see [`super::runner::make_runner`]) so follow-ups carry context.
//! A server-side rolling summary compacts older turns so long conversations
//! stay within the model's context budget.

use serde_json::{json, Value};
use sqlx::types::Json as SqlxJson;
use sqlx::PgPool;
use uuid::Uuid;

use super::state::AgentState;

/// How many recent turns to keep verbatim when a rolling summary is produced.
const KEEP_RECENT_TURNS: i32 = 4;

/// A prior conversation turn, loaded for replay into the agent context.
#[derive(Debug, Clone)]
pub struct Turn {
    /// `"user"` or `"assistant"`.
    pub role: String,
    pub text: String,
}

/// Resolved session context for an ask.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: Uuid,
    /// Prior turns to seed (already filtered past any rolling-summary boundary).
    pub history: Vec<Turn>,
    /// Rolling summary of the turns before `history`, if any.
    pub summary: Option<String>,
    /// Turn index at which to write the incoming user message.
    pub next_turn_index: i32,
}

/// Load an existing session (touching `last_active`) or create a fresh one.
///
/// A caller-supplied `requested` id that doesn't exist yet is created with
/// that id — this lets external callers (e.g. Claude Code hooks) map their own
/// session id onto a kyma session.
pub async fn load_or_create(
    pool: &PgPool,
    requested: Option<&str>,
    tenant: Uuid,
    auth_subject: &str,
    source: &str,
) -> SessionContext {
    if let Some(sid) = requested.and_then(|s| Uuid::parse_str(s).ok()) {
        let touched: Option<(Option<String>, i32)> = sqlx::query_as(
            "UPDATE agent_sessions SET last_active = NOW() \
             WHERE session_id = $1 RETURNING rolling_summary, summary_turn_index",
        )
        .bind(sid)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        if let Some((summary, summary_idx)) = touched {
            let next = next_turn_index(pool, sid).await;
            let rows: Vec<(String, SqlxJson<Value>)> = sqlx::query_as(
                "SELECT role, content_json FROM agent_session_turns \
                 WHERE session_id = $1 AND turn_index >= $2 ORDER BY turn_index ASC",
            )
            .bind(sid)
            .bind(summary_idx)
            .fetch_all(pool)
            .await
            .unwrap_or_default();

            let history = rows.into_iter().map(|(role, content)| Turn { role, text: text_of(&content.0) }).collect();
            return SessionContext { session_id: sid, history, summary, next_turn_index: next };
        }

        // Requested id not found — create it with that id.
        let _ = sqlx::query(
            "INSERT INTO agent_sessions (session_id, tenant_id, auth_subject, source) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (session_id) DO NOTHING",
        )
        .bind(sid)
        .bind(tenant)
        .bind(auth_subject)
        .bind(source)
        .execute(pool)
        .await;
        return SessionContext { session_id: sid, history: vec![], summary: None, next_turn_index: 0 };
    }

    // No id requested — brand-new session.
    let sid = Uuid::new_v4();
    let _ = sqlx::query(
        "INSERT INTO agent_sessions (session_id, tenant_id, auth_subject, source) VALUES ($1, $2, $3, $4)",
    )
    .bind(sid)
    .bind(tenant)
    .bind(auth_subject)
    .bind(source)
    .execute(pool)
    .await;
    SessionContext { session_id: sid, history: vec![], summary: None, next_turn_index: 0 }
}

/// Persist one conversation turn. Idempotent on `(session_id, turn_index)`.
pub async fn persist_turn(
    pool: &PgPool,
    session_id: Uuid,
    tenant: Uuid,
    turn_index: i32,
    role: &str,
    text: &str,
    run_id: Option<Uuid>,
) {
    let _ = sqlx::query(
        "INSERT INTO agent_session_turns (session_id, tenant_id, turn_index, role, content_json, run_id) \
         VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (session_id, turn_index) DO NOTHING",
    )
    .bind(session_id)
    .bind(tenant)
    .bind(turn_index)
    .bind(role)
    .bind(SqlxJson(json!({ "text": text })))
    .bind(run_id)
    .execute(pool)
    .await;
}

/// Spawn a detached, best-effort rolling-summary pass for `session_id`. When
/// the session has accumulated more than `every` un-summarized turns, the
/// older turns are summarized server-side and the boundary advances; the most
/// recent [`KEEP_RECENT_TURNS`] turns stay verbatim. Failures are logged only.
pub fn maybe_summarize_detached(state: AgentState, session_id: Uuid, every: i32) {
    tokio::spawn(async move {
        if let Err(e) = summarize_if_needed(&state, session_id, every).await {
            tracing::warn!(session_id = %session_id, error = %e, "rolling summary failed");
        }
    });
}

async fn summarize_if_needed(state: &AgentState, session_id: Uuid, every: i32) -> anyhow::Result<()> {
    let pool = &state.pool;
    let (prev_summary, summary_idx): (Option<String>, i32) =
        sqlx::query_as("SELECT rolling_summary, summary_turn_index FROM agent_sessions WHERE session_id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await?;

    let max_idx: Option<i32> =
        sqlx::query_scalar::<_, Option<i32>>("SELECT MAX(turn_index) FROM agent_session_turns WHERE session_id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .unwrap_or(None);
    let Some(max_idx) = max_idx else { return Ok(()) };

    if max_idx - summary_idx < every {
        return Ok(());
    }
    let boundary = max_idx + 1 - KEEP_RECENT_TURNS;
    if boundary <= summary_idx {
        return Ok(());
    }

    let rows: Vec<(String, SqlxJson<Value>)> = sqlx::query_as(
        "SELECT role, content_json FROM agent_session_turns \
         WHERE session_id = $1 AND turn_index >= $2 AND turn_index < $3 ORDER BY turn_index ASC",
    )
    .bind(session_id)
    .bind(summary_idx)
    .bind(boundary)
    .fetch_all(pool)
    .await?;

    let mut transcript = String::new();
    if let Some(ps) = &prev_summary {
        if !ps.trim().is_empty() {
            transcript.push_str(&format!("Summary so far:\n{ps}\n\n"));
        }
    }
    for (role, content) in &rows {
        transcript.push_str(&format!("{role}: {}\n", text_of(&content.0)));
    }

    let summary = super::runner::summarize_conversation(state, &transcript).await?;
    if summary.trim().is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE agent_sessions SET rolling_summary = $1, summary_turn_index = $2 WHERE session_id = $3")
        .bind(summary.trim())
        .bind(boundary)
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn next_turn_index(pool: &PgPool, session_id: Uuid) -> i32 {
    let max_idx: Option<i32> =
        sqlx::query_scalar::<_, Option<i32>>("SELECT MAX(turn_index) FROM agent_session_turns WHERE session_id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await
            .unwrap_or(None);
    max_idx.map(|m| m + 1).unwrap_or(0)
}

fn text_of(content: &Value) -> String {
    content.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string()
}
