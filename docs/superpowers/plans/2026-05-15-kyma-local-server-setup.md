# kyma Local Server Setup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Note for this plan specifically:** Unlike a feature build, this is a setup procedure against an existing binary. Each task ends at a **verification gate** with explicit expected output. No code is written, no commits are made beyond the spec already in place. Stop and ask the user if a verification gate doesn't match its expected output — do not "fix forward" by guessing.

**Goal:** Bring up a working kyma server on `shakedaskayo`'s Mac and confirm it via an ingest-then-KQL-query smoke test (n=2 of `CARD_DECLINED`).

**Architecture:** Hybrid. `docker compose` runs Postgres + MinIO + Redpanda + bucket-init from the repo's compose file. The `kyma-bin` binary runs natively via `cargo run` against `localhost`-mapped versions of those services. No production hosting, no Kubernetes.

**Tech Stack:** kyma (Rust, DataFusion), docker + docker compose v5, Postgres 16 (`pgvector/pgvector:pg16`), MinIO (S3-compatible), Redpanda (Kafka-compatible — present but unused for the smoke test), rustup stable toolchain, `curl` for HTTP smoke test.

**Spec:** [`../specs/2026-05-15-kyma-local-server-setup-design.md`](../specs/2026-05-15-kyma-local-server-setup-design.md)

---

## Conventions used in this plan

- `$KYMA` means `/Users/shakedaskayo/shaked/projects/kyma`.
- "Verification gate" means: run the listed command, compare to the **Expected** block, only mark the step done if it matches. If it doesn't match, stop and tell the user — do not silently retry or work around.
- Commands assume `bash`/`zsh`, run from `$KYMA` unless stated otherwise.

---

## Task 1: Pre-flight environment checks

**Files:** None (host inspection only).

- [ ] **Step 1.1: Confirm working directory and clone are intact**

Run:
```bash
cd /Users/shakedaskayo/shaked/projects/kyma && git log -1 --oneline && ls Cargo.toml docker-compose.yml rust-toolchain.toml
```

Expected: a single commit line is printed (the just-committed spec commit `e5671ab5 docs(spec): kyma local server setup…` or a later commit), and `ls` lists all three files with no errors.

If `cd` fails, the repo isn't cloned — stop and inform the user (the brainstorming step should have cloned it; if it isn't there something has been moved).

- [ ] **Step 1.2: Confirm docker daemon is running**

Run:
```bash
docker info --format '{{.ServerVersion}}'
```

Expected: a version string (e.g. `28.4.0` or similar) on stdout. **Not** a "Cannot connect to the Docker daemon" error.

If the daemon isn't running, ask the user to start Docker Desktop rather than starting it yourself.

- [ ] **Step 1.3: Confirm none of kyma's host ports are already taken**

Run:
```bash
for p in 5433 8080 9000 9001 9090 9092 9644; do
  echo -n "port $p: "
  lsof -nP -iTCP:$p -sTCP:LISTEN >/dev/null 2>&1 && echo "IN USE" || echo "free"
done
```

Expected: every line ends with `free`.

If any port is `IN USE`, stop and tell the user which port and what process is holding it (`lsof -nP -iTCP:<port> -sTCP:LISTEN`). Do not kill processes.

- [ ] **Step 1.4: Confirm rustup is available**

Run:
```bash
which rustup && rustup --version
```

Expected: a path under `/opt/homebrew/bin/rustup` (or similar) plus a version string.

If `which rustup` returns nothing, stop and ask the user to install rustup (`brew install rustup` then `rustup-init`). Do not auto-install developer toolchains.

---

## Task 2: Install the pinned Rust toolchain

**Files:** None modified in repo; rustup will populate `~/.rustup/` and `~/.cargo/`.

- [ ] **Step 2.1: Install the toolchain pinned by the repo**

Run, from `$KYMA`:
```bash
cd /Users/shakedaskayo/shaked/projects/kyma && rustup show
```

`rust-toolchain.toml` declares `channel = "stable"` with components `rustfmt clippy rust-src`. `rustup show` triggers an install of the channel if missing.

Expected: output lists `stable-aarch64-apple-darwin` (or `x86_64-apple-darwin` on Intel) as installed and active. First run will print download progress and may take 1–3 minutes.

- [ ] **Step 2.2: Ensure `~/.cargo/bin` is on PATH for the rest of this session**

Run:
```bash
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env" || true
which cargo && cargo --version
```

Expected: `which cargo` prints `/Users/shakedaskayo/.cargo/bin/cargo` (or similar) and `cargo --version` prints a stable version (e.g. `cargo 1.<n>.0`).

If `cargo` still isn't found, run `rustup default stable` and retry. If still failing, stop and tell the user — do not start tweaking shell init files.

---

## Task 3: Bring up docker-compose dependencies

**Files:** Reads `$KYMA/docker-compose.yml`; no edits.

- [ ] **Step 3.1: Start postgres, minio, the bucket-init job, and redpanda (but NOT the kyma service)**

Run, from `$KYMA`:
```bash
cd /Users/shakedaskayo/shaked/projects/kyma && docker compose up -d postgres minio minio-init redpanda
```

Expected: docker compose pulls images (one-time, may take a couple of minutes) and reports `Started`/`Created` for each named service. Exit code 0.

If any image fails to pull (network), stop and report the error verbatim.

- [ ] **Step 3.2: Wait for healthchecks to settle and confirm state**

Run:
```bash
docker compose ps
```

Expected output (status column):
- `kyma-postgres` → `Up … (healthy)`
- `kyma-minio` → `Up … (healthy)`
- `kyma-redpanda` → `Up … (healthy)`
- `kyma-minio-init` → `Exited (0)` (it's a one-shot job)

If any service is `Up (unhealthy)` or still `starting` after ~30 seconds, run `docker compose logs <service>` and stop, sharing the relevant log lines with the user. Do not restart in a loop.

- [ ] **Step 3.3: Confirm the buckets were created**

Run:
```bash
docker compose logs minio-init 2>&1 | tail -10
```

Expected: the log contains `Bucket created successfully` lines for `local/kyma`, `local/kyma-ingest`, `local/kyma-archive` (or a `--ignore-existing` no-op on a re-run), and the line `kyma buckets ready`.

If the line `kyma buckets ready` is missing, stop and report.

---

## Task 4: Build kyma-bin (debug profile first)

**Files:** Cargo populates `$KYMA/target/debug/`; no source edits.

- [ ] **Step 4.1: Kick off the debug build**

Run, from `$KYMA`:
```bash
cd /Users/shakedaskayo/shaked/projects/kyma && cargo build -p kyma-bin 2>&1 | tail -40
```

Expected: cargo downloads dependencies (one-time, several minutes), compiles ~200+ crates, and the final line is something like `Finished \`dev\` profile [unoptimized + debuginfo] target(s) in <N>s`.

Notes for the executor:
- First build of the workspace can take 10–15 minutes; that's expected. Don't abort.
- If a system dependency is missing (e.g. `pkg-config`, `openssl`, `protobuf`), the error will be explicit. Stop and report the error verbatim. Do not `brew install` system packages unilaterally — ask first.

- [ ] **Step 4.2: Verify the binary exists and links**

Run:
```bash
ls -l target/debug/kyma-bin && target/debug/kyma-bin --help 2>&1 | head -20
```

Expected: the file exists with execute bit set, and `--help` prints a usage summary (or, if the binary doesn't implement `--help`, a help-related parse error rather than a dyld/link failure).

If you see a dynamic linker error (e.g. `Library not loaded`), stop and report.

---

## Task 5: Run kyma-bin against the dockerized dependencies

**Files:** None modified.

- [ ] **Step 5.1: Export the host-mapped env vars and launch kyma-bin**

The env vars below mirror the `kyma` service block in `docker-compose.yml` but **replace container-internal hostnames** (`postgres`, `minio`) **with `localhost` + the host-published port**. This is the most error-prone part of the setup — do not copy the compose values verbatim.

Run in a dedicated terminal (or as a background process the executor can monitor):
```bash
cd /Users/shakedaskayo/shaked/projects/kyma && \
KYMA_CATALOG_URL=postgres://kyma:kyma_dev@localhost:5433/kyma \
KYMA_S3_ENDPOINT=http://localhost:9000 \
KYMA_S3_BUCKET=kyma \
KYMA_S3_REGION=us-east-1 \
KYMA_S3_ACCESS_KEY_ID=kyma_admin \
KYMA_S3_SECRET_ACCESS_KEY=kyma_admin_dev \
KYMA_S3_PATH_STYLE=true \
KYMA_S3_ALLOW_HTTP=true \
KYMA_HTTP_ADDR=0.0.0.0:8080 \
KYMA_GRPC_ADDR=0.0.0.0:9090 \
KYMA_OTLP_ADDR=off \
KYMA_OTLP_DATABASE=default \
KYMA_PATH_PREFIX=kyma \
KYMA_AUTH_TOKENS="" \
cargo run -p kyma-bin
```

If running this from an agent, prefer `run_in_background` (or `Bash` background mode) and capture the stdout log so the next step can read it.

Expected output (within ~10 seconds of launch):
- Log line(s) indicating the HTTP listener bound on `0.0.0.0:8080`.
- Log line(s) indicating the Flight gRPC listener bound on `0.0.0.0:9090`.
- No panic, no `Connection refused` to postgres, no `403`/`404` to minio.

Common failure modes:
- `connection refused` to `localhost:5433` → postgres container is down or port not published. Re-run Task 3 verification.
- `dispatch failure` / `nodename nor servname provided` → an env var still references `postgres`/`minio` instead of `localhost`. Fix the env line, do not modify code.
- Panic on startup → stop, capture the panic, share with the user.

- [ ] **Step 5.2: Confirm the listener is actually accepting connections**

Run from a separate shell:
```bash
lsof -nP -iTCP:8080 -sTCP:LISTEN && lsof -nP -iTCP:9090 -sTCP:LISTEN
```

Expected: both commands print a `LISTEN` line whose `COMMAND` column is `kyma-bin` (or similar).

---

## Task 6: Smoke test — health endpoint

**Files:** None.

- [ ] **Step 6.1: Hit `/health`**

Run:
```bash
curl -fsS -i http://localhost:8080/health
```

Expected: HTTP status line is `HTTP/1.1 200 OK` (or 204), body is small (often `ok` or empty), and `curl` exits 0.

If curl exits non-zero or the status is 5xx, the engine is not actually ready — stop and check Task 5's log output. Do not proceed to ingest.

- [ ] **Step 6.2 (informational): Sanity-check `/metrics` is reachable**

Run:
```bash
curl -fsS http://localhost:8080/metrics | head -10
```

Expected: Prometheus-format text starting with `# HELP …` or `# TYPE …` lines. This is a smoke check, not a gate — if missing it's a regression worth reporting but not a stop condition.

---

## Task 7: Smoke test — ingest two NDJSON log rows

**Files:** None.

- [ ] **Step 7.1: POST the README's two log lines**

Run:
```bash
curl -fsS -i -X POST http://localhost:8080/v1/ingest \
  -H "X-Database: obs" \
  -H "X-Table: otel_logs" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @- <<'EOF'
{"timestamp":"2026-04-20T14:23:01Z","service.name":"payments-svc","severity_text":"ERROR","message":"card declined","attributes":{"error.code":"CARD_DECLINED"}}
{"timestamp":"2026-04-20T14:23:04Z","service.name":"payments-svc","severity_text":"ERROR","message":"card declined","attributes":{"error.code":"CARD_DECLINED"}}
EOF
```

Expected: HTTP status `2xx` (typically `200` or `202`), `curl` exit 0. The response body may include row counts or be empty depending on the kyma-bin version.

If status is `4xx`, the request shape is wrong (header name, content-type, database name) — stop and capture the response body before guessing. Do not change the body of this request without re-reading the README.

If status is `5xx`, capture the kyma-bin log line that fired at the same wall-clock time and stop.

---

## Task 8: Smoke test — KQL query (the done gate)

**Files:** None.

- [ ] **Step 8.1: POST the README's KQL summarize query**

Run:
```bash
curl -fsS -X POST http://localhost:8080/v1/query \
  -H "X-Database: obs" \
  -H "Content-Type: application/x-kql" \
  --data-binary 'otel_logs | where severity_text == "ERROR" | summarize n = count() by error_code = tostring(attributes["error.code"])'
```

Expected: response body contains a row with `error_code` equal to `CARD_DECLINED` and `n` equal to `2`. Exact serialization may be JSON (`{"error_code":"CARD_DECLINED","n":2}`) or Arrow-JSON or a thin envelope — accept any shape that contains those two values.

Concrete check (any of these wording variants is fine; pick what's robust to JSON shape):
```bash
curl -fsS -X POST http://localhost:8080/v1/query \
  -H "X-Database: obs" \
  -H "Content-Type: application/x-kql" \
  --data-binary 'otel_logs | where severity_text == "ERROR" | summarize n = count() by error_code = tostring(attributes["error.code"])' \
  | grep -q 'CARD_DECLINED' \
  && echo "DONE GATE: PASSED" \
  || echo "DONE GATE: FAILED"
```

Expected: `DONE GATE: PASSED`, plus a manual eyeball of the raw JSON to confirm `n = 2` (not `1`, not `0`).

If `n != 2`:
- `n = 1` typically means only one of the two ingest rows made it (check Task 7's response).
- `n = 0` means the query found no matching rows — possible causes are wrong database (`obs` vs `default`), wrong table name (`otel_logs`), or the staging buffer hasn't flushed yet (wait 5s and retry).

If the query returns an empty result or an error, stop. Do not pad data, do not retry indefinitely, do not "fix" the query. Capture the response and the kyma-bin log for the user.

- [ ] **Step 8.2: Final summary back to the user**

Once `n = 2` is confirmed:

Report to the user:
1. Where the repo is cloned (`/Users/shakedaskayo/shaked/projects/kyma`).
2. Which services are running and on which ports (Postgres `:5433`, MinIO `:9000` / console `:9001`, Redpanda `:9092` / admin `:9644`, kyma HTTP `:8080`, Flight gRPC `:9090`).
3. How to stop everything cleanly: `Ctrl-C` the foreground `cargo run`, then `docker compose down` from `$KYMA`.
4. How to restart later: the env-var block from Task 5.1 plus `cargo run -p kyma-bin`.

---

## Self-Review

**Spec coverage:**
- §4.1 Clone — handled implicitly by Task 1.1 (clone happened during brainstorming; verification confirms it's intact). ✓
- §4.2 Rust toolchain — Task 2. ✓
- §4.3 Bring up deps — Task 3 (with all three healthchecks + bucket verification). ✓
- §4.4 Run kyma-bin — Task 5 (with localhost-mapped env vars called out explicitly). ✓
- §4.5 Smoke test — Tasks 6, 7, 8. ✓
- §5 Risks — each mapped to a stop-and-ask in the relevant task: ports (Task 1.3), build time (Task 4.1 note), cargo not on PATH (Task 2.2 fallback), bucket race (Task 3.3), env-var trap (Task 5.1 explicit warning). ✓
- §7 Done definition — Task 8.1 gate. ✓

**Placeholder scan:** No `TBD`/`TODO`/"implement later"/"handle edge cases" in the plan. Every step has the exact command. ✓

**Type/identifier consistency:** Container names, env var names, port numbers, and bucket names are consistent across Tasks 3, 5, 7, 8 (`kyma-postgres`/`kyma-minio`/`kyma-redpanda`; `KYMA_CATALOG_URL` etc.; `5433`/`9000`/`8080`/`9090`; `kyma`/`kyma-ingest`/`kyma-archive`). ✓

No issues to fix.
