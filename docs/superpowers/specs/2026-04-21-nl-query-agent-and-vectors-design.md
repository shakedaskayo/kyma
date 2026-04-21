# NL-Query Agent + Day-0 Vectors — Design

**Date:** 2026-04-21
**Status:** Design drafted; awaiting user review
**Owner:** Shaked

## 1. Goal and scope

Agents and humans should be able to ask **natural-language questions** against a kyma cluster without knowing which databases, tables, columns, or query language exist. The engine answers by running an internal, deterministic, configurable agentic loop that discovers schema, drafts KQL/SQL, executes it, and streams both reasoning and results back.

This slice also makes **vectors and embeddings first-class engine primitives** — pgvector inside the catalog, a user-declarable `vector(N)` column type, and an `EmbeddingBackend` trait — because the NL agent needs semantic schema retrieval and because "unified engine for telemetry + memories + arbitrary signals" without vectors is incoherent.

Both surfaces are exposed over **Model Context Protocol** on two transports so external agents (Claude Desktop, Cursor, bespoke agents) can plug in: a stdio binary for local/dev and HTTP+SSE routes inside the existing kyma server for remote/multi-tenant.

### In scope (slice-1)

- **Vector primitives in the engine, day-0:**
  - pgvector extension + `column_metadata`/`schema_embeddings` tables in the catalog.
  - `vector(N)` as a user-declarable column type, Arrow-backed as `FixedSizeList<Float32>`, exact similarity search only (no ANN index in Slice 1).
  - `EmbeddingBackend` trait with `fastembed-rs` local default (`bge-small-en-v1.5`, 384-dim) and pluggable Ollama / OpenAI-compat / Gemini variants.
  - DataFusion UDFs: `cosine_distance`, `l2_distance`, `inner_product` over `FixedSizeList<Float32>`.
  - KQL sugar: `| order by col <-> @qvec | take k`.
- **Agent layer (`kyma-agent-core` + `kyma-agent-adk`):**
  - Small, stable trait surface (`Backend`, `Tool`, `Runner`, `Event`, `ReplayCache`) that is the public contract.
  - `adk-rust` 0.6+ as the default backend behind the trait layer; not leaked into the public surface.
  - 12 read-only tools (raw MCP tool pack + composed `ask()` internal agent).
  - Two-layer replay cache (`GenerateReplayCache` around `Backend::generate`, `RunReplayCache` around full `Runner::run`), with `Off | Record | Replay | ReadThrough` modes.
  - Semantic-stable determinism in live traffic: `temperature=0`, `seed=42`, fixed system-prompt version, tool-schema version mixed into every cache key.
  - Configurable model providers — Ollama+Gemma default, OpenAI / Anthropic / Gemini via cargo features.
  - Multi-turn sessions via a Postgres `SessionService` backed by `agent_sessions` + `agent_session_turns`.
- **MCP gateway:**
  - `kyma-mcp` stdio binary (`remote` mode talking HTTP to a kyma server, `embedded` mode all-in-one).
  - `/mcp/v1/rpc` + `/mcp/v1/events/:session_id` HTTP+SSE routes mounted inside `kyma-server` behind existing bearer-token auth.
  - `ask(question, …)` meta-tool plus the 12 raw tools on both transports.
- **Resource governance & auth:**
  - `BudgetEnforcer` (max tool calls, max wall ms, max tokens, max concurrent runs per subject).
  - `AuthSubject { token_id, role, allowed_databases }` propagated through `ToolContext`, enforced inside every tool including the schema-RAG SQL filter.
- **Observability:**
  - Prometheus counters/histograms for runs, tool calls, tokens, budget exhaustion, replay hits.
  - Completion-persisted `agent_runs` with full event trace; `GET /v1/agent/runs/:run_id` endpoint for the UI.
- **Tests:** 7 new E2E scripts matching the existing `scripts/test-*.sh` pattern; replay fixtures committed to the repo so CI never reaches a live LLM.

### Out of scope (deferred, tracked as follow-ons)

- **Workflow/DAG execution engine.** Background agentic workflows, scheduled triggers, resumable mid-run persistence. Spec 2.
- **Context Graph.** Metadata graph of asset/data/service relationships, graph-populating agents, graph-query tools. Spec 2.
- **ANN indices on user-data vector columns.** HNSW inside the Phase-D custom format footer — adjacent to the inverted-index work already queued. Spec 2 or a Phase-D successor.
- **Write tools** (`create_table`, `alter_table`, `ingest_rows`). Different trust-boundary conversation (audit, permissions, rollback). Not in Slice 1.
- **Mid-run event persistence + resume-from-disconnect.** Spec 1 persists events at completion only. The future workflow engine will need real mid-run persistence; that's its problem.
- **`get_recent_queries` / `get_column_stats`** agent tools. Useful but premature; folded into `describe_table`'s output or deferred to Spec 2.
- **Evaluation harness** (`adk-eval`-style trajectory rubrics). Replay-cache fixtures are the slice-1 correctness strategy.
- **Vector indexing of workflow artifacts / run traces.** Spec 2.

## 2. Primitives reused

The agent layer is small because the engine already has most of the infrastructure:

| Primitive | Existing use | Agent use |
|---|---|---|
| Postgres catalog + sqlx migrations | Tables, snapshots, schema evolution, ingest ledger, background tasks, connectors | pgvector extension, schema embeddings, `agent_runs`, replay cache |
| `KYMA_AUTH_TOKENS` bearer middleware | HTTP ingest + query + connectors | MCP HTTP+SSE auth; per-token `allowed_databases` |
| `QueryBudget` pattern (header-driven, tokio timeout + memory pool) | Query budget enforcement | `BudgetEnforcer` for agent runs |
| Prometheus `metrics` facade | Existing counters/histograms | Agent metrics same pattern |
| KQL `graph-traverse` / `graph-shortest-path` operators | Tabular-graph queries | Wrapped as agent tools |
| Equality/text indexes + time-range pruning in `KymaTable` | Query performance | Make `vector_search` brute-force acceptable at extent scale |
| `background_tasks` work queue + claim/complete pattern | Compaction, retention, connectors | Low-priority schema-sample-refresh job |
| Arrow `FixedSizeList<Float32>` already handled through Arrow IPC | Generic Arrow support | `vector(N)` column storage in Slice 1 with no format change |

## 3. Crates and layout

Five new crates, in dependency order:

| Crate | Path | Depends on | Purpose |
|---|---|---|---|
| `kyma-embed` | `crates/kyma-embed/` | `kyma-core` | `EmbeddingBackend` trait + `FastembedBackend` (default) + `OllamaBackend`, `OpenAICompatBackend`, `GeminiBackend` impls (feature-gated). |
| `kyma-agent-core` | `crates/kyma-agent-core/` | `kyma-core`, `kyma-embed` | Traits + types: `Backend`, `Tool`, `Runner`, `Event`, `ReplayCache`, `RunConfig`, `RunHandle`, `AuthSubject`, `BudgetEnforcer`. **Zero ADK dependency.** |
| `kyma-agent-tools` | `crates/kyma-agent-tools/` | `kyma-agent-core`, `kyma-catalog`, `kyma-exec`, `kyma-kql`, `kyma-embed` | Twelve `impl Tool for …` structs + `SchemaRagIndex` (the pgvector client) + `EmbeddingUpdater` (DDL-transactional schema embed) + `SchemaSampleRefresher`. |
| `kyma-agent-adk` | `crates/kyma-agent-adk/` | `kyma-agent-core`, `adk-rust` | `AdkBackend` + `AdkRunner` + tool/event bridges + system-prompt V1 + provider factory. **Only crate that knows `adk-rust` exists.** |
| `kyma-mcp` | `crates/kyma-mcp/` | `kyma-agent-core`, `kyma-agent-tools`, `kyma-agent-adk`, `rmcp` | Stdio binary + mountable axum router for `kyma-server`. |

Wiring changes:

- `kyma-bin/src/main.rs` spawns the `SchemaSampleRefresher` task (analogous to compaction/retention).
- `kyma-server` mounts `kyma_mcp::router()` at `/mcp/v1/` and adds `GET /v1/agent/runs/:run_id`.
- `kyma-catalog` gains migration `006_agent_and_vectors.sql` and catalog helpers for embeddings, runs, sessions, and the replay cache.
- `kyma-core`'s `DataType` enum gains `Vector { dimension: u16, model_id: Option<String> }`.
- `kyma-exec` registers the three distance UDFs.
- `kyma-kql` parses and lowers `| order by col <-> @qvec | take k` into the SQL equivalent over `cosine_distance`.

## 4. Catalog schema (migration 006)

```sql
-- 006_agent_and_vectors.sql

CREATE EXTENSION IF NOT EXISTS vector;

-- Per-column user-facing metadata; vectors carry a model_id tag.
CREATE TABLE column_metadata (
    database           TEXT NOT NULL,
    table_name         TEXT NOT NULL,
    column_name        TEXT NOT NULL,
    column_type        TEXT NOT NULL,
    description        TEXT,
    embedding_model_id TEXT,        -- NULL unless column_type starts with 'vector('
    dimension          INT,
    distance_metric    TEXT,        -- 'cosine' | 'l2' | 'inner'; default cosine
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (database, table_name, column_name)
);

-- pgvector-backed schema RAG index. One row per table, one per column.
CREATE TABLE schema_embeddings (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    database            TEXT NOT NULL,
    table_name          TEXT NOT NULL,
    column_name         TEXT,        -- NULL for kind='table'
    kind                TEXT NOT NULL CHECK (kind IN ('table','column')),
    text_source         TEXT NOT NULL,
    text_source_sha256  BYTEA NOT NULL,
    text_format_version TEXT NOT NULL DEFAULT 'v1',
    model_id            TEXT NOT NULL,
    embedding           vector(384) NOT NULL,  -- default bge-small-en-v1.5
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
-- Upserts dedupe on (database, table_name, column_name-or-sentinel, model_id).
-- Partial-unique indexes handle the NULL column_name case for kind='table'
-- (Postgres default UNIQUE treats NULLs as distinct, so two 'table' rows
-- with NULL column_name would otherwise both insert).
CREATE UNIQUE INDEX schema_embeddings_uniq_table
    ON schema_embeddings (database, table_name, model_id)
    WHERE column_name IS NULL;
CREATE UNIQUE INDEX schema_embeddings_uniq_column
    ON schema_embeddings (database, table_name, column_name, model_id)
    WHERE column_name IS NOT NULL;
CREATE INDEX schema_embeddings_hnsw ON schema_embeddings
    USING hnsw (embedding vector_cosine_ops);
CREATE INDEX schema_embeddings_db ON schema_embeddings (database);

-- Completion-persisted agent run traces.
CREATE TABLE agent_runs (
    run_id             UUID PRIMARY KEY,
    question           TEXT NOT NULL,
    model_id           TEXT NOT NULL,
    auth_subject       TEXT NOT NULL,    -- token_id
    session_id         UUID,
    started_at         TIMESTAMPTZ NOT NULL,
    finished_at        TIMESTAMPTZ NOT NULL,
    status             TEXT NOT NULL CHECK (status IN
                         ('success','error','budget_exceeded','cancelled','replay_miss')),
    usage_json         JSONB NOT NULL,   -- {prompt_tokens, completion_tokens, wall_ms, tool_calls}
    trace_json         JSONB NOT NULL,   -- ordered array of Event objects
    replay_cache_hit   BOOL NOT NULL DEFAULT FALSE
);
CREATE INDEX agent_runs_subject_time ON agent_runs (auth_subject, started_at DESC);
CREATE INDEX agent_runs_session       ON agent_runs (session_id, started_at DESC);

-- Multi-turn sessions.
CREATE TABLE agent_sessions (
    session_id    UUID PRIMARY KEY,
    auth_subject  TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_active   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata_json JSONB NOT NULL DEFAULT '{}'
);
CREATE TABLE agent_session_turns (
    session_id   UUID NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_index   INT  NOT NULL,
    role         TEXT NOT NULL CHECK (role IN ('user','assistant')),
    content_json JSONB NOT NULL,
    run_id       UUID REFERENCES agent_runs(run_id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, turn_index)
);

-- Content-addressed replay cache for LLM generate() calls.
CREATE TABLE agent_replay_cache (
    cache_key     BYTEA PRIMARY KEY,    -- sha256
    layer         TEXT NOT NULL CHECK (layer IN ('generate','run')),
    response_json JSONB NOT NULL,
    model_id      TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    hit_count     INT NOT NULL DEFAULT 0
);
CREATE INDEX agent_replay_cache_layer_model ON agent_replay_cache (layer, model_id);
```

Data retention: `agent_runs` default 30 days (`KYMA_AGENT_RUN_RETENTION_DAYS`), cleaned by an extension to the existing `RetentionSweeper`. `agent_replay_cache` is never auto-cleaned (fixtures are intentional). `agent_sessions` + turns default 7-day idle TTL (`KYMA_AGENT_SESSION_IDLE_TTL_DAYS`).

## 5. Core trait surface (`kyma-agent-core`)

```rust
// ------- Events -------

pub enum Event {
    RunStarted    { run_id: Uuid, model_id: String, question: String },
    Plan          { run_id: Uuid, step_index: u32, description: String },
    ThinkingDelta { run_id: Uuid, step_index: u32, text: String },
    ToolCall      { run_id: Uuid, step_index: u32, tool: String, args: Value },
    ToolResult    { run_id: Uuid, step_index: u32, tool: String,
                    result: Value, elapsed_ms: u64 },
    AnswerDelta   { run_id: Uuid, text: String },
    AnswerFinal   { run_id: Uuid, text: String,
                    kql_used: Option<String>, rows: Option<Value>,
                    trace_id: Option<String> },
    RunError      { run_id: Uuid, code: String, message: String, retryable: bool },
    RunFinished   { run_id: Uuid, usage: Usage, replay_cache_hit: bool },
}

pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub wall_ms: u64,
    pub tool_calls: u32,
}

// ------- Backend (LLM abstraction) -------

#[async_trait]
pub trait Backend: Send + Sync {
    fn id(&self) -> &str;                     // e.g. "ollama/gemma3:4b"
    fn supports_tool_calls(&self) -> bool;
    fn supports_thinking(&self) -> bool;
    async fn generate(&self, req: GenerateRequest)
        -> Result<GenerateStream, BackendError>;
}

pub struct GenerateRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSchema>,
    pub params: GenParams,     // temperature, top_p, seed, max_tokens, thinking_cfg
}

pub type GenerateStream = Pin<Box<dyn Stream<Item = GeneratePart> + Send>>;

pub enum GeneratePart {
    Text       { delta: String },
    Thinking   { delta: String },
    ToolCall   { id: String, name: String, args: Value },
    Finish     { reason: FinishReason, usage: Usage },
    Error      { code: String, message: String, retryable: bool },
}

// ------- Tool -------

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &serde_json::Value;          // JSON schema
    async fn call(&self, ctx: &ToolContext, args: Value)
        -> Result<ToolOutcome, ToolError>;
}

pub struct ToolContext {
    pub run_id: Uuid,
    pub auth_subject: AuthSubject,
    pub deadline: Instant,
    pub budget_remaining: Budget,
}

pub struct ToolOutcome { pub result: Value, pub elapsed_ms: u64 }

pub struct ToolError {
    pub code: String,            // stable taxonomy — see §9
    pub message: String,
    pub retryable: bool,
    pub hint: Option<String>,    // prompt-facing self-correction hint
}

// ------- Runner -------

#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, cfg: RunConfig, question: &str)
        -> Result<RunHandle, RunError>;
}

pub struct RunHandle {
    pub run_id: Uuid,
    pub events: tokio::sync::mpsc::Receiver<Event>,
}

pub struct RunConfig {
    pub model:             Option<String>,         // named model lookup
    pub model_overrides:   Option<ModelConfig>,    // inline override
    pub budget:            Budget,
    pub replay_mode:       ReplayMode,
    pub include_thinking:  bool,
    pub stream_answer:     bool,
    pub auth_subject:      AuthSubject,
    pub session_id:        Option<Uuid>,
    pub database_hint:     Option<String>,         // user can pre-narrow
}

pub struct Budget {
    pub max_tool_calls:     u32,      // default 16
    pub max_wall_ms:        u64,      // default 60_000
    pub max_tokens:         u32,      // default 32_000
}

pub enum ReplayMode { Off, Record, Replay, ReadThrough }

pub struct AuthSubject {
    pub token_id:           String,
    pub role:               Role,     // Read | Write | Admin
    pub allowed_databases:  Vec<String>,   // empty vec = all
}
```

These are the stable contract. Anything ADK-specific stays inside `kyma-agent-adk`.

## 6. Vector + embedding primitives (`kyma-embed` + engine touches)

### 6.1 `EmbeddingBackend` trait

```rust
#[async_trait]
pub trait EmbeddingBackend: Send + Sync {
    fn id(&self) -> &str;              // "fastembed/bge-small-en-v1.5", "openai/text-embedding-3-small"
    fn dimension(&self) -> u16;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}
```

Implementations (all feature-gated):

- `FastembedBackend` (default): wraps `fastembed-rs`. Loads ONNX model at init, normalizes output. `KYMA_EMBED_MODEL_PATH` overrides the download path for air-gapped deployments.
- `OllamaBackend`: POSTs to `/api/embeddings`.
- `OpenAICompatBackend`: POSTs to `{base_url}/v1/embeddings` with optional bearer token — works with OpenAI, Together, Fireworks, local `llama.cpp` servers.
- `GeminiBackend`: native Gemini embedding API.

Configured identically to the chat model in `kyma.yaml`:

```yaml
agent:
  embedding_model:
    provider: fastembed
    id: bge-small-en-v1.5
    model_path: null     # air-gapped override
```

### 6.2 `vector(N)` as a column type

`kyma_core::DataType::Vector { dimension: u16, model_id: Option<String> }`.

- **DDL** accepts `vector(N)` with an optional trailing `MODEL 'id'` clause: `CREATE TABLE memos (content text, embedding vector(384) MODEL 'fastembed/bge-small-en-v1.5')`. If `MODEL` is omitted, the engine's default embedding model id is stamped. Stored in `column_metadata.embedding_model_id`. The grammar addition is one token in the KQL/SQL DDL parser — no ambiguity with existing syntax.
- **Ingest** coerces JSON arrays-of-float to Arrow `FixedSizeList<Float32, N>`. Dimension mismatch → 400 with explicit error.
- **Storage** in Slice 1 is vanilla Arrow IPC — `kyma-format-tlm` phase-A already handles `FixedSizeList` transparently, no format version bump needed.
- **Query** via DataFusion UDFs:
  - `cosine_distance(a, b)` (default for `<->` operator)
  - `l2_distance(a, b)`
  - `inner_product(a, b)`
- **KQL** extension: `T | order by embedding <-> @qvec | take 10` lowers to `SELECT * FROM T ORDER BY cosine_distance(embedding, @qvec) ASC LIMIT 10`. The `@qvec` binding is passed through the existing DataFusion parameter plumbing.

### 6.3 `model_id` tag prevents cross-space mistakes

Every vector column carries its embedding-model identifier. `vector_search` tool rejects a `query_vec` whose `model_id` doesn't match the target column, and auto-embeds `query_text` using the column's `model_id` (falling back to a hard error if that backend isn't configured). Mixing distance spaces silently is a class of bug we preempt from day 0.

## 7. Twelve-tool catalog (`kyma-agent-tools`)

| # | Tool | Args | Returns | Description (prompt-visible) |
|---|---|---|---|---|
| 1 | `list_databases` | — | `[{name, table_count}]` | List every database visible to the current caller. Call first when the user hasn't named one. |
| 2 | `list_tables` | `database` | `[{table, description, row_count_approx, columns_count}]` | List tables in a database with short descriptions. Use when you need to find which table holds a concept. |
| 3 | `describe_table` | `database, table` | `{columns: [{name, type, description, sample_values, is_indexed: {time, equality, text, vector}}], schema_version, row_count_approx}` | Full schema of a table with sample values and which indexes exist. Call before writing KQL/SQL against an unfamiliar table. |
| 4 | `search_schema` | `query, top_k?=8, database?` | `[{database, table, column?, score, snippet, kind}]` | Semantic search over table/column metadata via pgvector. Use this FIRST for vague questions. Scales to large catalogs. |
| 5 | `run_kql` | `database, query, max_rows?=1000, timeout_ms?=30_000` | `{columns, rows, stats}` | Run a KQL query. Prefer for typical telemetry analytics: time-range filters, `summarize`, `project`, `take`, text search. |
| 6 | `run_sql` | `database, query, max_rows?=1000, timeout_ms?=30_000` | `{columns, rows, stats}` | Run SQL via DataFusion. Prefer over KQL for recursive CTEs, window functions, complex joins. |
| 7 | `sample_rows` | `database, table, n?=5, where?` | `{rows}` | Fetch N representative rows. Call when `describe_table`'s samples aren't enough (JSON/dynamic cols). |
| 8 | `explain_query` | `database, query, language: 'kql'|'sql'` | `{plan, estimated_extents_scanned, estimated_cost_hint}` | Get the physical plan and estimated scan size before running. Call for any query that might be expensive. |
| 9 | `embed` | `text, model_id?` | `{vector: [f32], model_id, dimension}` | Turn text into an embedding vector. Used to compose vector searches. `model_id` must match the target column's. |
| 10 | `vector_search` | `database, table, column, (query_text XOR query_vec), top_k?=10, where? (KQL filter), distance?='cosine'` | `{rows, distances}` | Top-K most similar rows by vector similarity. Exact scan. Must supply exactly one of `query_text` (auto-embedded with the column's model_id) or `query_vec` (pre-computed; dimension + model_id must match the column); supplying both or neither returns `invalid_args`. |
| 11 | `graph_traverse` | `database, edges_table, source, from_col, to_col, max_hops, direction?='forward'` | `[{node, depth}]` | Traverse a graph stored as edges in a kyma table. Wraps the KQL `graph-traverse` operator. |
| 12 | `graph_shortest_path` | `database, edges_table, source, target, from_col, to_col, max_hops` | `{depth, found}` | Shortest path between two nodes in a tabular graph. |

**Tool errors** use the stable taxonomy in §9 and include a `hint` when there's a near-miss — e.g., unknown column returns `hint: "did you mean one of: region, regions, source_region"` (top-3 by edit distance). This is prompt-facing and measurably improves self-correction.

**Auth scoping** is enforced inside every tool via `ToolContext.auth_subject.allowed_databases`. Tools targeting a forbidden database return `ToolError { code: "forbidden" }` without touching the catalog.

### 7.1 Schema RAG pipeline

**What gets embedded.** One row per table, one row per column, in `schema_embeddings`:

- Table text: `"DB {db} · TABLE {name} · {desc} · cols: {c1:type}, {c2:type}, … (+N more)"` (≤15 cols listed).
- Column text: `"DB {db} · COL {table}.{name} {type} · {desc} · samples: {v1}, {v2}, {v3}"` (samples truncated to 32 chars each).

Terse, keyword-rich, deterministic format. Versioned via `schema_embeddings.text_format_version`.

**When it's refreshed.**

- **Transactionally with DDL.** `Catalog::create_table`, `Catalog::alter_table_add_column`, and new `Catalog::set_column_description` / `set_table_description` all invoke `EmbeddingUpdater::upsert_for(…)` inside the same sqlx transaction as the schema mutation. Embedding failure aborts the DDL. This guarantees the index never drifts.
- **Low-priority sample refresh.** `SchemaSampleRefresher` task re-queries sample values every `KYMA_SCHEMA_REFRESH_INTERVAL` (default 6h). Re-embeds only rows where `SHA256(new text) != SHA256(old)`. Holds a semaphore capping concurrent embedding calls (default 1/second). This is the pattern that generalizes to Spec 2's workflow resource governor.

**How `search_schema` queries it.**

```sql
SELECT database, table_name, column_name, kind, text_source AS snippet,
       1 - (embedding <=> $1) AS score
FROM schema_embeddings
WHERE model_id = $2
  AND ($3::text IS NULL OR database = $3)
  AND ($4::text[] IS NULL OR database = ANY($4))   -- auth scoping
ORDER BY embedding <=> $1
LIMIT $5;
```

Auth filter is in SQL, never post-filter: no existence leaks via scores.

## 8. ADK adapter + model provider configuration (`kyma-agent-adk`)

### 8.1 Adapter structure

```
kyma-agent-adk/src/
├── lib.rs              -- AdkRunner + AdkBackend
├── tool_bridge.rs      -- wraps a kyma Tool as an adk FunctionTool (input: adk::ToolContext + Value;
                          output: adk::ToolOutcome; propagates our ToolError codes into adk-compatible results)
├── event_bridge.rs     -- maps adk::Event → kyma::Event
├── system_prompt.rs    -- V1 prompt template (tool-pack summary + recipe block + determinism guardrails)
├── providers.rs        -- ModelConfig → Arc<dyn adk::Llm>
└── model_config.rs     -- ModelConfig struct + env + YAML loader
```

ADK dependency:

```toml
adk-rust = { version = "0.6", default-features = false,
             features = ["minimal", "tools", "sessions"] }
```

Provider cargo features on `kyma-agent-adk` flow through to ADK: `ollama` (default), `openai`, `anthropic`, `gemini`, `openai-compat`. A deployment enables only what it uses.

### 8.2 `ModelConfig`

```yaml
agent:
  default_model:
    provider: ollama
    id: gemma3:4b
    base_url: http://localhost:11434
    temperature: 0.0
    top_p: 1.0
    seed: 42
    max_tokens: 4096
    thinking:
      enabled: false
      max_budget_tokens: 2048
  models:
    gemma-local:   { provider: ollama,    id: gemma3:4b, ... }
    qwen-local:    { provider: ollama,    id: qwen2.5-coder:7b, ... }
    gpt-4o-mini:   { provider: openai,    id: gpt-4o-mini, api_key_env: OPENAI_API_KEY, ... }
    claude-sonnet: { provider: anthropic, id: claude-sonnet-4-6, api_key_env: ANTHROPIC_API_KEY, ... }
```

Every param is env-overridable (`KYMA_AGENT_MODEL_ID`, `KYMA_AGENT_TEMPERATURE`, etc.) and per-request overridable via `RunConfig.model` / `RunConfig.model_overrides`. Unknown model names or missing cargo features fail fast before burning any work.

### 8.3 System prompt V1

Short, stable, version-pinned. Contains:

1. Role: "You are kyma's internal data agent. Answer the user's question by using the tools below."
2. Auth scope: "You have access to databases: {allowed_databases}. Do not reference others."
3. Tool pack summary: tool names + one-line descriptions, auto-generated from the Tool registry.
4. Recipe block: the composition guidance from §7 (short if/else table).
5. Determinism guardrails: "Do not fabricate schema. If you are unsure which table or column, call `search_schema` or `describe_table`. Never claim a result you didn't get from a tool."
6. Budget hint: "You have at most {max_tool_calls} tool calls. Use them efficiently."
7. Output format: "When done, produce a final answer that cites the KQL or SQL you ran."

Stored as a const in `system_prompt.rs` with `SYSTEM_PROMPT_VERSION: &str = "v1"`. Included in every replay-cache key — changing it rotates caches.

## 9. Determinism and replay cache

Replay lives in `kyma-agent-core`, wrapping `AdkRunner` in two independent layers.

### 9.1 Inner layer — `GenerateReplayCache`

Wraps `Backend::generate`. Cache key:

```
sha256(
    backend.id()
  | "\0" | generate_request.system_prompt
  | "\0" | generate_request.messages_json      (canonical JSON)
  | "\0" | generate_request.tools_json         (canonical JSON of all tool schemas)
  | "\0" | generate_request.params_json        (temperature, top_p, seed, max_tokens)
  | "\0" | SYSTEM_PROMPT_VERSION
  | "\0" | TOOL_SCHEMA_VERSION                 (manual override; strictly redundant with tools_json but lets ops force-rotate caches without a tool schema edit)
)
```

Stored in `agent_replay_cache` with `layer='generate'`. Tool-schema and system-prompt versions are included so adding/changing a tool rotates the cache (old caches are stale — model saw a different action space).

### 9.2 Outer layer — `RunReplayCache`

Wraps `Runner::run`. Cache key:

```
sha256(
    question
  | "\0" | run_config_serialized (model, budget, include_thinking, stream_answer)
  | "\0" | for each allowed database: schema_snapshot_id_at_time_of_run
)
```

Stored value is the full event trace (ordered array of `Event`s). On replay, re-emits events at recorded timing or instantly (flag-controlled). The schema-snapshot piece is critical: the **same question** against a **changed schema** must miss.

### 9.3 Modes

| Mode | Miss behavior | Hit behavior |
|---|---|---|
| `Off` (prod default) | call backend | — |
| `Record` (fixture capture) | call backend, write cache | replay |
| `Replay` (tests, bug repro) | error `ReplayMiss` | replay |
| `ReadThrough` (staging) | call backend, write cache | replay |

Mode is selected by `RunConfig.replay_mode`, `KYMA_AGENT_REPLAY_MODE`, or header `X-Kyma-Agent-Replay: …`.

The two layers are composable but independent: you can record the inner layer (per-LLM-call fixtures) while the outer is `Off`, useful for unit tests that exercise the agent loop but pin the model behavior. Or record both for bit-exact E2E repro.

### 9.4 Semantic-stable live traffic

Off replay-cache, determinism comes from:

- `temperature=0`, `top_p=1`, `seed=42` (provider permitting) in every `GenerateRequest.params`.
- Frozen `SYSTEM_PROMPT_VERSION`, `TOOL_SCHEMA_VERSION`.
- Auth-scoped schema RAG (same caller sees same candidate tables in same order).
- pgvector index build is deterministic (same inputs → same index structure).

Cross-model-version drift is accepted; users who need bit-exact production repro must use `ReadThrough`.

## 10. Streaming event model

`AdkRunner::run` returns a `RunHandle { run_id, events: mpsc::Receiver<Event> }`. Events originate from three places:

- **ADK callbacks** (`before_model`/`after_model`/`before_tool`/`after_tool`) map to `Plan`/`ToolCall`/`ToolResult`. `before_model` may emit a `Plan` event (optional narration).
- **Provider token streams** — ADK's `LlmResponseStream` `Part::Text` → `AnswerDelta`; `Part::Thinking` → `ThinkingDelta` (suppressed unless `RunConfig.include_thinking=true`, per the Q7a decision).
- **Internal lifecycle** — `RunStarted` before the loop, `RunFinished` after (usage + `replay_cache_hit`), `RunError` on fatal failure.

Frontend marshaling:

- **Stdio MCP**: each `Event` → one MCP `notifications/progress` with the event type in `params`. `AnswerFinal` is returned as the `tools/call` response content.
- **HTTP+SSE MCP**: each `Event` → one SSE frame keyed by event type. `AnswerFinal` ends the stream.
- **Web UI**: consumes HTTP+SSE directly (new `AgentConsole` view).

Non-streaming `ask()` callers (`stream_answer=false`) receive a single `AnswerFinal`-shaped response; the runner buffers events internally. Event trace is still persisted.

## 11. MCP frontends (`kyma-mcp`)

### 11.1 Stdio binary

`cargo install --path crates/kyma-mcp` produces `kyma-mcp`. Two modes, env-selected:

- **`remote`** (default): `KYMA_URL=http://…` + `KYMA_TOKEN=…`. The binary is a thin client that:
  - owns the `AdkRunner + ReplayCache` stack in-process;
  - calls the running kyma server over HTTP for catalog / `run_kql` / `run_sql` / `sample_rows` / `explain_query` / `graph_*`;
  - runs `embed` and `vector_search` locally (embedding backend is in-proc; vector search calls the server's query API).
- **`embedded`** (single-binary demo): `KYMA_MCP_MODE=embedded`, `DATABASE_URL=…`, `OBJECT_STORE_URL=…`. Instantiates catalog + exec + embedder + runner in-process. Same surface, no server dependency.

Tool listing: `tools/list` returns `ask` + the 12 raw tools. The `ask` tool's args are `{question, database?, model?, include_thinking?, stream?}`. Streaming is via `notifications/progress` against the in-flight JSON-RPC request ID.

Config via env or optional `--config path/to/kyma.yaml`. Claude Desktop's config entry looks like:

```json
{
  "mcpServers": {
    "kyma": {
      "command": "kyma-mcp",
      "env": { "KYMA_URL": "http://localhost:8080", "KYMA_TOKEN": "tok_..." }
    }
  }
}
```

### 11.2 HTTP+SSE routes inside `kyma-server`

Mounted via `kyma_mcp::router()` at `/mcp/v1/`:

- `POST /mcp/v1/rpc` — JSON-RPC over HTTP for `tools/list`, `tools/call` (non-streaming invocations), initialization handshake.
- `GET /mcp/v1/events/:session_id` — SSE stream for server-initiated notifications and `ask()` streaming.
- `POST /mcp/v1/sessions` — create a session, returns `{session_id}`.

Authentication: existing `KYMA_AUTH_TOKENS` bearer middleware. `Authorization: Bearer <token>` on every request. The token's role must be ≥ `read`. `allowed_databases` is derived from `KYMA_AUTH_TOKEN_DBS=tok:db1,db2;tok2:*`, default `*` (all). The token's `AuthSubject` is threaded into every `RunConfig.auth_subject` and every `ToolContext`.

### 11.3 Budget enforcement

`BudgetEnforcer` wraps every `Runner::run`:

- `max_tool_calls` — checked in `before_tool` callback; exceeding emits `RunError { code: "budget_exceeded" }` and terminates.
- `max_wall_ms` — `tokio::time::timeout` around the whole `run()`. Per-tool `timeout_ms` arguments (on `run_kql`, `run_sql`) bound individual tool calls; the run-level wall is the ceiling across the whole loop. The smaller of `tool.timeout_ms` and `budget_remaining.wall_ms` is enforced per tool call so a long-running query cannot eat past the run budget.
- `max_tokens` — accumulated from `after_model` callbacks; checked after each model turn.
- `max_concurrent_runs_per_subject` — `tokio::sync::Semaphore` per auth subject. New runs over the cap get HTTP 429 with `Retry-After: 1`.

Header overrides on the HTTP surface (defaulted from env):

| Header | Default env | Default value |
|---|---|---|
| `X-Kyma-Agent-Max-Tool-Calls` | `KYMA_AGENT_MAX_TOOL_CALLS` | 16 |
| `X-Kyma-Agent-Max-Wall-Ms` | `KYMA_AGENT_MAX_WALL_MS` | 60000 |
| `X-Kyma-Agent-Max-Tokens` | `KYMA_AGENT_MAX_TOKENS` | 32000 |
| `X-Kyma-Agent-Replay` | `KYMA_AGENT_REPLAY_MODE` | `off` |

Prometheus metrics:

- `kyma_agent_runs_total{status, model}` (counter)
- `kyma_agent_tool_calls_total{tool, status}` (counter)
- `kyma_agent_tokens_total{model, kind}` (counter; kind = prompt|completion)
- `kyma_agent_budget_exceeded_total{kind}` (counter; kind = tool_calls|wall_ms|tokens|concurrency)
- `kyma_agent_run_duration_seconds{model, status}` (histogram)
- `kyma_agent_replay_hits_total{layer, mode}` (counter)
- `kyma_agent_schema_embed_updates_total{reason}` (counter; reason = ddl|refresh)

## 12. Error taxonomy

Stable codes on both `ToolError` and `RunError`:

| Code | Meaning | Retryable |
|---|---|---|
| `not_found` | Database/table/column/run doesn't exist | no |
| `forbidden` | Auth subject not allowed for this target | no |
| `invalid_args` | Tool args failed schema/semantic validation | no |
| `timeout` | Individual tool call exceeded its deadline | yes |
| `budget_exceeded` | Run-level budget (tool_calls/wall_ms/tokens) hit | no |
| `backend_unavailable` | LLM provider failed/unreachable | yes |
| `replay_miss` | Replay mode enabled but no cached response | no |
| `schema_drift` | Schema changed during run (snapshot mismatch) | yes |
| `tool_loop` | Same tool called with identical args 3×+ — runner aborts | no |
| `internal` | Unhandled panic/bug | no |

`tool_loop` detection: the runner keeps a 3-entry ring buffer of `(tool, canonical_args_hash)` per run; a match within the ring aborts with this code.

Frontend error marshaling:

- **Stdio MCP**: JSON-RPC error response with structured `data: { code, retryable, hint? }`.
- **HTTP+SSE MCP** (streaming): `RunError` event then clean stream close (HTTP 200 because the SSE was opened).
- **HTTP `ask()` non-streaming**: HTTP 200 with a response envelope `{status: "error", error: { code, message, retryable, hint? }}` — status codes only for transport-level failures (401, 429, 5xx).

## 13. Testing strategy

### 13.1 Unit tests (per crate)

- **`kyma-embed`**: `FastembedBackend::embed("hello world")` produces the expected 384-dim vector (committed golden file). Dimension / L2-norm asserts per model.
- **`kyma-agent-core`**: `Budget` accounting; `ReplayCache` contract against an in-memory store; `Event` canonical serialization; `AuthSubject` SQL-builder helpers.
- **`kyma-agent-tools`**: each tool tested against an in-memory mock `Catalog` / `Executor` / `Embedder`. `search_schema` ranking against seeded corpus. `vector_search` ordering against seeded vectors.
- **`kyma-agent-adk`**: test asserts `SHA256(system_prompt_template_text) == committed_hash`; any edit to the prompt literal must bump `SYSTEM_PROMPT_VERSION` and update the committed hash in the same PR — catches accidental prompt drift that would silently rotate the replay cache. `event_bridge.rs` property test: every `adk::Event` variant maps to exactly one `kyma::Event` (or is intentionally dropped).
- **`kyma-mcp`**: request/response JSON-RPC roundtrip through an in-memory stdio transport.

### 13.2 Integration tests (testcontainers Postgres + MinIO)

Build on the existing `kyma-catalog` IT pattern. New cases:

- `test_pgvector_migration_applies_and_extension_loads`
- `test_ddl_embedding_transaction_commits_together`
- `test_ddl_embedding_transaction_rolls_back_together`
- `test_search_schema_ranks_seeded_corpus`
- `test_vector_column_ingest_roundtrip`
- `test_cosine_distance_udf_registered`
- `test_vector_search_top_k_deterministic`
- `test_schema_sample_refresher_noops_when_text_unchanged`

### 13.3 E2E scripts (match existing `scripts/test-*.sh` pattern)

Seven new scripts, each using `KYMA_AGENT_REPLAY_MODE=replay` against fixtures committed to the repo — CI never calls a live LLM:

1. `test-agent-basic.sh` — HTTP+SSE `ask()` flow: seed tables → ask English question → assert answer mentions expected tables and cites a KQL snippet. Also verifies all expected events arrived.
2. `test-agent-mcp-stdio.sh` — spawn `kyma-mcp` as a subprocess, send JSON-RPC `tools/list` and `tools/call` via stdin, assert responses.
3. `test-agent-vectors.sh` — create a table with `vector(384)` column, ingest rows with embeddings, call `vector_search` tool with `query_text` and `query_vec`, assert ordering.
4. `test-agent-replay.sh` — run with `record` mode to capture; run again with `replay`; assert byte-identical event stream.
5. `test-agent-budgets.sh` — force `max_tool_calls=1`; assert `budget_exceeded` emitted and metrics incremented.
6. `test-agent-auth.sh` — token scoped to `db_a` cannot list/search/query `db_b`. Assert `forbidden` for direct calls and absence-from-results for `list_databases` / `search_schema`.
7. `test-agent-schema-drift.sh` — record a run; add a column; replay → outer cache misses; re-record → trace includes the new column in `describe_table` output.

### 13.4 Fixture management

Replay fixtures at `crates/kyma-agent-adk/tests/fixtures/replay/{scenario}.json`. Regenerated via:

```bash
KYMA_AGENT_REPLAY_MODE=record \
  cargo test -p kyma-agent-adk -- --ignored regen_fixtures
```

Each fixture is a JSON document `{ cache_key, generate_request, response, captured_at, model_id, kyma_version }`. Review in PRs when they change — prompt/tool-schema changes should visibly rotate keys.

### 13.5 Target state

- 12 crate-level test suites (5 new unit suites + 8 new integration IT cases in `kyma-catalog` + existing suites).
- 7 new E2E scripts → 17 E2E total.
- Zero CI dependence on an external LLM endpoint.

## 14. Seams for Spec 2 (workflow engine + Context Graph)

Decisions made in Slice 1 that preserve optionality for Spec 2:

- **`Runner` trait is the workflow primitive.** Spec 2's DAG executor submits `run()` calls with priority/budget presets. The `BudgetEnforcer` generalizes to a resource governor with global concurrency slots per priority class — additive, not a rewrite.
- **`EmbeddingUpdater` + semaphore** is the prototype for every background-step's resource discipline.
- **`agent_runs` schema** is the trace model workflow runs will reuse. No change needed.
- **`search_schema` → `search_graph` extension.** Spec 2 adds a `graph_node_embeddings` table (same columns as `schema_embeddings` + a `node_kind` enum). `search_schema` internally union-s the two views with an optional `kinds` filter — callers see a superset, no breaking change.
- **Context Graph ingest path.** Spec 2's graph-populating agents will store nodes + relationships as regular kyma tables (consistent with `docs/graphs.md`). Metadata for each node references `column_metadata` and `schema_embeddings` already defined here.
- **Stable error taxonomy, stable tool names, stable event schema.** These are the contract for external MCP agents AND for Spec 2 internal workflow agents. Treated as a public API from day 1.

Spec 2 will almost certainly need: mid-run event persistence (workflow agents can run for hours and no client is connected), pluggable triggers (schedule / on-ingest / on-demand), cross-run dependency linking (DAG edges). None of those are contradicted by Slice 1.

## 15. Open questions for Spec 2 (not Slice 1)

Flagged here so they don't surprise us later:

- **Provenance:** when the Context Graph says "service A depends on service B," what run produced that assertion and how do we decay confidence over time?
- **Fairness:** if two tenants each submit 1000 background graph-enrichment runs, how does the scheduler interleave them? (Likely: priority queue per tenant + global token-bucket.)
- **Multi-cluster:** does the replay cache federate? Probably scoped per cluster in Slice 1; Spec 2 may introduce a signed cross-cluster cache protocol.
- **Write tools:** if Spec 2 agents need to create tables (e.g., to materialize enrichment outputs), the trust-boundary design (dry-run, staged commits, auditable diffs) belongs there.

## 16. Rollout sketch

Not a plan (that's the next document), but a shape:

- **Week 1-2:** `kyma-embed` + pgvector migration + `vector` type in `DataType` + ingest coercion + DataFusion UDFs + KQL `<->` sugar. End-to-end: `CREATE TABLE t (v vector(384))` → ingest → `SELECT * FROM t ORDER BY v <-> @q LIMIT 5` works.
- **Week 3-4:** `kyma-agent-core` traits + `ReplayCache` implementations + `BudgetEnforcer` + unit-test skeleton.
- **Week 5-7:** `kyma-agent-tools` (twelve tools) + `SchemaRagIndex` + `EmbeddingUpdater` + `SchemaSampleRefresher` + integration tests.
- **Week 8-9:** `kyma-agent-adk` adapter + system-prompt V1 + provider factory + tool/event bridges + fixture recording.
- **Week 10-11:** `kyma-mcp` stdio binary + HTTP+SSE routes in `kyma-server` + auth wiring + Prometheus metrics.
- **Week 12:** E2E script suite + fixture commit + Claude Desktop / Cursor dogfood + docs.

12 weeks of ambitious-but-plausible slice-1 scope, consistent with the project's existing cadence of substantial-but-shipped slices.
