# Docs D3 — Doctests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Prerequisite:** [Docs D2 Generated Reference](2026-05-02-docs-d2-generated-reference.md) is complete and committed. Strongly recommended: a stable kyma binary (DB M1 or later shipped) so doctests have a real engine to talk to.

**Goal:** Eliminate documentation drift on **runnable code examples**. Add a `kyma-doctest` Rust binary that parses markdown files, extracts code blocks tagged ` ```kql-runnable ` and ` ```sql-runnable `, executes each against a real kyma instance, computes a deterministic Arrow-result fingerprint, and compares against committed snapshots. Failing snapshots fail the build. The PR-time UX is "your snapshot drifted; run with `--update-snapshots` to refresh, then commit." A docker-compose stack under `docs/site/doctest/` boots Postgres + MinIO + kyma with a deterministic seed dataset for these runs.

**Architecture:** A new private dev crate `kyma-doctest` lives at `crates/kyma-doctest/`. Markdown is parsed with `pulldown-cmark`. Code blocks with the right language tag are extracted along with their `(file, line)` position. Each block executes via the kyma HTTP query API (or via Arrow Flight if needed); the response is collected as Arrow `RecordBatch`es; a fingerprint is computed (schema + sorted-by-all-columns blake3 hash) plus a small "sample rows" preview for human-readable diffs. Snapshots live under `docs/site/.vitepress/generated/doctest-snapshots/<file-hash>/<index>.snap`.

**Tech Stack:** Rust 1.95, `pulldown-cmark`, `reqwest` (already in tree), `arrow` 53, `blake3`, `serde_yaml`. Spec: [`docs/superpowers/specs/2026-05-02-kyma-docs-site-design.md`](../specs/2026-05-02-kyma-docs-site-design.md).

---

## File Structure

**New files:**

- `crates/kyma-doctest/Cargo.toml`
- `crates/kyma-doctest/src/main.rs` — binary entry.
- `crates/kyma-doctest/src/parse.rs` — markdown parser; finds tagged code blocks.
- `crates/kyma-doctest/src/execute.rs` — runs queries against kyma over HTTP.
- `crates/kyma-doctest/src/fingerprint.rs` — Arrow-result fingerprint + sample preview.
- `crates/kyma-doctest/src/snapshot.rs` — read/write/diff snapshots.
- `docs/site/doctest/compose.yml` — docker-compose for the doctest stack.
- `docs/site/doctest/seed/otel_logs.ndjson` — deterministic seed.
- `docs/site/doctest/seed/traces.ndjson`
- `docs/site/doctest/seed/metrics.ndjson`
- `docs/site/doctest/seed/users.ndjson`
- `docs/site/doctest/seed.sh` — boots compose, ingests seed data, leaves stack running.
- `docs/site/.vitepress/generated/doctest-snapshots/.gitkeep`
- `docs/site/scripts/check-doctests.mjs` — CI gate (asserts no snapshot drift; runs the doctest binary if env says so).
- `.github/workflows/docs-doctests.yml`

**Modified files:**

- `Cargo.toml` (workspace) — add `kyma-doctest` to members.
- `docs/site/package.json` — `check:doctests` script + add to `build`.
- Existing markdown pages from D1 (Quickstart, Query) — flip select code blocks from ` ```kql ` / ` ```sql ` to ` ```kql-runnable ` / ` ```sql-runnable `.

---

## Task 1: Workspace + crate skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/kyma-doctest/Cargo.toml`
- Create: `crates/kyma-doctest/src/main.rs`

- [ ] **Step 1: Add to workspace members**

Edit root `Cargo.toml`, in `[workspace] members`:

```toml
"crates/kyma-doctest",
```

In `[workspace.dependencies]`:

```toml
pulldown-cmark = "0.12"
blake3 = "1"
serde_yaml = "0.9"
```

- [ ] **Step 2: Crate manifest**

Write `crates/kyma-doctest/Cargo.toml`:

```toml
[package]
name = "kyma-doctest"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

[lints]
workspace = true

[[bin]]
name = "kyma-doctest"
path = "src/main.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
tokio = { workspace = true }
reqwest = { workspace = true, features = ["json"] }
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
pulldown-cmark = { workspace = true }
arrow = { workspace = true }
arrow-array = { workspace = true }
arrow-schema = { workspace = true }
arrow-json = { workspace = true }
blake3 = { workspace = true }
walkdir = "2"
glob = "0.3"
anyhow = { workspace = true }
```

- [ ] **Step 3: Skeleton main**

Write `crates/kyma-doctest/src/main.rs`:

```rust
#![forbid(unsafe_code)]

mod execute;
mod fingerprint;
mod parse;
mod snapshot;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
struct Args {
    /// Glob patterns of markdown files to scan (e.g., "docs/site/**/*.md").
    #[arg(default_values = ["docs/site/**/*.md"])]
    paths: Vec<String>,

    /// kyma HTTP endpoint.
    #[arg(long, default_value = "http://localhost:8080")]
    url: String,

    /// Update snapshots in place instead of comparing.
    #[arg(long)]
    update_snapshots: bool,

    /// Snapshot directory.
    #[arg(long, default_value = "docs/site/.vitepress/generated/doctest-snapshots")]
    snap_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let files = expand_paths(&args.paths)?;
    let mut total = 0usize;
    let mut failed = 0usize;

    for f in files {
        let blocks = parse::extract_runnable_blocks(&f)?;
        for (idx, b) in blocks.iter().enumerate() {
            total += 1;
            let body = execute::run(&args.url, &b.lang, &b.code).await?;
            let fp = fingerprint::compute(&body)?;
            let snap_path = snapshot::path_for(&args.snap_dir, &f, idx);
            if args.update_snapshots {
                snapshot::write(&snap_path, &fp)?;
                println!("UPDATED  {} block#{idx} {snap_path:?}", f.display());
            } else {
                match snapshot::compare(&snap_path, &fp)? {
                    snapshot::Compare::Match => {
                        println!("PASS     {} block#{idx}", f.display());
                    }
                    snapshot::Compare::Drift(msg) => {
                        failed += 1;
                        eprintln!("FAIL     {} block#{idx}\n{msg}", f.display());
                    }
                    snapshot::Compare::Missing => {
                        failed += 1;
                        eprintln!("MISSING  {} block#{idx} (run with --update-snapshots)", f.display());
                    }
                }
            }
        }
    }

    println!("\n{total} blocks; {failed} failed");
    if failed > 0 { std::process::exit(1); }
    Ok(())
}

fn expand_paths(patterns: &[String]) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    for p in patterns {
        for path in glob::glob(p)? {
            let path = path?;
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: Build**

```bash
cargo build -p kyma-doctest
```

Expected: PASS (with stub modules; we'll fill them in).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/kyma-doctest/
git commit -m "feat(docs): kyma-doctest crate skeleton"
```

---

## Task 2: Markdown parser

**Files:**
- Modify: `crates/kyma-doctest/src/parse.rs`

- [ ] **Step 1: Implement `extract_runnable_blocks`**

```rust
use pulldown_cmark::{Event, Parser, Tag, CodeBlockKind};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RunnableBlock {
    pub lang: String,    // "kql" or "sql"
    pub code: String,
    pub line: u32,       // 1-indexed
}

pub fn extract_runnable_blocks(path: &Path) -> anyhow::Result<Vec<RunnableBlock>> {
    let body = std::fs::read_to_string(path)?;
    let mut blocks = Vec::new();
    let mut cur_lang: Option<String> = None;
    let mut cur_code = String::new();
    let mut line_at_start: u32 = 0;
    let mut current_line: u32 = 1;

    let parser = Parser::new(&body).into_offset_iter();
    for (event, range) in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                let l = lang.to_string();
                if l == "kql-runnable" || l == "sql-runnable" {
                    cur_lang = Some(if l == "kql-runnable" { "kql".into() } else { "sql".into() });
                    cur_code.clear();
                    line_at_start = body[..range.start].matches('\n').count() as u32 + 1;
                }
            }
            Event::Text(t) if cur_lang.is_some() => {
                cur_code.push_str(&t);
            }
            Event::End(Tag::CodeBlock(_)) if cur_lang.is_some() => {
                blocks.push(RunnableBlock {
                    lang: cur_lang.take().unwrap(),
                    code: cur_code.clone(),
                    line: line_at_start,
                });
            }
            _ => {}
        }
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_kql_runnable_blocks() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, r#"# Doc
Some prose.

```kql-runnable
otel_logs | take 1
```

More prose.

```kql
otel_logs | take 2
```
"#).unwrap();
        let blocks = extract_runnable_blocks(tmp.path()).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].lang, "kql");
        assert!(blocks[0].code.contains("take 1"));
    }
}
```

(Add `tempfile = "3"` to dev-deps.)

- [ ] **Step 2: Run + commit**

```bash
cargo test -p kyma-doctest parse
git add crates/kyma-doctest/src/parse.rs crates/kyma-doctest/Cargo.toml
git commit -m "feat(docs): doctest markdown extractor"
```

---

## Task 3: HTTP execution

**Files:**
- Modify: `crates/kyma-doctest/src/execute.rs`

- [ ] **Step 1: Send query to `/v1/query`**

```rust
use arrow::ipc::reader::StreamReader;
use arrow_array::RecordBatch;

pub async fn run(url: &str, lang: &str, code: &str) -> anyhow::Result<Vec<RecordBatch>> {
    let ct = match lang {
        "kql" => "application/x-kql",
        "sql" => "application/sql",
        other => anyhow::bail!("unknown lang: {other}"),
    };
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/query"))
        .header("Content-Type", ct)
        .header("Accept", "application/vnd.apache.arrow.stream")
        .body(code.to_string())
        .send().await?
        .error_for_status()?;
    let bytes = resp.bytes().await?;
    let cursor = std::io::Cursor::new(bytes);
    let reader = StreamReader::try_new(cursor, None)?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch?);
    }
    Ok(batches)
}
```

- [ ] **Step 2: Smoke test**

This task can't have a fully isolated unit test (it needs a kyma instance). Tests come at Task 7 once the docker-compose harness is stood up.

- [ ] **Step 3: Commit**

---

## Task 4: Fingerprint

**Files:**
- Modify: `crates/kyma-doctest/src/fingerprint.rs`

- [ ] **Step 1: Compute schema + sorted-by-all-columns blake3 + sample rows**

```rust
use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Fingerprint {
    pub schema: Vec<(String, String)>,    // (column name, arrow type)
    pub rows: usize,
    pub sorted_by_all_columns_blake3: String,
    pub sample_rows: serde_json::Value,    // first 5 rows as JSON for diff readability
}

pub fn compute(batches: &[RecordBatch]) -> anyhow::Result<Fingerprint> {
    if batches.is_empty() {
        return Ok(Fingerprint {
            schema: vec![],
            rows: 0,
            sorted_by_all_columns_blake3: blake3::hash(b"").to_hex().to_string(),
            sample_rows: serde_json::json!([]),
        });
    }
    let schema_ref: SchemaRef = batches[0].schema();
    let schema = schema_ref.fields().iter()
        .map(|f| (f.name().to_string(), format!("{}", f.data_type())))
        .collect::<Vec<_>>();

    // Concatenate batches → a single buffer of canonical row strings (JSON of each row), sort, hash.
    let mut row_strings: Vec<String> = Vec::new();
    for b in batches {
        let mut writer = arrow_json::ArrayWriter::new(Vec::new());
        writer.write(b)?;
        let bytes = writer.into_inner();
        let v: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let serde_json::Value::Array(arr) = v {
            for row in arr {
                row_strings.push(row.to_string());
            }
        }
    }
    let total_rows = row_strings.len();
    row_strings.sort();
    let mut hasher = blake3::Hasher::new();
    for s in &row_strings { hasher.update(s.as_bytes()); hasher.update(b"\n"); }
    let h = hasher.finalize().to_hex().to_string();

    let sample: Vec<serde_json::Value> = row_strings.iter().take(5)
        .map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::Null))
        .collect();

    Ok(Fingerprint {
        schema,
        rows: total_rows,
        sorted_by_all_columns_blake3: h,
        sample_rows: serde_json::Value::Array(sample),
    })
}
```

- [ ] **Step 2: Inline tests** with a hand-built RecordBatch.

- [ ] **Step 3: Commit**

---

## Task 5: Snapshot read/write/diff

**Files:**
- Modify: `crates/kyma-doctest/src/snapshot.rs`

- [ ] **Step 1: YAML format**

```rust
use crate::fingerprint::Fingerprint;
use std::path::{Path, PathBuf};

pub fn path_for(snap_dir: &Path, md_path: &Path, idx: usize) -> PathBuf {
    let rel = md_path.strip_prefix(std::env::current_dir().unwrap()).unwrap_or(md_path);
    let h = blake3::hash(rel.to_string_lossy().as_bytes()).to_hex().to_string();
    snap_dir.join(&h[..16]).join(format!("{idx:03}.snap"))
}

pub fn write(p: &Path, fp: &Fingerprint) -> anyhow::Result<()> {
    if let Some(dir) = p.parent() { std::fs::create_dir_all(dir)?; }
    let s = serde_yaml::to_string(fp)?;
    std::fs::write(p, s)?;
    Ok(())
}

pub fn read(p: &Path) -> anyhow::Result<Option<Fingerprint>> {
    if !p.exists() { return Ok(None); }
    let s = std::fs::read_to_string(p)?;
    let fp: Fingerprint = serde_yaml::from_str(&s)?;
    Ok(Some(fp))
}

pub enum Compare { Match, Drift(String), Missing }

pub fn compare(p: &Path, actual: &Fingerprint) -> anyhow::Result<Compare> {
    match read(p)? {
        None => Ok(Compare::Missing),
        Some(prev) if &prev == actual => Ok(Compare::Match),
        Some(prev) => {
            let msg = format!(
                "expected schema={:?} rows={} hash={}\n\
                 actual   schema={:?} rows={} hash={}\n\
                 expected sample: {}\n\
                 actual   sample: {}\n",
                prev.schema, prev.rows, prev.sorted_by_all_columns_blake3,
                actual.schema, actual.rows, actual.sorted_by_all_columns_blake3,
                prev.sample_rows, actual.sample_rows
            );
            Ok(Compare::Drift(msg))
        }
    }
}
```

- [ ] **Step 2: Inline tests.**

- [ ] **Step 3: Commit**

---

## Task 6: Docker-compose seed stack

**Files:**
- Create: `docs/site/doctest/compose.yml`
- Create: `docs/site/doctest/seed/{otel_logs,traces,metrics,users}.ndjson`
- Create: `docs/site/doctest/seed.sh`

- [ ] **Step 1: compose.yml**

A trimmed copy of root `docker-compose.yml` (no Kafka, no agent embeddings):

```yaml
services:
  postgres:
    image: pgvector/pgvector:pg16
    environment:
      POSTGRES_USER: kyma
      POSTGRES_PASSWORD: kyma_dev
      POSTGRES_DB: kyma
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U kyma"]
      interval: 1s
      retries: 30

  minio:
    image: minio/minio:latest
    command: server /data
    environment:
      MINIO_ROOT_USER: kyma
      MINIO_ROOT_PASSWORD: kyma_dev_minio
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9000/minio/health/live"]
      interval: 2s
      retries: 30

  kyma:
    build:
      context: ../../..
      dockerfile: Dockerfile
    depends_on:
      postgres: { condition: service_healthy }
      minio:    { condition: service_healthy }
    ports: ["8080:8080"]
    environment:
      KYMA_CATALOG_URL: postgres://kyma:kyma_dev@postgres:5432/kyma
      KYMA_HTTP_ADDR: 0.0.0.0:8080
      KYMA_GRPC_ADDR: "off"
      KYMA_OTLP_ADDR: "off"
      KYMA_S3_ENDPOINT: http://minio:9000
      KYMA_S3_BUCKET: kyma
      KYMA_S3_REGION: us-east-1
      KYMA_S3_ACCESS_KEY_ID: kyma
      KYMA_S3_SECRET_ACCESS_KEY: kyma_dev_minio
      KYMA_S3_PATH_STYLE: "true"
      KYMA_S3_ALLOW_HTTP: "true"
```

- [ ] **Step 2: Deterministic seed data**

Generate 10,000 OTLP log rows with a fixed timestamp range (e.g., `2026-01-01T00:00:00Z` to `2026-01-01T01:00:00Z`), 5 service names, INFO/WARN/ERROR distribution. 1,000 trace spans. 5,000 metric points. 100 user rows. Each NDJSON file is committed; bumping it is deliberate.

A small Rust generator under `docs/site/doctest/gen/` (separate Cargo project; not in workspace) can produce them deterministically — alternative: write them once by hand into the .ndjson files at this step. For maintainability, prefer a generator.

- [ ] **Step 3: seed.sh**

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"
docker compose up -d
echo "waiting for kyma to come up…"
until curl -sf http://localhost:8080/health > /dev/null; do sleep 1; done

for tbl in otel_logs traces metrics users; do
  echo "ingesting $tbl…"
  curl -sf -X POST http://localhost:8080/v1/ingest \
    -H 'Content-Type: application/x-ndjson' \
    -H "X-Database: default" \
    -H "X-Table: $tbl" \
    --data-binary "@seed/$tbl.ndjson" \
    | jq .
done
echo "seed complete."
```

`chmod +x docs/site/doctest/seed.sh`.

- [ ] **Step 4: Smoke**

```bash
docs/site/doctest/seed.sh
```

Expected: kyma running, 4 tables ingested.

- [ ] **Step 5: Commit**

---

## Task 7: End-to-end doctest run

**Files:**
- Modify: existing pages (flip blocks to runnable tags).

- [ ] **Step 1: In `docs/site/quickstart/five-minute-start.md`** flip the example query to ` ```kql-runnable `:

```markdown
```kql-runnable
otel_logs | summarize n=count() by service_name | order by n desc | take 5
```
```

(Use a query that hits the seed data deterministically.)

- [ ] **Step 2: Generate snapshot**

```bash
cargo run -p kyma-doctest -- --update-snapshots docs/site/quickstart/five-minute-start.md
```

Expected: `docs/site/.vitepress/generated/doctest-snapshots/<hash>/000.snap` created.

- [ ] **Step 3: Re-run without --update-snapshots**

```bash
cargo run -p kyma-doctest -- docs/site/quickstart/five-minute-start.md
```

Expected: `PASS`.

- [ ] **Step 4: Modify the seed slightly + rerun → assert FAIL**

Flip a single value in `users.ndjson`, rerun. Expected: `FAIL` with a sample-rows diff. Revert.

- [ ] **Step 5: Promote a few more blocks across `query/kql.md` and `query/sql.md` and `quickstart/`** to runnable. Generate snapshots.

- [ ] **Step 6: Commit**

```bash
git add docs/site/.vitepress/generated/doctest-snapshots/ docs/site/quickstart/ docs/site/query/
git commit -m "feat(docs): runnable code blocks with snapshot doctests"
```

---

## Task 8: CI integration

**Files:**
- Create: `.github/workflows/docs-doctests.yml`
- Modify: `docs/site/scripts/check-doctests.mjs` (CI gate; complements the build step)
- Modify: `docs/site/package.json` (`check:doctests`)

- [ ] **Step 1: GitHub workflow**

```yaml
name: docs-doctests
on:
  pull_request:
    paths:
      - 'crates/**'
      - 'docs/site/**'
  push:
    branches: [main]

jobs:
  doctests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build kyma + doctest binary
        run: cargo build --release -p kyma-bin -p kyma-cli -p kyma-doctest
      - name: Boot doctest stack
        run: |
          cd docs/site/doctest
          docker compose up -d
          ./seed.sh
      - name: Run doctests
        run: cargo run -p kyma-doctest
      - name: Tear down
        if: always()
        run: docker compose -f docs/site/doctest/compose.yml down -v
```

- [ ] **Step 2: `check:doctests` script** runs `git diff --exit-code` over the snapshots dir; build fails on uncommitted local drift.

```js
// docs/site/scripts/check-doctests.mjs
import { execSync } from 'node:child_process'
try {
  execSync('git diff --exit-code -- .vitepress/generated/doctest-snapshots', { stdio: 'inherit' })
} catch {
  console.error('Doctest snapshots are out of date.\nRun: cargo run -p kyma-doctest -- --update-snapshots\nThen commit the changes.')
  process.exit(1)
}
```

- [ ] **Step 3: Wire into build**

```json
"build": "npm run check:diagrams && npm run check:generated && npm run check:doctests && vitepress build",
```

- [ ] **Step 4: Commit**

---

## Task 9: D3 acceptance smoke

- [ ] `cargo run -p kyma-doctest` against the local doctest stack passes for every runnable block.
- [ ] Modifying the seed (intentional drift) makes the doctest run fail with a diff.
- [ ] `--update-snapshots` refreshes; subsequent runs pass.
- [ ] CI workflow `docs-doctests.yml` runs on PRs and gates merges.
- [ ] `pnpm build` includes the doctest snapshot drift check.
- [ ] Tag `docs-d3-doctests`.

---

## D3 Open Decisions

- **Time-windowed queries** — the seed dataset uses a fixed `_timestamp` range; documented examples use bounded ranges (`where _timestamp between datetime(2026-01-01T00:00:00Z) .. datetime(2026-01-01T01:00:00Z)`) NOT relative ranges (`ago(1h)`). Re-confirm at D3 review.
- **Cross-source examples** — federation + sync examples in connector docs (M4 / Docs M4 era) need an external Postgres seed; that's a follow-on extension to the doctest harness in M4, not D3.
- **Snapshot file size growth** — if the snapshot directory grows past ~1000 files, consider a single combined snapshot file per markdown source. v1: per-block. Revisit if file count is unwieldy.
