# Live Telemetry + Traces Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Pensieve's memory live stream honest and complete (capture-pipeline hardening, memory-op events, backfill-vs-live rendering), and add a Traces page backed by a new OTLP traces receiver that Pensieve's own server also exports into.

**Architecture:** One telemetry system: `pensieve-ingest-otlp` gains an `ExportTraceServiceRequest` receiver writing to `otel.otel_traces`; `pensieve-server` self-instruments with `tracing-opentelemetry` spans (target `pensieve_telemetry` only — that target allowlist is the recursion guard) exported in-process through the same row mapping. The web Memory pulse tails `otel_traces` alongside the firehose; the new `/traces` page lists and waterfalls the same table. CLI/installer changes eliminate the token drift that silently killed capture.

**Tech Stack:** Rust (axum, tonic, arrow, tracing-opentelemetry 0.28 / opentelemetry_sdk 0.27 — already in workspace `Cargo.toml:72-76`), React + TanStack Router + framer-motion, vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-06-11-live-telemetry-and-traces-design.md`

**Repo:** `/Users/shakedaskayo/shaked/projects/pensieve` (all paths below relative to repo root)

**Verification baseline before starting:** `cargo test -p pensieve-ingest-otlp` and `cd web && npx vitest run` must pass.

---

## Part A — Capture pipeline hardening

### Task 1: `pensieve service install` persists the token to `~/.pensieve/config.json`

The drift: `install.sh` mints a token, passes it to `pensieve service install` (plist), and separately to `pensieve connect` (config.json). When the two ever disagree (see Task 2), every CLI/hook call 401s. Defense in depth: the Rust install path itself syncs config.json.

**Files:**
- Modify: `crates/pensieve-cli/src/client.rs` (add `persist_local_connection`)
- Modify: `crates/pensieve-cli/src/main.rs:657-665` (`ServiceAction::Install` arm)

- [ ] **Step 1: Write the failing test** — append to `crates/pensieve-cli/src/client.rs`:

```rust
#[cfg(test)]
mod persist_tests {
    use super::*;

    #[test]
    fn persist_local_connection_writes_endpoint_and_token() {
        let dir = std::env::temp_dir().join(format!("pensieve-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        persist_local_connection_at(&path, "http://127.0.0.1:7777", Some("tok-abc")).unwrap();
        let cfg: ClientConfig =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.endpoint, "http://127.0.0.1:7777");
        assert_eq!(cfg.token.as_deref(), Some("tok-abc"));

        // Re-persisting with a new token preserves unrelated fields.
        let with_session = ClientConfig {
            endpoint: "http://127.0.0.1:7777".into(),
            token: Some("tok-abc".into()),
            last_session_id: Some("sess-1".into()),
        };
        std::fs::write(&path, serde_json::to_string(&with_session).unwrap()).unwrap();
        persist_local_connection_at(&path, "http://127.0.0.1:7777", Some("tok-new")).unwrap();
        let cfg: ClientConfig =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.token.as_deref(), Some("tok-new"));
        assert_eq!(cfg.last_session_id.as_deref(), Some("sess-1"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p pensieve-cli persist_local_connection_writes -- --nocapture`
Expected: FAIL — `persist_local_connection_at` not found.

- [ ] **Step 3: Implement** — add to `crates/pensieve-cli/src/client.rs` (below `save_config`):

```rust
/// Sync the local connection (endpoint + token) into a config file, preserving
/// any other persisted fields (e.g. `last_session_id`). Used by
/// `pensieve service install` so the plist/unit token and the CLI token can never
/// drift apart — the silent-401 capture outage of 2026-06-07.
pub(crate) fn persist_local_connection_at(
    path: &Path,
    endpoint: &str,
    token: Option<&str>,
) -> Result<()> {
    let mut cfg: ClientConfig = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    cfg.endpoint = endpoint.to_string();
    if let Some(t) = token {
        cfg.token = Some(t.to_string());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&cfg)?)
        .with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// `persist_local_connection_at` against the default `~/.pensieve/config.json`.
pub(crate) fn persist_local_connection(endpoint: &str, token: Option<&str>) -> Result<()> {
    persist_local_connection_at(&config_path()?, endpoint, token)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p pensieve-cli persist_local_connection_writes`
Expected: PASS

- [ ] **Step 5: Call it from the install arm** — in `crates/pensieve-cli/src/main.rs`, replace the `ServiceAction::Install` arm (currently lines 657-665):

```rust
            ServiceAction::Install { addr, token } => {
                pensieve_local::server_service::install(&pensieve_local::server_service::ServerOptions {
                    addr: addr.clone(),
                    token: token.clone(),
                    pensieve_home: None,
                    secret_key: None,
                })?;
                // Keep the CLI pointed at the service we just installed: the
                // plist/unit carries this token, so config.json must match or
                // every CLI call and capture hook 401s silently.
                if let Err(e) =
                    client::persist_local_connection(&format!("http://{addr}"), token.as_deref())
                {
                    eprintln!("warning: couldn't sync ~/.pensieve/config.json: {e}");
                }
                Ok(())
            }
```

Note: `install()` returns `Result<bool>`; the old code mapped it to `()`. The new code uses `?` and ignores the bool — that bool only signals "no service manager on this OS", in which case the connection sync is still correct (the caller falls back to `PENSIEVE_AUTH_TOKENS` nohup with the same token).

- [ ] **Step 6: Build + clippy**

Run: `cargo clippy -p pensieve-cli -- -D warnings && cargo build -p pensieve-cli`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add crates/pensieve-cli/src/client.rs crates/pensieve-cli/src/main.rs
git commit -m "fix(cli): pensieve service install syncs the token into config.json"
```

### Task 2: `install.sh` reuses the existing supervised server's token

The actual outage mechanism: re-running `install.sh` against an already-supervised, version-matched server mints a fresh `TOKEN` (`install.sh:302`), skips `pensieve service install` (server already running), then runs `pensieve connect --token $TOKEN` — overwriting config.json with a token the running server does not accept.

**Files:**
- Modify: `install.sh` (the `start_serve` function, lines ~301-356)

- [ ] **Step 1: Add a token-extraction helper** — in `install.sh`, directly above `start_serve()`:

```bash
# Pull the admin token out of an existing service definition so re-installs
# reuse it instead of minting a fresh one the running server won't accept.
# Prints the token or nothing.
existing_service_token() {
  local f="$1"
  [ -f "$f" ] || return 0
  # plist:  <key>PENSIEVE_AUTH_TOKENS</key>\n    <string>TOKEN:admin</string>
  # systemd: Environment=PENSIEVE_AUTH_TOKENS=TOKEN:admin
  grep -A1 'PENSIEVE_AUTH_TOKENS' "$f" 2>/dev/null \
    | sed -nE 's/.*<string>([^:<]+):admin<\/string>.*/\1/p; s/.*PENSIEVE_AUTH_TOKENS=([^:]+):admin.*/\1/p' \
    | head -1
}
```

- [ ] **Step 2: Use it in `start_serve`** — replace the first line of `start_serve()` body (`[ -z "$TOKEN" ] && TOKEN="pensieve-local-$(rand_hex)"`) with:

```bash
  # Reuse the token already pinned in the service definition (if any) so a
  # re-run never desyncs the CLI from a server we are NOT restarting.
  local existing_service_file=""
  case "$(uname -s)" in
    Darwin) existing_service_file="$HOME/Library/LaunchAgents/dev.getpensieve.pensieve-server.plist" ;;
    Linux)  existing_service_file="$HOME/.config/systemd/user/pensieve-server.service" ;;
  esac
  if [ -z "$TOKEN" ]; then
    TOKEN="$(existing_service_token "$existing_service_file")"
  fi
  [ -z "$TOKEN" ] && TOKEN="pensieve-local-$(rand_hex)"
```

(The later `service_file` local inside the function remains as-is; the duplication is two lines and keeps the diff minimal.)

- [ ] **Step 3: Verify extraction against the real-world plist shape**

Run:
```bash
cat > /tmp/pensieve-test.plist <<'EOF'
  <key>EnvironmentVariables</key>
  <dict>
    <key>PENSIEVE_AUTH_TOKENS</key>
    <string>pensieve-local-deadbeef:admin</string>
  </dict>
EOF
bash -c 'source <(sed -n "/^existing_service_token()/,/^}/p" install.sh); existing_service_token /tmp/pensieve-test.plist'
printf 'Environment=PENSIEVE_AUTH_TOKENS=pensieve-local-cafe:admin\n' > /tmp/pensieve-test.unit
bash -c 'source <(sed -n "/^existing_service_token()/,/^}/p" install.sh); existing_service_token /tmp/pensieve-test.unit'
```
Expected output: `pensieve-local-deadbeef` then `pensieve-local-cafe`.

- [ ] **Step 4: shellcheck**

Run: `shellcheck install.sh || true` — no NEW warnings versus `git stash && shellcheck install.sh; git stash pop` baseline.

- [ ] **Step 5: Commit**

```bash
git add install.sh
git commit -m "fix(install): reuse the supervised server's token instead of minting a drifting one"
```

### Task 3: `pensieve status` authenticated probe

`cmd_status` (`crates/pensieve-cli/src/main.rs:851-873`) only hits `/health` (unauthenticated) — it printed `Health: ok` all through the 4-day capture outage. Add a tokened probe against `GET /v1/auth/me` (exists: `crates/pensieve-server/src/auth_handler.rs:237`).

**Files:**
- Modify: `crates/pensieve-cli/src/main.rs` (`cmd_status`)

- [ ] **Step 1: Implement** — in `cmd_status`, after the `probe_health` match, add:

```rust
            match probe_auth(&cfg).await {
                Ok(true) => println!("Auth:      ok (token accepted)"),
                Ok(false) => println!(
                    "Auth:      TOKEN REJECTED — the server does not accept the configured token.\n           Fix: re-run the installer, or `pensieve service install --addr <addr> --token <tok>`,\n           or `pensieve connect {} --token <tok>` with the server's real token.",
                    cfg.endpoint
                ),
                Err(e) => println!("Auth:      probe error — {e}"),
            }
```

and add next to `probe_health` (same file, wherever `probe_health` is defined — find with `grep -n "fn probe_health" crates/pensieve-cli/src/main.rs`):

```rust
/// `true` = token accepted, `false` = 401/403, error = transport failure.
async fn probe_auth(cfg: &client::ClientConfig) -> Result<bool> {
    let url = format!("{}/v1/auth/me", cfg.endpoint.trim_end_matches('/'));
    let mut req = client::http_client().get(url);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await?;
    Ok(!matches!(res.status().as_u16(), 401 | 403))
}
```

(`ClientConfig` and `http_client` are already `pub(crate)` in `client.rs`; `cmd_status` already uses `load_config` from the same module.)

- [ ] **Step 2: Build**

Run: `cargo build -p pensieve-cli`
Expected: clean

- [ ] **Step 3: Verify against the live local server (both outcomes)**

```bash
./target/debug/pensieve status            # Auth: ok (token accepted)
PENSIEVE_TOKEN=wrong ./target/debug/pensieve status   # Auth: TOKEN REJECTED …
```
Expected: the two lines above respectively (env `PENSIEVE_TOKEN` overrides config — `client.rs:effective_config`... note `cmd_status` uses `load_config()`, not `effective_config()`; change `cmd_status` to use `client::effective_config()` for the probe so the env override is honored, while still printing the config-file endpoint/token presence from `load_config`).

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-cli/src/main.rs
git commit -m "feat(cli): pensieve status probes auth, not just /health"
```

### Task 4: Capture-health marker from the hooks

`pensieve_emit` (`integrations/claude-code/pensieve-memory/scripts/lib.sh:62-68`) is fire-and-forget: a 401 vanishes. Record the last failure (and clear on success) to `~/.pensieve/capture-health.json`; surface it in `pensieve status`.

**Files:**
- Modify: `integrations/claude-code/pensieve-memory/scripts/lib.sh`
- Modify: `crates/pensieve-cli/src/main.rs` (`cmd_status`)
- Modify: `integrations/claude-code/pensieve-memory/commands/pensieve-status.md` (mention capture health)

- [ ] **Step 1: Rewrite `pensieve_emit`** in `lib.sh`:

```bash
# Where hook-side capture failures are recorded for `pensieve status` to surface.
PENSIEVE_CAPTURE_HEALTH="${PENSIEVE_CAPTURE_HEALTH:-$HOME/.pensieve/capture-health.json}"

# Ship one compact NDJSON event line ($1) to the firehose. Detached so it
# survives the hook process exiting and never adds latency to the turn.
# Outcome is recorded to $PENSIEVE_CAPTURE_HEALTH: failures write a marker,
# the next success clears it — so a silent 401 streak is visible to
# `pensieve status` instead of vanishing.
pensieve_emit() {
  [ "$PENSIEVE_CC_CAPTURE" = "off" ] && return 0
  have pensieve || return 0
  (
    err=$(printf '%s\n' "$1" | pensieve ingest push --table "$PENSIEVE_CC_TABLE" --db "$PENSIEVE_CC_DB" 2>&1 >/dev/null)
    if [ $? -eq 0 ]; then
      rm -f "$PENSIEVE_CAPTURE_HEALTH" 2>/dev/null
    else
      mkdir -p "$(dirname "$PENSIEVE_CAPTURE_HEALTH")" 2>/dev/null
      detail=$(printf '%s' "$err" | head -c 300 | tr '"' "'" | tr '\n' ' ')
      printf '{"ts":"%s","status":"error","detail":"%s"}\n' "$(now_ts)" "$detail" \
        >"$PENSIEVE_CAPTURE_HEALTH" 2>/dev/null
    fi
  ) >/dev/null 2>&1 &
  return 0
}
```

- [ ] **Step 2: Test the failure path with a stub `pensieve` binary**

```bash
tmp=$(mktemp -d)
cat > "$tmp/pensieve" <<'EOF'
#!/bin/sh
echo "ingest returned 401 Unauthorized: unknown token" >&2
exit 1
EOF
chmod +x "$tmp/pensieve"
env PATH="$tmp:$PATH" PENSIEVE_CAPTURE_HEALTH="$tmp/health.json" bash -c '
  source integrations/claude-code/pensieve-memory/scripts/lib.sh
  pensieve_emit "{\"kind\":\"test\"}"; sleep 1; cat "$PENSIEVE_CAPTURE_HEALTH"'
```
Expected: a JSON line with `"status":"error"` and the 401 detail.

Then the success path (stub exits 0): re-run with `exit 0` stub — expected: `health.json` removed (cat errors with "No such file").

- [ ] **Step 3: Surface in `pensieve status`** — in `cmd_status` in `crates/pensieve-cli/src/main.rs`, after the Auth line:

```rust
            // Hook-side capture health (written by the pensieve-memory plugin hooks).
            let health_path = client::config_dir()
                .map(|d| d.join("capture-health.json"))
                .ok();
            if let Some(p) = health_path {
                if let Ok(raw) = std::fs::read_to_string(&p) {
                    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
                    println!(
                        "Capture:   LAST INGEST FAILED at {} — {}",
                        v["ts"].as_str().unwrap_or("?"),
                        v["detail"].as_str().unwrap_or("unknown error"),
                    );
                } else {
                    println!("Capture:   ok (no recorded hook failures)");
                }
            }
```

- [ ] **Step 4: Update the plugin's `/pensieve-status` command doc** — in `integrations/claude-code/pensieve-memory/commands/pensieve-status.md`, add one bullet to whatever checklist it renders: `Check ~/.pensieve/capture-health.json — if present, capture is failing; show its ts + detail and suggest `pensieve status` / re-running the installer.` (Read the file first and match its format.)

- [ ] **Step 5: Build + verify, then commit**

Run: `cargo build -p pensieve-cli && ./target/debug/pensieve status`
Expected: `Capture: ok (no recorded hook failures)` (you repaired the local token earlier).

```bash
git add integrations/claude-code/pensieve-memory/scripts/lib.sh integrations/claude-code/pensieve-memory/commands/pensieve-status.md crates/pensieve-cli/src/main.rs
git commit -m "feat(capture): record hook ingest failures to capture-health.json and surface in pensieve status"
```

---

## Part B — OTLP traces receiver

### Task 5: Trace row mapping (pure function) in `pensieve-ingest-otlp`

New module `traces.rs`: schema + `ExportTraceServiceRequest → RecordBatch` mapping. Mirrors the logs code; promotes `pensieve.subject` / `pensieve.tenant` span attributes into real columns.

**Files:**
- Create: `crates/pensieve-ingest-otlp/src/traces.rs`
- Modify: `crates/pensieve-ingest-otlp/src/lib.rs` (add `pub mod traces;` and make `hex_encode`, `keyvalue_to_json`, `any_value_to_json`, `split_service_and_json` `pub(crate)`)

- [ ] **Step 1: Write the failing test** — at the bottom of the new `crates/pensieve-ingest-otlp/src/traces.rs` (write the test module first; the implementation skeleton in step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Int64Array, StringArray, TimestampNanosecondArray};
    use opentelemetry_proto::tonic::common::v1::{any_value::Value as V, AnyValue, KeyValue};
    use opentelemetry_proto::tonic::resource::v1::Resource;
    use opentelemetry_proto::tonic::trace::v1::{span, ResourceSpans, ScopeSpans, Span, Status};

    fn kv(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.into(),
            value: Some(AnyValue { value: Some(V::StringValue(value.into())) }),
        }
    }

    fn sample_request() -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![kv("service.name", "pensieve-server")],
                    dropped_attributes_count: 0,
                }),
                scope_spans: vec![ScopeSpans {
                    scope: None,
                    spans: vec![Span {
                        trace_id: vec![0xaa; 16],
                        span_id: vec![0xbb; 8],
                        trace_state: String::new(),
                        parent_span_id: vec![],
                        flags: 0,
                        name: "memory.recall".into(),
                        kind: span::SpanKind::Internal as i32,
                        start_time_unix_nano: 1_700_000_000_000_000_000,
                        end_time_unix_nano: 1_700_000_000_250_000_000,
                        attributes: vec![
                            kv("pensieve.subject", "ws-mbp-shaked"),
                            kv("pensieve.tenant", "default"),
                            kv("memory.query", "okta sso"),
                        ],
                        dropped_attributes_count: 0,
                        events: vec![],
                        dropped_events_count: 0,
                        links: vec![],
                        dropped_links_count: 0,
                        status: Some(Status { message: String::new(), code: 1 }),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        }
    }

    #[test]
    fn maps_spans_to_rows() {
        let batch = request_to_batch(&sample_request()).expect("batch");
        assert_eq!(batch.num_rows(), 1);
        let schema = batch.schema();
        let col = |name: &str| schema.index_of(name).unwrap();

        let start = batch.column(col("start_time")).as_any()
            .downcast_ref::<TimestampNanosecondArray>().unwrap();
        assert_eq!(start.value(0), 1_700_000_000_000_000_000);
        let dur = batch.column(col("duration_ns")).as_any()
            .downcast_ref::<Int64Array>().unwrap();
        assert_eq!(dur.value(0), 250_000_000);

        let s = |name: &str| batch.column(col(name)).as_any()
            .downcast_ref::<StringArray>().unwrap().value(0).to_string();
        assert_eq!(s("trace_id"), "aa".repeat(16));
        assert_eq!(s("span_id"), "bb".repeat(8));
        let parents = batch.column(col("parent_span_id")).as_any()
            .downcast_ref::<StringArray>().unwrap();
        assert!(parents.is_null(0)); // root span
        assert_eq!(s("name"), "memory.recall");
        assert_eq!(s("kind"), "INTERNAL");
        assert_eq!(s("status_code"), "OK");
        assert_eq!(s("service_name"), "pensieve-server");
        assert_eq!(s("subject"), "ws-mbp-shaked");
        assert_eq!(s("tenant"), "default");
        // pensieve.* promoted OUT of attributes_json; the rest stays in.
        let attrs = s("attributes_json");
        assert!(attrs.contains("memory.query"));
        assert!(!attrs.contains("pensieve.subject"));
    }

    #[test]
    fn empty_request_yields_no_batch() {
        let req = ExportTraceServiceRequest { resource_spans: vec![] };
        assert!(request_to_batch(&req).expect("ok").num_rows() == 0);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p pensieve-ingest-otlp traces::`
Expected: FAIL to compile — `request_to_batch` / module missing.

- [ ] **Step 3: Implement the mapping** — top of `crates/pensieve-ingest-otlp/src/traces.rs`:

```rust
//! OTLP traces ingest — `ExportTraceServiceRequest` → `otel_traces` rows.
//!
//! Same shape as the logs path in `lib.rs`: one RecordBatch per export,
//! built column-at-a-time, written through the shared [`WritePath`].
//! `pensieve.subject` / `pensieve.tenant` span attributes are promoted to real
//! columns — the Traces page filters on them constantly.

use arrow_array::builder::{Int64Builder, StringBuilder, TimestampNanosecondBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::KeyValue;
use opentelemetry_proto::tonic::trace::v1::span::SpanKind;
use std::sync::Arc;
use tonic::Status;

use crate::{any_value_to_string, hex_encode, keyvalue_to_json, split_service_and_json};

pub const OTEL_TRACES_TABLE: &str = "otel_traces";

pub fn otel_traces_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("start_time", DataType::Timestamp(TimeUnit::Nanosecond, None), true),
        Field::new("end_time", DataType::Timestamp(TimeUnit::Nanosecond, None), true),
        Field::new("duration_ns", DataType::Int64, true),
        Field::new("trace_id", DataType::Utf8, true),
        Field::new("span_id", DataType::Utf8, true),
        Field::new("parent_span_id", DataType::Utf8, true),
        Field::new("name", DataType::Utf8, true),
        Field::new("kind", DataType::Utf8, true),
        Field::new("status_code", DataType::Utf8, true),
        Field::new("status_message", DataType::Utf8, true),
        Field::new("service_name", DataType::Utf8, true),
        Field::new("subject", DataType::Utf8, true),
        Field::new("tenant", DataType::Utf8, true),
        Field::new("attributes_json", DataType::Utf8, true),
        Field::new("resource_json", DataType::Utf8, true),
    ]))
}

fn kind_label(kind: i32) -> &'static str {
    match SpanKind::try_from(kind) {
        Ok(SpanKind::Internal) => "INTERNAL",
        Ok(SpanKind::Server) => "SERVER",
        Ok(SpanKind::Client) => "CLIENT",
        Ok(SpanKind::Producer) => "PRODUCER",
        Ok(SpanKind::Consumer) => "CONSUMER",
        _ => "UNSPECIFIED",
    }
}

fn status_label(code: i32) -> &'static str {
    match code {
        1 => "OK",
        2 => "ERROR",
        _ => "UNSET",
    }
}

/// Split span attributes into (subject, tenant, remaining-as-json).
fn split_pensieve_attrs(attrs: &[KeyValue]) -> (Option<String>, Option<String>, String) {
    let mut subject = None;
    let mut tenant = None;
    let mut rest: Vec<KeyValue> = Vec::with_capacity(attrs.len());
    for kv in attrs {
        let val = kv.value.as_ref().and_then(any_value_to_string);
        match kv.key.as_str() {
            "pensieve.subject" => subject = val,
            "pensieve.tenant" => tenant = val,
            _ => rest.push(kv.clone()),
        }
    }
    let json = serde_json::to_string(&keyvalue_to_json(&rest)).unwrap_or_else(|_| "{}".into());
    (subject, tenant, json)
}

/// Pure mapping: the whole export request → one RecordBatch.
pub fn request_to_batch(req: &ExportTraceServiceRequest) -> Result<RecordBatch, Status> {
    let total: usize = req
        .resource_spans
        .iter()
        .flat_map(|rs| rs.scope_spans.iter())
        .map(|ss| ss.spans.len())
        .sum();

    let mut start_b = TimestampNanosecondBuilder::with_capacity(total);
    let mut end_b = TimestampNanosecondBuilder::with_capacity(total);
    let mut dur_b = Int64Builder::with_capacity(total);
    let mut trace_b = StringBuilder::with_capacity(total, total * 32);
    let mut span_b = StringBuilder::with_capacity(total, total * 16);
    let mut parent_b = StringBuilder::with_capacity(total, total * 16);
    let mut name_b = StringBuilder::with_capacity(total, total * 24);
    let mut kind_b = StringBuilder::with_capacity(total, total * 8);
    let mut status_b = StringBuilder::with_capacity(total, total * 5);
    let mut status_msg_b = StringBuilder::with_capacity(total, total * 8);
    let mut service_b = StringBuilder::with_capacity(total, total * 16);
    let mut subject_b = StringBuilder::with_capacity(total, total * 16);
    let mut tenant_b = StringBuilder::with_capacity(total, total * 8);
    let mut attrs_b = StringBuilder::with_capacity(total, total * 64);
    let mut resource_b = StringBuilder::with_capacity(total, total * 32);

    for rs in &req.resource_spans {
        let (service_name, resource_json) = match &rs.resource {
            Some(r) => split_service_and_json(&r.attributes),
            None => (None, "{}".to_string()),
        };
        for ss in &rs.scope_spans {
            for sp in &ss.spans {
                start_b.append_value(sp.start_time_unix_nano as i64);
                end_b.append_value(sp.end_time_unix_nano as i64);
                dur_b.append_value(
                    (sp.end_time_unix_nano.saturating_sub(sp.start_time_unix_nano)) as i64,
                );
                trace_b.append_value(hex_encode(&sp.trace_id));
                span_b.append_value(hex_encode(&sp.span_id));
                if sp.parent_span_id.is_empty() {
                    parent_b.append_null();
                } else {
                    parent_b.append_value(hex_encode(&sp.parent_span_id));
                }
                name_b.append_value(&sp.name);
                kind_b.append_value(kind_label(sp.kind));
                let (code, msg) = sp
                    .status
                    .as_ref()
                    .map(|s| (s.code, s.message.clone()))
                    .unwrap_or((0, String::new()));
                status_b.append_value(status_label(code));
                if msg.is_empty() {
                    status_msg_b.append_null();
                } else {
                    status_msg_b.append_value(&msg);
                }
                match &service_name {
                    Some(s) => service_b.append_value(s),
                    None => service_b.append_null(),
                }
                let (subject, tenant, attrs_json) = split_pensieve_attrs(&sp.attributes);
                match subject {
                    Some(s) => subject_b.append_value(&s),
                    None => subject_b.append_null(),
                }
                match tenant {
                    Some(t) => tenant_b.append_value(&t),
                    None => tenant_b.append_null(),
                }
                attrs_b.append_value(&attrs_json);
                resource_b.append_value(&resource_json);
            }
        }
    }

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(start_b.finish()),
        Arc::new(end_b.finish()),
        Arc::new(dur_b.finish()),
        Arc::new(trace_b.finish()),
        Arc::new(span_b.finish()),
        Arc::new(parent_b.finish()),
        Arc::new(name_b.finish()),
        Arc::new(kind_b.finish()),
        Arc::new(status_b.finish()),
        Arc::new(status_msg_b.finish()),
        Arc::new(service_b.finish()),
        Arc::new(subject_b.finish()),
        Arc::new(tenant_b.finish()),
        Arc::new(attrs_b.finish()),
        Arc::new(resource_b.finish()),
    ];
    RecordBatch::try_new(otel_traces_schema(), arrays)
        .map_err(|e| Status::internal(format!("build trace batch: {e}")))
}
```

In `lib.rs`, add `pub mod traces;` after the imports and change the four helper fns' visibility: `fn hex_encode` → `pub(crate) fn hex_encode`, same for `any_value_to_string`, `keyvalue_to_json`, `split_service_and_json` (`any_value_to_json` stays private — `keyvalue_to_json` covers it).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pensieve-ingest-otlp traces::`
Expected: PASS (2 tests). If `opentelemetry_proto::tonic::trace` doesn't resolve, add the `"trace"`... it ships with `gen-tonic-messages` (logs already work the same way) — check `cargo doc -p opentelemetry-proto --no-deps` if needed.

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-ingest-otlp/src/traces.rs crates/pensieve-ingest-otlp/src/lib.rs
git commit -m "feat(otlp): trace request -> otel_traces row mapping (subject/tenant promoted)"
```

### Task 6: `OtlpTraceService` + wiring into `pensieve-bin`

**Files:**
- Modify: `crates/pensieve-ingest-otlp/src/lib.rs` (generalize `ensure_table`)
- Modify: `crates/pensieve-ingest-otlp/src/traces.rs` (the service)
- Modify: `crates/pensieve-bin/src/main.rs:1279-1305` (add the trace service)
- Modify: `crates/pensieve-ingest-otlp/tests/otlp_smoke.rs` (ignored live test)

- [ ] **Step 1: Generalize table bootstrap** — in `lib.rs`, extract the body of `OtlpLogsService::ensure_table` into a free function and make the method delegate:

```rust
/// Ensure `table` exists in `database` with `schema`, creating the database
/// on first use (OTLP has no separate "create" step). Idempotent.
pub(crate) async fn ensure_otel_table(
    catalog: &Arc<dyn Catalog>,
    database: &str,
    table: &str,
    schema: Arc<Schema>,
) -> Result<pensieve_core::catalog::TableRef, Status> {
    match catalog.lookup_table(database, table).await {
        Ok(t) => Ok(t),
        Err(_) => {
            let db_id = match find_database_id(&**catalog, database).await {
                Some(id) => id,
                None => catalog
                    .create_database(database)
                    .await
                    .map_err(|e| Status::internal(format!("create_database: {e}")))?,
            };
            catalog
                .create_table(db_id, table, schema, TableConfig::default())
                .await
                .map_err(|e| Status::internal(format!("create_table: {e}")))?;
            catalog
                .lookup_table(database, table)
                .await
                .map_err(|e| Status::internal(format!("lookup after create: {e}")))
        }
    }
}
```

and in `OtlpLogsService::ensure_table`: `ensure_otel_table(&self.catalog, &self.database, OTEL_LOGS_TABLE, otel_logs_schema()).await`.

Run: `cargo test -p pensieve-ingest-otlp` — still green.

- [ ] **Step 2: The trace service** — append to `traces.rs`:

```rust
use pensieve_core::catalog::Catalog;
use pensieve_ingest_core::WritePath;
use opentelemetry_proto::tonic::collector::trace::v1::{
    trace_service_server::{TraceService, TraceServiceServer},
    ExportTracePartialSuccess, ExportTraceServiceResponse,
};
use tonic::{Request, Response};
use tracing::{debug, info};

pub struct OtlpTraceService {
    catalog: Arc<dyn Catalog>,
    write_path: WritePath,
    database: String,
}

impl OtlpTraceService {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        write_path: WritePath,
        database: impl Into<String>,
    ) -> Self {
        Self { catalog, write_path, database: database.into() }
    }

    pub fn into_server(self) -> TraceServiceServer<Self> {
        TraceServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl TraceService for OtlpTraceService {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let req = request.into_inner();
        let batch = request_to_batch(&req)?;
        let total = batch.num_rows();
        debug!(resource_spans = req.resource_spans.len(), total, "otlp trace export");
        if total == 0 {
            return Ok(Response::new(ExportTraceServiceResponse::default()));
        }
        let table_ref = crate::ensure_otel_table(
            &self.catalog,
            &self.database,
            OTEL_TRACES_TABLE,
            otel_traces_schema(),
        )
        .await?;
        let ack = self
            .write_path
            .ingest(&self.database, &table_ref, vec![batch])
            .await
            .map_err(|e| Status::internal(format!("ingest: {e}")))?;
        ::metrics::counter!("pensieve_otlp_spans_total").increment(ack.rows_ingested);
        info!(rows = ack.rows_ingested, "otlp trace export committed");
        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: if ack.rows_ingested == total as u64 {
                None
            } else {
                Some(ExportTracePartialSuccess {
                    rejected_spans: (total as i64 - ack.rows_ingested as i64).max(0),
                    error_message: String::new(),
                })
            },
        }))
    }
}
```

- [ ] **Step 3: Wire in pensieve-bin** — `crates/pensieve-bin/src/main.rs`: add import `use pensieve_ingest_otlp::traces::OtlpTraceService;` (next to the existing `OtlpLogsService` import at line 32 — note the existing alias `OtlpLogsServer` there; check how it's imported with `grep -n "OtlpLogs" crates/pensieve-bin/src/main.rs`). In the OTLP block (lines ~1283-1305), after constructing `otlp_svc`, build and add the trace service:

```rust
        let otlp_trace_svc = OtlpTraceService::new(
            catalog.clone(),
            write_path.clone(),
            cli.otlp_database.clone(),
        )
        .into_server();
        // … inside the spawned server builder:
            let res = tonic::transport::Server::builder()
                .add_service(otlp_svc)
                .add_service(otlp_trace_svc)
                .serve_with_shutdown(otlp_addr, async move { let _ = otlp_rx.recv().await; })
                .await;
```

(`otlp_trace_svc` must be moved into the same `tokio::spawn` — construct it before the `Some(tokio::spawn(...))`.)

- [ ] **Step 4: Build + clippy**

Run: `cargo clippy -p pensieve-ingest-otlp -p pensieve-bin -- -D warnings && cargo build -p pensieve-bin`
Expected: clean

- [ ] **Step 5: Add the ignored live smoke test** — append to `crates/pensieve-ingest-otlp/tests/otlp_smoke.rs` (reuse its `kv` helper):

```rust
#[tokio::test]
#[ignore = "requires a live pensieve OTLP server on 127.0.0.1:4317"]
async fn otlp_export_traces() {
    use opentelemetry_proto::tonic::collector::trace::v1::trace_service_client::TraceServiceClient;
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::trace::v1::{span, ResourceSpans, ScopeSpans, Span, Status};

    let mut client = TraceServiceClient::connect("http://127.0.0.1:4317")
        .await
        .expect("connect to OTLP server");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let req = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(opentelemetry_proto::tonic::resource::v1::Resource {
                attributes: vec![kv("service.name", "otlp-trace-smoke")],
                dropped_attributes_count: 0,
            }),
            scope_spans: vec![ScopeSpans {
                scope: None,
                spans: vec![Span {
                    trace_id: vec![0xcc; 16],
                    span_id: vec![0xdd; 8],
                    trace_state: String::new(),
                    parent_span_id: vec![],
                    flags: 0,
                    name: "smoke.root".into(),
                    kind: span::SpanKind::Server as i32,
                    start_time_unix_nano: now - 50_000_000,
                    end_time_unix_nano: now,
                    attributes: vec![kv("pensieve.subject", "smoke-test")],
                    dropped_attributes_count: 0,
                    events: vec![],
                    dropped_events_count: 0,
                    links: vec![],
                    dropped_links_count: 0,
                    status: Some(Status { message: String::new(), code: 1 }),
                }],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    let resp = client.export(req).await.expect("export").into_inner();
    assert!(resp.partial_success.is_none());
}
```

Run: `cargo test -p pensieve-ingest-otlp` (ignored test skipped, everything green).

- [ ] **Step 6: Commit**

```bash
git add crates/pensieve-ingest-otlp crates/pensieve-bin/src/main.rs
git commit -m "feat(otlp): ExportTraceService receiver -> otel.otel_traces"
```

---

## Part C — Self-instrumentation

### Task 7: `SelfTraceExporter` (SpanData → otel_traces, in-process)

An `opentelemetry_sdk` `SpanExporter` that maps exported `SpanData` straight onto the Task 5 schema and writes through `WritePath` — no loopback gRPC. It holds an `Arc<OnceLock<…>>` so the tracing stack can be installed at process start and wired to storage later; until then it drops batches.

**Files:**
- Create: `crates/pensieve-ingest-otlp/src/self_export.rs`
- Modify: `crates/pensieve-ingest-otlp/src/lib.rs` (add `pub mod self_export;`)
- Modify: `crates/pensieve-ingest-otlp/Cargo.toml` (add `opentelemetry.workspace = true`, `opentelemetry_sdk.workspace = true`, `futures = "0.3"` — match the workspace-dep style used by sibling crates; check with `grep -n "workspace = true" crates/pensieve-ingest-otlp/Cargo.toml`)

- [ ] **Step 1: Write the failing test** — bottom of new `self_export.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Int64Array, StringArray};
    use opentelemetry::trace::{SpanContext, SpanId, SpanKind, Status as OtelStatus, TraceFlags, TraceId, TraceState};
    use opentelemetry::KeyValue as OtelKv;
    use opentelemetry_sdk::export::trace::SpanData;
    use std::borrow::Cow;
    use std::time::{Duration, UNIX_EPOCH};

    fn sample_span() -> SpanData {
        SpanData {
            span_context: SpanContext::new(
                TraceId::from_u128(0xabad1dea_u128),
                SpanId::from_u64(0xbeef),
                TraceFlags::SAMPLED,
                false,
                TraceState::default(),
            ),
            parent_span_id: SpanId::INVALID,
            span_kind: SpanKind::Server,
            name: Cow::Borrowed("memory.recall"),
            start_time: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            end_time: UNIX_EPOCH + Duration::from_secs(1_700_000_000) + Duration::from_millis(250),
            attributes: vec![
                OtelKv::new("pensieve.subject", "ws-mbp-shaked"),
                OtelKv::new("pensieve.tenant", "default"),
                OtelKv::new("memory.results", 7_i64),
            ],
            dropped_attributes_count: 0,
            events: Default::default(),
            links: Default::default(),
            status: OtelStatus::Ok,
            instrumentation_scope: Default::default(),
        }
    }

    #[test]
    fn span_data_maps_to_row() {
        let batch = spans_to_batch(&[sample_span()], "pensieve-server").expect("batch");
        assert_eq!(batch.num_rows(), 1);
        let schema = batch.schema();
        let col = |n: &str| schema.index_of(n).unwrap();
        let s = |n: &str| batch.column(col(n)).as_any()
            .downcast_ref::<StringArray>().unwrap().value(0).to_string();
        assert_eq!(s("name"), "memory.recall");
        assert_eq!(s("service_name"), "pensieve-server");
        assert_eq!(s("subject"), "ws-mbp-shaked");
        assert_eq!(s("status_code"), "OK");
        assert_eq!(s("kind"), "SERVER");
        let parents = batch.column(col("parent_span_id")).as_any()
            .downcast_ref::<StringArray>().unwrap();
        assert!(parents.is_null(0));
        let dur = batch.column(col("duration_ns")).as_any()
            .downcast_ref::<Int64Array>().unwrap();
        assert_eq!(dur.value(0), 250_000_000);
        assert!(s("attributes_json").contains("memory.results"));
        assert!(!s("attributes_json").contains("pensieve.subject"));
    }

    #[test]
    fn unwired_exporter_drops_without_error() {
        let exporter = SelfTraceExporter::unwired();
        // Must not panic / block; just drops the batch.
        futures::executor::block_on(async {
            let mut e = exporter;
            use opentelemetry_sdk::export::trace::SpanExporter as _;
            e.export(vec![sample_span()]).await.expect("drop ok");
        });
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p pensieve-ingest-otlp self_export::`
Expected: compile FAIL — module missing.

- [ ] **Step 3: Implement** — top of `self_export.rs`:

```rust
//! Pensieve's own spans → its own `otel_traces` table, in-process.
//!
//! The exporter is installed into the tracing stack at process start but
//! only begins writing once [`SelfTraceExporter::wire`]d to a catalog +
//! WritePath (they are built later in startup). Until then batches are
//! dropped — never buffered, never blocking.
//!
//! Recursion guard lives one level up: the tracing-opentelemetry layer is
//! filtered to `target = "pensieve_telemetry"` spans only, and nothing in the
//! ingest/storage path uses that target.

use crate::traces::{otel_traces_schema, OTEL_TRACES_TABLE};
use arrow_array::builder::{Int64Builder, StringBuilder, TimestampNanosecondBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use futures::future::BoxFuture;
use pensieve_core::catalog::Catalog;
use pensieve_ingest_core::WritePath;
use opentelemetry::trace::{SpanId, SpanKind, Status as OtelStatus};
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

/// Storage wiring, set once when the server's write path is ready.
pub struct SelfTraceCtx {
    pub catalog: Arc<dyn Catalog>,
    pub write_path: WritePath,
    pub database: String,
}

#[derive(Clone)]
pub struct SelfTraceExporter {
    ctx: Arc<OnceLock<SelfTraceCtx>>,
    service_name: String,
}

impl SelfTraceExporter {
    /// Create an exporter with no storage attached; pair with [`Self::handle`].
    pub fn unwired() -> Self {
        Self { ctx: Arc::new(OnceLock::new()), service_name: "pensieve-server".to_string() }
    }

    /// The shared slot to wire later: `handle.set(SelfTraceCtx{…})`.
    pub fn handle(&self) -> Arc<OnceLock<SelfTraceCtx>> {
        self.ctx.clone()
    }
}

fn ns(t: std::time::SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(0)
}

fn kind_label(k: &SpanKind) -> &'static str {
    match k {
        SpanKind::Internal => "INTERNAL",
        SpanKind::Server => "SERVER",
        SpanKind::Client => "CLIENT",
        SpanKind::Producer => "PRODUCER",
        SpanKind::Consumer => "CONSUMER",
    }
}

/// Pure mapping: a batch of SDK spans → one RecordBatch on the shared schema.
pub fn spans_to_batch(batch: &[SpanData], service_name: &str) -> anyhow::Result<RecordBatch> {
    let total = batch.len();
    let mut start_b = TimestampNanosecondBuilder::with_capacity(total);
    let mut end_b = TimestampNanosecondBuilder::with_capacity(total);
    let mut dur_b = Int64Builder::with_capacity(total);
    let mut trace_b = StringBuilder::new();
    let mut span_b = StringBuilder::new();
    let mut parent_b = StringBuilder::new();
    let mut name_b = StringBuilder::new();
    let mut kind_b = StringBuilder::new();
    let mut status_b = StringBuilder::new();
    let mut status_msg_b = StringBuilder::new();
    let mut service_b = StringBuilder::new();
    let mut subject_b = StringBuilder::new();
    let mut tenant_b = StringBuilder::new();
    let mut attrs_b = StringBuilder::new();
    let mut resource_b = StringBuilder::new();

    for sp in batch {
        let start = ns(sp.start_time);
        let end = ns(sp.end_time);
        start_b.append_value(start);
        end_b.append_value(end);
        dur_b.append_value((end - start).max(0));
        trace_b.append_value(sp.span_context.trace_id().to_string());
        span_b.append_value(sp.span_context.span_id().to_string());
        if sp.parent_span_id == SpanId::INVALID {
            parent_b.append_null();
        } else {
            parent_b.append_value(sp.parent_span_id.to_string());
        }
        name_b.append_value(sp.name.as_ref());
        kind_b.append_value(kind_label(&sp.span_kind));
        match &sp.status {
            OtelStatus::Ok => { status_b.append_value("OK"); status_msg_b.append_null(); }
            OtelStatus::Error { description } => {
                status_b.append_value("ERROR");
                if description.is_empty() { status_msg_b.append_null(); }
                else { status_msg_b.append_value(description.as_ref()); }
            }
            OtelStatus::Unset => { status_b.append_value("UNSET"); status_msg_b.append_null(); }
        }
        service_b.append_value(service_name);

        let mut subject: Option<String> = None;
        let mut tenant: Option<String> = None;
        let mut rest = serde_json::Map::new();
        for kv in &sp.attributes {
            let key = kv.key.as_str();
            let val = kv.value.to_string();
            match key {
                "pensieve.subject" => subject = Some(val),
                "pensieve.tenant" => tenant = Some(val),
                _ => { rest.insert(key.to_string(), serde_json::Value::String(val)); }
            }
        }
        match subject { Some(s) => subject_b.append_value(&s), None => subject_b.append_null() }
        match tenant { Some(t) => tenant_b.append_value(&t), None => tenant_b.append_null() }
        attrs_b.append_value(serde_json::Value::Object(rest).to_string());
        resource_b.append_value("{}");
    }

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(start_b.finish()), Arc::new(end_b.finish()), Arc::new(dur_b.finish()),
        Arc::new(trace_b.finish()), Arc::new(span_b.finish()), Arc::new(parent_b.finish()),
        Arc::new(name_b.finish()), Arc::new(kind_b.finish()),
        Arc::new(status_b.finish()), Arc::new(status_msg_b.finish()),
        Arc::new(service_b.finish()), Arc::new(subject_b.finish()), Arc::new(tenant_b.finish()),
        Arc::new(attrs_b.finish()), Arc::new(resource_b.finish()),
    ];
    Ok(RecordBatch::try_new(otel_traces_schema(), arrays)?)
}

impl SpanExporter for SelfTraceExporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let ctx = self.ctx.clone();
        let service_name = self.service_name.clone();
        Box::pin(async move {
            let Some(ctx) = ctx.get() else { return Ok(()) }; // unwired — drop
            let rb = match spans_to_batch(&batch, &service_name) {
                Ok(rb) if rb.num_rows() > 0 => rb,
                _ => return Ok(()),
            };
            let table = match crate::ensure_otel_table(
                &ctx.catalog, &ctx.database, OTEL_TRACES_TABLE, otel_traces_schema(),
            ).await {
                Ok(t) => t,
                Err(_) => { ::metrics::counter!("pensieve_self_trace_dropped_total").increment(batch.len() as u64); return Ok(()); }
            };
            if ctx.write_path.ingest(&ctx.database, &table, vec![rb]).await.is_err() {
                ::metrics::counter!("pensieve_self_trace_dropped_total").increment(batch.len() as u64);
            }
            Ok(())
        })
    }
}
```

API-drift watch: in opentelemetry_sdk 0.27, `SpanData.attributes` is `Vec<opentelemetry::KeyValue>` and `kv.value` is `opentelemetry::Value` (`.to_string()` works); `SpanExporter::export` takes `Vec<SpanData>` and returns `BoxFuture`. If the trait shape differs at compile time, match what `cargo doc -p opentelemetry_sdk --no-deps` shows for `export::trace::SpanExporter` — the mapping function is the tested invariant, the trait impl is glue.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p pensieve-ingest-otlp self_export::`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-ingest-otlp
git commit -m "feat(otlp): in-process SelfTraceExporter — pensieve's own spans into otel_traces"
```

### Task 8: Install the OTel tracing layer in `pensieve-bin`

**Files:**
- Modify: `crates/pensieve-bin/src/main.rs` (tracing init ~line 82, and wiring after `write_path` exists)
- Modify: `crates/pensieve-bin/Cargo.toml` (add `tracing-opentelemetry.workspace = true`, `opentelemetry.workspace = true`, `opentelemetry_sdk.workspace = true`)

- [ ] **Step 1: Replace the `tracing_subscriber::fmt()` init.** Current shape (main.rs:82-86) is `tracing_subscriber::fmt().with_env_filter(...)` + `.init()` (read the exact lines first). Replace with a layered registry that adds the OTel layer gated to `pensieve_telemetry`:

```rust
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    use tracing_subscriber::Layer as _;

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn,h2=warn"));

    // Self-tracing: spans with target `pensieve_telemetry` are exported into our
    // own otel_traces table. The exporter starts unwired (drops batches) and
    // is connected to storage further down, once the write path exists.
    let self_exporter = pensieve_ingest_otlp::self_export::SelfTraceExporter::unwired();
    let self_trace_handle = self_exporter.handle();
    let tracer_provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(self_exporter, opentelemetry_sdk::runtime::Tokio)
        .build();
    use opentelemetry::trace::TracerProvider as _;
    let tracer = tracer_provider.tracer("pensieve-server");
    let otel_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(
            tracing_subscriber::filter::Targets::new()
                .with_target("pensieve_telemetry", tracing::Level::INFO),
        );

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(otel_layer)
        .init();
```

Preserve whatever options the current `fmt()` call sets (e.g. `.json()`, `.with_target(...)`) by moving them onto `tracing_subscriber::fmt::layer()`. Note `EnvFilter` as a registry-level `.with()` filters ALL layers including the otel one — `pensieve_telemetry` spans are emitted at INFO so the default `info` filter passes them.

- [ ] **Step 2: Wire the exporter once storage exists.** Right after `write_path` is constructed in main (find with `grep -n "let write_path" crates/pensieve-bin/src/main.rs`), add:

```rust
    // Connect self-tracing to storage (drops silently before this point).
    let _ = self_trace_handle.set(pensieve_ingest_otlp::self_export::SelfTraceCtx {
        catalog: catalog.clone(),
        write_path: write_path.clone(),
        database: cli.otlp_database.clone(),
    });
```

- [ ] **Step 3: Build + boot smoke**

Run: `cargo build -p pensieve-bin && cargo run -p pensieve-bin -- serve --addr 127.0.0.1:7997 &` then `curl -s http://127.0.0.1:7997/health && kill %1`
(Adjust the serve invocation to this binary's actual CLI — check `grep -n "struct Cli" -A 20 crates/pensieve-bin/src/main.rs`; if `pensieve-bin` IS the `pensieve` binary, it's `cargo run -p pensieve-bin -- serve …`.)
Expected: health ok, no panic from tracing init.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-bin
git commit -m "feat(telemetry): pensieve_telemetry-target spans export into our own otel_traces"
```

### Task 9: Root request span in the auth middleware

**Files:**
- Modify: `crates/pensieve-server/src/auth/middleware.rs`
- Modify: `crates/pensieve-server/Cargo.toml` only if `tracing` isn't already a dep (it is — verify `grep tracing crates/pensieve-server/Cargo.toml`)

- [ ] **Step 1: Restructure `require_role_middleware`** so BOTH branches (auth-disabled and authenticated) run the request inside a `pensieve_telemetry` span. Replace the function body with:

```rust
pub async fn require_role_middleware(
    State(state): State<AuthLayerState>,
    mut req: Request,
    next: Next,
) -> Response {
    let principal = if !state.backend.enabled() {
        // Auth-disabled mode: pretend an Admin principal in the default tenant
        // so downstream extractors see consistent extensions.
        super::backend::Principal {
            tenant: pensieve_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: None,
            allowed_databases: None,
        }
    } else {
        let token = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(str::trim);
        let Some(token) = token else {
            return unauthorized("missing Authorization: Bearer <token>");
        };
        let principal = match state.backend.authenticate(token).await {
            Ok(p) => p,
            Err(AuthError::UnknownToken) | Err(AuthError::MissingToken) => {
                return unauthorized("unknown token");
            }
            Err(AuthError::Backend(e)) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("auth backend: {e}"),
                )
                    .into_response();
            }
        };
        if principal.role < state.required {
            return forbidden(&format!(
                "token role `{:?}` below required `{:?}`",
                principal.role, state.required
            ));
        }
        principal
    };

    let tenant = principal.tenant;
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let method = req.method().clone();
    req.extensions_mut().insert(principal.clone());
    req.extensions_mut().insert(tenant);

    // Skip self-tracing for noise: health checks, metrics scrapes, and the
    // long-lived live-tail socket (its span would only close on disconnect).
    if route == "/health" || route.starts_with("/metrics") || route.starts_with("/v1/explore/live")
    {
        return next.run(req).await;
    }

    use tracing::Instrument as _;
    let span = tracing::info_span!(
        target: "pensieve_telemetry",
        "request",
        otel.name = %format!("{method} {route}"),
        http.method = %method,
        http.route = %route,
        pensieve.tenant = %tenant,
        pensieve.subject = tracing::field::Empty,
        http.status = tracing::field::Empty,
    );
    if let Some(s) = &subject {
        span.record("pensieve.subject", s.as_str());
    }
    let resp = next.run(req).instrument(span.clone()).await;
    span.record("http.status", resp.status().as_u16());
    if resp.status().is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
    resp
}
```

…where `subject` is captured BEFORE the principal moves into extensions — i.e. just above the two `insert` lines:

```rust
    let subject = principal.subject.clone();
    req.extensions_mut().insert(principal.clone());
    req.extensions_mut().insert(tenant);
```

(so `principal` no longer needs an extra read-back from extensions, and the earlier `principal.clone()` insert can become a move: `insert(principal)` with `subject` already cloned out — `Clone` on `Principal` is then only needed if other middleware re-inserts it; keep the derive anyway, downstream handlers take it by `Extension<Principal>` which clones).

Notes for the implementer:
- `Principal` needs `#[derive(Clone)]` — check `crates/pensieve-server/src/auth/backend.rs:34`; add `Clone` to its derive list if missing.
- `tracing-opentelemetry` maps the special fields `otel.name` / `otel.status_code` onto the OTel span name/status; the span's tracing-name (`"request"`) is the fallback.
- `MatchedPath` requires the axum feature `matched-path` (default-on) — if the extension is `None` (nested routers), the raw path fallback is fine.
- This file currently has no `tracing` import — the macros are fully qualified above, so none is needed.

- [ ] **Step 2: Build + existing tests**

Run: `cargo clippy -p pensieve-server -- -D warnings && cargo test -p pensieve-server`
Expected: clean; existing auth tests still pass (behavior of 401/403 unchanged).

- [ ] **Step 3: Live verify** — restart the dev server with the new binary (`cargo build -p pensieve-bin`, then restart however the dev loop does it — for the supervised local service: `cp target/debug/pensieve ~/.local/bin/pensieve && launchctl kickstart -k gui/$(id -u)/dev.getpensieve.pensieve-server`), make any authed call, wait ~6s (batch export), then:

```bash
TOKEN=$(python3 -c "import json;print(json.load(open('$HOME/.pensieve/config.json'))['token'])")
curl -s -X POST http://127.0.0.1:7777/v1/query \
  -H "authorization: Bearer $TOKEN" -H "content-type: application/x-kql" -H "x-database: otel" \
  -d 'otel_traces | sort by start_time desc | take 3'
```
Expected: NDJSON rows with `name` like `GET /v1/auth/me`, `subject`, `tenant`, `http.status` inside `attributes_json`.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/auth
git commit -m "feat(telemetry): root request span (subject/tenant/route/status) in auth middleware"
```

### Task 10: Memory-op child spans

**Files:**
- Modify: `crates/pensieve-server/src/agent/routes.rs` (`memory_query_handler` ~line 447; the import handler ~line 133; the export handler ~line 250; the ask/run handler ~line 728 — locate each with grep)

- [ ] **Step 1: Instrument `memory_query_handler`** (routes.rs:447-478). Add an `Extension(principal)` param and wrap `retrieve`:

```rust
async fn memory_query_handler(
    State(state): State<AgentState>,
    axum::Extension(principal): axum::Extension<crate::auth::backend::Principal>,
    Json(body): Json<MemoryQueryRequest>,
) -> Json<Value> {
    let span = tracing::info_span!(
        target: "pensieve_telemetry",
        "memory.recall",
        pensieve.subject = principal.subject.as_deref().unwrap_or(""),
        pensieve.tenant = %principal.tenant,
        memory.query = %body.retrieve.query.chars().take(200).collect::<String>(),
        memory.results = tracing::field::Empty,
        memory.took_ms = tracing::field::Empty,
    );
    let shared = SharedToolCtx { /* … unchanged … */ };
    use tracing::Instrument as _;
    let result = retrieve(&shared, &body.retrieve).instrument(span.clone()).await;
    span.record("memory.results", result.memories.len() as u64);
    span.record("memory.took_ms", result.took_ms as u64);
    /* … rest unchanged … */
}
```

(Adjust the import path for `Principal` to however `routes.rs` refers to crate types — check its existing `use` block.)

- [ ] **Step 2: Same pattern on the other three memory handlers** — span names and attributes:
  - import handler → `"memory.import"`, record `memory.imported` count from whatever count the handler already computes for its response.
  - export handler → `"memory.export"`, record `memory.exported` similarly.
  - ask/agent-run handler → `"agent.query"`, attribute `agent.question` truncated to 200 chars (handler at routes.rs ~728 already has `question`).

  In each, the transformation is exactly the Step 1 shape: add the `axum::Extension(principal): axum::Extension<…Principal>` param, build `tracing::info_span!(target: "pensieve_telemetry", "<name>", pensieve.subject = …, pensieve.tenant = %principal.tenant, <attrs as fields>, <counts> = tracing::field::Empty)`, `.instrument(span.clone())` the core future, `span.record(…)` the counts after. The middleware root span is the parent automatically (same task-local context).

- [ ] **Step 2b: Instrument REST ingest as `ingest.batch`** — `crates/pensieve-ingest-rest/src/lib.rs:89` (`ingest_handler`). Wrap the handler body:

```rust
async fn ingest_handler(State(state): State<IngestState>, req: Request) -> Response {
    let table = req
        .headers()
        .get("x-table")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let subject = req
        .extensions()
        .get::<pensieve_server_auth_principal_type>() // see note below
        .and_then(|p| p.subject.clone())
        .unwrap_or_default();
    let span = tracing::info_span!(
        target: "pensieve_telemetry",
        "ingest.batch",
        pensieve.subject = %subject,
        ingest.table = %table,
        ingest.rows = tracing::field::Empty,
    );
    use tracing::Instrument as _;
    /* existing body, wrapped: */
    async move { /* …existing handler body… */ }.instrument(span).await
}
```

  Dependency note: `pensieve-ingest-rest` must not depend on `pensieve-server` (it's the other way around). Check `grep -n "Principal" crates/pensieve-ingest-rest/src/lib.rs` — if the Principal type isn't visible here, skip the subject attribute in this span (the middleware root span carries the subject for the same request; the child only adds table/rows), i.e. drop the `subject` lookup and the `pensieve.subject` field entirely rather than adding a dependency edge. Record `ingest.rows` where the existing body learns the row count (it returns an ack — find `rows_ingested` in the body).

- [ ] **Step 3: Build + tests**

Run: `cargo clippy -p pensieve-server -- -D warnings && cargo test -p pensieve-server`
Expected: clean

- [ ] **Step 4: Live verify** — rebuild + restart (as Task 9 step 3), run `./target/debug/pensieve recall "test"`, wait ~6s, then the same KQL with `| where name startswith 'memory.'`.
Expected: a `memory.recall` row whose `parent_span_id` is non-null and whose `attributes_json` has `memory.query` + `memory.results`.

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-server/src/agent/routes.rs
git commit -m "feat(telemetry): memory.recall/import/export + agent.query child spans"
```

---

## Part D — Web: live pulse fixes + Traces page

### Task 11: Pulse event classification helpers + new kinds

**Files:**
- Modify: `web/src/features/memory/lib.ts`
- Create: `web/src/features/memory/lib.test.ts`

- [ ] **Step 1: Write the failing tests** — `web/src/features/memory/lib.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { isBackfill, kindStyle, spanRowToEvent, STALL_THRESHOLD_MS } from "./lib";

describe("isBackfill", () => {
  const now = Date.parse("2026-06-11T12:00:00Z");
  it("flags rows older than 5 minutes at arrival", () => {
    expect(isBackfill("2026-06-11T11:54:00Z", now)).toBe(true);
    expect(isBackfill("2026-06-11T11:58:00Z", now)).toBe(false);
  });
  it("treats unparseable timestamps as backfill (never animate junk)", () => {
    expect(isBackfill("not-a-date", now)).toBe(true);
  });
});

describe("spanRowToEvent", () => {
  it("maps an otel_traces row to a pulse event", () => {
    const ev = spanRowToEvent({
      start_time: "2026-06-11T11:59:58Z",
      end_time: "2026-06-11T11:59:59Z",
      name: "memory.recall",
      subject: "ws-mbp-shaked",
      trace_id: "aabb",
      duration_ns: 1_000_000_000,
      attributes_json: JSON.stringify({ "memory.query": "okta sso", "memory.results": "7" }),
    });
    expect(ev.kind).toBe("memory.recall");
    expect(ev.ts).toBe("2026-06-11T11:59:59Z");
    expect(ev.sessionId).toBe("ws-mbp-shaked");
    expect(ev.text).toContain("okta sso");
  });
  it("falls back to trace id when subject is missing", () => {
    const ev = spanRowToEvent({ end_time: "2026-06-11T11:59:59Z", name: "agent.query", trace_id: "deadbeef" });
    expect(ev.sessionId).toBe("deadbeef");
  });
});

describe("kindStyle for op kinds", () => {
  it("has dedicated styles for memory ops", () => {
    for (const k of ["memory.recall", "memory.import", "memory.export", "agent.query", "ingest.batch"]) {
      expect(kindStyle(k).label).not.toBe(k); // mapped, not the raw fallback
    }
  });
});

it("stall threshold is 10 minutes", () => {
  expect(STALL_THRESHOLD_MS).toBe(10 * 60 * 1000);
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && npx vitest run src/features/memory/lib.test.ts`
Expected: FAIL — missing exports.

- [ ] **Step 3: Implement in `lib.ts`** — add:

```ts
/** Backfill horizon: events older than this AT ARRIVAL are history, not live. */
export const BACKFILL_THRESHOLD_MS = 5 * 60 * 1000;
/** Stalled-capture horizon: connected but nothing newer than this = stalled. */
export const STALL_THRESHOLD_MS = 10 * 60 * 1000;

/** True when a source timestamp is too old (or unparseable) to animate as live. */
export function isBackfill(iso: string, arrivalNow: number): boolean {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return true;
  return arrivalNow - t > BACKFILL_THRESHOLD_MS;
}

/** Map an `otel_traces` row (memory/agent/ingest op span) to a pulse event. */
export function spanRowToEvent(row: Record<string, unknown>): {
  key: string; ts: string; kind: string; sessionId: string; realm: string; text: string;
} {
  const ts = str(row.end_time) || str(row.start_time) || new Date().toISOString();
  const kind = str(row.name) || "op";
  const sessionId = str(row.subject) || str(row.trace_id) || "—";
  let text = "";
  try {
    const attrs = JSON.parse(str(row.attributes_json) || "{}") as Record<string, unknown>;
    text =
      str(attrs["memory.query"]) ||
      str(attrs["agent.question"]) ||
      [str(attrs["memory.results"]) && `${str(attrs["memory.results"])} memories`]
        .filter(Boolean)
        .join("");
  } catch { /* attributes are best-effort */ }
  const key = `span|${str(row.trace_id)}|${str(row.span_id)}|${ts}`;
  return { key, ts, kind, sessionId, realm: "", text };
}
```

and extend `KIND_STYLE`:

```ts
  "memory.recall": { dot: "bg-teal-400", hsl: "174 70% 55%", label: "recall" },
  "memory.import": { dot: "bg-emerald-400", hsl: "152 70% 50%", label: "memory saved" },
  "memory.export": { dot: "bg-slate-400", hsl: "215 16% 64%", label: "memory export" },
  "agent.query": { dot: "bg-fuchsia-400", hsl: "289 85% 66%", label: "agent query" },
  "ingest.batch": { dot: "bg-lime-400", hsl: "84 70% 55%", label: "ingestion" },
```

- [ ] **Step 4: Run the tests**

Run: `cd web && npx vitest run src/features/memory/lib.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add web/src/features/memory/lib.ts web/src/features/memory/lib.test.ts
git commit -m "feat(web): pulse event classification — backfill horizon, op-span mapping, op kind styles"
```

### Task 12: `useMemoryStream` — honest backfill + ops source

**Files:**
- Modify: `web/src/features/memory/useMemoryStream.ts`

- [ ] **Step 1: Extend `PulseEvent` and split fresh/backfill on ingest.** In `useMemoryStream.ts`:

1. Add `backfill: boolean` to `PulseEvent`, and define `type RawEvent = Omit<PulseEvent, "backfill">` — both `rowToEvent` and `spanRowToEvent` return `RawEvent` (change `rowToEvent`'s return type; ingest stamps `backfill`).
2. Import `isBackfill, spanRowToEvent` from `./lib`.
3. Replace the body of the `ingest` ref fn (typed `(rows: Record<string, unknown>[], toEvent?: (r: Record<string, unknown>) => RawEvent) => void`):

```ts
  const ingest = useRef((rows: Record<string, unknown>[], toEvent = rowToEvent) => {
    const arrival = Date.now();
    const freshLive: PulseEvent[] = [];
    const history: PulseEvent[] = [];
    for (const r of rows) {
      const ev = toEvent(r);
      if (seenRef.current.has(ev.key)) continue;
      seenRef.current.add(ev.key);
      const backfill = isBackfill(ev.ts, arrival);
      (backfill ? history : freshLive).push({ ...ev, backfill });
    }
    // History renders immediately, unanimated — old rows must never tick in
    // one-per-second pretending to be live (the original "1d ago" illusion).
    if (history.length > 0) {
      history.sort((a, b) => b.ts.localeCompare(a.ts));
      setEvents((cur) => [...cur, ...history].slice(0, MAX_EVENTS));
    }
    if (freshLive.length === 0) return;
    freshLive.sort((a, b) => a.ts.localeCompare(b.ts));
    bufferRef.current.push(...freshLive);
    if (!staged) flushAll();
  });
```

4. `MAX_EVENTS` slicing now competes between fresh and history; keep it simple — the cap applies to the combined list.

- [ ] **Step 2: Add the ops live source.** Constants next to `FIREHOSE_SOURCE`:

```ts
const OPS_SOURCE = "otel.otel_traces";
const OPS_DB = "otel";
const OPS_KQL =
  "otel_traces | where name startswith 'memory.' or name startswith 'agent.' or name startswith 'ingest.' | sort by start_time desc | take 12";
```

In the socket effect, open a SECOND `LiveSession` alongside the first:

```ts
    let opsSession: LiveSession | null = null;
    opsSession = new LiveSession({
      endpoint,
      token,
      body: {
        query: OPS_KQL,
        scope: { kind: "sources", sources: [OPS_SOURCE] },
        time_range: null,
      },
      // The ops tail is additive: it never drives status/fallback — the
      // firehose session owns connection state. If this socket errors we
      // simply see no op events until the next polling tick.
      onStatus: () => {},
      onFrame: (f) => {
        if (disposed || usingPolling) return;
        if (f.type === "rows" && Array.isArray(f.rows)) ingest.current(f.rows, spanRowToEvent);
      },
    });
```

and in the cleanup: `opsSession?.close();`. In `startPolling`'s `tick`, add a second fetch after the firehose one (same shape, `x-database: OPS_DB`, body `OPS_KQL` with `| take 20`), feeding `ingest.current(rows, spanRowToEvent)`.

- [ ] **Step 3: Expose stall detection.** Compute the newest event ts in the hook's return:

```ts
  const newestTs = events.reduce<string>((acc, e) => (e.ts > acc ? e.ts : acc), "");
  return { events, status, liveStatus, eventsPerMin, newestTs };
```

(And add `newestTs: string` to the fallback object in `stream-context.tsx`'s `useSharedMemoryStream`: `newestTs: ""`.)

- [ ] **Step 4: Typecheck + tests**

Run: `cd web && npm run typecheck && npx vitest run`
Expected: clean (existing suites unaffected)

- [ ] **Step 5: Commit**

```bash
git add web/src/features/memory/useMemoryStream.ts web/src/features/memory/stream-context.tsx
git commit -m "feat(web): memory pulse tails op spans + classifies backfill at arrival"
```

### Task 13: `LivePulse` — earlier divider, no fake animation, stalled badge

**Files:**
- Modify: `web/src/features/memory/LivePulse.tsx`

- [ ] **Step 1: Split rendering by `backfill` and add the divider.** In `LivePulse`, replace the events `<ul>` block:

```tsx
            <ul className="space-y-1.5">
              <AnimatePresence initial={false}>
                {fresh.map((ev) => (
                  <PulseRow key={ev.key} ev={ev} now={now} reduce={reduce} />
                ))}
              </AnimatePresence>
              {history.length > 0 && (
                <li className="flex items-center gap-2 px-1 pt-2 text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
                  <span className="h-px flex-1 bg-border/60" />
                  earlier
                  <span className="h-px flex-1 bg-border/60" />
                </li>
              )}
              {history.map((ev) => (
                <PulseRow key={ev.key} ev={ev} now={now} reduce={true} />
              ))}
            </ul>
```

with, above the return:

```tsx
  const fresh = events.filter((e) => !e.backfill);
  const history = events.filter((e) => e.backfill);
```

(History rows pass `reduce={true}` → `PulseRow` already renders opacity-only, no slide/height animation, and they're OUTSIDE `AnimatePresence` so they never enter-animate.)

- [ ] **Step 2: Stalled badge.** In the rail header, derive:

```tsx
  const { events, status, eventsPerMin, newestTs } = useSharedMemoryStream();
  const stalled =
    connected && newestTs !== "" && now - new Date(newestTs).getTime() > STALL_THRESHOLD_MS;
```

(import `STALL_THRESHOLD_MS` and `relTime` is already imported) and replace the status text span content:

```tsx
          {stalled ? (
            <span className="text-amber-400" title="The stream is connected but no events are arriving. Check `pensieve status` — capture may be failing.">
              capture stalled · last event {relTime(newestTs, now)}
            </span>
          ) : (
            <>
              {status === "live" ? "streaming" : status === "polling" ? "polling" : "idle"}
              {eventsPerMin > 0 && (
                <span className="tabular-nums text-foreground/70">· {eventsPerMin}/min</span>
              )}
            </>
          )}
```

Also flip the header dot to amber when stalled (`stalled ? "bg-amber-400" : …` in both the ping and dot class chains).

- [ ] **Step 3: Typecheck + visual verify**

Run: `cd web && npm run typecheck && npm run dev` → open the Memory overview against the local server. Expected: 4-day-old firehose rows sit under "earlier" with honest "4d ago" stamps and no slide-in; fresh hook events (generate some by using Claude Code) tick in above the divider; the header shows amber "capture stalled" only when nothing fresh exists.

- [ ] **Step 4: Commit**

```bash
git add web/src/features/memory/LivePulse.tsx
git commit -m "fix(web): live pulse renders backfill honestly + amber capture-stalled state"
```

### Task 14: Traces feature — lib + data hook

**Files:**
- Create: `web/src/features/traces/lib.ts`
- Create: `web/src/features/traces/lib.test.ts`
- Create: `web/src/features/traces/useTraces.ts`

- [ ] **Step 1: Failing tests** — `web/src/features/traces/lib.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { buildSpanTree, fmtDurationNs, type SpanRow } from "./lib";

const span = (over: Partial<SpanRow>): SpanRow => ({
  start_time: "2026-06-11T12:00:00Z",
  end_time: "2026-06-11T12:00:01Z",
  duration_ns: 1e9,
  trace_id: "t1",
  span_id: "s1",
  parent_span_id: null,
  name: "request",
  kind: "SERVER",
  status_code: "OK",
  status_message: null,
  service_name: "pensieve-server",
  subject: "ws-1",
  tenant: "default",
  attributes_json: "{}",
  ...over,
});

describe("buildSpanTree", () => {
  it("nests children under parents, sorted by start", () => {
    const rows = [
      span({ span_id: "child2", parent_span_id: "root", start_time: "2026-06-11T12:00:00.500Z" }),
      span({ span_id: "root" }),
      span({ span_id: "child1", parent_span_id: "root", start_time: "2026-06-11T12:00:00.100Z" }),
    ];
    const roots = buildSpanTree(rows);
    expect(roots).toHaveLength(1);
    expect(roots[0].row.span_id).toBe("root");
    expect(roots[0].children.map((c) => c.row.span_id)).toEqual(["child1", "child2"]);
  });
  it("orphans (missing parent) surface as roots, never dropped", () => {
    const roots = buildSpanTree([span({ span_id: "x", parent_span_id: "gone" })]);
    expect(roots).toHaveLength(1);
  });
});

describe("fmtDurationNs", () => {
  it("scales units", () => {
    expect(fmtDurationNs(950)).toBe("950ns");
    expect(fmtDurationNs(2_500_000)).toBe("2.5ms");
    expect(fmtDurationNs(1_250_000_000)).toBe("1.25s");
  });
});
```

- [ ] **Step 2: Run to verify failure**: `cd web && npx vitest run src/features/traces` → FAIL (module missing).

- [ ] **Step 3: Implement `lib.ts`:**

```ts
// Pure helpers for the Traces surface: row types, span-tree assembly,
// duration formatting. No React.

export interface SpanRow {
  start_time: string;
  end_time: string;
  duration_ns: number;
  trace_id: string;
  span_id: string;
  parent_span_id: string | null;
  name: string;
  kind: string;
  status_code: string;
  status_message: string | null;
  service_name: string | null;
  subject: string | null;
  tenant: string | null;
  attributes_json: string;
}

export interface SpanNode {
  row: SpanRow;
  children: SpanNode[];
  depth: number;
}

/** Assemble parent/child trees from a flat span list. Orphans become roots. */
export function buildSpanTree(rows: SpanRow[]): SpanNode[] {
  const nodes = new Map<string, SpanNode>();
  for (const row of rows) nodes.set(row.span_id, { row, children: [], depth: 0 });
  const roots: SpanNode[] = [];
  for (const node of nodes.values()) {
    const pid = node.row.parent_span_id;
    const parent = pid ? nodes.get(pid) : undefined;
    if (parent) parent.children.push(node);
    else roots.push(node);
  }
  const sortRec = (list: SpanNode[], depth: number) => {
    list.sort((a, b) => a.row.start_time.localeCompare(b.row.start_time));
    for (const n of list) {
      n.depth = depth;
      sortRec(n.children, depth + 1);
    }
  };
  sortRec(roots, 0);
  return roots;
}

export function fmtDurationNs(ns: number): string {
  if (ns < 1_000) return `${ns}ns`;
  if (ns < 1_000_000) return `${+(ns / 1_000).toFixed(1)}µs`;
  if (ns < 1_000_000_000) return `${+(ns / 1_000_000).toFixed(1)}ms`;
  return `${+(ns / 1_000_000_000).toFixed(2)}s`;
}

export const STATUS_TONE: Record<string, string> = {
  OK: "text-emerald-300",
  ERROR: "text-rose-300",
  UNSET: "text-muted-foreground",
};
```

- [ ] **Step 4: Run tests**: `cd web && npx vitest run src/features/traces` → PASS.

- [ ] **Step 5: Implement `useTraces.ts`** (NDJSON `/v1/query` against `otel`, same fetch shape as `useMemoryStream`'s polling tick):

```ts
import { useCallback, useEffect, useState } from "react";
import { useSession } from "@/sdk/session";
import { type SpanRow } from "./lib";

async function kql(endpoint: string, token: string, query: string): Promise<SpanRow[]> {
  const res = await fetch(`${endpoint.replace(/\/$/, "")}/v1/query`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/x-kql",
      "x-database": "otel",
    },
    body: query,
  });
  if (!res.ok) throw new Error(`query failed (${res.status})`);
  const rows: SpanRow[] = [];
  for (const line of (await res.text()).split("\n")) {
    const t = line.trim();
    if (!t) continue;
    try {
      rows.push(JSON.parse(t) as SpanRow);
    } catch {
      /* skip non-row lines */
    }
  }
  return rows;
}

const esc = (s: string) => s.replace(/'/g, "\\'");

/** Root spans (= one row per trace), newest first, with optional filters. */
export function useTraceList(opts: {
  agoExpr: string; // e.g. "ago(1h)"; "" = no time filter
  subject: string | null;
  service: string | null;
  text: string;
  refreshKey: number;
}) {
  const { endpoint, token } = useSession();
  const [rows, setRows] = useState<SpanRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!endpoint || !token) return;
    let cancelled = false;
    const parts = ["otel_traces", `| where parent_span_id == ''`];
    if (opts.agoExpr) parts.push(`| where start_time > ${opts.agoExpr}`);
    if (opts.subject) parts.push(`| where subject == '${esc(opts.subject)}'`);
    if (opts.service) parts.push(`| where service_name == '${esc(opts.service)}'`);
    if (opts.text.trim()) parts.push(`| where name contains '${esc(opts.text.trim())}'`);
    parts.push("| sort by start_time desc", "| take 100");
    setLoading(true);
    kql(endpoint, token, parts.join("\n"))
      .then((r) => { if (!cancelled) { setRows(r); setError(null); } })
      .catch((e) => { if (!cancelled) setError(String(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [endpoint, token, opts.agoExpr, opts.subject, opts.service, opts.text, opts.refreshKey]);

  return { rows, error, loading };
}

/** All spans of one trace (capped) for the waterfall. */
export function useTraceSpans(traceId: string | null) {
  const { endpoint, token } = useSession();
  const [rows, setRows] = useState<SpanRow[]>([]);
  const [loading, setLoading] = useState(false);
  const load = useCallback(() => {
    if (!endpoint || !token || !traceId) { setRows([]); return; }
    setLoading(true);
    kql(endpoint, token,
      `otel_traces | where trace_id == '${esc(traceId)}' | sort by start_time asc | take 500`)
      .then(setRows)
      .catch(() => setRows([]))
      .finally(() => setLoading(false));
  }, [endpoint, token, traceId]);
  useEffect(load, [load]);
  return { rows, loading };
}
```

NOTE: `parent_span_id == ''` vs NULL — KQL nulls may not equal `''`. Verify against the live server: `otel_traces | where parent_span_id == '' | take 1` vs `isempty()`. The KQL crate's supported function list (`crates/pensieve-kql/src/lib.rs:14-36`) doesn't list `isempty` — if `== ''` returns nothing for NULL columns, change the receiver/exporter mapping (Tasks 5/7) to write `""` instead of NULL for root parent ids, and re-run their unit tests (update the `is_null` assertions to `value(0) == ""`). Decide by testing, not by assumption.

- [ ] **Step 6: Typecheck + commit**

Run: `cd web && npm run typecheck`

```bash
git add web/src/features/traces
git commit -m "feat(web): traces data layer — span tree, duration fmt, KQL hooks"
```

### Task 15: Traces page UI + route + sidebar

**Files:**
- Create: `web/src/features/traces/TracesList.tsx`
- Create: `web/src/features/traces/TraceWaterfall.tsx`
- Create: `web/src/routes/_app.traces.tsx`
- Modify: `web/src/app/Sidebar.tsx:38-61` (nav entry)

- [ ] **Step 1: `TraceWaterfall.tsx`** — the drawer body for one trace:

```tsx
import { useMemo } from "react";
import { cn } from "@/lib/utils";
import { buildSpanTree, fmtDurationNs, STATUS_TONE, type SpanNode, type SpanRow } from "./lib";
import { useTraceSpans } from "./useTraces";

/** Flatten the tree depth-first for row rendering. */
function flatten(nodes: SpanNode[]): SpanNode[] {
  const out: SpanNode[] = [];
  const walk = (n: SpanNode) => {
    out.push(n);
    n.children.forEach(walk);
  };
  nodes.forEach(walk);
  return out;
}

export function TraceWaterfall({ traceId }: { traceId: string }) {
  const { rows, loading } = useTraceSpans(traceId);
  const { flat, t0, total } = useMemo(() => {
    const tree = buildSpanTree(rows);
    const flat = flatten(tree);
    const starts = rows.map((r) => new Date(r.start_time).getTime());
    const ends = rows.map((r) => new Date(r.end_time).getTime());
    const t0 = Math.min(...(starts.length ? starts : [0]));
    const total = Math.max(1, Math.max(...(ends.length ? ends : [1])) - t0);
    return { flat, t0, total };
  }, [rows]);

  if (loading) return <p className="p-4 text-xs text-muted-foreground">Loading spans…</p>;
  if (rows.length === 0)
    return <p className="p-4 text-xs text-muted-foreground">No spans for this trace.</p>;

  return (
    <div className="space-y-1 p-3">
      {flat.map((n) => (
        <WaterfallRow key={n.row.span_id} node={n} t0={t0} total={total} />
      ))}
    </div>
  );
}

function WaterfallRow({ node, t0, total }: { node: SpanNode; t0: number; total: number }) {
  const r = node.row;
  const start = new Date(r.start_time).getTime();
  const end = new Date(r.end_time).getTime();
  const leftPct = ((start - t0) / total) * 100;
  const widthPct = Math.max(0.5, ((end - start) / total) * 100);
  const attrs = useMemo(() => {
    try { return Object.entries(JSON.parse(r.attributes_json || "{}") as Record<string, unknown>); }
    catch { return []; }
  }, [r.attributes_json]);

  return (
    <details className="group rounded-md border border-border/40 bg-card/30 hover:border-border-strong/60">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2 py-1.5">
        <span
          className="truncate text-xs text-foreground/85"
          style={{ paddingLeft: `${node.depth * 14}px` }}
          title={r.name}
        >
          {r.name}
        </span>
        <span className={cn("text-2xs", STATUS_TONE[r.status_code] ?? STATUS_TONE.UNSET)}>
          {r.status_code}
        </span>
        <span className="ml-auto shrink-0 text-2xs tabular-nums text-muted-foreground">
          {fmtDurationNs(r.duration_ns)}
        </span>
        <span className="relative h-1.5 w-40 shrink-0 overflow-hidden rounded bg-border/30">
          <span
            className={cn(
              "absolute inset-y-0 rounded",
              r.status_code === "ERROR" ? "bg-rose-400/80" : "bg-violet-400/80",
            )}
            style={{ left: `${leftPct}%`, width: `${widthPct}%` }}
          />
        </span>
      </summary>
      {attrs.length > 0 && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 border-t border-border/40 px-3 py-2 text-2xs">
          {attrs.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="font-mono text-muted-foreground">{k}</dt>
              <dd className="truncate text-foreground/80" title={String(v)}>{String(v)}</dd>
            </div>
          ))}
        </dl>
      )}
    </details>
  );
}
```

- [ ] **Step 2: `TracesList.tsx`** — filters + table + selection:

```tsx
import { useEffect, useMemo, useState } from "react";
import { RefreshCw } from "lucide-react";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { fmtDurationNs, STATUS_TONE, type SpanRow } from "./lib";
import { useTraceList } from "./useTraces";

const RANGES: { label: string; ago: string }[] = [
  { label: "15m", ago: "ago(15m)" },
  { label: "1h", ago: "ago(1h)" },
  { label: "6h", ago: "ago(6h)" },
  { label: "24h", ago: "ago(24h)" },
  { label: "7d", ago: "ago(7d)" },
];

export function TracesList({
  selected,
  onSelect,
}: {
  selected: string | null;
  onSelect: (traceId: string | null) => void;
}) {
  const [agoExpr, setAgoExpr] = useState("ago(1h)");
  const [subject, setSubject] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [refreshKey, setRefreshKey] = useState(0);
  const [live, setLive] = useState(false);
  const [now] = useState(() => Date.now());
  const { rows, error, loading } = useTraceList({ agoExpr, subject, service: null, text, refreshKey });

  // Live mode: keep the list fresh by re-querying every 5s (the self-trace
  // batch exporter flushes on a similar cadence, so sockets buy nothing here).
  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setRefreshKey((k) => k + 1), 5000);
    return () => clearInterval(t);
  }, [live]);

  const subjects = useMemo(
    () => [...new Set(rows.map((r) => r.subject).filter((s): s is string => Boolean(s)))],
    [rows],
  );

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex rounded-md border border-border/60 p-0.5">
          {RANGES.map((r) => (
            <button
              key={r.label}
              onClick={() => setAgoExpr(r.ago)}
              className={cn(
                "rounded px-2 py-1 text-xs",
                agoExpr === r.ago ? "bg-card text-foreground" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {r.label}
            </button>
          ))}
        </div>
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="filter by operation…"
          className="h-8 w-56 rounded-md border border-border/60 bg-card/40 px-2 text-xs outline-none focus:border-border-strong"
        />
        {subjects.length > 0 && (
          <div className="flex flex-wrap items-center gap-1">
            {subjects.map((s) => (
              <button
                key={s}
                onClick={() => setSubject(subject === s ? null : s)}
                className={cn(
                  "rounded-full border px-2 py-0.5 font-mono text-[10px]",
                  subject === s
                    ? "border-violet-400/60 bg-violet-500/15 text-violet-200"
                    : "border-border/60 text-muted-foreground hover:text-foreground",
                )}
              >
                {s}
              </button>
            ))}
          </div>
        )}
        <label className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={live}
            onChange={(e) => setLive(e.target.checked)}
            className="h-3.5 w-3.5 accent-violet-400"
          />
          live
        </label>
        <button
          onClick={() => setRefreshKey((k) => k + 1)}
          className="flex h-8 items-center gap-1 rounded-md border border-border/60 px-2 text-xs text-muted-foreground hover:text-foreground"
        >
          <RefreshCw className={cn("h-3 w-3", loading && "animate-spin")} /> refresh
        </button>
      </div>

      {error && <p className="text-xs text-rose-300">{error}</p>}
      {!error && rows.length === 0 && !loading && (
        <p className="px-1 py-8 text-center text-xs text-muted-foreground">
          No traces in this window. Pensieve's own API operations appear here as they happen;
          external services can ship spans to the OTLP endpoint (port 4317).
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-border/50">
        <table className="w-full text-left text-xs">
          <thead className="sticky top-0 bg-surface text-2xs uppercase tracking-wider text-muted-foreground">
            <tr>
              <th className="px-3 py-2">time</th>
              <th className="px-3 py-2">operation</th>
              <th className="px-3 py-2">entity</th>
              <th className="px-3 py-2">service</th>
              <th className="px-3 py-2 text-right">duration</th>
              <th className="px-3 py-2">status</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <TraceRow key={r.span_id} r={r} now={now} active={selected === r.trace_id} onSelect={onSelect} />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function TraceRow({
  r, now, active, onSelect,
}: {
  r: SpanRow; now: number; active: boolean; onSelect: (id: string | null) => void;
}) {
  return (
    <tr
      onClick={() => onSelect(active ? null : r.trace_id)}
      className={cn(
        "cursor-pointer border-t border-border/40 transition-colors hover:bg-card/60",
        active && "bg-card/80",
      )}
    >
      <td className="whitespace-nowrap px-3 py-2 tabular-nums text-muted-foreground">
        {relTime(r.start_time, now)}
      </td>
      <td className="max-w-[22rem] truncate px-3 py-2 text-foreground/90" title={r.name}>{r.name}</td>
      <td className="px-3 py-2 font-mono text-[11px] text-foreground/75">{r.subject ?? "—"}</td>
      <td className="px-3 py-2 text-muted-foreground">{r.service_name ?? "—"}</td>
      <td className="px-3 py-2 text-right tabular-nums">{fmtDurationNs(r.duration_ns)}</td>
      <td className={cn("px-3 py-2", STATUS_TONE[r.status_code] ?? STATUS_TONE.UNSET)}>{r.status_code}</td>
    </tr>
  );
}
```

- [ ] **Step 3: Route `web/src/routes/_app.traces.tsx`** (list + slide-over detail via `?trace=` search param, matching the file-route pattern of `_app.graph.tsx`):

```tsx
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { X } from "lucide-react";
import { TracesList } from "@/features/traces/TracesList";
import { TraceWaterfall } from "@/features/traces/TraceWaterfall";

type TracesSearch = { trace?: string };

export const Route = createFileRoute("/_app/traces")({
  validateSearch: (search: Record<string, unknown>): TracesSearch => ({
    trace: typeof search.trace === "string" ? search.trace : undefined,
  }),
  component: TracesPage,
});

function TracesPage() {
  const { trace } = Route.useSearch();
  const navigate = useNavigate({ from: "/traces" });
  const select = (traceId: string | null) =>
    navigate({ search: { trace: traceId ?? undefined }, replace: true });

  return (
    <div className="flex h-full">
      <div className="min-w-0 flex-1 p-4">
        <h1 className="mb-3 text-sm font-medium text-foreground/90">Traces</h1>
        <TracesList selected={trace ?? null} onSelect={select} />
      </div>
      {trace && (
        <aside className="flex w-[34rem] shrink-0 flex-col border-l border-border/60 bg-surface">
          <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
            <span className="font-mono text-2xs text-muted-foreground" title={trace}>
              trace {trace.slice(0, 16)}…
            </span>
            <button onClick={() => select(null)} className="ml-auto text-muted-foreground hover:text-foreground">
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            <TraceWaterfall traceId={trace} />
          </div>
        </aside>
      )}
    </div>
  );
}
```

(If the repo uses generated route trees, run the router codegen — check how other routes appear in `web/src/routeTree.gen.ts` if present; vite tanstack-router plugin regenerates on `npm run dev`/`build`.)

- [ ] **Step 4: Sidebar entry** — in `web/src/app/Sidebar.tsx`, import `Activity` from `lucide-react` and add to the Explore group after Graph:

```tsx
      { to: "/traces", label: "Traces", icon: Activity },
```

- [ ] **Step 5: Typecheck, lint, visual verify**

Run: `cd web && npm run typecheck && npm run lint && npm run dev`
Open `/traces` against the local server (it now self-traces): expect rows for your own `pensieve recall` / UI API calls; click one → waterfall drawer with the root request span and `memory.recall` child.

- [ ] **Step 6: Commit**

```bash
git add web/src/features/traces web/src/routes/_app.traces.tsx web/src/app/Sidebar.tsx
git commit -m "feat(web): Traces page — entity-filterable trace list + span waterfall"
```

### Task 16: Playwright e2e (on-demand) — traces end to end

**Files:**
- Create: `web/e2e/traces.spec.ts`

- [ ] **Step 1: Write the spec** (same env-driven shape as `golden-path.spec.ts` — runs on demand against a live server):

```ts
import { test, expect } from "@playwright/test";

const TOKEN    = process.env.PENSIEVE_E2E_TOKEN ?? "";
const ENDPOINT = process.env.PENSIEVE_WEB_URL  ?? "http://localhost:8080";

test("memory op → self-trace → visible on Traces page", async ({ page, request }) => {
  // 1. Cause a memory.recall span via the API.
  const res = await request.post(`${ENDPOINT}/v1/agent/memory/query`, {
    headers: { authorization: `Bearer ${TOKEN}` },
    data: { query: "traces e2e probe" },
  });
  expect(res.ok()).toBeTruthy();

  // 2. Connect the UI.
  await page.goto("/");
  await expect(page).toHaveURL(/\/settings/);
  await page.fill("input#endpoint", ENDPOINT);
  await page.fill("input#token", TOKEN);
  await page.fill("input#database", "default");
  await page.getByRole("button", { name: /save \+ connect/i }).click();

  // 3. Traces page shows the request trace (batch exporter flushes ~5s).
  await page.goto("/traces");
  await expect(
    page.getByRole("cell", { name: /POST \/v1\/agent\/memory\/query/ }).first(),
  ).toBeVisible({ timeout: 30_000 });

  // 4. Drill in: waterfall shows the memory.recall child span.
  await page.getByRole("cell", { name: /POST \/v1\/agent\/memory\/query/ }).first().click();
  await expect(page.getByText("memory.recall").first()).toBeVisible({ timeout: 10_000 });
});
```

If the connect screen's selectors differ from golden-path (it may auto-skip when already connected), mirror whatever golden-path does verbatim. If rows don't appear within 30s, refresh once inside an `expect.poll` — the page has a refresh button (`getByRole("button", { name: /refresh/i }).click()`).

- [ ] **Step 2: Run it against the local stack**

```bash
cd web && PENSIEVE_WEB_URL=http://127.0.0.1:7777 \
  PENSIEVE_E2E_TOKEN=$(python3 -c "import json;print(json.load(open('$HOME/.pensieve/config.json'))['token'])") \
  npx playwright test e2e/traces.spec.ts
```
Expected: PASS (needs the Task 8-10 server build running — restart the service on the new binary first).

- [ ] **Step 3: Commit**

```bash
git add web/e2e/traces.spec.ts
git commit -m "test(e2e): memory op self-trace visible end-to-end on the Traces page"
```

### Task 17: Full verification sweep + docs touch

- [ ] **Step 1: Whole-workspace checks**

```bash
cargo clippy --workspace -- -D warnings
cargo test --workspace
cd web && npm run typecheck && npm run lint && npx vitest run && npm run build
```
Expected: all green (workspace tests can be slow; `-p` the touched crates first if iterating).

- [ ] **Step 2: Update `docs/architecture.md`** — add a short "Self-telemetry" subsection: OTLP receiver signals (logs + traces), the `otel.otel_traces` schema table from the spec, the `pensieve_telemetry` target convention ("spans with this target are exported into our own store; never use it inside the ingest/storage path"), and the capture-health.json contract.

- [ ] **Step 3: Final commit + ship per repo convention**

```bash
git add docs/architecture.md
git commit -m "docs: self-telemetry, otel_traces schema, pensieve_telemetry target convention"
```

Then use superpowers:finishing-a-development-branch (current branch: `feat/federated-sources` — consider whether this work should live on its own branch `feat/live-telemetry-traces` cut from main before starting Task 1; if so, cut it first and cherry-pick the two docs commits).

---

## Execution notes (read before Task 1)

- **Branch:** this plan's commits are independent of the in-flight `feat/federated-sources` work. Prefer `git checkout main && git pull && git checkout -b feat/live-telemetry-traces`, then `git cherry-pick 5606098a` (the spec commit) so the spec travels with the branch.
- **Local service restarts:** the user's machine runs the supervised server (`launchctl`, label `dev.getpensieve.pensieve-server`, binary `~/.local/bin/pensieve`). Live-verify steps that need the new build: `cargo build -p pensieve-bin && cp target/debug/pensieve ~/.local/bin/pensieve && launchctl kickstart -k gui/$(id -u)/dev.getpensieve.pensieve-server`. (Confirm `pensieve-bin` produces the `pensieve` binary: `grep -n "^name" crates/pensieve-bin/Cargo.toml` / `crates/pensieve-cli` — whichever crate owns the `pensieve` bin target is the one to copy.)
- **API-drift expectations:** opentelemetry_sdk 0.27 / tracing-opentelemetry 0.28 APIs in Tasks 7-8 are written from the workspace's pinned versions but not compiled yet — if `with_batch_exporter` or the `SpanExporter` trait signature differs, adapt the glue; the row mapping and its tests are the contract.
- **NULL vs `''` for `parent_span_id`:** Task 14 step 5's note is load-bearing — resolve it empirically before building the Traces list on `where parent_span_id == ''`.
