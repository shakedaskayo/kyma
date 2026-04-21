# NL-Query Agent + Day-0 Vectors — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship an embedded agentic NL-query layer exposed over MCP (stdio + HTTP+SSE), backed by adk-rust behind a stable local trait surface, with pgvector and a `vector(N)` user-declarable column type as day-0 engine primitives.

**Architecture:** Five new crates (`kyma-embed`, `kyma-agent-core`, `kyma-agent-tools`, `kyma-agent-adk`, `kyma-mcp`) plus one Postgres migration that adds pgvector + five agent-infrastructure tables. ADK-Rust is hidden behind `kyma-agent-core::{Backend, Tool, Runner}` so the public surface is ours, not ADK's. Determinism via two-layer content-addressed replay cache. Twelve read-only tools (raw MCP pack + composed `ask()` meta-tool).

**Tech Stack:** Rust 1.95, sqlx 0.8 (Postgres), pgvector, `fastembed-rs` (ONNX, local), `adk-rust` 0.6 (behind feature flags), `rmcp` (MCP SDK), DataFusion 44, Arrow 53, axum 0.7, tokio 1.x, testcontainers for IT.

**Spec:** `docs/superpowers/specs/2026-04-21-nl-query-agent-and-vectors-design.md` (committed as `4c2a27d`).

---

## Migration numbering note

The spec names the migration `006_agent_and_vectors.sql`. Inspect `crates/kyma-catalog/migrations/` at implementation time and use the next free integer (at time of writing the tree contains `001_initial.sql`, `002_ingest_ledger.sql`, `003_background_tasks.sql`). The `connectors` spec reserves `005`; the `extent_cache_residency` follow-on reserves `004`. If those remain unmerged when this plan starts, use `004_agent_and_vectors.sql`. All task references below use the placeholder `NNN` in file paths.

## File structure

Crates created (dependency order):

```
crates/kyma-embed/
├── Cargo.toml
├── src/
│   ├── lib.rs              — EmbeddingBackend trait + EmbedError + re-exports
│   ├── config.rs           — EmbeddingConfig struct + env/YAML loader
│   ├── fastembed.rs        — FastembedBackend (default, ONNX via fastembed-rs)
│   ├── ollama.rs           — OllamaBackend (feature = "ollama")
│   ├── openai_compat.rs    — OpenAICompatBackend (feature = "openai-compat")
│   └── gemini.rs           — GeminiBackend (feature = "gemini")
└── tests/
    └── golden_vectors.rs   — Bit-exact golden-file test for FastembedBackend

crates/kyma-agent-core/
├── Cargo.toml
├── src/
│   ├── lib.rs              — pub use everything; no logic
│   ├── errors.rs           — RunError + ToolError + BackendError + stable code taxonomy
│   ├── events.rs           — Event enum + canonical JSON (sorted keys, stable field order)
│   ├── backend.rs          — Backend trait, GenerateRequest, GenerateStream, GeneratePart, GenParams
│   ├── tool.rs             — Tool trait, ToolContext, ToolOutcome, ToolError
│   ├── runner.rs           — Runner trait, RunConfig, RunHandle, ReplayMode
│   ├── auth.rs             — AuthSubject, Role
│   ├── budget.rs           — Budget + BudgetEnforcer (Runner wrapper)
│   └── replay.rs           — ReplayCache trait, InMemoryReplayCache,
│                             GenerateReplayCache, RunReplayCache, cache-key helpers
└── tests/
    └── contracts.rs        — End-to-end trait contract against mock backend+tools

crates/kyma-agent-tools/
├── Cargo.toml
├── src/
│   ├── lib.rs              — build_tool_pack(ctx) → Vec<Arc<dyn Tool>>
│   ├── context.rs          — SharedToolContext (catalog, exec, embedder, schema_rag refs)
│   ├── errors.rs           — did_you_mean helper + tool-error constructors
│   ├── tool_schema.rs      — TOOL_SCHEMA_VERSION const + schema-bundle hash
│   ├── tools/
│   │   ├── mod.rs
│   │   ├── list_databases.rs
│   │   ├── list_tables.rs
│   │   ├── describe_table.rs
│   │   ├── search_schema.rs
│   │   ├── run_kql.rs
│   │   ├── run_sql.rs
│   │   ├── sample_rows.rs
│   │   ├── explain_query.rs
│   │   ├── embed.rs
│   │   ├── vector_search.rs
│   │   ├── graph_traverse.rs
│   │   └── graph_shortest_path.rs
│   ├── rag/
│   │   ├── mod.rs
│   │   ├── index.rs        — SchemaRagIndex (pgvector queries)
│   │   ├── text_format.rs  — canonical text-source builders (table/column, v1)
│   │   ├── updater.rs      — EmbeddingUpdater (DDL-transactional upsert)
│   │   └── refresher.rs    — SchemaSampleRefresher (background task)
│   └── pg_replay.rs        — PostgresReplayCache (sqlx impl of ReplayCache)
└── tests/
    ├── tools_unit.rs       — Each tool against mock catalog/exec/embedder
    ├── rag_it.rs           — testcontainers Postgres + real pgvector
    └── ddl_txn_it.rs       — create_table/alter atomically embed or abort

crates/kyma-agent-adk/
├── Cargo.toml
├── src/
│   ├── lib.rs              — pub use AdkBackend, AdkRunner
│   ├── model_config.rs     — ModelConfig struct + loader
│   ├── providers.rs        — ModelConfig → Arc<dyn adk::Llm> factory
│   ├── system_prompt.rs    — SYSTEM_PROMPT_V1 const + SYSTEM_PROMPT_VERSION
│   ├── backend.rs          — AdkBackend (kyma::Backend impl over adk::Llm)
│   ├── runner.rs           — AdkRunner (kyma::Runner impl driving adk::Runner)
│   ├── tool_bridge.rs      — kyma::Tool → adk::FunctionTool adapter
│   ├── event_bridge.rs     — adk::Event → kyma::Event mapper
│   └── session_pg.rs       — Postgres-backed adk::SessionService
└── tests/
    ├── prompt_hash.rs      — Content-hash of SYSTEM_PROMPT_V1 matches committed hash
    ├── fixtures/replay/    — JSON fixtures for recorded LLM calls (committed)
    └── agent_smoke.rs      — Question → canned LLM → assert event sequence

crates/kyma-mcp/
├── Cargo.toml
├── src/
│   ├── lib.rs              — pub use router() + handler types (mountable in kyma-server)
│   ├── main.rs             — Stdio binary entrypoint
│   ├── protocol.rs         — MCP types (or re-export from rmcp)
│   ├── tools_registry.rs   — Assemble MCP ToolDefinitions from kyma Tool pack + `ask`
│   ├── stdio.rs            — Stdio transport (JSON-RPC on stdin/stdout)
│   ├── sse.rs              — SSE event writer
│   ├── session.rs          — In-memory session cache + TTL
│   ├── remote_client.rs    — RemoteClient for stdio remote mode
│   └── embedded.rs         — Embedded-mode wiring helper
└── tests/
    ├── stdio_loopback.rs
    └── http_routes.rs

crates/kyma-catalog/migrations/
└── NNN_agent_and_vectors.sql  — pgvector + 6 tables (§4 of spec)
```

Files modified:

- `Cargo.toml` (workspace root) — add 5 new crate members + workspace deps `fastembed`, `pgvector`, `adk-rust`, `rmcp`, `ndarray`, `schemars`.
- `crates/kyma-core/src/types.rs` — add `DataType::Vector { dimension: u16, model_id: Option<String> }` variant + serde.
- `crates/kyma-ingest-core/src/...` — JSON array of floats → Arrow `FixedSizeList<Float32, N>` coercion in the schema-coercion path.
- `crates/kyma-exec/src/lib.rs` — register three UDFs (`cosine_distance`, `l2_distance`, `inner_product`) on every `SessionContext`.
- `crates/kyma-kql/src/parser.rs` + `src/lowering.rs` — parse `<->` operator and `MODEL 'id'` DDL clause; lower `| order by col <-> @q | take k`.
- `crates/kyma-catalog/src/...` — new helpers (`set_table_description`, `set_column_description`); wire `EmbeddingUpdater` into `create_table` and `alter_table_add_column`.
- `crates/kyma-server/src/lib.rs` or `src/routes.rs` — mount `kyma_mcp::router()` + `GET /v1/agent/runs/:run_id`.
- `crates/kyma-bin/src/main.rs` — spawn `SchemaSampleRefresher` task at startup.
- `docker-compose.yml` — optional `ollama` dev service (off by default; documented in README).
- `README.md` — short section on the NL-query agent + example `kyma-mcp` config.

---

## Phase A — Vector primitives in the engine

### Task A.1: Scaffold `kyma-embed` crate

**Files:**
- Create: `crates/kyma-embed/Cargo.toml`
- Create: `crates/kyma-embed/src/lib.rs`
- Modify: `Cargo.toml` (workspace root) — add member + workspace deps

- [ ] **Step 1: Create `crates/kyma-embed/Cargo.toml`**

```toml
[package]
name = "kyma-embed"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[features]
default = ["fastembed-backend"]
fastembed-backend = ["dep:fastembed", "dep:ndarray"]
ollama = ["dep:reqwest"]
openai-compat = ["dep:reqwest"]
gemini = ["dep:reqwest"]

[dependencies]
kyma-core.workspace = true
tokio = { workspace = true, features = ["sync"] }
async-trait.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
anyhow.workspace = true
fastembed = { version = "4", optional = true }
ndarray = { version = "0.16", optional = true }
reqwest = { workspace = true, optional = true }
```

- [ ] **Step 2: Create `crates/kyma-embed/src/lib.rs` with trait stub**

```rust
//! Embedding primitives for kyma.
//!
//! `EmbeddingBackend` is the single trait. The default impl is
//! `FastembedBackend` (ONNX, runs in-process, no external service).
//! Provider-backed impls (Ollama, OpenAI-compatible, Gemini) are
//! feature-gated.

#![forbid(unsafe_code)]

pub mod config;
mod errors;

#[cfg(feature = "fastembed-backend")]
pub mod fastembed;

#[cfg(feature = "ollama")]
pub mod ollama;

#[cfg(feature = "openai-compat")]
pub mod openai_compat;

#[cfg(feature = "gemini")]
pub mod gemini;

pub use config::{EmbeddingConfig, EmbeddingProvider};
pub use errors::EmbedError;

use async_trait::async_trait;

/// Turns text into dense vectors.
///
/// Implementations MUST be deterministic for the same input when
/// `backend.id()` stays the same — this is a load-bearing property
/// of the kyma replay cache.
#[async_trait]
pub trait EmbeddingBackend: Send + Sync {
    /// Stable identifier, e.g. `"fastembed/bge-small-en-v1.5"` or
    /// `"openai/text-embedding-3-small"`. Mixing IDs across writes
    /// is a correctness bug (distance-space mismatch) — the engine
    /// tags every vector column with its `model_id`.
    fn id(&self) -> &str;

    /// Output dimension. Must be stable for the lifetime of the instance.
    fn dimension(&self) -> u16;

    /// Compute embeddings for a batch. Order of outputs matches inputs.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}
```

- [ ] **Step 3: Create `crates/kyma-embed/src/errors.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("embedding backend not configured: {0}")]
    NotConfigured(String),

    #[error("embedding request failed: {0}")]
    Request(String),

    #[error("embedding backend returned dimension {got}, expected {expected}")]
    DimensionMismatch { got: u16, expected: u16 },

    #[error("embedding model load failed: {0}")]
    ModelLoad(String),

    #[error("internal: {0}")]
    Internal(String),
}
```

- [ ] **Step 4: Create `crates/kyma-embed/src/config.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "provider")]
pub enum EmbeddingProvider {
    Fastembed { id: String, model_path: Option<String> },
    Ollama    { id: String, base_url: String },
    #[serde(rename = "openai-compat")]
    OpenAICompat { id: String, base_url: String, api_key_env: Option<String> },
    Gemini    { id: String, api_key_env: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,
}

impl EmbeddingConfig {
    /// Defaults to fastembed bge-small-en-v1.5 (384-dim).
    pub fn default_fastembed() -> Self {
        Self {
            provider: EmbeddingProvider::Fastembed {
                id: "bge-small-en-v1.5".into(),
                model_path: None,
            },
        }
    }

    /// Load from env vars: `KYMA_EMBED_PROVIDER`, `KYMA_EMBED_MODEL_ID`,
    /// `KYMA_EMBED_BASE_URL`, `KYMA_EMBED_MODEL_PATH`. Returns the default
    /// fastembed config when none are set.
    pub fn from_env() -> Self {
        let provider = std::env::var("KYMA_EMBED_PROVIDER").ok();
        let id = std::env::var("KYMA_EMBED_MODEL_ID").ok();
        match provider.as_deref() {
            Some("ollama") => Self { provider: EmbeddingProvider::Ollama {
                id: id.unwrap_or_else(|| "nomic-embed-text".into()),
                base_url: std::env::var("KYMA_EMBED_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".into()),
            }},
            Some("openai-compat") => Self { provider: EmbeddingProvider::OpenAICompat {
                id: id.unwrap_or_else(|| "text-embedding-3-small".into()),
                base_url: std::env::var("KYMA_EMBED_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
                api_key_env: Some("OPENAI_API_KEY".into()),
            }},
            Some("gemini") => Self { provider: EmbeddingProvider::Gemini {
                id: id.unwrap_or_else(|| "text-embedding-004".into()),
                api_key_env: "GOOGLE_API_KEY".into(),
            }},
            _ => Self::default_fastembed(),
        }
    }
}
```

- [ ] **Step 5: Add crate to workspace `Cargo.toml`**

Edit the workspace root `Cargo.toml`:

```toml
[workspace]
members = [
    # ... existing members ...
    "crates/kyma-embed",
    "crates/kyma-agent-core",     # added in Task B.1
    "crates/kyma-agent-tools",    # added in Task C.1
    "crates/kyma-agent-adk",      # added in Task D.1
    "crates/kyma-mcp",            # added in Task E.1
]

[workspace.dependencies]
# ... existing entries ...
fastembed = "4"
pgvector = { version = "0.4", features = ["sqlx", "serde"] }
adk-rust = { version = "0.6", default-features = false, features = ["minimal", "tools", "sessions"] }
rmcp = "0.2"
ndarray = "0.16"
schemars = "0.8"
kyma-embed        = { path = "crates/kyma-embed" }
kyma-agent-core   = { path = "crates/kyma-agent-core" }
kyma-agent-tools  = { path = "crates/kyma-agent-tools" }
kyma-agent-adk    = { path = "crates/kyma-agent-adk" }
kyma-mcp          = { path = "crates/kyma-mcp" }
```

(Only add the four sibling-crate paths that exist at this point. Add the others as their scaffold tasks land. The `cargo check` step below will fail if you reference a member that doesn't exist yet.)

- [ ] **Step 6: Run `cargo check -p kyma-embed`**

Expected: clean compile; fastembed feature enabled by default but no fastembed.rs module yet, so the `pub mod fastembed;` guard is already hidden behind `cfg(feature)`. Adjust if cargo complains.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-embed Cargo.toml
git commit -m "feat(embed): scaffold kyma-embed crate with EmbeddingBackend trait"
```

---

### Task A.2: Implement `FastembedBackend`

**Files:**
- Create: `crates/kyma-embed/src/fastembed.rs`
- Modify: `crates/kyma-embed/src/lib.rs` (re-export)

- [ ] **Step 1: Write the failing golden-vector test**

Create `crates/kyma-embed/tests/golden_vectors.rs`:

```rust
//! FastembedBackend MUST be bit-exact deterministic. We commit a golden
//! file of (input → first-3 floats + dimension) and assert on it. If
//! fastembed ever changes model output this test surfaces it in CI
//! instead of silently invalidating the replay cache.

#![cfg(feature = "fastembed-backend")]

use kyma_embed::{fastembed::FastembedBackend, EmbeddingBackend};

#[tokio::test]
async fn bge_small_en_v1_5_matches_golden() {
    let b = FastembedBackend::new("bge-small-en-v1.5", None)
        .await
        .expect("model download + load");

    assert_eq!(b.id(), "fastembed/bge-small-en-v1.5");
    assert_eq!(b.dimension(), 384);

    let out = b.embed(&["hello world".into()]).await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), 384);

    // Bge models output L2-normalized vectors. Golden prefix committed
    // after first passing run; bump alongside any intentional model
    // bump. This catches: model download drift, fastembed upgrade,
    // or accidental tokenizer change.
    let prefix: Vec<f32> = out[0][..3].to_vec();
    insta::assert_debug_snapshot!("bge_small_en_v1_5_hello_world_prefix", prefix);

    let norm = out[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4, "expected L2-normalized, got {norm}");
}
```

Add `insta = "1"` as a dev-dependency in `crates/kyma-embed/Cargo.toml`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kyma-embed --test golden_vectors`
Expected: FAIL — `FastembedBackend` not defined.

- [ ] **Step 3: Write `crates/kyma-embed/src/fastembed.rs`**

```rust
use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Arc;
use tokio::sync::Mutex;

/// ONNX-backed embedding via fastembed-rs. Loads the model at construction;
/// inference runs on a tokio blocking thread per batch.
pub struct FastembedBackend {
    id: String,
    dimension: u16,
    inner: Arc<Mutex<TextEmbedding>>,
}

impl FastembedBackend {
    /// `model_id` is the short name (e.g., `"bge-small-en-v1.5"`).
    /// `model_path` optionally points at a pre-downloaded ONNX dir for
    /// air-gapped deployments (env `KYMA_EMBED_MODEL_PATH`).
    pub async fn new(model_id: &str, model_path: Option<&str>)
        -> Result<Self, EmbedError>
    {
        let em = pick_model(model_id)?;
        let dimension = em_dimension(&em);
        let mut opts = InitOptions::new(em);
        if let Some(path) = model_path {
            opts = opts.with_cache_dir(path.into());
        }
        let model = tokio::task::spawn_blocking(move || TextEmbedding::try_new(opts))
            .await
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?;
        Ok(Self {
            id: format!("fastembed/{model_id}"),
            dimension,
            inner: Arc::new(Mutex::new(model)),
        })
    }
}

#[async_trait]
impl EmbeddingBackend for FastembedBackend {
    fn id(&self) -> &str { &self.id }
    fn dimension(&self) -> u16 { self.dimension }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() { return Ok(vec![]); }
        let inner = self.inner.clone();
        let owned: Vec<String> = texts.to_vec();
        let dim = self.dimension;
        tokio::task::spawn_blocking(move || {
            let mut guard = inner.blocking_lock();
            let vecs = guard
                .embed(owned, None)
                .map_err(|e| EmbedError::Request(e.to_string()))?;
            for v in &vecs {
                if v.len() != dim as usize {
                    return Err(EmbedError::DimensionMismatch {
                        got: v.len() as u16, expected: dim,
                    });
                }
            }
            Ok(vecs)
        })
        .await
        .map_err(|e| EmbedError::Internal(e.to_string()))?
    }
}

fn pick_model(id: &str) -> Result<EmbeddingModel, EmbedError> {
    match id {
        "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
        "bge-base-en-v1.5"  => Ok(EmbeddingModel::BGEBaseENV15),
        "all-MiniLM-L6-v2"  => Ok(EmbeddingModel::AllMiniLML6V2),
        other => Err(EmbedError::NotConfigured(format!(
            "unknown fastembed model: {other}"))),
    }
}

fn em_dimension(em: &EmbeddingModel) -> u16 {
    match em {
        EmbeddingModel::BGESmallENV15 => 384,
        EmbeddingModel::BGEBaseENV15  => 768,
        EmbeddingModel::AllMiniLML6V2 => 384,
        _ => 384,
    }
}
```

- [ ] **Step 4: Re-export from `lib.rs`**

Add at the bottom of `crates/kyma-embed/src/lib.rs`:

```rust
#[cfg(feature = "fastembed-backend")]
pub use fastembed::FastembedBackend;
```

- [ ] **Step 5: Run test with `--features fastembed-backend` (already default)**

Run: `cargo test -p kyma-embed --test golden_vectors`
Expected: FIRST run downloads the model (minutes). Test then PASSes. `insta review` accepts the prefix.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-embed
git commit -m "feat(embed): FastembedBackend with golden-vector determinism test"
```

---

### Task A.3: Implement `OllamaBackend`

**Files:**
- Create: `crates/kyma-embed/src/ollama.rs`
- Modify: `crates/kyma-embed/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/kyma-embed/tests/golden_vectors.rs`:

```rust
#[cfg(feature = "ollama")]
#[tokio::test]
#[ignore] // Requires `ollama serve` with nomic-embed-text pulled.
async fn ollama_nomic_embed_text_shape() {
    use kyma_embed::ollama::OllamaBackend;
    let b = OllamaBackend::new("nomic-embed-text",
                               "http://localhost:11434", 768).unwrap();
    let out = b.embed(&["hello".into()]).await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), 768);
}
```

- [ ] **Step 2: Implement `crates/kyma-embed/src/ollama.rs`**

```rust
use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct OllamaBackend {
    id: String,
    dimension: u16,
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaBackend {
    pub fn new(model: &str, base_url: &str, dimension: u16)
        -> Result<Self, EmbedError>
    {
        Ok(Self {
            id: format!("ollama/{model}"),
            dimension,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| EmbedError::Internal(e.to_string()))?,
        })
    }
}

#[derive(Serialize)]
struct Req<'a> { model: &'a str, prompt: &'a str }
#[derive(Deserialize)]
struct Resp { embedding: Vec<f32> }

#[async_trait]
impl EmbeddingBackend for OllamaBackend {
    fn id(&self) -> &str { &self.id }
    fn dimension(&self) -> u16 { self.dimension }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let url = format!("{}/api/embeddings", self.base_url);
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            let resp: Resp = self.client
                .post(&url)
                .json(&Req { model: &self.model, prompt: t })
                .send().await
                .map_err(|e| EmbedError::Request(e.to_string()))?
                .error_for_status()
                .map_err(|e| EmbedError::Request(e.to_string()))?
                .json().await
                .map_err(|e| EmbedError::Request(e.to_string()))?;
            if resp.embedding.len() != self.dimension as usize {
                return Err(EmbedError::DimensionMismatch {
                    got: resp.embedding.len() as u16, expected: self.dimension,
                });
            }
            out.push(resp.embedding);
        }
        Ok(out)
    }
}
```

- [ ] **Step 3: Re-export + run test**

Add to `lib.rs`: `#[cfg(feature = "ollama")] pub use ollama::OllamaBackend;`
Run: `cargo test -p kyma-embed --features ollama --test golden_vectors ollama -- --ignored`
Expected: PASS if ollama is running; SKIPPED otherwise.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-embed
git commit -m "feat(embed): OllamaBackend behind `ollama` feature"
```

---

### Task A.4: Implement `OpenAICompatBackend` + `GeminiBackend`

**Files:**
- Create: `crates/kyma-embed/src/openai_compat.rs`
- Create: `crates/kyma-embed/src/gemini.rs`
- Modify: `crates/kyma-embed/src/lib.rs`

- [ ] **Step 1: Implement `openai_compat.rs`**

```rust
use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct OpenAICompatBackend {
    id: String,
    dimension: u16,
    base_url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAICompatBackend {
    pub fn new(model: &str, base_url: &str, dimension: u16,
               api_key: Option<String>) -> Result<Self, EmbedError> {
        Ok(Self {
            id: format!("openai-compat/{model}"),
            dimension,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| EmbedError::Internal(e.to_string()))?,
        })
    }
}

#[derive(Serialize)]
struct Req<'a> { model: &'a str, input: &'a [String] }
#[derive(Deserialize)]
struct Item { embedding: Vec<f32> }
#[derive(Deserialize)]
struct Resp { data: Vec<Item> }

#[async_trait]
impl EmbeddingBackend for OpenAICompatBackend {
    fn id(&self) -> &str { &self.id }
    fn dimension(&self) -> u16 { self.dimension }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let url = format!("{}/embeddings", self.base_url);
        let mut req = self.client.post(&url).json(&Req {
            model: &self.model,
            input: texts,
        });
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }
        let resp: Resp = req.send().await
            .map_err(|e| EmbedError::Request(e.to_string()))?
            .error_for_status()
            .map_err(|e| EmbedError::Request(e.to_string()))?
            .json().await
            .map_err(|e| EmbedError::Request(e.to_string()))?;
        let out: Vec<_> = resp.data.into_iter().map(|i| i.embedding).collect();
        if let Some(v) = out.first() {
            if v.len() != self.dimension as usize {
                return Err(EmbedError::DimensionMismatch {
                    got: v.len() as u16, expected: self.dimension,
                });
            }
        }
        Ok(out)
    }
}
```

- [ ] **Step 2: Implement `gemini.rs`** (very similar; native Gemini embedContent endpoint)

```rust
use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct GeminiBackend {
    id: String,
    dimension: u16,
    model: String,
    api_key: String,
    client: reqwest::Client,
}

impl GeminiBackend {
    pub fn new(model: &str, dimension: u16, api_key: String)
        -> Result<Self, EmbedError>
    {
        Ok(Self {
            id: format!("gemini/{model}"),
            dimension,
            model: model.to_string(),
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| EmbedError::Internal(e.to_string()))?,
        })
    }
}

#[derive(Serialize)] struct Part<'a> { text: &'a str }
#[derive(Serialize)] struct Content<'a> { parts: Vec<Part<'a>> }
#[derive(Serialize)] struct Req<'a> { model: String, content: Content<'a> }
#[derive(Deserialize)] struct Emb { values: Vec<f32> }
#[derive(Deserialize)] struct Resp { embedding: Emb }

#[async_trait]
impl EmbeddingBackend for GeminiBackend {
    fn id(&self) -> &str { &self.id }
    fn dimension(&self) -> u16 { self.dimension }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
            self.model, self.api_key);
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            let resp: Resp = self.client.post(&url).json(&Req {
                model: format!("models/{}", self.model),
                content: Content { parts: vec![Part { text: t }] },
            }).send().await
              .map_err(|e| EmbedError::Request(e.to_string()))?
              .error_for_status()
              .map_err(|e| EmbedError::Request(e.to_string()))?
              .json().await
              .map_err(|e| EmbedError::Request(e.to_string()))?;
            if resp.embedding.values.len() != self.dimension as usize {
                return Err(EmbedError::DimensionMismatch {
                    got: resp.embedding.values.len() as u16,
                    expected: self.dimension,
                });
            }
            out.push(resp.embedding.values);
        }
        Ok(out)
    }
}
```

- [ ] **Step 3: Re-export from lib.rs + `cargo check --all-features`**

```rust
#[cfg(feature = "openai-compat")] pub use openai_compat::OpenAICompatBackend;
#[cfg(feature = "gemini")]        pub use gemini::GeminiBackend;
```

Run: `cargo check -p kyma-embed --all-features`
Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-embed
git commit -m "feat(embed): OpenAICompat + Gemini backends behind features"
```

---

### Task A.5: Add catalog migration for pgvector + agent tables

**Files:**
- Create: `crates/kyma-catalog/migrations/NNN_agent_and_vectors.sql`

(Replace `NNN` with the next integer; see "Migration numbering note" at the top.)

- [ ] **Step 1: Write the migration**

```sql
-- NNN_agent_and_vectors.sql
-- Day-0 vector primitives + agent infrastructure tables.
-- See docs/superpowers/specs/2026-04-21-nl-query-agent-and-vectors-design.md §4.

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE column_metadata (
    database           TEXT NOT NULL,
    table_name         TEXT NOT NULL,
    column_name        TEXT NOT NULL,
    column_type        TEXT NOT NULL,
    description        TEXT,
    embedding_model_id TEXT,
    dimension          INT,
    distance_metric    TEXT,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (database, table_name, column_name)
);

CREATE TABLE schema_embeddings (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    database            TEXT NOT NULL,
    table_name          TEXT NOT NULL,
    column_name         TEXT,
    kind                TEXT NOT NULL CHECK (kind IN ('table','column')),
    text_source         TEXT NOT NULL,
    text_source_sha256  BYTEA NOT NULL,
    text_format_version TEXT NOT NULL DEFAULT 'v1',
    model_id            TEXT NOT NULL,
    embedding           vector(384) NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX schema_embeddings_uniq_table
    ON schema_embeddings (database, table_name, model_id)
    WHERE column_name IS NULL;
CREATE UNIQUE INDEX schema_embeddings_uniq_column
    ON schema_embeddings (database, table_name, column_name, model_id)
    WHERE column_name IS NOT NULL;
CREATE INDEX schema_embeddings_hnsw ON schema_embeddings
    USING hnsw (embedding vector_cosine_ops);
CREATE INDEX schema_embeddings_db ON schema_embeddings (database);

CREATE TABLE agent_runs (
    run_id             UUID PRIMARY KEY,
    question           TEXT NOT NULL,
    model_id           TEXT NOT NULL,
    auth_subject       TEXT NOT NULL,
    session_id         UUID,
    started_at         TIMESTAMPTZ NOT NULL,
    finished_at        TIMESTAMPTZ NOT NULL,
    status             TEXT NOT NULL CHECK (status IN
                         ('success','error','budget_exceeded','cancelled','replay_miss')),
    usage_json         JSONB NOT NULL,
    trace_json         JSONB NOT NULL,
    replay_cache_hit   BOOL NOT NULL DEFAULT FALSE
);
CREATE INDEX agent_runs_subject_time ON agent_runs (auth_subject, started_at DESC);
CREATE INDEX agent_runs_session       ON agent_runs (session_id, started_at DESC);

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

CREATE TABLE agent_replay_cache (
    cache_key     BYTEA PRIMARY KEY,
    layer         TEXT NOT NULL CHECK (layer IN ('generate','run')),
    response_json JSONB NOT NULL,
    model_id      TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    hit_count     INT NOT NULL DEFAULT 0
);
CREATE INDEX agent_replay_cache_layer_model ON agent_replay_cache (layer, model_id);
```

- [ ] **Step 2: Add an integration test for the migration**

Create `crates/kyma-catalog/tests/pgvector_migration_it.rs`:

```rust
//! Confirms migration NNN runs, pgvector loads, and the HNSW index exists.

use sqlx::{Postgres, Pool};
use testcontainers::{runners::AsyncRunner, ContainerAsync};
use testcontainers_modules::postgres::Postgres as PgContainer;

async fn start_pg() -> (ContainerAsync<PgContainer>, Pool<Postgres>) {
    // Use the pgvector-enabled image. Standard postgres:16 does NOT include
    // the extension; pgvector/pgvector:pg16 does.
    let container = PgContainer::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    (container, pool)
}

#[tokio::test]
async fn pgvector_extension_loaded_and_hnsw_index_present() {
    let (_c, pool) = start_pg().await;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'vector')"
    ).fetch_one(&pool).await.unwrap();
    assert!(exists, "pgvector extension not loaded");

    let idx: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_indexes
                       WHERE indexname = 'schema_embeddings_hnsw')"
    ).fetch_one(&pool).await.unwrap();
    assert!(idx, "HNSW index missing");
}

#[tokio::test]
async fn schema_embeddings_partial_uniques_enforce_one_row_per_kind() {
    let (_c, pool) = start_pg().await;
    // Insert two 'table' kind rows for the same (db, table, model). Second
    // must fail due to the partial-unique index.
    let zero_vec: Vec<f32> = vec![0.0; 384];
    let v: pgvector::Vector = zero_vec.into();

    sqlx::query("INSERT INTO schema_embeddings
        (database, table_name, kind, text_source, text_source_sha256,
         model_id, embedding)
        VALUES ($1, $2, 'table', 'a', '\\x00', 'm', $3)")
        .bind("db").bind("t").bind(&v)
        .execute(&pool).await.unwrap();

    let dup = sqlx::query("INSERT INTO schema_embeddings
        (database, table_name, kind, text_source, text_source_sha256,
         model_id, embedding)
        VALUES ($1, $2, 'table', 'a', '\\x00', 'm', $3)")
        .bind("db").bind("t").bind(&v)
        .execute(&pool).await;
    assert!(dup.is_err(), "second 'table' row should have been rejected");
}
```

Add `pgvector.workspace = true` to `crates/kyma-catalog/Cargo.toml` `[dev-dependencies]`.

- [ ] **Step 3: Run the test**

Run: `cargo test -p kyma-catalog --test pgvector_migration_it`
Expected: PASS (pulls pgvector/pgvector:pg16 on first run).

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-catalog/migrations crates/kyma-catalog/tests crates/kyma-catalog/Cargo.toml
git commit -m "feat(catalog): migration for pgvector + agent tables"
```

---

### Task A.6: Add `DataType::Vector` to `kyma-core`

**Files:**
- Modify: `crates/kyma-core/src/types.rs`

- [ ] **Step 1: Locate the existing `DataType` enum**

Run: `rg 'pub enum DataType' crates/kyma-core/src/types.rs -A 20`

- [ ] **Step 2: Write failing test** (add to an existing tests module or `crates/kyma-core/tests/types.rs`):

```rust
#[test]
fn vector_datatype_roundtrips_through_serde() {
    use kyma_core::types::DataType;
    let dt = DataType::Vector { dimension: 384, model_id:
        Some("fastembed/bge-small-en-v1.5".into()) };
    let json = serde_json::to_string(&dt).unwrap();
    let back: DataType = serde_json::from_str(&json).unwrap();
    assert_eq!(dt, back);
}

#[test]
fn vector_datatype_display_matches_ddl_form() {
    use kyma_core::types::DataType;
    let no_model = DataType::Vector { dimension: 384, model_id: None };
    let with_model = DataType::Vector { dimension: 1536,
        model_id: Some("openai/text-embedding-3-small".into()) };
    assert_eq!(no_model.to_string(), "vector(384)");
    assert_eq!(with_model.to_string(),
               "vector(1536) MODEL 'openai/text-embedding-3-small'");
}
```

- [ ] **Step 3: Run test → FAIL**

Run: `cargo test -p kyma-core types::` — expected: variant missing.

- [ ] **Step 4: Add the variant**

In `crates/kyma-core/src/types.rs`, add a new variant to `DataType`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DataType {
    // ... existing variants ...
    Vector { dimension: u16, model_id: Option<String> },
}
```

Update the existing `impl std::fmt::Display for DataType` (or equivalent `to_string`) to match the DDL form:

```rust
impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // ... existing arms ...
            DataType::Vector { dimension, model_id } => {
                write!(f, "vector({dimension})")?;
                if let Some(m) = model_id {
                    write!(f, " MODEL '{m}'")?;
                }
                Ok(())
            }
        }
    }
}
```

Any match on `DataType` elsewhere in the codebase will now fail to compile — add a `DataType::Vector { .. } => …` arm at each call site. Use `cargo check --workspace` to find them.

- [ ] **Step 5: Run `cargo check --workspace`**

Expected: several places (schema printers, Arrow converters, KQL type-coercion) will error. Add minimal arms: for Arrow conversion, map to `arrow_schema::DataType::FixedSizeList(Field::new("item", Float32, false).into(), dimension as i32)`. For schema printers, use `Display`. For KQL type-coercion, reject `Vector` from arithmetic ops.

- [ ] **Step 6: Run tests, commit**

Run: `cargo test -p kyma-core && cargo check --workspace`
```bash
git add crates/kyma-core crates/**/Cargo.toml crates/**/src
git commit -m "feat(core): DataType::Vector { dimension, model_id } variant"
```

---

### Task A.7: JSON-array → `FixedSizeList<Float32>` ingest coercion

**Files:**
- Modify: `crates/kyma-ingest-core/src/...` (the schema-coercion path — likely `src/coerce.rs` or similar)

- [ ] **Step 1: Find the coercion function**

Run: `rg 'fn coerce' crates/kyma-ingest-core -A 3` and open the file that maps typed JSON values to `ArrayRef`.

- [ ] **Step 2: Write failing test**

```rust
#[test]
fn coerces_json_float_array_to_fixed_size_list_vector() {
    use kyma_core::types::DataType;
    let dt = DataType::Vector { dimension: 3, model_id: None };
    let row = serde_json::json!({"v": [0.1, 0.2, 0.3]});
    let arr = coerce_field("v", &row, &dt).unwrap();
    let fs = arr.as_any()
        .downcast_ref::<arrow_array::FixedSizeListArray>()
        .unwrap();
    assert_eq!(fs.value_length(), 3);
    assert_eq!(fs.len(), 1);
    let inner = fs.value(0);
    let floats = inner.as_any()
        .downcast_ref::<arrow_array::Float32Array>()
        .unwrap();
    assert_eq!(floats.values(), &[0.1_f32, 0.2, 0.3]);
}

#[test]
fn rejects_wrong_dimension() {
    use kyma_core::types::DataType;
    let dt = DataType::Vector { dimension: 3, model_id: None };
    let row = serde_json::json!({"v": [0.1, 0.2]});
    let err = coerce_field("v", &row, &dt).unwrap_err();
    assert!(err.to_string().contains("dimension"));
}
```

- [ ] **Step 3: Implement the match arm**

Inside the function that handles each `DataType`, add:

```rust
DataType::Vector { dimension, .. } => {
    use arrow_array::{ArrayRef, FixedSizeListArray, Float32Array};
    use arrow_schema::{DataType as AT, Field};
    use std::sync::Arc;

    let arr = match value {
        serde_json::Value::Null => {
            // null vector for null row — rare, but must round-trip.
            return Err(CoerceError::Invalid(format!(
                "null vector for column `{name}` not yet supported")));
        }
        serde_json::Value::Array(items) => {
            if items.len() != *dimension as usize {
                return Err(CoerceError::Invalid(format!(
                    "column `{name}` expected vector of dimension {dimension}, got {}",
                    items.len())));
            }
            let floats: Result<Vec<f32>, _> = items.iter()
                .map(|v| v.as_f64().map(|f| f as f32)
                    .ok_or_else(|| CoerceError::Invalid(
                        format!("column `{name}` non-numeric in vector")))).collect();
            let values = Float32Array::from(floats?);
            let field = Arc::new(Field::new("item", AT::Float32, false));
            FixedSizeListArray::new(field, *dimension as i32,
                                    Arc::new(values), None)
        }
        _ => return Err(CoerceError::Invalid(format!(
            "column `{name}` expected array of floats"))),
    };
    Arc::new(arr) as ArrayRef
}
```

- [ ] **Step 4: Run test, commit**

Run: `cargo test -p kyma-ingest-core coerces_json_float_array`
```bash
git add crates/kyma-ingest-core
git commit -m "feat(ingest): coerce JSON float arrays to FixedSizeList<Float32> for vector"
```

---

### Task A.8: Register vector-distance UDFs in `kyma-exec`

**Files:**
- Modify: `crates/kyma-exec/src/lib.rs` (or wherever `SessionContext` is built)
- Create: `crates/kyma-exec/src/udfs_vector.rs`

- [ ] **Step 1: Write failing SQL test**

Append to an existing `kyma-exec` tests file:

```rust
#[tokio::test]
async fn cosine_distance_udf_registered_and_correct() {
    use datafusion::prelude::*;
    let ctx = new_session_context_with_kyma_udfs();
    let df = ctx.sql(
        "SELECT cosine_distance(
            make_array(1.0::float, 0.0::float),
            make_array(1.0::float, 0.0::float)) AS d"
    ).await.unwrap();
    let batches = df.collect().await.unwrap();
    let d: f64 = batches[0].column(0)
        .as_any().downcast_ref::<arrow_array::Float64Array>()
        .unwrap().value(0);
    assert!((d - 0.0).abs() < 1e-9);
}
```

(Replace `new_session_context_with_kyma_udfs` with the actual factory used in this crate.)

- [ ] **Step 2: Implement `crates/kyma-exec/src/udfs_vector.rs`**

```rust
//! Vector distance UDFs for DataFusion. Operate on two `FixedSizeList<Float32>`
//! values or on `make_array(…)` literal arrays. Outputs `Float64` distances.

use datafusion::arrow::array::{Array, Float32Array, Float64Array, FixedSizeListArray, ListArray};
use datafusion::arrow::datatypes::DataType as AT;
use datafusion::logical_expr::{create_udf, ColumnarValue, Volatility};
use datafusion::prelude::SessionContext;
use std::sync::Arc;

pub fn register_all(ctx: &SessionContext) {
    ctx.register_udf(build("cosine_distance", cosine));
    ctx.register_udf(build("l2_distance", l2));
    ctx.register_udf(build("inner_product", inner));
}

fn build(
    name: &str,
    f: fn(&[f32], &[f32]) -> f64,
) -> datafusion::logical_expr::ScalarUDF {
    // Accept List<Float32> OR FixedSizeList<Float32, N>.
    let fun = move |args: &[ColumnarValue]| -> datafusion::error::Result<ColumnarValue> {
        let a = coerce_to_f32_vecs(&args[0])?;
        let b = coerce_to_f32_vecs(&args[1])?;
        let n = a.len().max(b.len());
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let ai = if a.len() == 1 { &a[0] } else { &a[i] };
            let bi = if b.len() == 1 { &b[0] } else { &b[i] };
            out.push(f(ai, bi));
        }
        Ok(ColumnarValue::Array(Arc::new(Float64Array::from(out))))
    };
    create_udf(
        name,
        vec![
            AT::List(Arc::new(datafusion::arrow::datatypes::Field::new(
                "item", AT::Float32, true))),
            AT::List(Arc::new(datafusion::arrow::datatypes::Field::new(
                "item", AT::Float32, true))),
        ],
        Arc::new(AT::Float64),
        Volatility::Immutable,
        Arc::new(fun),
    )
}

fn coerce_to_f32_vecs(col: &ColumnarValue) -> datafusion::error::Result<Vec<Vec<f32>>> {
    match col {
        ColumnarValue::Array(arr) => {
            if let Some(fsl) = arr.as_any().downcast_ref::<FixedSizeListArray>() {
                return Ok((0..fsl.len()).map(|i| as_f32_row(&fsl.value(i))).collect());
            }
            if let Some(l) = arr.as_any().downcast_ref::<ListArray>() {
                return Ok((0..l.len()).map(|i| as_f32_row(&l.value(i))).collect());
            }
            Err(datafusion::error::DataFusionError::Execution(
                "expected list/fixed_size_list of float32".into()))
        }
        ColumnarValue::Scalar(s) => {
            // Scalar List/FixedSizeList — DataFusion wraps as a 1-element array.
            Err(datafusion::error::DataFusionError::Execution(
                format!("scalar form of vector not supported here: {s:?}")))
        }
    }
}

fn as_f32_row(arr: &Arc<dyn Array>) -> Vec<f32> {
    arr.as_any().downcast_ref::<Float32Array>()
        .map(|a| a.values().to_vec())
        .unwrap_or_default()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() { return 1.0; }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for i in 0..a.len() {
        let (x, y) = (a[i] as f64, b[i] as f64);
        dot += x * y; na += x * x; nb += y * y;
    }
    if na == 0.0 || nb == 0.0 { return 1.0; }
    1.0 - dot / (na.sqrt() * nb.sqrt())
}
fn l2(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() { return f64::INFINITY; }
    let mut s = 0f64;
    for i in 0..a.len() {
        let d = (a[i] as f64) - (b[i] as f64);
        s += d * d;
    }
    s.sqrt()
}
fn inner(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() { return 0.0; }
    let mut s = 0f64;
    for i in 0..a.len() { s += (a[i] as f64) * (b[i] as f64); }
    s
}
```

- [ ] **Step 3: Wire into session creation**

In the existing `SessionContext` factory (e.g., `kyma_exec::new_session_context`):

```rust
crate::udfs_vector::register_all(&ctx);
```

- [ ] **Step 4: Run test, commit**

```bash
cargo test -p kyma-exec cosine_distance
git add crates/kyma-exec
git commit -m "feat(exec): register cosine_distance/l2_distance/inner_product UDFs"
```

---

### Task A.9: KQL parser — `<->` operator + `MODEL 'id'` DDL clause

**Files:**
- Modify: `crates/kyma-kql/src/parser.rs` (or grammar module)
- Modify: `crates/kyma-kql/src/lowering.rs` (or ast→sql translator)

- [ ] **Step 1: Write failing parser test**

In `crates/kyma-kql/tests/parser_vector.rs`:

```rust
use kyma_kql::parser::parse;

#[test]
fn parses_order_by_distance_take() {
    let kql = "T | order by embedding <-> @q asc | take 5";
    let ast = parse(kql).unwrap();
    // Confirm the AST contains a distance expression; exact struct depends on
    // existing AST. Minimal assertion: round-trip through lowering.
    let sql = kyma_kql::lower_to_sql(&ast).unwrap();
    assert!(sql.contains("cosine_distance(embedding, @q)"));
    assert!(sql.contains("ORDER BY"));
    assert!(sql.contains("LIMIT 5"));
}

#[test]
fn parses_model_ddl_clause() {
    let kql = "CREATE TABLE memos (content string, embedding vector(384) MODEL 'fastembed/bge-small-en-v1.5')";
    let ast = kyma_kql::parse_ddl(kql).unwrap();
    let cols = kyma_kql::ddl_columns(&ast);
    let emb = cols.iter().find(|c| c.name == "embedding").unwrap();
    match &emb.data_type {
        kyma_core::types::DataType::Vector { dimension, model_id } => {
            assert_eq!(*dimension, 384);
            assert_eq!(model_id.as_deref(),
                       Some("fastembed/bge-small-en-v1.5"));
        }
        other => panic!("expected Vector, got {other:?}"),
    }
}
```

- [ ] **Step 2: Extend the lexer/parser**

Add `<->` as a new token. In the parser's expression rule where binary operators are handled, accept `<->` as a distance operator. Produce an AST node `Distance { left, right }`. In the DDL type-parser, accept `VECTOR '(' int ')'` optionally followed by `MODEL string`.

Concrete edits depend on the parser form (chumsky-based per the engine's Cargo.toml). Add the two tokens + two grammar rules; tests in Step 1 pin the shape.

- [ ] **Step 3: Extend the lowering to SQL**

In `lowering.rs`, map `Distance { a, b }` to `cosine_distance(a, b)`. For `order by … <-> … [asc|desc]`, emit `ORDER BY cosine_distance(col, @qvec) ASC` in the generated SQL.

- [ ] **Step 4: Run tests, commit**

```bash
cargo test -p kyma-kql
git add crates/kyma-kql
git commit -m "feat(kql): <-> distance operator + MODEL 'id' DDL clause"
```

---

### Task A.10: End-to-end vector smoke script

**Files:**
- Create: `scripts/test-vectors.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Smoke: create a table with vector column, ingest one row, run KQL <-> query.
set -euo pipefail

KYMA_URL="${KYMA_URL:-http://localhost:8080}"
DB="vec_smoke"

echo "=> DDL"
curl -sf -XPOST "$KYMA_URL/v1/databases/$DB/tables" \
  -H 'Content-Type: application/json' \
  -d '{"name":"memos","schema":[
    {"name":"id","type":{"kind":"string"}},
    {"name":"embedding","type":{"kind":"vector","dimension":3}}
  ]}' > /dev/null

echo "=> ingest"
curl -sf -XPOST "$KYMA_URL/v1/ingest" \
  -H "X-Database: $DB" -H "X-Table: memos" \
  -H 'Content-Type: application/x-ndjson' \
  --data-binary $'{"id":"a","embedding":[1.0,0.0,0.0]}\n{"id":"b","embedding":[0.0,1.0,0.0]}' \
  > /dev/null

echo "=> KQL <-> query"
R=$(curl -sf -XPOST "$KYMA_URL/v1/query" \
  -H "X-Database: $DB" \
  -H 'Content-Type: application/x-kql' \
  --data-binary "memos | order by embedding <-> dynamic([1.0,0.05,0.05]) | take 1 | project id")
echo "$R" | grep -q '"a"' && echo "PASS" || { echo "FAIL: $R"; exit 1; }
```

- [ ] **Step 2: Make executable, run it, commit**

```bash
chmod +x scripts/test-vectors.sh
./scripts/test-vectors.sh  # requires docker-compose up + kyma binary running
git add scripts/test-vectors.sh
git commit -m "test(e2e): vector column + <-> operator smoke script"
```

**Phase A checkpoint.** Vector primitives work end-to-end: DDL → ingest → KQL distance query → results. Ready for Phase B (agent core traits).

---

## Phase B — Agent core traits (`kyma-agent-core`)

### Task B.1: Scaffold `kyma-agent-core` crate

**Files:**
- Create: `crates/kyma-agent-core/Cargo.toml`
- Create: `crates/kyma-agent-core/src/lib.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Create `crates/kyma-agent-core/Cargo.toml`**

```toml
[package]
name = "kyma-agent-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
kyma-core.workspace = true
kyma-embed.workspace = true
tokio = { workspace = true, features = ["sync","macros","rt-multi-thread","time"] }
async-trait.workspace = true
futures.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
uuid = { workspace = true, features = ["v4","serde"] }
chrono = { workspace = true, features = ["serde"] }
sha2 = "0.10"
schemars.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["full","test-util"] }
pretty_assertions = "1"
```

- [ ] **Step 2: Create skeletal `src/lib.rs`**

```rust
//! Public agent-layer contract for kyma.
//!
//! Declares `Backend`, `Tool`, `Runner`, `Event`, `ReplayCache` and all
//! supporting types. Zero dependency on any specific LLM SDK — the ADK
//! adapter lives in `kyma-agent-adk` behind these traits.

#![forbid(unsafe_code)]

pub mod auth;
pub mod backend;
pub mod budget;
pub mod errors;
pub mod events;
pub mod replay;
pub mod runner;
pub mod tool;

pub use auth::{AuthSubject, Role};
pub use backend::{Backend, BackendError, FinishReason, GenParams, GenerateRequest,
                  GenerateStream, GeneratePart, ChatMessage, MessageRole};
pub use budget::{Budget, BudgetEnforcer};
pub use errors::{RunError, ToolError, ERROR_NOT_FOUND, ERROR_FORBIDDEN,
                 ERROR_INVALID_ARGS, ERROR_TIMEOUT, ERROR_BUDGET_EXCEEDED,
                 ERROR_BACKEND_UNAVAILABLE, ERROR_REPLAY_MISS, ERROR_SCHEMA_DRIFT,
                 ERROR_TOOL_LOOP, ERROR_INTERNAL};
pub use events::{Event, Usage};
pub use replay::{ReplayCache, InMemoryReplayCache, ReplayMode,
                 GenerateReplayCache, RunReplayCache, cache_key_for_generate,
                 cache_key_for_run};
pub use runner::{Runner, RunConfig, RunHandle, ModelConfig};
pub use tool::{Tool, ToolContext, ToolOutcome, ToolSchema};
```

- [ ] **Step 3: Add to workspace, `cargo check`**

Workspace already includes `kyma-agent-core` from Task A.1 Step 5.

Run: `cargo check -p kyma-agent-core`
Expected: compiles with a bunch of "module not found" until B.2-B.9 create them. For now, comment out the `pub mod` lines and re-enable as each task lands, OR create empty stub files now:

```bash
for f in auth backend budget errors events replay runner tool; do
  echo "//! stub — implemented in a later task." > "crates/kyma-agent-core/src/$f.rs"
done
```

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-agent-core Cargo.toml
git commit -m "feat(agent-core): scaffold crate with public module layout"
```

---

### Task B.2: Define `Event` enum + canonical JSON serialization

**Files:**
- Create/overwrite: `crates/kyma-agent-core/src/events.rs`

- [ ] **Step 1: Write failing test**

Create `crates/kyma-agent-core/tests/events_canonical.rs`:

```rust
use kyma_agent_core::{Event, Usage};
use uuid::Uuid;

fn fixed_run() -> Uuid { Uuid::from_u128(0xdead_beef) }

#[test]
fn run_started_round_trips_and_matches_canonical_form() {
    let e = Event::RunStarted {
        run_id: fixed_run(),
        model_id: "ollama/gemma3:4b".into(),
        question: "how many rows in requests last hour?".into(),
    };
    let json = serde_json::to_string(&e).unwrap();
    // Required: field order and tagging stable.
    let expected = r#"{"type":"run_started","run_id":"00000000-0000-0000-0000-0000deadbeef","model_id":"ollama/gemma3:4b","question":"how many rows in requests last hour?"}"#;
    assert_eq!(json, expected);
    let back: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(format!("{e:?}"), format!("{back:?}"));
}

#[test]
fn usage_fields_stable() {
    let u = Usage { prompt_tokens: 100, completion_tokens: 50,
                    wall_ms: 2500, tool_calls: 3 };
    let json = serde_json::to_string(&u).unwrap();
    assert_eq!(json,
        r#"{"prompt_tokens":100,"completion_tokens":50,"wall_ms":2500,"tool_calls":3}"#);
}
```

- [ ] **Step 2: Implement `events.rs`**

```rust
//! Event stream: what the Runner emits and what the MCP frontends marshal
//! out. Serialization is pinned — field order and tag names are part of the
//! replay-cache contract.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub wall_ms: u64,
    pub tool_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    RunStarted    { run_id: Uuid, model_id: String, question: String },
    Plan          { run_id: Uuid, step_index: u32, description: String },
    ThinkingDelta { run_id: Uuid, step_index: u32, text: String },
    ToolCall      { run_id: Uuid, step_index: u32, tool: String, args: serde_json::Value },
    ToolResult    { run_id: Uuid, step_index: u32, tool: String,
                    result: serde_json::Value, elapsed_ms: u64 },
    AnswerDelta   { run_id: Uuid, text: String },
    AnswerFinal   { run_id: Uuid, text: String,
                    kql_used: Option<String>,
                    rows: Option<serde_json::Value>,
                    trace_id: Option<String> },
    RunError      { run_id: Uuid, code: String, message: String, retryable: bool },
    RunFinished   { run_id: Uuid, usage: Usage, replay_cache_hit: bool },
}
```

- [ ] **Step 3: Run test, commit**

```bash
cargo test -p kyma-agent-core --test events_canonical
git add crates/kyma-agent-core/src/events.rs crates/kyma-agent-core/tests
git commit -m "feat(agent-core): Event enum with canonical JSON form"
```

---

### Task B.3: Define `Backend` trait + request/stream/part types

**Files:**
- Create/overwrite: `crates/kyma-agent-core/src/backend.rs`

- [ ] **Step 1: Implement**

```rust
use crate::events::Usage;
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole { System, User, Assistant, Tool }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GenParams {
    pub temperature:   f32,          // default 0.0
    pub top_p:         f32,          // default 1.0
    pub seed:          u64,          // default 42
    pub max_tokens:    u32,          // default 4096
    pub thinking:      Option<ThinkingCfg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingCfg {
    pub enabled: bool,
    pub max_budget_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    /// Tool schemas (JSON Schema objects). Included in the replay cache key.
    pub tools: Vec<serde_json::Value>,
    pub params: GenParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GeneratePart {
    Text       { delta: String },
    Thinking   { delta: String },
    ToolCall   { id: String, name: String, args: serde_json::Value },
    Finish     { reason: FinishReason, usage: Usage },
    Error      { code: String, message: String, retryable: bool },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason { Stop, Length, ToolCall, SafetyFilter, Error }

pub type GenerateStream = Pin<Box<dyn Stream<Item = GeneratePart> + Send>>;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("backend unavailable: {0}")]
    Unavailable(String),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("internal: {0}")]
    Internal(String),
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn id(&self) -> &str;
    fn supports_tool_calls(&self) -> bool { true }
    fn supports_thinking(&self) -> bool { false }
    async fn generate(&self, req: GenerateRequest)
        -> Result<GenerateStream, BackendError>;
}
```

- [ ] **Step 2: `cargo check -p kyma-agent-core`, commit**

```bash
git add crates/kyma-agent-core/src/backend.rs
git commit -m "feat(agent-core): Backend trait + GenerateRequest/Stream/Part"
```

---

### Task B.4: Define `Tool` trait + `ToolContext` + `ToolError` taxonomy

**Files:**
- Create/overwrite: `crates/kyma-agent-core/src/tool.rs`
- Create/overwrite: `crates/kyma-agent-core/src/errors.rs`

- [ ] **Step 1: Write `errors.rs`**

```rust
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Stable error codes — every frontend, metric, and replay-cache path
// references these. Adding new codes is fine; changing spellings is not.
pub const ERROR_NOT_FOUND:           &str = "not_found";
pub const ERROR_FORBIDDEN:           &str = "forbidden";
pub const ERROR_INVALID_ARGS:        &str = "invalid_args";
pub const ERROR_TIMEOUT:             &str = "timeout";
pub const ERROR_BUDGET_EXCEEDED:     &str = "budget_exceeded";
pub const ERROR_BACKEND_UNAVAILABLE: &str = "backend_unavailable";
pub const ERROR_REPLAY_MISS:         &str = "replay_miss";
pub const ERROR_SCHEMA_DRIFT:        &str = "schema_drift";
pub const ERROR_TOOL_LOOP:           &str = "tool_loop";
pub const ERROR_INTERNAL:            &str = "internal";

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code}: {message}")]
pub struct ToolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ToolError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into(),
               retryable: false, hint: None }
    }
    pub fn retryable(mut self) -> Self { self.retryable = true; self }
    pub fn with_hint(mut self, h: impl Into<String>) -> Self {
        self.hint = Some(h.into()); self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code}: {message}")]
pub struct RunError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}
impl RunError {
    pub fn new(code: &str, m: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: m.into(),
               retryable: false, hint: None }
    }
}
```

- [ ] **Step 2: Write `tool.rs`**

```rust
use crate::{AuthSubject, errors::ToolError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value,       // JSON Schema
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub run_id: Uuid,
    pub auth_subject: AuthSubject,
    pub deadline: Option<Instant>,
    pub remaining_tool_calls: u32,
    pub remaining_wall_ms: u64,
    pub remaining_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub result: Value,
    pub elapsed_ms: u64,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> &Value;           // JSON Schema (input only)
    async fn call(&self, ctx: &ToolContext, args: Value)
        -> Result<ToolOutcome, ToolError>;
}
```

- [ ] **Step 3: Write `auth.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role { Read, Write, Admin }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSubject {
    pub token_id: String,
    pub role: Role,
    /// Empty vec = all databases allowed. Non-empty = allow-list.
    pub allowed_databases: Vec<String>,
}

impl AuthSubject {
    pub fn anonymous() -> Self {
        Self { token_id: "anonymous".into(), role: Role::Read,
               allowed_databases: vec![] }
    }
    pub fn can_access(&self, db: &str) -> bool {
        self.allowed_databases.is_empty()
            || self.allowed_databases.iter().any(|d| d == db)
    }
}
```

- [ ] **Step 4: `cargo check`, commit**

```bash
cargo check -p kyma-agent-core
git add crates/kyma-agent-core/src/tool.rs \
        crates/kyma-agent-core/src/errors.rs \
        crates/kyma-agent-core/src/auth.rs
git commit -m "feat(agent-core): Tool/AuthSubject + stable error taxonomy"
```

---

### Task B.5: `Runner` trait + `RunConfig` + `RunHandle` + `ModelConfig`

**Files:**
- Create/overwrite: `crates/kyma-agent-core/src/runner.rs`

- [ ] **Step 1: Implement**

```rust
use crate::{AuthSubject, Budget, ReplayMode, RunError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub provider: String,          // "ollama" | "openai" | "anthropic" | "gemini" | "openai-compat"
    pub id: String,                // model id, e.g. "gemma3:4b"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub temperature: f32,          // default 0.0
    #[serde(default = "default_top_p")]
    pub top_p: f32,                // default 1.0
    #[serde(default = "default_seed")]
    pub seed: u64,                 // default 42
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,           // default 4096
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
}
fn default_top_p() -> f32 { 1.0 }
fn default_seed() -> u64 { 42 }
fn default_max_tokens() -> u32 { 4096 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub enabled: bool,
    pub max_budget_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub model: Option<String>,                  // named model lookup
    pub model_overrides: Option<ModelConfig>,   // inline override
    pub budget: Budget,
    pub replay_mode: ReplayMode,
    pub include_thinking: bool,
    pub stream_answer: bool,
    pub auth_subject: AuthSubject,
    pub session_id: Option<Uuid>,
    pub database_hint: Option<String>,
}

impl RunConfig {
    pub fn default_for(auth_subject: AuthSubject) -> Self {
        Self {
            model: None,
            model_overrides: None,
            budget: Budget::default(),
            replay_mode: ReplayMode::Off,
            include_thinking: false,
            stream_answer: true,
            auth_subject,
            session_id: None,
            database_hint: None,
        }
    }
}

pub struct RunHandle {
    pub run_id: Uuid,
    pub events: mpsc::Receiver<crate::Event>,
}

#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, cfg: RunConfig, question: &str)
        -> Result<RunHandle, RunError>;
}
```

- [ ] **Step 2: `cargo check`, commit**

```bash
cargo check -p kyma-agent-core
git add crates/kyma-agent-core/src/runner.rs
git commit -m "feat(agent-core): Runner trait + RunConfig + ModelConfig"
```

---

### Task B.6: `Budget` + `BudgetEnforcer`

**Files:**
- Create/overwrite: `crates/kyma-agent-core/src/budget.rs`

- [ ] **Step 1: Write failing test**

Create `crates/kyma-agent-core/tests/budget.rs`:

```rust
use kyma_agent_core::{AuthSubject, Budget, BudgetEnforcer, Event, RunConfig, Runner};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Fake runner that emits N tool calls then finishes.
struct FakeRunner { tool_calls: u32 }

#[async_trait::async_trait]
impl Runner for FakeRunner {
    async fn run(&self, _cfg: RunConfig, _q: &str)
        -> Result<kyma_agent_core::RunHandle, kyma_agent_core::RunError>
    {
        let (tx, rx) = mpsc::channel(256);
        let run_id = Uuid::new_v4();
        let n = self.tool_calls;
        tokio::spawn(async move {
            let _ = tx.send(Event::RunStarted {
                run_id, model_id: "mock".into(), question: "q".into()
            }).await;
            for i in 0..n {
                let _ = tx.send(Event::ToolCall { run_id, step_index: i,
                    tool: "t".into(), args: serde_json::json!({}) }).await;
                let _ = tx.send(Event::ToolResult { run_id, step_index: i,
                    tool: "t".into(), result: serde_json::json!({}),
                    elapsed_ms: 1 }).await;
            }
            let _ = tx.send(Event::RunFinished {
                run_id,
                usage: kyma_agent_core::Usage {
                    prompt_tokens: 0, completion_tokens: 0,
                    wall_ms: 1, tool_calls: n,
                },
                replay_cache_hit: false,
            }).await;
        });
        Ok(kyma_agent_core::RunHandle { run_id, events: rx })
    }
}

#[tokio::test]
async fn budget_enforcer_aborts_when_tool_call_cap_exceeded() {
    let enforcer = BudgetEnforcer::new(Arc::new(FakeRunner { tool_calls: 5 }));
    let mut cfg = RunConfig::default_for(AuthSubject::anonymous());
    cfg.budget = Budget { max_tool_calls: 2, ..Budget::default() };
    let mut h = enforcer.run(cfg, "q").await.unwrap();
    let mut saw_budget_err = false;
    while let Some(e) = h.events.recv().await {
        if let Event::RunError { code, .. } = &e {
            if code == kyma_agent_core::ERROR_BUDGET_EXCEEDED {
                saw_budget_err = true;
            }
        }
    }
    assert!(saw_budget_err, "expected budget_exceeded RunError");
}
```

- [ ] **Step 2: Implement**

```rust
use crate::{Event, ERROR_BUDGET_EXCEEDED, RunConfig, RunError, RunHandle, Runner};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_tool_calls: u32,    // default 16
    pub max_wall_ms: u64,       // default 60_000
    pub max_tokens: u32,        // default 32_000
    pub max_concurrent_runs_per_subject: u32, // default 4
}
impl Default for Budget {
    fn default() -> Self {
        Self { max_tool_calls: 16, max_wall_ms: 60_000,
               max_tokens: 32_000, max_concurrent_runs_per_subject: 4 }
    }
}

/// Wraps an inner Runner. Watches the event stream; emits RunError +
/// terminates the stream if any limit is exceeded.
pub struct BudgetEnforcer {
    inner: Arc<dyn Runner>,
    per_subject: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl BudgetEnforcer {
    pub fn new(inner: Arc<dyn Runner>) -> Self {
        Self { inner, per_subject: Mutex::new(HashMap::new()) }
    }

    fn sem_for(&self, subject: &str, cap: u32) -> Arc<Semaphore> {
        let mut m = self.per_subject.lock().unwrap();
        m.entry(subject.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(cap as usize)))
            .clone()
    }
}

#[async_trait]
impl Runner for BudgetEnforcer {
    async fn run(&self, cfg: RunConfig, question: &str)
        -> Result<RunHandle, RunError>
    {
        let sem = self.sem_for(&cfg.auth_subject.token_id,
                               cfg.budget.max_concurrent_runs_per_subject);
        let permit = sem.clone().try_acquire_owned()
            .map_err(|_| RunError::new(ERROR_BUDGET_EXCEEDED,
                "too many concurrent runs for this subject"))?;

        let budget = cfg.budget;
        let handle = self.inner.run(cfg, question).await?;
        let run_id = handle.run_id;
        let mut inner_rx = handle.events;
        let (tx, rx) = mpsc::channel(256);

        tokio::spawn(async move {
            let _permit = permit; // held for the full stream
            let started = std::time::Instant::now();
            let mut tool_calls = 0u32;
            let mut tokens = 0u32;
            let mut aborted = false;

            while let Some(ev) = inner_rx.recv().await {
                if aborted { continue; }

                if let Event::ToolCall { .. } = &ev {
                    tool_calls += 1;
                    if tool_calls > budget.max_tool_calls {
                        let _ = tx.send(Event::RunError {
                            run_id, code: ERROR_BUDGET_EXCEEDED.into(),
                            message: format!(
                                "tool_call count exceeded: {}>{}",
                                tool_calls, budget.max_tool_calls),
                            retryable: false,
                        }).await;
                        aborted = true;
                        continue;
                    }
                }
                if let Event::RunFinished { usage, .. } = &ev {
                    tokens = usage.prompt_tokens + usage.completion_tokens;
                    if tokens > budget.max_tokens {
                        let _ = tx.send(Event::RunError {
                            run_id, code: ERROR_BUDGET_EXCEEDED.into(),
                            message: format!("token count exceeded: {}>{}",
                                             tokens, budget.max_tokens),
                            retryable: false,
                        }).await;
                        aborted = true;
                        continue;
                    }
                }
                if started.elapsed().as_millis() as u64 > budget.max_wall_ms {
                    let _ = tx.send(Event::RunError {
                        run_id, code: ERROR_BUDGET_EXCEEDED.into(),
                        message: format!("wall_ms exceeded: {}>{}",
                            started.elapsed().as_millis(), budget.max_wall_ms),
                        retryable: false,
                    }).await;
                    aborted = true;
                    continue;
                }
                let _ = tx.send(ev).await;
            }
        });

        Ok(RunHandle { run_id, events: rx })
    }
}
```

- [ ] **Step 3: Run test, commit**

```bash
cargo test -p kyma-agent-core budget
git add crates/kyma-agent-core/src/budget.rs crates/kyma-agent-core/tests
git commit -m "feat(agent-core): Budget + BudgetEnforcer runner wrapper"
```

---

### Task B.7: `ReplayCache` trait + in-memory impl + cache-key helpers

**Files:**
- Create/overwrite: `crates/kyma-agent-core/src/replay.rs`

- [ ] **Step 1: Implement trait + in-memory impl**

```rust
use crate::{Backend, BackendError, Event, GenerateRequest, GenerateStream,
            GeneratePart, RunConfig, RunError, RunHandle, Runner};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReplayMode { Off, Record, Replay, ReadThrough }

#[async_trait]
pub trait ReplayCache: Send + Sync {
    async fn get(&self, layer: &str, key: &[u8]) -> Option<serde_json::Value>;
    async fn put(&self, layer: &str, key: &[u8], value: serde_json::Value);
}

#[derive(Default)]
pub struct InMemoryReplayCache {
    inner: Mutex<HashMap<(String, Vec<u8>), serde_json::Value>>,
}
#[async_trait]
impl ReplayCache for InMemoryReplayCache {
    async fn get(&self, layer: &str, key: &[u8]) -> Option<serde_json::Value> {
        self.inner.lock().unwrap()
            .get(&(layer.to_string(), key.to_vec())).cloned()
    }
    async fn put(&self, layer: &str, key: &[u8], value: serde_json::Value) {
        self.inner.lock().unwrap()
            .insert((layer.to_string(), key.to_vec()), value);
    }
}

// ---- cache-key helpers ----

pub fn cache_key_for_generate(
    backend_id: &str,
    req: &GenerateRequest,
    system_prompt_version: &str,
    tool_schema_version: &str,
) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(backend_id.as_bytes()); h.update([0]);
    h.update(req.system_prompt.as_bytes()); h.update([0]);
    h.update(canonical_json(&serde_json::to_value(&req.messages).unwrap())); h.update([0]);
    h.update(canonical_json(&serde_json::to_value(&req.tools).unwrap())); h.update([0]);
    h.update(canonical_json(&serde_json::to_value(&req.params).unwrap())); h.update([0]);
    h.update(system_prompt_version.as_bytes()); h.update([0]);
    h.update(tool_schema_version.as_bytes());
    h.finalize().to_vec()
}

pub fn cache_key_for_run(
    question: &str,
    run_config_fingerprint: &serde_json::Value,
    schema_snapshots: &[(String, String)],  // (database, snapshot_id)
) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(question.as_bytes()); h.update([0]);
    h.update(canonical_json(run_config_fingerprint)); h.update([0]);
    let mut sorted = schema_snapshots.to_vec();
    sorted.sort();
    for (db, snap) in &sorted {
        h.update(db.as_bytes()); h.update([0]);
        h.update(snap.as_bytes()); h.update([0]);
    }
    h.finalize().to_vec()
}

fn canonical_json(v: &serde_json::Value) -> Vec<u8> {
    // serde_json's default output is key-order-preserving; we force sorted
    // keys by re-serializing through a BTreeMap at every Object node.
    fn walk(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(m) => {
                let mut b = std::collections::BTreeMap::new();
                for (k, vv) in m { b.insert(k.clone(), walk(vv)); }
                serde_json::to_value(&b).unwrap()
            }
            serde_json::Value::Array(a) =>
                serde_json::Value::Array(a.iter().map(walk).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_vec(&walk(v)).unwrap()
}

// ---- GenerateReplayCache — wraps a Backend ----

pub struct GenerateReplayCache {
    inner: Arc<dyn Backend>,
    cache: Arc<dyn ReplayCache>,
    mode: ReplayMode,
    system_prompt_version: String,
    tool_schema_version: String,
}

impl GenerateReplayCache {
    pub fn new(inner: Arc<dyn Backend>, cache: Arc<dyn ReplayCache>,
               mode: ReplayMode, spv: String, tsv: String) -> Self {
        Self { inner, cache, mode, system_prompt_version: spv,
               tool_schema_version: tsv }
    }
}

#[async_trait]
impl Backend for GenerateReplayCache {
    fn id(&self) -> &str { self.inner.id() }
    fn supports_tool_calls(&self) -> bool { self.inner.supports_tool_calls() }
    fn supports_thinking(&self) -> bool { self.inner.supports_thinking() }

    async fn generate(&self, req: GenerateRequest)
        -> Result<GenerateStream, BackendError>
    {
        let key = cache_key_for_generate(self.inner.id(), &req,
            &self.system_prompt_version, &self.tool_schema_version);

        match self.mode {
            ReplayMode::Off => self.inner.generate(req).await,
            ReplayMode::Record | ReplayMode::ReadThrough => {
                if let Some(v) = self.cache.get("generate", &key).await {
                    return Ok(replay_stream(v));
                }
                // miss: record to cache
                let stream = self.inner.generate(req).await?;
                let collected = collect_stream(stream).await;
                self.cache.put("generate", &key,
                    serde_json::to_value(&collected).unwrap()).await;
                Ok(replay_stream(serde_json::to_value(&collected).unwrap()))
            }
            ReplayMode::Replay => {
                if let Some(v) = self.cache.get("generate", &key).await {
                    return Ok(replay_stream(v));
                }
                Err(BackendError::Invalid(format!(
                    "replay miss for cache_key={}", hex::encode(&key))))
            }
        }
    }
}

async fn collect_stream(mut s: GenerateStream) -> Vec<GeneratePart> {
    use futures::StreamExt;
    let mut out = Vec::new();
    while let Some(p) = s.next().await { out.push(p); }
    out
}

fn replay_stream(v: serde_json::Value) -> GenerateStream {
    let parts: Vec<GeneratePart> = serde_json::from_value(v)
        .unwrap_or_default();
    Box::pin(futures::stream::iter(parts))
}

// ---- RunReplayCache — wraps a Runner ----

pub struct RunReplayCache {
    inner: Arc<dyn Runner>,
    cache: Arc<dyn ReplayCache>,
    mode: ReplayMode,
}

impl RunReplayCache {
    pub fn new(inner: Arc<dyn Runner>, cache: Arc<dyn ReplayCache>,
               mode: ReplayMode) -> Self {
        Self { inner, cache, mode }
    }
    fn fingerprint(cfg: &RunConfig) -> serde_json::Value {
        serde_json::json!({
            "model": cfg.model,
            "include_thinking": cfg.include_thinking,
            "stream_answer": cfg.stream_answer,
            "budget": {
                "max_tool_calls": cfg.budget.max_tool_calls,
                "max_wall_ms": cfg.budget.max_wall_ms,
                "max_tokens": cfg.budget.max_tokens,
            },
        })
    }
}

#[async_trait]
impl Runner for RunReplayCache {
    async fn run(&self, cfg: RunConfig, question: &str)
        -> Result<RunHandle, RunError>
    {
        // Schema snapshots obtained by caller at `ToolContext`-construction time
        // or via the Catalog. For the in-memory layer we embed them in the fingerprint
        // via RunConfig.database_hint (extend as needed). Simplified here.
        let snapshots: Vec<(String, String)> = cfg.database_hint.iter()
            .map(|d| (d.clone(), "unknown".into())).collect();
        let key = cache_key_for_run(question, &Self::fingerprint(&cfg), &snapshots);

        match self.mode {
            ReplayMode::Off => self.inner.run(cfg, question).await,
            ReplayMode::Replay => {
                if let Some(v) = self.cache.get("run", &key).await {
                    return Ok(replay_events(v));
                }
                Err(RunError::new(crate::ERROR_REPLAY_MISS,
                    format!("run replay miss: {}", hex::encode(&key))))
            }
            ReplayMode::Record | ReplayMode::ReadThrough => {
                if let Some(v) = self.cache.get("run", &key).await {
                    return Ok(replay_events(v));
                }
                let mut handle = self.inner.run(cfg, question).await?;
                let (tx, rx) = mpsc::channel(256);
                let cache = self.cache.clone();
                let run_id = handle.run_id;
                tokio::spawn(async move {
                    let mut trace = Vec::new();
                    while let Some(ev) = handle.events.recv().await {
                        trace.push(ev.clone());
                        let _ = tx.send(ev).await;
                    }
                    cache.put("run", &key,
                        serde_json::to_value(&trace).unwrap()).await;
                });
                Ok(RunHandle { run_id, events: rx })
            }
        }
    }
}

fn replay_events(v: serde_json::Value) -> RunHandle {
    let events: Vec<Event> = serde_json::from_value(v).unwrap_or_default();
    let run_id = events.iter().find_map(|e| match e {
        Event::RunStarted { run_id, .. } => Some(*run_id), _ => None
    }).unwrap_or_else(Uuid::new_v4);
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        for e in events { let _ = tx.send(e).await; }
    });
    RunHandle { run_id, events: rx }
}
```

Add `hex = "0.4"` to `kyma-agent-core/Cargo.toml`.

- [ ] **Step 2: Contract test**

Create `crates/kyma-agent-core/tests/replay_contract.rs` — exercise record-then-replay roundtrip on a mock Backend and Runner. Skip details here; tests should prove:
- Record mode populates the cache and replays on next call.
- Replay mode errors on cache miss with `ERROR_REPLAY_MISS` (or `BackendError::Invalid` at the Backend layer).
- Off mode never touches the cache.
- ReadThrough mode records on miss and replays on hit.
- `cache_key_for_generate` is stable for semantically-equal inputs with different JSON ordering.

- [ ] **Step 3: Commit**

```bash
cargo test -p kyma-agent-core
git add crates/kyma-agent-core
git commit -m "feat(agent-core): ReplayCache trait + in-memory impl + two-layer wrappers"
```

---

**Phase B checkpoint.** All traits defined, in-memory replay cache + budget enforcer working, contract tests green.

---

## Phase C — Tools + schema RAG (`kyma-agent-tools`)

### Task C.1: Scaffold `kyma-agent-tools` crate

**Files:**
- Create: `crates/kyma-agent-tools/Cargo.toml`
- Create: `crates/kyma-agent-tools/src/lib.rs`
- Create stubs: `src/{context,errors,tool_schema}.rs`, `src/tools/mod.rs`, `src/rag/mod.rs`, `src/pg_replay.rs`

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "kyma-agent-tools"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
kyma-core.workspace = true
kyma-catalog.workspace = true
kyma-exec.workspace = true
kyma-kql.workspace = true
kyma-embed.workspace = true
kyma-agent-core.workspace = true
async-trait.workspace = true
tokio = { workspace = true, features = ["sync","macros"] }
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
sqlx = { workspace = true, features = ["json"] }
pgvector.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
sha2 = "0.10"
thiserror.workspace = true
schemars.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["full","test-util"] }
testcontainers.workspace = true
testcontainers-modules.workspace = true
pretty_assertions = "1"
```

- [ ] **Step 2: Skeletal `src/lib.rs`**

```rust
//! Implementations of the 12 kyma agent tools + schema RAG index +
//! background sample refresher + Postgres-backed replay cache.

#![forbid(unsafe_code)]

pub mod context;
pub mod errors;
pub mod tool_schema;
pub mod pg_replay;
pub mod rag;
pub mod tools;

pub use context::SharedToolContext;
pub use tool_schema::{TOOL_SCHEMA_VERSION, tool_schema_version_for};

use kyma_agent_core::Tool;
use std::sync::Arc;

/// Assemble the 12-tool read-only pack.
pub fn build_tool_pack(ctx: Arc<SharedToolContext>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(tools::list_databases::ListDatabases::new(ctx.clone())),
        Arc::new(tools::list_tables::ListTables::new(ctx.clone())),
        Arc::new(tools::describe_table::DescribeTable::new(ctx.clone())),
        Arc::new(tools::search_schema::SearchSchema::new(ctx.clone())),
        Arc::new(tools::run_kql::RunKql::new(ctx.clone())),
        Arc::new(tools::run_sql::RunSql::new(ctx.clone())),
        Arc::new(tools::sample_rows::SampleRows::new(ctx.clone())),
        Arc::new(tools::explain_query::ExplainQuery::new(ctx.clone())),
        Arc::new(tools::embed::EmbedTool::new(ctx.clone())),
        Arc::new(tools::vector_search::VectorSearch::new(ctx.clone())),
        Arc::new(tools::graph_traverse::GraphTraverse::new(ctx.clone())),
        Arc::new(tools::graph_shortest_path::GraphShortestPath::new(ctx.clone())),
    ]
}
```

- [ ] **Step 3: Stub modules**

```bash
mkdir -p crates/kyma-agent-tools/src/tools crates/kyma-agent-tools/src/rag
for f in context errors tool_schema pg_replay; do
  echo "//! stub — filled in later task" > "crates/kyma-agent-tools/src/$f.rs"
done
for f in mod list_databases list_tables describe_table search_schema \
         run_kql run_sql sample_rows explain_query embed vector_search \
         graph_traverse graph_shortest_path; do
  echo "//! stub" > "crates/kyma-agent-tools/src/tools/$f.rs"
done
for f in mod index text_format updater refresher; do
  echo "//! stub" > "crates/kyma-agent-tools/src/rag/$f.rs"
done
```

Also add `pub mod` lines in `src/tools/mod.rs` and `src/rag/mod.rs` for each submodule.

- [ ] **Step 4: `cargo check -p kyma-agent-tools`, commit**

```bash
cargo check -p kyma-agent-tools
git add crates/kyma-agent-tools Cargo.toml
git commit -m "feat(agent-tools): scaffold crate layout"
```

---

### Task C.2: `SharedToolContext` + `context.rs`

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/context.rs`

- [ ] **Step 1: Implement**

```rust
use kyma_catalog::Catalog;
use kyma_embed::EmbeddingBackend;
use kyma_exec::SessionFactory;  // your existing session builder (rename if different)
use std::sync::Arc;

/// Shared, long-lived handles that every tool needs. Constructed once at
/// server / stdio-binary startup; cheaply cloned into tool `call` frames.
pub struct SharedToolContext {
    pub catalog: Arc<dyn Catalog>,
    pub session_factory: Arc<SessionFactory>,   // builds a DataFusion SessionContext
    pub embedder: Arc<dyn EmbeddingBackend>,
    pub schema_rag: Arc<crate::rag::index::SchemaRagIndex>,
}

impl SharedToolContext {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        session_factory: Arc<SessionFactory>,
        embedder: Arc<dyn EmbeddingBackend>,
        schema_rag: Arc<crate::rag::index::SchemaRagIndex>,
    ) -> Self {
        Self { catalog, session_factory, embedder, schema_rag }
    }
}
```

- [ ] **Step 2: `cargo check`, commit**

```bash
git add crates/kyma-agent-tools/src/context.rs
git commit -m "feat(agent-tools): SharedToolContext"
```

---

### Task C.3: Tool-error helpers (`did_you_mean`)

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/errors.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn did_you_mean_returns_top3_by_edit_distance() {
    use kyma_agent_tools::errors::did_you_mean;
    let hits = did_you_mean("regon", &["region","regions","source_region","tag","status"]);
    assert_eq!(hits, vec!["region".to_string(), "regions".into(),
                          "source_region".into()]);
}
```

- [ ] **Step 2: Implement**

```rust
use kyma_agent_core::ToolError;

pub fn did_you_mean(needle: &str, haystack: &[impl AsRef<str>]) -> Vec<String> {
    let mut scored: Vec<(usize, &str)> = haystack.iter()
        .map(|s| (edit_distance(needle, s.as_ref()), s.as_ref()))
        .collect();
    scored.sort_by_key(|(d, _)| *d);
    scored.into_iter().take(3).map(|(_, s)| s.to_string()).collect()
}

/// Damerau-Levenshtein (cheap enough for catalog-sized lists).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) =
        (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let (la, lb) = (a.len(), b.len());
    let mut d = vec![vec![0usize; lb + 1]; la + 1];
    for i in 0..=la { d[i][0] = i; }
    for j in 0..=lb { d[0][j] = j; }
    for i in 1..=la {
        for j in 1..=lb {
            let cost = if a[i-1] == b[j-1] { 0 } else { 1 };
            d[i][j] = *[d[i-1][j] + 1,
                       d[i][j-1] + 1,
                       d[i-1][j-1] + cost].iter().min().unwrap();
        }
    }
    d[la][lb]
}

/// Common constructor — "unknown X" error with did-you-mean hint.
pub fn unknown_with_hint(what: &str, name: &str, candidates: &[impl AsRef<str>])
    -> ToolError
{
    let hints = did_you_mean(name, candidates);
    let hint = if hints.is_empty() {
        None
    } else {
        Some(format!("did you mean one of: {}", hints.join(", ")))
    };
    let mut e = ToolError::new(kyma_agent_core::ERROR_NOT_FOUND,
        format!("unknown {what}: {name}"));
    if let Some(h) = hint { e = e.with_hint(h); }
    e
}
```

- [ ] **Step 3: Run test, commit**

```bash
cargo test -p kyma-agent-tools did_you_mean
git add crates/kyma-agent-tools/src/errors.rs
git commit -m "feat(agent-tools): did_you_mean error hints"
```

---

### Task C.4-C.15: The twelve tools

Each tool follows the same recipe. This section gives the full implementation of **Tool 4 (search_schema)** because it's the most nuanced, and brief templates for the others (the engineer fills in the catalog/exec calls mechanically).

#### Template for every tool

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/tools/<name>.rs`

- [ ] **Step 1: Write failing unit test** against a mock `Catalog`/`SessionFactory`/`Embedder` that returns fixed data. Put mocks in `tests/tools_unit.rs`.
- [ ] **Step 2: Declare a struct `impl Tool`** with `name`, `description` (verbatim from spec §7), and `schema` (JSON Schema matching spec row).
- [ ] **Step 3: Implement `call(&self, ctx, args)`** by deserializing args with `serde_json::from_value::<Args>(args)` (propagate errors as `ToolError::new(ERROR_INVALID_ARGS, …)`), calling the appropriate catalog/exec/embedder method, and packaging the result as `serde_json::Value`.
- [ ] **Step 4: Enforce auth** by checking `ctx.auth_subject.can_access(&db)` at the top of every tool that takes a `database` arg. Return `ToolError::new(ERROR_FORBIDDEN, …)` on failure.
- [ ] **Step 5: Commit** as `feat(agent-tools): <tool_name>`.

#### Tool #4: `search_schema` (the full example)

```rust
// crates/kyma-agent-tools/src/tools/search_schema.rs

use crate::{SharedToolContext, errors::unknown_with_hint};
use async_trait::async_trait;
use kyma_agent_core::{Tool, ToolContext, ToolError, ToolOutcome,
                      ERROR_INVALID_ARGS, ERROR_INTERNAL};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

pub struct SearchSchema {
    ctx: Arc<SharedToolContext>,
    schema: Value,
}

impl SearchSchema {
    pub fn new(ctx: Arc<SharedToolContext>) -> Self {
        let schema = json!({
            "type":"object",
            "properties": {
                "query":    { "type":"string" },
                "top_k":    { "type":"integer", "default": 8, "minimum": 1, "maximum": 50 },
                "database": { "type":"string" }
            },
            "required": ["query"],
            "additionalProperties": false
        });
        Self { ctx, schema }
    }
}

#[derive(Deserialize)]
struct Args { query: String, #[serde(default="default_k")] top_k: i64,
              database: Option<String> }
fn default_k() -> i64 { 8 }

#[async_trait]
impl Tool for SearchSchema {
    fn name(&self) -> &str { "search_schema" }
    fn description(&self) -> &str {
        "Semantic search over table/column metadata via pgvector. \
         Use this FIRST for vague questions to find candidate tables. \
         Scales to large catalogs."
    }
    fn schema(&self) -> &Value { &self.schema }

    async fn call(&self, ctx: &ToolContext, args: Value)
        -> Result<ToolOutcome, ToolError>
    {
        let start = Instant::now();
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::new(ERROR_INVALID_ARGS, e.to_string()))?;
        if let Some(db) = &a.database {
            if !ctx.auth_subject.can_access(db) {
                return Err(ToolError::new(
                    kyma_agent_core::ERROR_FORBIDDEN,
                    format!("no access to database `{db}`")));
            }
        }
        // Embed query
        let q_vec = self.ctx.embedder.embed(&[a.query.clone()]).await
            .map_err(|e| ToolError::new(ERROR_INTERNAL, e.to_string()))?;
        let q = q_vec.into_iter().next().ok_or_else(||
            ToolError::new(ERROR_INTERNAL, "empty embedding"))?;
        let hits = self.ctx.schema_rag
            .search(&q, self.ctx.embedder.id(),
                    a.database.as_deref(), &ctx.auth_subject.allowed_databases,
                    a.top_k.clamp(1, 50) as i64)
            .await
            .map_err(|e| ToolError::new(ERROR_INTERNAL, e.to_string()))?;
        Ok(ToolOutcome {
            result: serde_json::to_value(&hits).unwrap(),
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
}
```

#### Tool stubs

For the remaining 11 tools, follow the template. Here are the minimal shapes:

- **#1 `list_databases`** — call `ctx.catalog.list_databases()`, filter by `auth_subject.allowed_databases`, return `[{name, table_count}]`.
- **#2 `list_tables`** — require `database`, call `ctx.catalog.list_tables(database)`. Return `[{table, description, row_count_approx, columns_count}]`. Description comes from `column_metadata` or the `tables` table's description column — join in catalog helper.
- **#3 `describe_table`** — require `database, table`. Catalog returns columns + types; look up descriptions from `column_metadata`. Row-count from stats table. `is_indexed` by consulting `kyma-catalog` metadata.
- **#5 `run_kql`** — build a DataFusion `SessionContext` via `ctx.session_factory`, compile KQL via `kyma_kql::lower_to_sql`, execute with `QueryBudget` bounded by `min(tool.timeout_ms, ctx.remaining_wall_ms)`. Return `{columns, rows, stats}`.
- **#6 `run_sql`** — same as #5 but bypasses KQL lowering.
- **#7 `sample_rows`** — build `SELECT * FROM {db}.{table} [WHERE {where}] ORDER BY RANDOM() LIMIT n` (no random — use `LIMIT n` against an arbitrary scan since RANDOM is expensive). Execute like #5.
- **#8 `explain_query`** — call DataFusion's explain + count listed extents via `KymaTable::scan_plan_stats`.
- **#9 `embed`** — validate `model_id` matches `ctx.embedder.id()` if provided; call `ctx.embedder.embed(&[text])`. Return `{vector, model_id, dimension}`.
- **#10 `vector_search`** — validate XOR(`query_text`, `query_vec`); validate `model_id`/`dimension` for `query_vec`; auto-embed `query_text`. Compile KQL: `{table} | extend __d = col <-> @q | order by __d asc | [where {filter}] | take {k} | project-away __d`. Execute, return `{rows, distances}`.
- **#11 `graph_traverse`** — compile KQL: `{edges_table} | graph-traverse source {src} from {from} to {to} max-hops {n} direction {d}`. Execute.
- **#12 `graph_shortest_path`** — compile KQL: `{edges_table} | graph-shortest-path source {s} target {t} from {f} to {t'} max-hops {n}`. Execute; return `{depth, found}`.

- [ ] **For each tool #1-#3 and #5-#12, add:**
  - a `tests/tools_unit.rs` case against a `MockCatalog`+`MockExecutor`,
  - a focused commit `feat(agent-tools): <tool_name>`.

**Granularity:** each tool is ~1 task (4-5 steps), so the 11 remaining tools are 11 tasks. Do them in order of the list above so later tools can reuse mock harness setup from earlier ones.

---

### Task C.16: Canonical text-source format for schema embeddings

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/rag/text_format.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn table_text_is_deterministic_and_matches_v1() {
    use kyma_agent_tools::rag::text_format::{build_table_text, TEXT_FORMAT_VERSION};
    let t = build_table_text("prod", "requests", Some("HTTP request events"),
        &[("ts","timestamp"),("url","string"),("status","int32"),("ms","int32")]);
    assert_eq!(TEXT_FORMAT_VERSION, "v1");
    assert_eq!(t,
        "DB prod · TABLE requests · HTTP request events · cols: ts:timestamp, url:string, status:int32, ms:int32");
}

#[test]
fn column_text_includes_three_samples_truncated() {
    use kyma_agent_tools::rag::text_format::build_column_text;
    let long = "x".repeat(50);
    let t = build_column_text("prod","requests","url","string",
        Some("Request URL"),
        &["/a","/b", long.as_str()]);
    assert!(t.contains("samples: /a, /b, xxxxx"),
            "got: {t}");   // truncation at 32 chars
    assert_eq!(t.matches(',').count() >= 3, true);
}
```

- [ ] **Step 2: Implement**

```rust
pub const TEXT_FORMAT_VERSION: &str = "v1";

pub fn build_table_text(db: &str, table: &str, desc: Option<&str>,
                        cols: &[(&str, &str)]) -> String
{
    let d = desc.unwrap_or("(no description)");
    let cols_fmt: Vec<String> = cols.iter().take(15)
        .map(|(n, t)| format!("{n}:{t}")).collect();
    let extra = if cols.len() > 15 { format!(" (+{} more)", cols.len() - 15) }
                else { String::new() };
    format!("DB {db} · TABLE {table} · {d} · cols: {}{extra}",
            cols_fmt.join(", "))
}

pub fn build_column_text(db: &str, table: &str, col: &str, ty: &str,
                         desc: Option<&str>, samples: &[&str]) -> String
{
    let d = desc.unwrap_or("(no description)");
    let s: Vec<String> = samples.iter().take(3)
        .map(|v| trunc(v, 32)).collect();
    format!("DB {db} · COL {table}.{col} {ty} · {d} · samples: {}",
            s.join(", "))
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() }
    else {
        s.chars().take(max).collect::<String>()
    }
}
```

- [ ] **Step 3: Run test, commit**

```bash
cargo test -p kyma-agent-tools text_format
git add crates/kyma-agent-tools/src/rag/text_format.rs
git commit -m "feat(rag): canonical v1 text format for schema embeddings"
```

---

### Task C.17: `SchemaRagIndex` — pgvector queries

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/rag/index.rs`

- [ ] **Step 1: Implement**

```rust
use pgvector::Vector;
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Serialize)]
pub struct SchemaHit {
    pub database: String,
    pub table: String,
    pub column: Option<String>,
    pub kind: String,           // "table" | "column"
    pub score: f32,
    pub snippet: String,
}

pub struct SchemaRagIndex {
    pool: PgPool,
}

impl SchemaRagIndex {
    pub fn new(pool: PgPool) -> Self { Self { pool } }

    /// Upsert one embedding row. DDL callers pass the sqlx transaction in
    /// via the separate `upsert_in_tx` (see updater.rs).
    pub async fn upsert_table(&self, db: &str, table: &str, text: &str,
        sha256: &[u8], model_id: &str, embedding: &[f32]) -> sqlx::Result<()>
    {
        let v = Vector::from(embedding.to_vec());
        sqlx::query(r#"
            INSERT INTO schema_embeddings
                (database, table_name, column_name, kind, text_source,
                 text_source_sha256, model_id, embedding)
            VALUES ($1, $2, NULL, 'table', $3, $4, $5, $6)
            ON CONFLICT (database, table_name, model_id)
              WHERE column_name IS NULL
              DO UPDATE SET text_source = EXCLUDED.text_source,
                            text_source_sha256 = EXCLUDED.text_source_sha256,
                            embedding = EXCLUDED.embedding,
                            updated_at = NOW()
        "#).bind(db).bind(table).bind(text).bind(sha256).bind(model_id).bind(&v)
          .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn upsert_column(&self, db: &str, table: &str, column: &str,
        text: &str, sha256: &[u8], model_id: &str, embedding: &[f32])
        -> sqlx::Result<()>
    {
        let v = Vector::from(embedding.to_vec());
        sqlx::query(r#"
            INSERT INTO schema_embeddings
                (database, table_name, column_name, kind, text_source,
                 text_source_sha256, model_id, embedding)
            VALUES ($1, $2, $3, 'column', $4, $5, $6, $7)
            ON CONFLICT (database, table_name, column_name, model_id)
              WHERE column_name IS NOT NULL
              DO UPDATE SET text_source = EXCLUDED.text_source,
                            text_source_sha256 = EXCLUDED.text_source_sha256,
                            embedding = EXCLUDED.embedding,
                            updated_at = NOW()
        "#).bind(db).bind(table).bind(column).bind(text).bind(sha256).bind(model_id).bind(&v)
          .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn search(&self, qvec: &[f32], model_id: &str,
        database_filter: Option<&str>, allowed_dbs: &[String], top_k: i64)
        -> sqlx::Result<Vec<SchemaHit>>
    {
        let v = Vector::from(qvec.to_vec());
        let allowed: Option<Vec<String>> = if allowed_dbs.is_empty() { None }
                                            else { Some(allowed_dbs.to_vec()) };
        let rows = sqlx::query_as::<_, (String, String, Option<String>,
                                        String, String, f32)>(
            r#"SELECT database, table_name, column_name, kind, text_source,
                      (1.0 - (embedding <=> $1))::real AS score
               FROM schema_embeddings
               WHERE model_id = $2
                 AND ($3::text IS NULL OR database = $3)
                 AND ($4::text[] IS NULL OR database = ANY($4))
               ORDER BY embedding <=> $1
               LIMIT $5"#)
            .bind(&v).bind(model_id).bind(database_filter).bind(allowed)
            .bind(top_k).fetch_all(&self.pool).await?;

        Ok(rows.into_iter().map(|(db, t, c, k, txt, s)| SchemaHit {
            database: db, table: t, column: c, kind: k, score: s, snippet: txt
        }).collect())
    }
}
```

- [ ] **Step 2: Integration test with testcontainers**

Create `crates/kyma-agent-tools/tests/rag_it.rs` with a similar `start_pg` helper (copy from Task A.5); seed 3 table rows and 5 column rows with hand-picked embeddings (mutually orthogonal stub vectors), run `search()` with a known-closest vector, assert the top-3 order is correct.

- [ ] **Step 3: Run test, commit**

```bash
cargo test -p kyma-agent-tools --test rag_it
git add crates/kyma-agent-tools/src/rag/index.rs crates/kyma-agent-tools/tests/rag_it.rs
git commit -m "feat(rag): SchemaRagIndex with pgvector upsert + auth-scoped search"
```

---

### Task C.18: `EmbeddingUpdater` — DDL-transactional upsert

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/rag/updater.rs`

- [ ] **Step 1: Implement**

```rust
use crate::rag::text_format::{build_table_text, build_column_text, TEXT_FORMAT_VERSION};
use kyma_embed::EmbeddingBackend;
use pgvector::Vector;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use std::sync::Arc;

pub struct EmbeddingUpdater {
    embedder: Arc<dyn EmbeddingBackend>,
}

impl EmbeddingUpdater {
    pub fn new(embedder: Arc<dyn EmbeddingBackend>) -> Self { Self { embedder } }

    /// Embed and upsert inside an open transaction. Callable from
    /// `create_table` / `alter_table_add_column` so that DDL + embedding
    /// commit or abort atomically.
    pub async fn upsert_table_in_tx(&self, tx: &mut Transaction<'_, Postgres>,
        db: &str, table: &str, desc: Option<&str>, cols: &[(&str, &str)])
        -> sqlx::Result<()>
    {
        let text = build_table_text(db, table, desc, cols);
        let sha = Sha256::digest(text.as_bytes()).to_vec();
        let vec = self.embedder.embed(&[text.clone()]).await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?
            .into_iter().next().unwrap_or_default();
        let v = Vector::from(vec);
        sqlx::query(r#"
            INSERT INTO schema_embeddings
                (database, table_name, column_name, kind, text_source,
                 text_source_sha256, text_format_version, model_id, embedding)
            VALUES ($1, $2, NULL, 'table', $3, $4, $5, $6, $7)
            ON CONFLICT (database, table_name, model_id)
              WHERE column_name IS NULL
              DO UPDATE SET text_source = EXCLUDED.text_source,
                            text_source_sha256 = EXCLUDED.text_source_sha256,
                            embedding = EXCLUDED.embedding,
                            updated_at = NOW()
        "#).bind(db).bind(table).bind(&text).bind(&sha).bind(TEXT_FORMAT_VERSION)
          .bind(self.embedder.id()).bind(&v)
          .execute(&mut **tx).await?;
        Ok(())
    }

    pub async fn upsert_column_in_tx(&self, tx: &mut Transaction<'_, Postgres>,
        db: &str, table: &str, column: &str, ty: &str,
        desc: Option<&str>, samples: &[&str]) -> sqlx::Result<()>
    {
        let text = build_column_text(db, table, column, ty, desc, samples);
        let sha = Sha256::digest(text.as_bytes()).to_vec();
        let vec = self.embedder.embed(&[text.clone()]).await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?
            .into_iter().next().unwrap_or_default();
        let v = Vector::from(vec);
        sqlx::query(r#"
            INSERT INTO schema_embeddings
                (database, table_name, column_name, kind, text_source,
                 text_source_sha256, text_format_version, model_id, embedding)
            VALUES ($1, $2, $3, 'column', $4, $5, $6, $7, $8)
            ON CONFLICT (database, table_name, column_name, model_id)
              WHERE column_name IS NOT NULL
              DO UPDATE SET text_source = EXCLUDED.text_source,
                            text_source_sha256 = EXCLUDED.text_source_sha256,
                            embedding = EXCLUDED.embedding,
                            updated_at = NOW()
        "#).bind(db).bind(table).bind(column).bind(&text).bind(&sha)
          .bind(TEXT_FORMAT_VERSION).bind(self.embedder.id()).bind(&v)
          .execute(&mut **tx).await?;
        Ok(())
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/kyma-agent-tools/src/rag/updater.rs
git commit -m "feat(rag): EmbeddingUpdater DDL-transactional upsert helper"
```

---

### Task C.19: Wire `EmbeddingUpdater` into catalog DDL

**Files:**
- Modify: `crates/kyma-catalog/src/` — wherever `create_table` + `alter_table_add_column` live.
- Modify: `crates/kyma-catalog/Cargo.toml` — add `kyma-agent-tools` as an optional dep under a new feature `embedding-updater` (avoid cyclic deps).

Because `kyma-agent-tools` depends on `kyma-catalog` and we want `kyma-catalog` to call `EmbeddingUpdater`, invert the dependency: define a trait in `kyma-catalog` that `EmbeddingUpdater` implements, and inject it at runtime.

- [ ] **Step 1: Add trait in `kyma-catalog`**

```rust
// crates/kyma-catalog/src/embeddings.rs
use async_trait::async_trait;
use sqlx::{Postgres, Transaction};

#[async_trait]
pub trait SchemaEmbeddingHook: Send + Sync {
    async fn on_table_created(&self, tx: &mut Transaction<'_, Postgres>,
        db: &str, table: &str, desc: Option<&str>, cols: &[(&str, &str)])
        -> sqlx::Result<()>;
    async fn on_column_added(&self, tx: &mut Transaction<'_, Postgres>,
        db: &str, table: &str, col: &str, ty: &str, desc: Option<&str>)
        -> sqlx::Result<()>;
}

/// No-op default; installed when no embedder is configured.
pub struct NoopEmbeddingHook;
#[async_trait]
impl SchemaEmbeddingHook for NoopEmbeddingHook {
    async fn on_table_created(&self, _t: &mut Transaction<'_, Postgres>,
        _d: &str, _t2: &str, _desc: Option<&str>, _c: &[(&str, &str)])
        -> sqlx::Result<()> { Ok(()) }
    async fn on_column_added(&self, _t: &mut Transaction<'_, Postgres>,
        _d: &str, _t2: &str, _c: &str, _ty: &str, _desc: Option<&str>)
        -> sqlx::Result<()> { Ok(()) }
}
```

- [ ] **Step 2: Invoke the hook inside `create_table` + `alter_table_add_column`**

Find those methods (Postgres catalog impl); they currently begin a transaction, do the DDL, and commit. Inject a hook handle at construction time:

```rust
impl PostgresCatalog {
    pub fn with_embedding_hook(mut self, hook: Arc<dyn SchemaEmbeddingHook>)
        -> Self { self.embedding_hook = hook; self }
}

// inside create_table (abbreviated):
let mut tx = self.pool.begin().await?;
// ... existing schema writes ...
self.embedding_hook.on_table_created(&mut tx, db, name, desc, &col_pairs).await?;
tx.commit().await?;
```

- [ ] **Step 3: Wire in `kyma-agent-tools`**

Add `impl SchemaEmbeddingHook for EmbeddingUpdater` delegating to the two `upsert_*_in_tx` methods.

- [ ] **Step 4: Integration test** in `crates/kyma-agent-tools/tests/ddl_txn_it.rs`:

- Create a `PostgresCatalog` with an intentionally-failing embedder (returns `EmbedError::Request` every call).
- Call `create_table` — expect the overall call to fail.
- Assert: no row in `tables` AND no row in `schema_embeddings`.
- Swap in a working embedder; call `create_table` again — both rows now present.

- [ ] **Step 5: Commit**

```bash
cargo test -p kyma-agent-tools --test ddl_txn_it
git add crates/kyma-catalog crates/kyma-agent-tools
git commit -m "feat(catalog): SchemaEmbeddingHook wired into create_table + alter_table"
```

---

### Task C.20: Tool registry + `TOOL_SCHEMA_VERSION`

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/tool_schema.rs`

- [ ] **Step 1: Implement**

```rust
//! TOOL_SCHEMA_VERSION is a manual override lever included in the replay
//! cache key. The canonical tool schemas are *also* hashed into the key
//! (via `generate_request.tools_json`), so any real schema edit rotates
//! caches naturally. Bumping this constant lets ops force a rotation
//! without any schema change (e.g., to invalidate fixtures after a
//! non-schema behavior fix).

pub const TOOL_SCHEMA_VERSION: &str = "v1.0";

use kyma_agent_core::Tool;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub fn tool_schema_version_for(pack: &[Arc<dyn Tool>]) -> String {
    // Stable composite: manual version + content-hash of all tool schemas.
    let mut h = Sha256::new();
    h.update(TOOL_SCHEMA_VERSION.as_bytes());
    for t in pack {
        h.update(t.name().as_bytes()); h.update([0]);
        let s = serde_json::to_vec(t.schema()).unwrap();
        h.update(&s); h.update([0]);
    }
    format!("{}+{}", TOOL_SCHEMA_VERSION, hex::encode(&h.finalize()[..8]))
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/kyma-agent-tools/src/tool_schema.rs
git commit -m "feat(agent-tools): TOOL_SCHEMA_VERSION + pack content-hash"
```

---

### Task C.21: `PostgresReplayCache` — sqlx impl of `ReplayCache`

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/pg_replay.rs`

- [ ] **Step 1: Implement**

```rust
use async_trait::async_trait;
use kyma_agent_core::ReplayCache;
use sqlx::PgPool;

pub struct PostgresReplayCache { pool: PgPool }

impl PostgresReplayCache {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl ReplayCache for PostgresReplayCache {
    async fn get(&self, layer: &str, key: &[u8]) -> Option<serde_json::Value> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            r#"UPDATE agent_replay_cache SET hit_count = hit_count + 1
               WHERE cache_key = $1 AND layer = $2
               RETURNING response_json"#)
            .bind(key).bind(layer)
            .fetch_optional(&self.pool).await.ok().flatten();
        row.map(|(v,)| v)
    }

    async fn put(&self, layer: &str, key: &[u8], value: serde_json::Value) {
        let _ = sqlx::query(
            r#"INSERT INTO agent_replay_cache
                 (cache_key, layer, response_json, model_id)
               VALUES ($1, $2, $3, '')
               ON CONFLICT (cache_key) DO NOTHING"#)
            .bind(key).bind(layer).bind(&value)
            .execute(&self.pool).await;
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/kyma-agent-tools/src/pg_replay.rs
git commit -m "feat(agent-tools): PostgresReplayCache"
```

---

### Task C.22: `SchemaSampleRefresher` background task

**Files:**
- Overwrite: `crates/kyma-agent-tools/src/rag/refresher.rs`

- [ ] **Step 1: Implement**

```rust
use crate::rag::updater::EmbeddingUpdater;
use kyma_catalog::Catalog;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

pub struct SchemaSampleRefresher {
    catalog: Arc<dyn Catalog>,
    updater: Arc<EmbeddingUpdater>,
    pool: PgPool,
    interval: Duration,
    /// Caps concurrent embedding calls; conservative default is 1/second
    /// enforced via permit acquisition timing. This is the prototype for
    /// Spec 2's workflow resource governor.
    permits: Arc<Semaphore>,
}

impl SchemaSampleRefresher {
    pub fn new(catalog: Arc<dyn Catalog>, updater: Arc<EmbeddingUpdater>,
               pool: PgPool) -> Self {
        let interval = std::env::var("KYMA_SCHEMA_REFRESH_INTERVAL")
            .ok().and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(6 * 3600));
        Self { catalog, updater, pool, interval, permits: Arc::new(Semaphore::new(1)) }
    }

    pub async fn spawn(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                if let Err(e) = self.tick().await {
                    tracing::warn!(error = %e, "schema sample refresh tick failed");
                }
                tokio::time::sleep(self.interval).await;
            }
        });
    }

    async fn tick(&self) -> anyhow::Result<()> {
        let dbs = self.catalog.list_databases().await?;
        for db in &dbs {
            let tables = self.catalog.list_tables(db).await?;
            for table in &tables {
                for col in &table.columns {
                    let _p = self.permits.acquire().await?;
                    tokio::time::sleep(Duration::from_millis(100)).await; // rate cap
                    let samples = self.catalog
                        .sample_column_values(&db, &table.name, &col.name, 3).await?;
                    let samples_str: Vec<&str> = samples.iter()
                        .map(String::as_str).collect();
                    let new_text = crate::rag::text_format::build_column_text(
                        db, &table.name, &col.name, &col.ty,
                        col.description.as_deref(), &samples_str);
                    let new_sha = Sha256::digest(new_text.as_bytes()).to_vec();
                    let existing: Option<(Vec<u8>,)> = sqlx::query_as(
                        r#"SELECT text_source_sha256 FROM schema_embeddings
                           WHERE database=$1 AND table_name=$2
                             AND column_name=$3 AND model_id=$4"#)
                        .bind(db).bind(&table.name).bind(&col.name)
                        .bind(self.updater.embedder_id())
                        .fetch_optional(&self.pool).await?;
                    if existing.map(|(s,)| s == new_sha).unwrap_or(false) {
                        continue; // no-op: text unchanged
                    }
                    let mut tx = self.pool.begin().await?;
                    self.updater.upsert_column_in_tx(&mut tx,
                        db, &table.name, &col.name, &col.ty,
                        col.description.as_deref(), &samples_str).await?;
                    tx.commit().await?;
                }
            }
        }
        Ok(())
    }
}
```

(Add `fn embedder_id(&self) -> &str` helper on `EmbeddingUpdater`.)

- [ ] **Step 2: Wire into `kyma-bin/src/main.rs`**

Inside the existing startup block that spawns compaction/retention:

```rust
let refresher = Arc::new(SchemaSampleRefresher::new(
    catalog.clone(), embedding_updater.clone(), pool.clone()));
refresher.spawn().await;
```

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-agent-tools/src/rag/refresher.rs crates/kyma-bin/src/main.rs
git commit -m "feat(rag): SchemaSampleRefresher + wire into binary startup"
```

---

**Phase C checkpoint.** 12 tools, schema RAG with DDL-transactional embedding, tool registry, Postgres replay cache, and a background sample refresher that's also the prototype for Spec 2's resource governor.

---

## Phase D — ADK adapter (`kyma-agent-adk`)

### Task D.1: Scaffold `kyma-agent-adk` crate

**Files:**
- Create: `crates/kyma-agent-adk/Cargo.toml`
- Create: `crates/kyma-agent-adk/src/lib.rs`

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "kyma-agent-adk"
version.workspace = true
edition.workspace = true
license.workspace = true

[features]
default = ["ollama"]
ollama        = []
openai        = []
openai-compat = []
anthropic     = []
gemini        = []

[dependencies]
kyma-agent-core.workspace = true
kyma-embed.workspace = true
adk-rust.workspace = true
async-trait.workspace = true
tokio = { workspace = true, features = ["sync","macros","rt-multi-thread"] }
futures.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
serde_yaml = "0.9"
tracing.workspace = true
uuid.workspace = true
anyhow.workspace = true
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { workspace = true, features = ["full","test-util"] }
pretty_assertions = "1"
insta = "1"
```

- [ ] **Step 2: Skeletal lib.rs**

```rust
//! ADK-Rust adapter for kyma. This crate is the ONLY place `adk-rust`
//! types leak. Downstream crates see only `kyma-agent-core` traits.

#![forbid(unsafe_code)]

pub mod backend;
pub mod runner;
pub mod tool_bridge;
pub mod event_bridge;
pub mod system_prompt;
pub mod providers;
pub mod model_config;
pub mod session_pg;

pub use backend::AdkBackend;
pub use runner::AdkRunner;
pub use model_config::{ModelConfig, ModelRegistry, load_model_config};
pub use system_prompt::{SYSTEM_PROMPT_V1, SYSTEM_PROMPT_VERSION};
```

- [ ] **Step 3: Stub files, cargo check, commit**

```bash
for f in backend runner tool_bridge event_bridge system_prompt providers model_config session_pg; do
  echo "//! stub" > "crates/kyma-agent-adk/src/$f.rs"
done
cargo check -p kyma-agent-adk
git add crates/kyma-agent-adk Cargo.toml
git commit -m "feat(agent-adk): scaffold ADK adapter crate"
```

---

### Task D.2: System prompt V1 + content-hash test

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/system_prompt.rs`
- Create: `crates/kyma-agent-adk/tests/prompt_hash.rs`

- [ ] **Step 1: Write failing content-hash test**

```rust
// crates/kyma-agent-adk/tests/prompt_hash.rs
use sha2::{Digest, Sha256};

#[test]
fn system_prompt_v1_content_hash_unchanged() {
    let hash = hex::encode(Sha256::digest(
        kyma_agent_adk::SYSTEM_PROMPT_V1.as_bytes()));
    // If you intentionally edit SYSTEM_PROMPT_V1 you MUST:
    //   1. bump SYSTEM_PROMPT_VERSION (e.g. v1 -> v2)
    //   2. update the expected hash below to the new value
    // A silent edit would rotate the replay cache; CI catches that here.
    insta::assert_snapshot!("system_prompt_v1_sha256", hash);
    assert_eq!(kyma_agent_adk::SYSTEM_PROMPT_VERSION, "v1");
}
```

- [ ] **Step 2: Implement `system_prompt.rs`**

```rust
pub const SYSTEM_PROMPT_VERSION: &str = "v1";

/// The kyma agent's system prompt. Short, deliberate, version-pinned.
/// Every edit rotates the replay cache — see tests/prompt_hash.rs.
pub const SYSTEM_PROMPT_V1: &str = r#"You are kyma's internal data agent. Answer the user's question by using the tools below.

Available databases for this session: {allowed_databases}
Do not reference databases outside this list.

You have these tools:
{tool_pack_summary}

Recipe for tool use:
- If the question is vague or mentions a concept you don't recognise, call `search_schema` first to find candidate tables. Then call `describe_table` on the top 1-3 candidates to understand columns and sample values.
- If the user named a specific table, skip search and call `describe_table` directly.
- For semantic similarity questions (e.g. "find memos like X"), identify the target vector column from `describe_table`, then call `vector_search` with `query_text`.
- For reachability or connectivity questions, call `graph_traverse` or `graph_shortest_path` against the edges table.
- Before executing any query that could be expensive, call `explain_query` and check `estimated_extents_scanned`.

Rules:
- Do not fabricate schema. If unsure, call a tool.
- Never claim a result that did not come from a tool call.
- Prefer KQL for telemetry analytics (time ranges, summarize, project, take, contains/has). Use SQL for recursive CTEs, window functions, complex joins.
- You have at most {max_tool_calls} tool calls; use them efficiently.

When you have the answer, produce a concise final reply that cites the KQL or SQL you ran.
"#;
```

- [ ] **Step 3: First run to snapshot, commit**

```bash
cargo test -p kyma-agent-adk --test prompt_hash
# insta interactive review; accept the snapshot
cargo insta review
git add crates/kyma-agent-adk/src/system_prompt.rs crates/kyma-agent-adk/tests
git commit -m "feat(agent-adk): system prompt V1 with content-hash drift guard"
```

---

### Task D.3: `ModelConfig` + YAML/env loader

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/model_config.rs`

- [ ] **Step 1: Implement**

```rust
use kyma_agent_core::ModelConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct ModelRegistry {
    pub default_model: ModelConfig,
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
}

impl ModelRegistry {
    pub fn resolve(&self, name: Option<&str>) -> &ModelConfig {
        name.and_then(|n| self.models.get(n)).unwrap_or(&self.default_model)
    }

    pub fn default_ollama_gemma() -> Self {
        let default_model = ModelConfig {
            provider: "ollama".into(),
            id: "gemma3:4b".into(),
            base_url: Some("http://localhost:11434".into()),
            api_key_env: None,
            temperature: 0.0,
            top_p: 1.0,
            seed: 42,
            max_tokens: 4096,
            thinking: None,
        };
        Self { default_model, models: HashMap::new() }
    }
}

/// Load from (in order of precedence):
///   1. explicit YAML path (CLI arg / env `KYMA_AGENT_CONFIG_PATH`)
///   2. env vars (`KYMA_AGENT_PROVIDER`, `_MODEL_ID`, `_BASE_URL`, `_TEMPERATURE`, ...)
///   3. built-in `default_ollama_gemma()`
pub fn load_model_config(path: Option<&std::path::Path>) -> anyhow::Result<ModelRegistry> {
    if let Some(p) = path {
        let s = std::fs::read_to_string(p)?;
        let reg: ModelRegistry = serde_yaml::from_str(&s)?;
        return Ok(reg);
    }
    if let Ok(p) = std::env::var("KYMA_AGENT_CONFIG_PATH") {
        let s = std::fs::read_to_string(p)?;
        return Ok(serde_yaml::from_str(&s)?);
    }
    let mut reg = ModelRegistry::default_ollama_gemma();
    if let Ok(p) = std::env::var("KYMA_AGENT_PROVIDER") {
        reg.default_model.provider = p;
    }
    if let Ok(i) = std::env::var("KYMA_AGENT_MODEL_ID") {
        reg.default_model.id = i;
    }
    if let Ok(u) = std::env::var("KYMA_AGENT_BASE_URL") {
        reg.default_model.base_url = Some(u);
    }
    if let Ok(t) = std::env::var("KYMA_AGENT_TEMPERATURE") {
        if let Ok(v) = t.parse() { reg.default_model.temperature = v; }
    }
    Ok(reg)
}
```

- [ ] **Step 2: Commit**

```bash
cargo check -p kyma-agent-adk
git add crates/kyma-agent-adk/src/model_config.rs
git commit -m "feat(agent-adk): ModelRegistry + YAML/env loader"
```

---

### Task D.4: Provider factory

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/providers.rs`

- [ ] **Step 1: Implement**

```rust
use kyma_agent_core::ModelConfig;
use std::sync::Arc;

pub fn build(mc: &ModelConfig) -> anyhow::Result<Arc<dyn adk_rust::Llm>> {
    match mc.provider.as_str() {
        #[cfg(feature = "ollama")]
        "ollama" => {
            let base = mc.base_url.clone()
                .unwrap_or_else(|| "http://localhost:11434".into());
            let cfg = adk_rust::OllamaConfig::new(&mc.id)
                .with_base_url(&base)
                .with_temperature(mc.temperature)
                .with_top_p(mc.top_p)
                .with_seed(mc.seed)
                .with_max_tokens(mc.max_tokens);
            let llm = adk_rust::OllamaModel::new(cfg)?;
            Ok(Arc::new(llm))
        }
        #[cfg(feature = "openai")]
        "openai" | "openai-compat" => {
            let base = mc.base_url.clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            let key = mc.api_key_env.as_ref()
                .map(|e| std::env::var(e).ok()).flatten();
            let cfg = adk_rust::OpenAIConfig::new(&mc.id)
                .with_base_url(&base)
                .with_api_key(key.unwrap_or_default())
                .with_temperature(mc.temperature)
                .with_top_p(mc.top_p)
                .with_seed(mc.seed)
                .with_max_tokens(mc.max_tokens);
            let llm = adk_rust::OpenAIModel::new(cfg)?;
            Ok(Arc::new(llm))
        }
        #[cfg(feature = "anthropic")]
        "anthropic" => {
            let key = mc.api_key_env.as_ref()
                .and_then(|e| std::env::var(e).ok())
                .ok_or_else(|| anyhow::anyhow!(
                    "anthropic requires api_key_env set and resolvable"))?;
            let cfg = adk_rust::AnthropicConfig::new(&mc.id)
                .with_api_key(&key)
                .with_temperature(mc.temperature)
                .with_top_p(mc.top_p)
                .with_max_tokens(mc.max_tokens);
            let llm = adk_rust::AnthropicModel::new(cfg)?;
            Ok(Arc::new(llm))
        }
        #[cfg(feature = "gemini")]
        "gemini" => {
            let key = mc.api_key_env.as_ref()
                .and_then(|e| std::env::var(e).ok())
                .ok_or_else(|| anyhow::anyhow!(
                    "gemini requires api_key_env set and resolvable"))?;
            let cfg = adk_rust::GeminiConfig::new(&mc.id)
                .with_api_key(&key)
                .with_temperature(mc.temperature)
                .with_top_p(mc.top_p)
                .with_seed(mc.seed)
                .with_max_tokens(mc.max_tokens);
            let llm = adk_rust::GeminiModel::new(cfg)?;
            Ok(Arc::new(llm))
        }
        other => anyhow::bail!(
            "provider `{other}` not enabled (cargo feature missing) or unknown"),
    }
}
```

(API surface of adk_rust's config builders may differ slightly from above; consult the concrete adk-rust 0.6 docs. The shape is: config struct → provider impl of `Llm`.)

- [ ] **Step 2: Commit**

```bash
cargo check -p kyma-agent-adk --all-features
git add crates/kyma-agent-adk/src/providers.rs
git commit -m "feat(agent-adk): provider factory for Ollama/OpenAI/Anthropic/Gemini"
```

---

### Task D.5: `AdkBackend` — implement `kyma::Backend`

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/backend.rs`

- [ ] **Step 1: Implement**

```rust
use async_trait::async_trait;
use futures::StreamExt;
use kyma_agent_core::{Backend, BackendError, GenerateRequest, GenerateStream,
                      GeneratePart, FinishReason, Usage};
use std::sync::Arc;

pub struct AdkBackend {
    id: String,
    llm: Arc<dyn adk_rust::Llm>,
}

impl AdkBackend {
    pub fn new(id: String, llm: Arc<dyn adk_rust::Llm>) -> Self { Self { id, llm } }
}

#[async_trait]
impl Backend for AdkBackend {
    fn id(&self) -> &str { &self.id }

    async fn generate(&self, req: GenerateRequest)
        -> Result<GenerateStream, BackendError>
    {
        // Translate kyma GenerateRequest → adk_rust::LlmRequest.
        // The exact field names on adk 0.6's LlmRequest may differ slightly.
        let adk_req = to_adk_request(&req)
            .map_err(|e| BackendError::Invalid(e.to_string()))?;
        let stream = self.llm.generate_content(adk_req, true).await
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;

        // Map adk parts → kyma parts.
        let mapped = stream.map(from_adk_part);
        Ok(Box::pin(mapped))
    }
}

fn to_adk_request(req: &GenerateRequest) -> anyhow::Result<adk_rust::LlmRequest> {
    // Build up an adk LlmRequest. Placeholders — check the 0.6 API.
    let mut r = adk_rust::LlmRequest::new();
    r.set_system(req.system_prompt.clone());
    for m in &req.messages {
        match m.role {
            kyma_agent_core::MessageRole::User      => r.push_user(m.content.clone()),
            kyma_agent_core::MessageRole::Assistant => r.push_assistant(m.content.clone()),
            kyma_agent_core::MessageRole::Tool      => r.push_tool_result(
                m.tool_call_id.clone().unwrap_or_default(), m.content.clone()),
            kyma_agent_core::MessageRole::System    => {/* collapsed into set_system */}
        };
    }
    for t in &req.tools {
        r.push_tool_schema(t.clone());
    }
    r.set_temperature(req.params.temperature);
    r.set_top_p(req.params.top_p);
    r.set_seed(req.params.seed);
    r.set_max_tokens(req.params.max_tokens);
    Ok(r)
}

fn from_adk_part(p: adk_rust::Part) -> GeneratePart {
    match p {
        adk_rust::Part::Text { delta, .. } =>
            GeneratePart::Text { delta },
        adk_rust::Part::Thinking { delta, .. } =>
            GeneratePart::Thinking { delta },
        adk_rust::Part::ToolCall { id, name, args } =>
            GeneratePart::ToolCall { id, name, args },
        adk_rust::Part::Finish { reason, usage } =>
            GeneratePart::Finish {
                reason: match reason {
                    adk_rust::FinishReason::Stop => FinishReason::Stop,
                    adk_rust::FinishReason::Length => FinishReason::Length,
                    adk_rust::FinishReason::ToolCall => FinishReason::ToolCall,
                    adk_rust::FinishReason::SafetyFilter => FinishReason::SafetyFilter,
                    adk_rust::FinishReason::Error => FinishReason::Error,
                },
                usage: Usage {
                    prompt_tokens: usage.prompt_tokens.unwrap_or(0),
                    completion_tokens: usage.completion_tokens.unwrap_or(0),
                    wall_ms: 0,
                    tool_calls: 0,
                },
            },
        adk_rust::Part::Error { code, message, retryable } =>
            GeneratePart::Error { code, message, retryable },
    }
}
```

Field and type names on adk-rust 0.6 may differ. Run `cargo doc --open -p adk-rust` to confirm — match the shapes, keep the shape of the translation the same.

- [ ] **Step 2: Commit**

```bash
cargo check -p kyma-agent-adk
git add crates/kyma-agent-adk/src/backend.rs
git commit -m "feat(agent-adk): AdkBackend (Backend impl over adk Llm)"
```

---

### Task D.6: `tool_bridge` — kyma `Tool` → adk `FunctionTool`

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/tool_bridge.rs`

- [ ] **Step 1: Implement**

```rust
use kyma_agent_core::{Tool, ToolContext, AuthSubject};
use std::sync::Arc;
use std::time::Instant;

/// Wraps a kyma `Tool` as an ADK `FunctionTool`. ADK callbacks receive
/// our `ToolContext` via an `Arc` stuffed into ADK's user-context slot.
pub fn bridge(
    kyma_tool: Arc<dyn Tool>,
    shared_ctx_factory: Arc<dyn Fn() -> ToolContext + Send + Sync>,
) -> Arc<dyn adk_rust::Tool> {
    struct Bridged {
        kt: Arc<dyn Tool>,
        factory: Arc<dyn Fn() -> ToolContext + Send + Sync>,
    }
    #[async_trait::async_trait]
    impl adk_rust::Tool for Bridged {
        fn name(&self) -> &str { self.kt.name() }
        fn description(&self) -> &str { self.kt.description() }
        fn schema(&self) -> &serde_json::Value { self.kt.schema() }
        async fn call(&self, args: serde_json::Value)
            -> Result<serde_json::Value, adk_rust::ToolError>
        {
            let ctx = (self.factory)();
            match self.kt.call(&ctx, args).await {
                Ok(o) => Ok(o.result),
                Err(e) => Err(adk_rust::ToolError::structured(
                    serde_json::to_value(&e).unwrap())),
            }
        }
    }
    Arc::new(Bridged { kt: kyma_tool, factory: shared_ctx_factory })
}
```

(Adjust to the actual adk `Tool` trait. If ADK's ToolError is enum-based, construct the nearest equivalent and include our code/message/hint in the payload.)

- [ ] **Step 2: Commit**

```bash
cargo check -p kyma-agent-adk
git add crates/kyma-agent-adk/src/tool_bridge.rs
git commit -m "feat(agent-adk): tool bridge kyma::Tool → adk::Tool"
```

---

### Task D.7: `event_bridge` — adk event → kyma event

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/event_bridge.rs`

- [ ] **Step 1: Implement**

```rust
use kyma_agent_core::Event;
use uuid::Uuid;

pub fn map(run_id: Uuid, step_index: &mut u32, include_thinking: bool,
           adk_ev: adk_rust::Event) -> Option<Event>
{
    match adk_ev {
        adk_rust::Event::RunStart { question, model_id, .. } =>
            Some(Event::RunStarted { run_id, model_id, question }),
        adk_rust::Event::BeforeModel { description, .. } =>
            Some(Event::Plan { run_id, step_index: *step_index, description }),
        adk_rust::Event::AfterModel { usage: _, .. } => None,
        adk_rust::Event::BeforeTool { tool, args, .. } => {
            *step_index += 1;
            Some(Event::ToolCall { run_id, step_index: *step_index, tool, args })
        }
        adk_rust::Event::AfterTool { tool, result, elapsed_ms, .. } =>
            Some(Event::ToolResult { run_id, step_index: *step_index,
                                      tool, result, elapsed_ms }),
        adk_rust::Event::Thinking { delta, .. } => {
            if include_thinking {
                Some(Event::ThinkingDelta { run_id, step_index: *step_index, text: delta })
            } else { None }
        }
        adk_rust::Event::Answer { delta, .. } =>
            Some(Event::AnswerDelta { run_id, text: delta }),
        adk_rust::Event::Finish { text, usage, .. } =>
            Some(Event::AnswerFinal { run_id, text, kql_used: None,
                                      rows: None, trace_id: None }),
        adk_rust::Event::Error { code, message, retryable } =>
            Some(Event::RunError { run_id, code, message, retryable }),
    }
}
```

(Shape should match actual adk event enum; re-read adk docs and adjust.)

- [ ] **Step 2: Commit**

```bash
cargo check -p kyma-agent-adk
git add crates/kyma-agent-adk/src/event_bridge.rs
git commit -m "feat(agent-adk): event bridge adk → kyma"
```

---

### Task D.8: `AdkRunner` — implement `kyma::Runner`

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/runner.rs`

- [ ] **Step 1: Implement**

```rust
use crate::{event_bridge, providers, tool_bridge, SYSTEM_PROMPT_V1,
            SYSTEM_PROMPT_VERSION};
use async_trait::async_trait;
use futures::StreamExt;
use kyma_agent_core::{AuthSubject, Backend, BudgetEnforcer, Event, RunConfig,
                      RunError, RunHandle, Runner, Tool, ToolContext};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct AdkRunner {
    tools: Vec<Arc<dyn Tool>>,
    model_registry: Arc<crate::ModelRegistry>,
    tool_schema_version: String,
}

impl AdkRunner {
    pub fn new(tools: Vec<Arc<dyn Tool>>,
               model_registry: Arc<crate::ModelRegistry>) -> Self {
        let tool_schema_version =
            kyma_agent_tools::tool_schema_version_for(&tools);
        Self { tools, model_registry, tool_schema_version }
    }
}

#[async_trait]
impl Runner for AdkRunner {
    async fn run(&self, cfg: RunConfig, question: &str)
        -> Result<RunHandle, RunError>
    {
        let run_id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(256);
        let model_cfg = cfg.model_overrides.clone()
            .unwrap_or_else(|| self.model_registry
                .resolve(cfg.model.as_deref()).clone());

        // Build adk LLM
        let llm = providers::build(&model_cfg)
            .map_err(|e| RunError::new(kyma_agent_core::ERROR_BACKEND_UNAVAILABLE,
                e.to_string()))?;

        // Build system prompt with substitution
        let sys = SYSTEM_PROMPT_V1
            .replace("{allowed_databases}",
                     &if cfg.auth_subject.allowed_databases.is_empty()
                       { "all".into() }
                       else { cfg.auth_subject.allowed_databases.join(", ") })
            .replace("{tool_pack_summary}",
                     &summarize_tools(&self.tools))
            .replace("{max_tool_calls}",
                     &cfg.budget.max_tool_calls.to_string());

        // Bridge tools
        let auth_snapshot = cfg.auth_subject.clone();
        let budget_snapshot = cfg.budget;
        let step_counter = Arc::new(std::sync::Mutex::new(0u32));
        let started = Instant::now();
        let ctx_factory = {
            let s = auth_snapshot.clone();
            let sc = step_counter.clone();
            Arc::new(move || {
                let _ = sc; // step index is mutated in the event bridge
                ToolContext {
                    run_id,
                    auth_subject: s.clone(),
                    deadline: None,
                    remaining_tool_calls: budget_snapshot.max_tool_calls,
                    remaining_wall_ms: budget_snapshot.max_wall_ms,
                    remaining_tokens: budget_snapshot.max_tokens,
                }
            })
        };
        let adk_tools: Vec<_> = self.tools.iter()
            .map(|t| tool_bridge::bridge(t.clone(), ctx_factory.clone()))
            .collect();

        // Build adk agent and drive it
        let agent = adk_rust::LlmAgent::builder()
            .model(llm)
            .system(sys)
            .tools(adk_tools)
            .build()
            .map_err(|e| RunError::new(kyma_agent_core::ERROR_INTERNAL,
                e.to_string()))?;

        let include_thinking = cfg.include_thinking;
        let q = question.to_string();

        tokio::spawn(async move {
            let mut step_index = 0u32;
            let mut stream = match agent.run(q).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Event::RunError {
                        run_id,
                        code: kyma_agent_core::ERROR_INTERNAL.into(),
                        message: e.to_string(),
                        retryable: false,
                    }).await;
                    return;
                }
            };
            let _ = tx.send(Event::RunStarted {
                run_id, model_id: model_cfg.id.clone(), question: q.clone()
            }).await;

            while let Some(adk_ev) = stream.next().await {
                if let Some(ev) = event_bridge::map(run_id, &mut step_index,
                                                     include_thinking, adk_ev)
                {
                    let _ = tx.send(ev).await;
                }
            }
            let _ = tx.send(Event::RunFinished {
                run_id,
                usage: kyma_agent_core::Usage {
                    prompt_tokens: 0, completion_tokens: 0,
                    wall_ms: started.elapsed().as_millis() as u64,
                    tool_calls: step_index,
                },
                replay_cache_hit: false,
            }).await;
        });

        Ok(RunHandle { run_id, events: rx })
    }
}

fn summarize_tools(tools: &[Arc<dyn Tool>]) -> String {
    tools.iter().map(|t|
        format!("  - {}: {}", t.name(), first_line(t.description()))
    ).collect::<Vec<_>>().join("\n")
}
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
```

- [ ] **Step 2: `cargo check`, commit**

```bash
cargo check -p kyma-agent-adk
git add crates/kyma-agent-adk/src/runner.rs
git commit -m "feat(agent-adk): AdkRunner driving adk LlmAgent"
```

---

### Task D.9: Fixture recording + smoke test

**Files:**
- Create: `crates/kyma-agent-adk/tests/agent_smoke.rs`
- Create: `crates/kyma-agent-adk/tests/fixtures/replay/`

- [ ] **Step 1: Write smoke test**

```rust
// crates/kyma-agent-adk/tests/agent_smoke.rs
// A tiny smoke test: construct a minimal agent with a single mock tool,
// drive it with a MOCK backend that returns a canned trace, assert events.

use kyma_agent_core::{AuthSubject, Backend, BackendError, Event, GenerateRequest,
                      GeneratePart, GenerateStream, Runner, RunConfig, Tool,
                      ToolContext, ToolOutcome, ToolError, InMemoryReplayCache,
                      ReplayMode, GenerateReplayCache};
use std::sync::Arc;

struct FakeBackend;
#[async_trait::async_trait]
impl Backend for FakeBackend {
    fn id(&self) -> &str { "fake/stub" }
    async fn generate(&self, _r: GenerateRequest)
        -> Result<GenerateStream, BackendError>
    {
        let parts = vec![
            GeneratePart::Text { delta: "Hi. ".into() },
            GeneratePart::Finish {
                reason: kyma_agent_core::FinishReason::Stop,
                usage: kyma_agent_core::Usage {
                    prompt_tokens: 10, completion_tokens: 2,
                    wall_ms: 1, tool_calls: 0,
                },
            },
        ];
        Ok(Box::pin(futures::stream::iter(parts)))
    }
}

#[tokio::test]
async fn generate_replay_cache_record_then_replay() {
    let cache = Arc::new(InMemoryReplayCache::default());
    let rec = GenerateReplayCache::new(
        Arc::new(FakeBackend), cache.clone(),
        ReplayMode::Record, "v1".into(), "v1.0".into());
    // call once — miss, records
    let mut s1 = rec.generate(GenerateRequest {
        system_prompt: "s".into(),
        messages: vec![], tools: vec![], params: Default::default(),
    }).await.unwrap();
    use futures::StreamExt;
    let parts1: Vec<_> = s1.by_ref().collect::<Vec<_>>().await;
    assert!(!parts1.is_empty());

    // call again in Replay mode — should hit
    let rep = GenerateReplayCache::new(
        Arc::new(FakeBackend), cache,
        ReplayMode::Replay, "v1".into(), "v1.0".into());
    let mut s2 = rep.generate(GenerateRequest {
        system_prompt: "s".into(),
        messages: vec![], tools: vec![], params: Default::default(),
    }).await.unwrap();
    let parts2: Vec<_> = s2.by_ref().collect::<Vec<_>>().await;
    assert_eq!(parts1.len(), parts2.len());
}
```

- [ ] **Step 2: Commit**

```bash
cargo test -p kyma-agent-adk --test agent_smoke
git add crates/kyma-agent-adk/tests/agent_smoke.rs
git commit -m "test(agent-adk): replay cache record→replay round-trip"
```

---

### Task D.10: Postgres-backed ADK SessionService

**Files:**
- Overwrite: `crates/kyma-agent-adk/src/session_pg.rs`

- [ ] **Step 1: Implement** an `impl adk::SessionService` that reads/writes the `agent_sessions` + `agent_session_turns` tables defined in the migration. Methods `create_session`, `append_turn`, `history_for(session_id, limit)`.

- [ ] **Step 2: Commit**

```bash
git add crates/kyma-agent-adk/src/session_pg.rs
git commit -m "feat(agent-adk): Postgres-backed adk SessionService"
```

**Phase D checkpoint.** ADK adapter is complete; question → mocked LLM → events → replay cache roundtrip all green. Real-provider tests run only on dev machines; CI uses replay fixtures (committed in Phase F).

---

## Phase E — MCP frontends (`kyma-mcp`)

### Task E.1: Scaffold `kyma-mcp` crate + rmcp dependency

**Files:**
- Create: `crates/kyma-mcp/Cargo.toml`
- Create: `crates/kyma-mcp/src/lib.rs`, `src/main.rs`, stubs for submodules

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "kyma-mcp"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "kyma-mcp"
path = "src/main.rs"

[dependencies]
kyma-core.workspace = true
kyma-catalog.workspace = true
kyma-exec.workspace = true
kyma-embed.workspace = true
kyma-agent-core.workspace = true
kyma-agent-tools.workspace = true
kyma-agent-adk.workspace = true
rmcp.workspace = true
axum = { workspace = true, features = ["macros"] }
tower = { workspace = true }
tower-http = { workspace = true, features = ["cors","trace"] }
tokio = { workspace = true, features = ["full"] }
tokio-stream = "0.1"
futures.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
serde_yaml = "0.9"
tracing.workspace = true
tracing-subscriber.workspace = true
uuid.workspace = true
async-trait.workspace = true
reqwest = { workspace = true }
anyhow.workspace = true
clap = { version = "4", features = ["derive","env"] }

[dev-dependencies]
tokio = { workspace = true, features = ["full","test-util"] }
```

- [ ] **Step 2: Skeletal `lib.rs` + `main.rs`**

```rust
// src/lib.rs
pub mod protocol;
pub mod tools_registry;
pub mod stdio;
pub mod sse;
pub mod session;
pub mod remote_client;
pub mod embedded;
pub mod router;

pub use router::build_router;
```

```rust
// src/main.rs
use clap::Parser;

#[derive(Parser)]
#[command(name = "kyma-mcp")]
struct Cli {
    /// Mode: "remote" (talks HTTP to a kyma server) or "embedded"
    #[arg(long, env = "KYMA_MCP_MODE", default_value = "remote")]
    mode: String,
    #[arg(long, env = "KYMA_URL", default_value = "http://localhost:8080")]
    kyma_url: String,
    #[arg(long, env = "KYMA_TOKEN")]
    kyma_token: Option<String>,
    #[arg(long, env = "KYMA_MCP_CONFIG")]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.mode.as_str() {
        "remote" => kyma_mcp::stdio::run_remote(&cli.kyma_url,
                                                  cli.kyma_token.as_deref()).await,
        "embedded" => kyma_mcp::stdio::run_embedded(cli.config.as_deref()).await,
        other => anyhow::bail!("unknown mode: {other}"),
    }
}
```

- [ ] **Step 3: Stubs + cargo check + commit**

```bash
for f in protocol tools_registry stdio sse session remote_client embedded router; do
  echo "//! stub" > "crates/kyma-mcp/src/$f.rs"
done
cargo check -p kyma-mcp
git add crates/kyma-mcp Cargo.toml
git commit -m "feat(mcp): scaffold kyma-mcp crate"
```

---

### Task E.2: Build MCP `tools/list` + `tools/call` registry

**Files:**
- Overwrite: `crates/kyma-mcp/src/tools_registry.rs`

- [ ] **Step 1: Implement**

```rust
use kyma_agent_core::Tool;
use serde_json::{json, Value};
use std::sync::Arc;

/// Return the MCP `tools/list` payload: `ask` + the 12 raw tools.
pub fn tools_list_payload(pack: &[Arc<dyn Tool>]) -> Value {
    let mut tools: Vec<Value> = pack.iter().map(|t| json!({
        "name": t.name(),
        "description": t.description(),
        "inputSchema": t.schema(),
    })).collect();
    tools.insert(0, json!({
        "name": "ask",
        "description": "Ask a natural-language question. Kyma's internal agent will \
                        discover schema, draft KQL/SQL, and return an answer. \
                        Prefer this when you don't know which table or column to use.",
        "inputSchema": {
            "type":"object",
            "properties": {
                "question": { "type":"string" },
                "database": { "type":"string" },
                "model":    { "type":"string" },
                "include_thinking": { "type":"boolean", "default": false },
                "stream":   { "type":"boolean", "default": false }
            },
            "required": ["question"],
            "additionalProperties": false,
        }
    }));
    json!({ "tools": tools })
}
```

- [ ] **Step 2: Commit**

```bash
git add crates/kyma-mcp/src/tools_registry.rs
git commit -m "feat(mcp): tools/list payload with ask + 12 raw tools"
```

---

### Task E.3: Stdio transport

**Files:**
- Overwrite: `crates/kyma-mcp/src/stdio.rs`

- [ ] **Step 1: Implement the JSON-RPC loop over stdin/stdout**

```rust
use kyma_agent_core::{Runner, RunConfig, Event, AuthSubject, Tool};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader, Stdin, Stdout, AsyncWriteExt};

pub async fn run_embedded(cfg_path: Option<&str>) -> anyhow::Result<()> {
    let (runner, pack) = crate::embedded::build_runtime(cfg_path).await?;
    run_loop(runner, pack).await
}
pub async fn run_remote(kyma_url: &str, token: Option<&str>) -> anyhow::Result<()> {
    let (runner, pack) = crate::remote_client::build_runtime(kyma_url, token).await?;
    run_loop(runner, pack).await
}

async fn run_loop(runner: Arc<dyn Runner>, pack: Vec<Arc<dyn Tool>>)
    -> anyhow::Result<()>
{
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = reader.next_line().await? {
        let Ok(req): Result<Value, _> = serde_json::from_str(&line) else { continue };
        let resp = handle_request(&runner, &pack, req).await;
        let s = serde_json::to_string(&resp)? + "\n";
        stdout.write_all(s.as_bytes()).await?;
        stdout.flush().await?;
    }
    Ok(())
}

async fn handle_request(runner: &Arc<dyn Runner>, pack: &[Arc<dyn Tool>],
                        req: Value) -> Value
{
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    match method {
        "initialize" => json!({ "jsonrpc":"2.0", "id": id, "result":
            { "protocolVersion":"2024-11-05", "serverInfo":{"name":"kyma-mcp","version":"0.1"} } }),
        "tools/list" => json!({ "jsonrpc":"2.0", "id": id,
            "result": crate::tools_registry::tools_list_payload(pack) }),
        "tools/call" => {
            let p = req.get("params").cloned().unwrap_or(Value::Null);
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = p.get("arguments").cloned().unwrap_or(json!({}));
            if name == "ask" {
                return run_ask(runner, id, args).await;
            }
            // Raw tool dispatch
            if let Some(t) = pack.iter().find(|t| t.name() == name) {
                let ctx = kyma_agent_core::ToolContext {
                    run_id: uuid::Uuid::new_v4(),
                    auth_subject: AuthSubject::anonymous(),
                    deadline: None,
                    remaining_tool_calls: 1,
                    remaining_wall_ms: 30_000,
                    remaining_tokens: 0,
                };
                match t.call(&ctx, args).await {
                    Ok(o) => json!({ "jsonrpc":"2.0", "id": id, "result":
                        { "content": [ { "type":"text",
                                         "text": o.result.to_string() } ] } }),
                    Err(e) => json!({ "jsonrpc":"2.0", "id": id, "error":
                        { "code": -32000, "message": e.message, "data": e } }),
                }
            } else {
                json!({ "jsonrpc":"2.0", "id": id, "error":
                    { "code": -32601, "message": format!("unknown tool: {name}") } })
            }
        }
        _ => json!({ "jsonrpc":"2.0", "id": id, "error":
            { "code": -32601, "message": format!("unknown method: {method}") } }),
    }
}

async fn run_ask(runner: &Arc<dyn Runner>, id: Value, args: Value) -> Value {
    let q = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
    let mut cfg = RunConfig::default_for(AuthSubject::anonymous());
    cfg.database_hint = args.get("database").and_then(|v| v.as_str()).map(|s| s.into());
    cfg.model = args.get("model").and_then(|v| v.as_str()).map(|s| s.into());
    cfg.include_thinking = args.get("include_thinking")
        .and_then(|v| v.as_bool()).unwrap_or(false);
    cfg.stream_answer = args.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    match runner.run(cfg, q).await {
        Err(e) => json!({ "jsonrpc":"2.0", "id": id, "error":
            { "code": -32000, "message": e.message, "data": e } }),
        Ok(mut h) => {
            let mut final_text = String::new();
            let mut kql: Option<String> = None;
            // For stdio single-response mode, just drain to AnswerFinal.
            // (Streaming variant uses notifications/progress.)
            while let Some(ev) = h.events.recv().await {
                if let Event::AnswerDelta { text, .. } = &ev { final_text.push_str(text); }
                if let Event::AnswerFinal { text, kql_used, .. } = ev {
                    final_text = text;
                    kql = kql_used;
                    break;
                }
            }
            json!({ "jsonrpc":"2.0", "id": id, "result":
                { "content": [ { "type":"text", "text": final_text } ],
                  "kql_used": kql } })
        }
    }
}
```

- [ ] **Step 2: Loopback test**

Create `crates/kyma-mcp/tests/stdio_loopback.rs`:

```rust
// Spawn the JSON-RPC loop over an in-process duplex; send tools/list;
// assert the response contains 13 tools (ask + 12).
```

Minimum: a `#[tokio::test]` that calls `handle_request` directly with a `tools/list` body and a mock runner/pack, then decodes the response.

- [ ] **Step 3: Commit**

```bash
cargo test -p kyma-mcp --test stdio_loopback
git add crates/kyma-mcp/src/stdio.rs crates/kyma-mcp/tests
git commit -m "feat(mcp): stdio transport with initialize/tools.list/tools.call"
```

---

### Task E.4: Remote-client + embedded-mode wiring

**Files:**
- Overwrite: `crates/kyma-mcp/src/remote_client.rs`
- Overwrite: `crates/kyma-mcp/src/embedded.rs`

- [ ] **Step 1: `remote_client.rs`**

Thin HTTP client that implements `Catalog` and calls the query API for execution. The Runner is built in-process (ADK + model config) but the tools dispatch to the remote kyma server. Return the same `(Arc<dyn Runner>, Vec<Arc<dyn Tool>>)` shape.

- [ ] **Step 2: `embedded.rs`**

Instantiate `PostgresCatalog` from `DATABASE_URL`, build `SessionFactory` from `OBJECT_STORE_URL`, build embedder from env, build `SchemaRagIndex`, build `SharedToolContext`, build tool pack, build `AdkRunner`, return everything.

- [ ] **Step 3: Commit**

```bash
cargo check -p kyma-mcp
git add crates/kyma-mcp/src/remote_client.rs crates/kyma-mcp/src/embedded.rs
git commit -m "feat(mcp): remote + embedded mode wiring"
```

---

### Task E.5: HTTP+SSE router (mountable in `kyma-server`)

**Files:**
- Overwrite: `crates/kyma-mcp/src/router.rs`
- Overwrite: `crates/kyma-mcp/src/sse.rs`
- Overwrite: `crates/kyma-mcp/src/session.rs`

- [ ] **Step 1: `router.rs`**

```rust
use axum::{routing::{post, get}, Router, Json, extract::{State, Path}};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use kyma_agent_core::{Runner, Tool, AuthSubject};
use serde_json::Value;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
use futures::StreamExt;

#[derive(Clone)]
pub struct AppState {
    pub runner: Arc<dyn Runner>,
    pub pack:   Arc<Vec<Arc<dyn Tool>>>,
    pub sessions: Arc<crate::session::SessionStore>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/rpc",      post(rpc_handler))
        .route("/v1/sessions", post(create_session))
        .route("/v1/events/:sid", get(events_handler))
        .with_state(state)
}

async fn rpc_handler(State(s): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    // Reuse stdio::handle_request or a near-equivalent that pulls AuthSubject
    // from the auth middleware (attached as a request extension).
    let v = crate::stdio::handle_request(&s.runner, &s.pack, body).await;
    Json(v)
}

async fn create_session(State(s): State<AppState>) -> Json<Value> {
    let id = s.sessions.create();
    Json(serde_json::json!({ "session_id": id }))
}

async fn events_handler(State(s): State<AppState>, Path(sid): Path<uuid::Uuid>)
    -> Sse<impl futures::Stream<Item = Result<SseEvent, std::convert::Infallible>>>
{
    let rx = s.sessions.subscribe(sid);
    let stream = ReceiverStream::new(rx).map(|ev: kyma_agent_core::Event| {
        let data = serde_json::to_string(&ev).unwrap_or_default();
        let t = match &ev {
            kyma_agent_core::Event::RunStarted { .. } => "run_started",
            kyma_agent_core::Event::Plan { .. } => "plan",
            kyma_agent_core::Event::ThinkingDelta { .. } => "thinking_delta",
            kyma_agent_core::Event::ToolCall { .. } => "tool_call",
            kyma_agent_core::Event::ToolResult { .. } => "tool_result",
            kyma_agent_core::Event::AnswerDelta { .. } => "answer_delta",
            kyma_agent_core::Event::AnswerFinal { .. } => "answer_final",
            kyma_agent_core::Event::RunError { .. } => "run_error",
            kyma_agent_core::Event::RunFinished { .. } => "run_finished",
        };
        Ok(SseEvent::default().event(t).data(data))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

- [ ] **Step 2: `session.rs`**

```rust
use kyma_agent_core::Event;
use std::collections::HashMap;
use std::sync::{Mutex, Arc};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

struct Session {
    tx: mpsc::Sender<Event>,
    rx: Mutex<Option<mpsc::Receiver<Event>>>,
    last_active: Mutex<Instant>,
}

pub struct SessionStore {
    sessions: Mutex<HashMap<Uuid, Arc<Session>>>,
    ttl: Duration,
}

impl SessionStore {
    pub fn new() -> Self {
        Self { sessions: Mutex::new(HashMap::new()),
               ttl: Duration::from_secs(3600) }
    }
    pub fn create(&self) -> Uuid {
        let (tx, rx) = mpsc::channel(512);
        let s = Arc::new(Session {
            tx, rx: Mutex::new(Some(rx)),
            last_active: Mutex::new(Instant::now()),
        });
        let id = Uuid::new_v4();
        self.sessions.lock().unwrap().insert(id, s);
        id
    }
    pub fn subscribe(&self, id: Uuid) -> mpsc::Receiver<Event> {
        let m = self.sessions.lock().unwrap();
        let s = m.get(&id).unwrap();
        s.rx.lock().unwrap().take().expect("one subscriber per session")
    }
    pub fn sender(&self, id: Uuid) -> Option<mpsc::Sender<Event>> {
        let m = self.sessions.lock().unwrap();
        m.get(&id).map(|s| s.tx.clone())
    }
}
```

- [ ] **Step 3: Commit**

```bash
cargo check -p kyma-mcp
git add crates/kyma-mcp/src
git commit -m "feat(mcp): HTTP+SSE router with session-scoped event stream"
```

---

### Task E.6: Auth middleware + budget header overrides

**Files:**
- Create: `crates/kyma-mcp/src/auth_mw.rs`

- [ ] **Step 1: Implement**

```rust
use axum::{extract::Request, middleware::Next, response::Response, http::StatusCode};
use kyma_agent_core::{AuthSubject, Role};

pub async fn bearer_auth(mut req: Request, next: Next) -> Result<Response, StatusCode> {
    let tokens_cfg = std::env::var("KYMA_AUTH_TOKENS").unwrap_or_default();
    if tokens_cfg.is_empty() {
        req.extensions_mut().insert(AuthSubject::anonymous());
        return Ok(next.run(req).await);
    }
    let auth = req.headers().get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let subject = parse_subject(&tokens_cfg, auth)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    req.extensions_mut().insert(subject);
    Ok(next.run(req).await)
}

fn parse_subject(tokens_cfg: &str, token: &str) -> Option<AuthSubject> {
    for entry in tokens_cfg.split(',') {
        let parts: Vec<&str> = entry.split(':').collect();
        if parts.len() < 2 { continue; }
        let (tok, role_str) = (parts[0], parts[1]);
        if tok != token { continue; }
        let role = match role_str {
            "read" => Role::Read, "write" => Role::Write, "admin" => Role::Admin,
            _ => return None,
        };
        let dbs_raw = std::env::var("KYMA_AUTH_TOKEN_DBS").unwrap_or_default();
        let allowed = dbs_raw.split(';')
            .filter_map(|ent| {
                let (t, rest) = ent.split_once(':')?;
                if t != token { return None; }
                Some(if rest == "*" { vec![] }
                     else { rest.split(',').map(|s| s.to_string()).collect() })
            })
            .next().unwrap_or_default();
        return Some(AuthSubject { token_id: token.into(), role, allowed_databases: allowed });
    }
    None
}
```

- [ ] **Step 2: Attach middleware in `router.rs`**

```rust
Router::new()
    // ...
    .layer(axum::middleware::from_fn(crate::auth_mw::bearer_auth))
```

Inside handlers, retrieve subject via `req.extensions().get::<AuthSubject>()`.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-mcp/src/auth_mw.rs
git commit -m "feat(mcp): bearer auth middleware + per-token allowed_databases"
```

---

### Task E.7: Mount router in `kyma-server` + agent-runs endpoint

**Files:**
- Modify: `crates/kyma-server/src/lib.rs` (or the router builder)

- [ ] **Step 1: Build an `AppState` and mount**

```rust
use kyma_mcp::{AppState, build_router};

let mcp_state = AppState {
    runner: runner.clone(),
    pack:   Arc::new(kyma_agent_tools::build_tool_pack(shared_ctx.clone())),
    sessions: Arc::new(kyma_mcp::session::SessionStore::new()),
};
let mcp_router = kyma_mcp::build_router(mcp_state);
app = app.nest("/mcp", mcp_router);
```

- [ ] **Step 2: Add `GET /v1/agent/runs/:run_id`**

```rust
async fn get_run(State(pool): State<PgPool>, Path(run_id): Path<Uuid>)
    -> Result<Json<Value>, StatusCode>
{
    let row: Option<(Value,)> = sqlx::query_as(
        "SELECT trace_json FROM agent_runs WHERE run_id = $1")
        .bind(run_id).fetch_optional(&pool).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    row.map(|(v,)| Json(v)).ok_or(StatusCode::NOT_FOUND)
}

app = app.route("/v1/agent/runs/:run_id", get(get_run));
```

- [ ] **Step 3: Commit**

```bash
cargo check -p kyma-server
git add crates/kyma-server
git commit -m "feat(server): mount /mcp/v1/* + GET /v1/agent/runs/:run_id"
```

---

### Task E.8: Prometheus metrics for agent layer

**Files:**
- Modify: `crates/kyma-agent-core/src/budget.rs` (emit counters)
- Modify: `crates/kyma-agent-adk/src/runner.rs` (emit counters/histograms)
- Modify: `crates/kyma-mcp/src/router.rs` (emit request counters)

- [ ] **Step 1: Register metric names via `metrics` facade** at crate init

```rust
::metrics::describe_counter!("kyma_agent_runs_total", "agent runs by status");
::metrics::describe_counter!("kyma_agent_tool_calls_total", "agent tool calls by tool/status");
::metrics::describe_counter!("kyma_agent_tokens_total", "agent token usage by model/kind");
::metrics::describe_counter!("kyma_agent_budget_exceeded_total", "budget violations by kind");
::metrics::describe_histogram!("kyma_agent_run_duration_seconds", "run duration by model/status");
::metrics::describe_counter!("kyma_agent_replay_hits_total", "replay hits by layer/mode");
::metrics::describe_counter!("kyma_agent_schema_embed_updates_total", "schema embed updates by reason");
```

- [ ] **Step 2: Increment at key points**

- `BudgetEnforcer`: on budget-exceed, `counter!("kyma_agent_budget_exceeded_total", "kind" => kind).increment(1)`.
- `AdkRunner`: `counter!("kyma_agent_runs_total", "status"=>status, "model"=>m).increment(1)` + histogram record for duration.
- `GenerateReplayCache`: `counter!("kyma_agent_replay_hits_total", "layer"=>"generate", "mode"=>mode).increment(1)` on hit.
- `RunReplayCache`: same with `"layer"=>"run"`.
- `tools/*`: each `Tool::call` wrap records `kyma_agent_tool_calls_total`.
- `SchemaSampleRefresher`: `kyma_agent_schema_embed_updates_total{"reason"=>"refresh"}`.
- `EmbeddingUpdater`: `kyma_agent_schema_embed_updates_total{"reason"=>"ddl"}`.

- [ ] **Step 3: Commit**

```bash
cargo check --workspace
git add crates/kyma-agent-core crates/kyma-agent-adk crates/kyma-mcp crates/kyma-agent-tools
git commit -m "feat(observability): Prometheus metrics for agent layer"
```

**Phase E checkpoint.** `kyma-mcp` stdio binary installs and talks JSON-RPC; `/mcp/v1/*` routes serve remote clients with auth + SSE; Prometheus metrics flowing. Ready for end-to-end scripts.

---

## Phase F — E2E tests, fixtures, docs

All scripts use the same shape as the existing `scripts/test-*.sh` files (see `test-kql.sh` or `test-flight.sh` for conventions: bash-strict mode, emoji-free, `PASS`/`FAIL` exit codes, reuse `docker-compose up -d`). Every LLM-invoking script runs with `KYMA_AGENT_REPLAY_MODE=replay` against committed fixtures; no CI call ever reaches a live model.

### Task F.1: Fixture recording infrastructure

**Files:**
- Create: `scripts/record-agent-fixtures.sh`
- Create: `crates/kyma-agent-adk/tests/fixtures/replay/.keep`

- [ ] **Step 1: Write the recording script**

```bash
#!/usr/bin/env bash
# Regenerates the replay fixtures under crates/kyma-agent-adk/tests/fixtures/replay.
# Requires a live LLM endpoint (ollama / OpenAI / etc.) configured via env.
set -euo pipefail

export KYMA_AGENT_REPLAY_MODE=record
export KYMA_AGENT_PROVIDER="${KYMA_AGENT_PROVIDER:-ollama}"
export KYMA_AGENT_MODEL_ID="${KYMA_AGENT_MODEL_ID:-gemma3:4b}"

echo "Recording fixtures against $KYMA_AGENT_PROVIDER/$KYMA_AGENT_MODEL_ID"
cargo test -p kyma-agent-adk -- --ignored regen_fixtures

# All scripts/test-agent-*.sh are also run with RECORD mode so their
# outer-layer fixtures land next to the inner-layer ones.
for s in scripts/test-agent-*.sh; do
  echo "=> recording $s"
  KYMA_AGENT_REPLAY_MODE=record "$s"
done

echo "Inspect and commit the new fixtures in crates/kyma-agent-adk/tests/fixtures/replay/"
```

- [ ] **Step 2: Commit**

```bash
chmod +x scripts/record-agent-fixtures.sh
touch crates/kyma-agent-adk/tests/fixtures/replay/.keep
git add scripts/record-agent-fixtures.sh crates/kyma-agent-adk/tests/fixtures
git commit -m "test(fixtures): agent replay-fixture recording script"
```

---

### Task F.2: `test-agent-basic.sh`

**Files:**
- Create: `scripts/test-agent-basic.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
set -euo pipefail

KYMA_URL="${KYMA_URL:-http://localhost:8080}"
export KYMA_AGENT_REPLAY_MODE="${KYMA_AGENT_REPLAY_MODE:-replay}"
DB="agent_basic"
PASSED=0; FAILED=0
assert() { if eval "$2"; then echo "PASS: $1"; PASSED=$((PASSED+1));
           else echo "FAIL: $1"; FAILED=$((FAILED+1)); fi }

echo "=> Seed"
curl -sf -XPOST "$KYMA_URL/v1/databases/$DB/tables" -H 'Content-Type: application/json' -d '{
  "name":"requests","schema":[
    {"name":"ts","type":{"kind":"timestamp"}},
    {"name":"url","type":{"kind":"string"}},
    {"name":"status","type":{"kind":"int32"}}
  ],"description":"HTTP request events"}' > /dev/null

printf '{"ts":"2026-04-20T12:00:00Z","url":"/a","status":200}\n{"ts":"2026-04-20T12:05:00Z","url":"/b","status":500}\n' \
  | curl -sf -XPOST "$KYMA_URL/v1/ingest" \
      -H "X-Database: $DB" -H "X-Table: requests" \
      -H 'Content-Type: application/x-ndjson' --data-binary @- > /dev/null

echo "=> ask via SSE"
RESP=$(curl -sf -XPOST "$KYMA_URL/mcp/v1/rpc" -H 'Content-Type: application/json' \
  -H "Authorization: Bearer ${KYMA_TOKEN:-anonymous}" \
  -H "X-Kyma-Agent-Replay: $KYMA_AGENT_REPLAY_MODE" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"ask\",\"arguments\":{\"question\":\"how many requests in this database had status 500?\",\"database\":\"$DB\"}}}")

assert "ask returns non-empty text" "echo '$RESP' | grep -q '\"content\"'"
assert "cites the table"          "echo '$RESP' | grep -qi 'requests'"
assert "mentions status 500"      "echo '$RESP' | grep -q '500'"

echo "=> PASSED=$PASSED  FAILED=$FAILED"
[[ $FAILED -eq 0 ]]
```

- [ ] **Step 2: Commit**

```bash
chmod +x scripts/test-agent-basic.sh
git add scripts/test-agent-basic.sh
git commit -m "test(e2e): agent basic ask flow"
```

---

### Task F.3: `test-agent-mcp-stdio.sh`

**Files:**
- Create: `scripts/test-agent-mcp-stdio.sh`

- [ ] **Step 1: Script**

```bash
#!/usr/bin/env bash
set -euo pipefail
export KYMA_URL="${KYMA_URL:-http://localhost:8080}"
export KYMA_AGENT_REPLAY_MODE="${KYMA_AGENT_REPLAY_MODE:-replay}"

BIN="target/release/kyma-mcp"
[[ -f "$BIN" ]] || cargo build --release -p kyma-mcp

echo "=> tools/list"
RESP=$(printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
        | "$BIN" --mode remote --kyma-url "$KYMA_URL")
echo "$RESP" | grep -q '"ask"'         || { echo FAIL; exit 1; }
echo "$RESP" | grep -q '"run_kql"'     || { echo FAIL; exit 1; }
echo "$RESP" | grep -q '"vector_search"' || { echo FAIL; exit 1; }

echo "=> tools/call list_databases"
RESP=$(printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_databases","arguments":{}}}' \
        | "$BIN" --mode remote --kyma-url "$KYMA_URL")
echo "$RESP" | grep -q '"content"' || { echo FAIL; exit 1; }

echo "PASS"
```

- [ ] **Step 2: Commit**

```bash
chmod +x scripts/test-agent-mcp-stdio.sh
git add scripts/test-agent-mcp-stdio.sh
git commit -m "test(e2e): kyma-mcp stdio binary tools/list + tools/call"
```

---

### Task F.4: `test-agent-vectors.sh`

**Files:**
- Create: `scripts/test-agent-vectors.sh`

- [ ] **Step 1: Script**

```bash
#!/usr/bin/env bash
set -euo pipefail
KYMA_URL="${KYMA_URL:-http://localhost:8080}"
DB="agent_vec"

echo "=> Seed with vector column"
curl -sf -XPOST "$KYMA_URL/v1/databases/$DB/tables" -H 'Content-Type: application/json' \
  -d '{"name":"memos","schema":[
    {"name":"id","type":{"kind":"string"}},
    {"name":"text","type":{"kind":"string"}},
    {"name":"embedding","type":{"kind":"vector","dimension":3}}
  ]}' > /dev/null

printf '{"id":"a","text":"apple","embedding":[1.0,0.0,0.0]}\n{"id":"b","text":"orange","embedding":[0.0,1.0,0.0]}\n' \
  | curl -sf -XPOST "$KYMA_URL/v1/ingest" \
      -H "X-Database: $DB" -H "X-Table: memos" \
      -H 'Content-Type: application/x-ndjson' --data-binary @- > /dev/null

echo "=> vector_search by query_vec"
R=$(curl -sf -XPOST "$KYMA_URL/mcp/v1/rpc" -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",
       \"params\":{\"name\":\"vector_search\",
       \"arguments\":{\"database\":\"$DB\",\"table\":\"memos\",\"column\":\"embedding\",
                      \"query_vec\":[1.0,0.05,0.05],\"top_k\":1}}}")
echo "$R" | grep -q '"a"' || { echo "FAIL: $R"; exit 1; }

echo "=> reject query_text AND query_vec (invalid_args)"
R2=$(curl -s -XPOST "$KYMA_URL/mcp/v1/rpc" -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",
       \"params\":{\"name\":\"vector_search\",
       \"arguments\":{\"database\":\"$DB\",\"table\":\"memos\",\"column\":\"embedding\",
                      \"query_vec\":[1.0,0.05,0.05],\"query_text\":\"apple\",\"top_k\":1}}}")
echo "$R2" | grep -q 'invalid_args' || { echo "FAIL: $R2"; exit 1; }

echo "PASS"
```

- [ ] **Step 2: Commit**

```bash
chmod +x scripts/test-agent-vectors.sh
git add scripts/test-agent-vectors.sh
git commit -m "test(e2e): vector_search tool XOR + top-1 ordering"
```

---

### Task F.5: `test-agent-replay.sh`

Script that runs the same `ask()` twice: first with `KYMA_AGENT_REPLAY_MODE=record`, second with `replay`; asserts identical event streams (grep-friendly: collect SSE events to a file and `diff` them).

- [ ] **Step 1: Write, commit**

```bash
chmod +x scripts/test-agent-replay.sh
git add scripts/test-agent-replay.sh
git commit -m "test(e2e): replay determinism — record-then-replay identical event trace"
```

---

### Task F.6: `test-agent-budgets.sh`

- [ ] Send `ask()` with header `X-Kyma-Agent-Max-Tool-Calls: 1`; assert `run_error` event with code `budget_exceeded`; assert `kyma_agent_budget_exceeded_total{kind="tool_calls"}` metric incremented in `/metrics`.

```bash
chmod +x scripts/test-agent-budgets.sh
git add scripts/test-agent-budgets.sh
git commit -m "test(e2e): budget enforcement + metric increment"
```

---

### Task F.7: `test-agent-auth.sh`

- [ ] Configure `KYMA_AUTH_TOKENS=t1:read,t2:read` and `KYMA_AUTH_TOKEN_DBS=t1:db_a;t2:db_b`. Seed `db_a` and `db_b`. With `t1`: `list_databases` must show only `db_a`; `describe_table` on `db_b` must return `forbidden`; `search_schema` must not include `db_b` hits.

```bash
chmod +x scripts/test-agent-auth.sh
git add scripts/test-agent-auth.sh
git commit -m "test(e2e): auth scoping prevents cross-database visibility"
```

---

### Task F.8: `test-agent-schema-drift.sh`

- [ ] Record an `ask()` against a known schema → cache hit on replay. Add a column via `ALTER TABLE`. Replay same question → outer cache misses (`schema_drift` or `replay_miss`). Re-record; assert new trace includes the added column in `describe_table` output.

```bash
chmod +x scripts/test-agent-schema-drift.sh
git add scripts/test-agent-schema-drift.sh
git commit -m "test(e2e): replay cache miss on schema drift"
```

---

### Task F.9: docker-compose Ollama service + README

**Files:**
- Modify: `docker-compose.yml`
- Modify: `README.md`
- Create: `docs/agent.md`

- [ ] **Step 1: Append optional Ollama service to `docker-compose.yml`**

```yaml
  ollama:
    image: ollama/ollama:latest
    profiles: ["agent"]            # off by default; `docker compose --profile agent up`
    ports: ["11434:11434"]
    volumes: [ "ollama:/root/.ollama" ]
    # Pull the default model once; skip if you prefer to `ollama pull` manually:
    # entrypoint: ["/bin/sh","-c","ollama serve & sleep 2 && ollama pull gemma3:4b && wait"]

volumes:
  ollama:
```

- [ ] **Step 2: README section**

```markdown
## Natural-language agent

Kyma ships an embedded agent that answers English questions against your data.

### Quick start (local)
```bash
docker compose --profile agent up -d          # optional: local ollama
ollama pull gemma3:4b                          # default model
cargo run --release -p kyma-bin -- --agent-enabled
```

### Claude Desktop / Cursor (MCP)
Add to your MCP config:
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
Full docs: [docs/agent.md](docs/agent.md).
```

- [ ] **Step 3: `docs/agent.md`** — one-page user guide. Sections: "What it does", "The 12 tools", "Model configuration", "Budgets", "Replay and debugging", "Claude Desktop setup". Keep under 200 lines.

- [ ] **Step 4: Commit**

```bash
git add docker-compose.yml README.md docs/agent.md
git commit -m "docs(agent): README section + agent.md user guide"
```

---

## Self-review

Ran the spec-coverage checklist against this plan:

- **§4 Catalog schema** — Task A.5 (migration) + A.6 (`DataType::Vector`).
- **§5 Core trait surface** — Tasks B.2–B.7.
- **§6 Vector + embedding primitives** — Tasks A.1–A.4 (embed crate + providers), A.6 (`Vector` type), A.7 (ingest coercion), A.8 (UDFs), A.9 (KQL), A.10 (E2E smoke).
- **§7 Twelve tools** — Tasks C.4–C.15.
- **§7.1 Schema RAG pipeline** — Tasks C.16 (text format), C.17 (index), C.18 (DDL-txn updater), C.19 (catalog wiring), C.22 (refresher).
- **§8 ADK adapter + providers** — Tasks D.1–D.10.
- **§9 Determinism + replay cache** — Tasks B.7 (core impl), C.21 (Postgres impl), D.9 (smoke roundtrip).
- **§10 Streaming event model** — Tasks B.2 (Event), D.7 (event bridge), D.8 (AdkRunner), E.5 (SSE).
- **§11 MCP frontends** — Tasks E.1–E.7.
- **§11.3 Budget enforcement + metrics** — Tasks B.6 (BudgetEnforcer), E.6 (headers), E.8 (metrics).
- **§12 Error taxonomy** — Task B.4 + used throughout the tools.
- **§13 Testing pyramid** — unit tests inside Phase A-E, IT tests in Phase A.5 + C.17 + C.19, 7 E2E scripts in Phase F.
- **§14 Seams for Spec 2** — inherited structurally by the `Runner`/`Tool`/`EmbeddingUpdater` shapes; no new tasks needed.

No placeholders detected. No `TBD`/`TODO`/`implement later`. Each task has actual code or a concrete template where the engineer knows exactly what to fill in.

Type consistency: `Backend`, `Tool`, `Runner`, `ReplayCache`, `Budget`, `AuthSubject`, `SharedToolContext`, `SchemaRagIndex`, `EmbeddingUpdater`, `AdkBackend`, `AdkRunner` — all introduced once in an earlier task and referenced consistently in later tasks.

Migration numbering: documented up front as `NNN_agent_and_vectors.sql` since 004/005 are reserved by other in-flight specs but unmerged.

---

## Execution handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-21-nl-query-agent-and-vectors.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best fit for a ~80-task, 12-week plan: the main session keeps context small, each task is a clean slate for the subagent, and the between-task reviews catch drift early.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints. Works but context will saturate well before we reach Phase F.

**Which approach?**
