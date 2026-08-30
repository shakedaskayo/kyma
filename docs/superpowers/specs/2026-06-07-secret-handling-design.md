# End-to-End Secret Handling for the Agent Engine

**Date:** 2026-06-07
**Status:** Approved
**Scope:** Full defense-in-depth — detection/redaction engine, model-context protection, subprocess containment, UI masking, backfill, repo hygiene.

## Problem

Agent platforms leak secrets through three distinct surfaces: (1) the sandbox/execution
environment, (2) the model's context window (and therefore the LLM provider), and
(3) the persisted transcript and its UI rendering. Pensieve already does
reference-not-value credential handling where it matters (`credential_id` in engine
config, AES-256-GCM encrypted catalog, 0600 MCP config temp files), but has **no
secret hygiene at the trace/tool-result layer**:

- `agent_runs.trace_json` and `agent_session_turns.content_json` persist tool args and
  results verbatim (`Emitter::tool_call/tool_result`, `routes.rs:454,470`).
- A SQL/KQL query touching a token-bearing column sends those values into model
  context, the DB, and the web UI unredacted.
- The spawned `claude` subprocess (`claude_cli.rs:144`) inherits the full server env
  (`PENSIEVE_SECRET_KEY`, `OPENAI_API_KEY`, Supabase secrets, …).
- The question is passed as argv — visible via `ps`.
- Memory redaction is opt-in (`<private>` tags only); error messages and logs are
  unsanitized; the UI renders raw tool I/O JSON.
- A plaintext GitHub PAT (`read_only_token`) sits in the repo root (untracked).

## Decisions (locked)

| Decision | Choice |
|---|---|
| Scope | Full defense-in-depth (all layers) |
| On detection | **Mask + audit event** — replace with `[REDACTED:<kind>]`, continue the run, emit `security_event` |
| Architecture | Central `SecretGuard` + choke-point integration (Approach A); a separate broker process (Approach C) can layer on later without API changes |
| `read_only_token` | Delete + gitignore patterns; user rotates the PAT |
| User-pasted secrets in questions | Still go to the model (explicit user choice) but are redacted in trace/persistence/UI |

## 1. New crate: `pensieve-redact`

Workspace crate with no pensieve dependencies so `pensieve-server`, `pensieve-mcp`,
`pensieve-memory`, `pensieve-catalog`, and `pensieve-connectors` can all use it.

```rust
pub struct Finding { pub kind: String, pub surface_hint: Option<String> } // never the value

pub struct SecretGuard { /* known-value matcher + pattern set */ }
impl SecretGuard {
    /// Register a live secret value (>= 8 chars). Rebuilds the Aho-Corasick automaton.
    pub fn register_value(&self, kind: &str, value: &str);
    pub fn scan(&self, text: &str) -> Vec<Finding>;
    pub fn redact_text(&self, text: &str) -> (String, Vec<Finding>);
    /// Deep-walk JSON: redact string values and stringified keys.
    pub fn redact_json(&self, v: &mut serde_json::Value) -> Vec<Finding>;
}

/// Rolling holdback buffer (~256 bytes) for streaming text deltas, so secrets
/// spanning chunk boundaries are still caught. Flush on finalize.
pub struct StreamScrubber { /* wraps a SecretGuard */ }
```

**Detectors:**

- **Known-value** (exact, zero false positives): Aho-Corasick automaton over values
  registered at runtime. Replacement: `[REDACTED:<kind>]`.
- **Pattern** (curated regex set, no entropy heuristics): GitHub PATs
  (`github_pat_…`, `ghp_…`, `gho_…`, `ghs_…`), Anthropic/OpenAI keys (`sk-ant-…`,
  `sk-…`), AWS (`AKIA…` access key ids + adjacent secret pairs), JWTs (`eyJ….eyJ….sig`),
  connection-string credentials (`scheme://user:pass@`), `Bearer <token>` header
  values, Slack tokens (`xox[bpars]-…`), PEM private-key blocks. Replacement:
  `[REDACTED:github-pat]`, `[REDACTED:jwt]`, etc.

Redaction is idempotent: `[REDACTED:…]` markers are never re-matched.

## 2. Server wiring — registration and choke points

One `Arc<SecretGuard>` in server state.

**Value registration (at the source):**

| Where | What |
|---|---|
| Server startup | `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `PENSIEVE_SECRET_KEY`, Supabase secret env vars |
| `pensieve-catalog/src/credentials.rs` fetch/decrypt | every decrypted connector credential value |
| `agent/routes.rs` ask handler | the per-request bearer token |

**Choke points (innermost first):**

| Where | Change |
|---|---|
| `agent/tools.rs`, `agent/memory_tools.rs` (ADK path); `pensieve-mcp/src/tools.rs` (Claude CLI path) | `redact_json` every tool result **before it returns to the model** — the core "never enters model context" guarantee |
| `Emitter::tool_call` / `tool_result` (`agent/routes.rs:454,470`) | `redact_json` args/results before `record()` and the UI push; emit `security_event` on findings |
| `Emitter::answer_delta` / `thinking_delta` | route text through `StreamScrubber` (catches the model echoing a user-pasted secret) |
| `run_started` (question) and `answer_final` | redact in trace/persistence only; the live question still reaches the model |
| `persist_run` (`agent/routes.rs:880`) | belt-and-braces scrub of whole `trace_json` before bind; same for `agent_session_turns.content_json` |
| `pensieve-memory/src/writer.rs` | auto-scan content on save; detected secrets redacted alongside existing `<private>`-tag handling in `redact_create()` |
| Logging setup | wrap the `tracing` fmt layer's `MakeWriter` in a scrubbing writer — every formatted log line passes the guard; covers resolver/connector/credential error messages without chasing individual error sites |

## 3. Claude CLI subprocess containment (`agent/engine/claude_cli.rs`)

- `cmd.env_clear()` + explicit allowlist: `PATH`, `HOME`, `TERM`, `SHELL`,
  `LANG`/`LC_*`, `ANTHROPIC_API_KEY` (the CLI's own auth), `CLAUDE_*`. The spawned
  CLI can never read `PENSIEVE_SECRET_KEY`, `OPENAI_API_KEY`, or Supabase secrets.
- Pass the question via **stdin** instead of argv (no `ps` exposure).
- MCP config handling unchanged (0600 temp file is already correct).

## 4. Web UI masking (defense-in-depth)

- Shared `redactDisplay(text)` util in `web/src` — TypeScript port of the pattern
  list (known-value matching is server-only).
- Applied in the tool I/O renderer (`components/ai-elements/tool.tsx`) and message
  text rendering in `AgentConsole.tsx`.
- A small shield badge renders when a `security_event` data part arrives, so users
  see that masking happened.

## 5. Backfill + repo hygiene

- **Backfill:** idempotent startup task (completion marked in a settings/meta row)
  that re-scrubs existing `agent_runs.trace_json` and
  `agent_session_turns.content_json` with the full guard. Code-based rather than a
  SQL migration because known-value matching needs the live registry.
- **Repo:** delete `read_only_token`; add `.gitignore` patterns (`*_token`,
  `*.token`, `.env*` where missing); verify via `git log --all` that the file was
  never committed. PAT rotation is the user's action.

## 6. Audit & observability

On any finding, three signals — none carrying the secret value:

1. `security_event` trace frame: `{ kind, surface, count }`.
2. `tracing::warn!` (itself passing through the scrubbing writer).
3. UI data part driving the shield badge.

No new audit table (trace + logs suffice).

## 7. Testing

- **`pensieve-redact` unit tests:** every pattern; known-value matching; JSON deep-walk
  (values and keys); stream-boundary spanning (secret split across deltas);
  idempotency of `[REDACTED:…]`.
- **Integration:** seed a fake credential in the catalog → agent turn whose SQL
  result contains it → assert masked in tool result, `trace_json`, and UI stream
  events; `security_event` present.
- **Subprocess env:** build the CLI command → assert env is exactly the allowlist.
- **Backfill:** insert dirty trace row → run startup task → assert scrubbed and
  idempotent (second run is a no-op).

## Accepted trade-offs

- Pattern detection can miss exotic foreign-secret formats; known-value matching is
  exact for everything Pensieve itself holds.
- The `Bearer` regex may occasionally mask non-secret data — masking (not blocking)
  plus an audit event keeps the harm low.
- The ~256-byte stream holdback adds imperceptible UI streaming latency.

## Out of scope

- Separate secret-broker process (Approach C) — future layer; `SecretGuard` API is
  stable under it.
- High-entropy heuristic detection — false-positive machine.
- Rotation/expiry of user-held tokens (user action).
