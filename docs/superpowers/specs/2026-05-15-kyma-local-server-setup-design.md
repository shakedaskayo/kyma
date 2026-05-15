# kyma Local Server Setup — Design

**Status:** draft for review
**Date:** 2026-05-15
**Owner:** shaked (local dev environment)
**Scope:** single-shot procedure to bring a working kyma server up on a developer Mac, starting from a freshly cloned repo

---

## 1. Summary

Goal: stand up a working kyma server on `shakedaskayo`'s Mac, from a freshly cloned repo, ending at the point where the README's quick-start ingest-then-query path returns the expected row. No production hosting, no Kubernetes, no remote infra. This is the canonical local dev loop the README already documents, made explicit so the steps, env-var mapping, and "done" gate are unambiguous.

The non-trivial choice is **where the kyma binary runs**: natively via `cargo run` against docker-compose dependencies (the README's recommended path), or fully inside docker-compose (no host Rust toolchain). This spec picks the hybrid approach — docker for deps, native cargo for `kyma-bin` — because it produces the fastest subsequent iteration loop and the host already has `rustup` available.

The non-obvious correctness risk is that the compose file's `kyma` service uses **container-internal hostnames** (`postgres`, `minio`) for its env vars, while a native cargo run needs **`localhost` plus the published host port** (5433 for Postgres, 9000 for MinIO). Mis-copying those env vars is the most likely way for the engine to start up but fail to talk to its catalog / object store. Section 4 spells out the localhost-mapped values.

---

## 2. Goals & non-goals

### Goals

- Clone `shakedaskayo/kyma` to `~/shaked/projects/kyma` (sibling of existing `a+e_networks` checkout).
- Install the pinned Rust toolchain via `rustup` reading `rust-toolchain.toml`.
- Bring up Postgres + MinIO + MinIO bucket init + Redpanda from the repo's `docker-compose.yml`.
- Build and run `kyma-bin` natively, listening on HTTP `:8080` and Flight gRPC `:9090`.
- Execute the README quick-start: ingest two NDJSON log rows, run a KQL `summarize count by error_code`, confirm `CARD_DECLINED = 2`.
- Document the exact env vars used, so a future restart needs no guessing.

### Non-goals

- Production deployment (cloud, Kubernetes, multi-node).
- OTLP gRPC ingest (`KYMA_OTLP_ADDR=off`); REST ingest is enough for the smoke test.
- Auth tokens (`KYMA_AUTH_TOKENS=""`); local dev is unauthenticated.
- Running `scripts/e2e-test.sh` or the broader chaos / load test suite.
- Custom telemetry data — the smoke test uses the README's two synthetic log rows verbatim.
- Persistent systemd / launchd service. The engine is expected to run in a terminal foreground process during dev.

---

## 3. Approach (chosen) and alternative considered

**Chosen — hybrid:** `docker compose up -d postgres minio minio-init redpanda`, then `cargo run --release -p kyma-bin` on the host with env vars pointing at `localhost:5433` (Postgres) and `localhost:9000` (MinIO).

**Alternative — all-in-docker:** `docker compose up` runs the `kyma` service too (compose builds the image from the in-repo `Dockerfile`).

| Dimension              | Hybrid (chosen)                                           | All-in-docker                                  |
|------------------------|-----------------------------------------------------------|------------------------------------------------|
| Host requirement       | rustup + stable toolchain                                 | Docker only                                    |
| First build time       | ~10–20 min (host `cargo build`)                            | ~10–20 min (in-container build) + image layers |
| Incremental rebuild    | Fast (host cargo cache, incremental linking)              | Slow (full image rebuild per change)           |
| Debugging              | Native debugger, host logs, host profiler                 | Docker logs only, exec into container          |
| Closer to README       | Yes — README quick-start runs `cargo run`                 | No — README treats this as an alternative      |

Recommendation stands: hybrid. The Rust toolchain is a one-time install via the existing `rustup` and unlocks the standard kyma dev loop. All-in-docker is the right pick on a host that genuinely cannot install a Rust toolchain — not the case here.

---

## 4. Procedure

### 4.1 Clone

```bash
gh repo clone shakedaskayo/kyma ~/shaked/projects/kyma
cd ~/shaked/projects/kyma
```

(Already done during brainstorming, in order to land this spec inside the repo.)

### 4.2 Rust toolchain

```bash
# from inside the repo, so rust-toolchain.toml is read
rustup show
```

`rust-toolchain.toml` pins `channel = "stable"` with `rustfmt clippy rust-src` and `profile = default`. `rustup show` triggers an install if the channel isn't present.

Verification gate: `cargo --version` resolves after this step.

### 4.3 Bring up dependencies

```bash
docker compose up -d postgres minio minio-init redpanda
```

- We deliberately **omit the `kyma` service** from this `up` — we'll run it natively.
- `minio-init` is a one-shot container that creates buckets `kyma`, `kyma-ingest`, `kyma-archive`, then exits.
- Healthchecks: Postgres `pg_isready`, MinIO `mc ready local`, Redpanda `rpk cluster info`.

Verification gate: `docker compose ps` shows postgres/minio/redpanda healthy, minio-init exited 0.

Pre-flight: ports `5433`, `9000`, `9001`, `9092`, `9644` must be free. `lsof -i :5433` etc. before bringing up.

### 4.4 Run kyma-bin

Env vars (localhost-mapped — this is the spot where mis-copying the compose file would silently break the catalog connection):

```bash
export KYMA_CATALOG_URL=postgres://kyma:kyma_dev@localhost:5433/kyma
export KYMA_S3_ENDPOINT=http://localhost:9000
export KYMA_S3_BUCKET=kyma
export KYMA_S3_REGION=us-east-1
export KYMA_S3_ACCESS_KEY_ID=kyma_admin
export KYMA_S3_SECRET_ACCESS_KEY=kyma_admin_dev
export KYMA_S3_PATH_STYLE=true
export KYMA_S3_ALLOW_HTTP=true
export KYMA_HTTP_ADDR=0.0.0.0:8080
export KYMA_GRPC_ADDR=0.0.0.0:9090
export KYMA_OTLP_ADDR=off
export KYMA_OTLP_DATABASE=default
export KYMA_PATH_PREFIX=kyma
export KYMA_AUTH_TOKENS=""

# Debug build first for a faster first-run; release later if desired.
cargo run -p kyma-bin
```

Run in a foreground terminal so logs are visible.

Verification gate: server logs report HTTP listener on `:8080` and Flight on `:9090` without panic.

### 4.5 Smoke test (the "done" gate)

```bash
# Health
curl -fsS http://localhost:8080/health

# Ingest the README's two log rows
curl -fsS -X POST http://localhost:8080/v1/ingest \
  -H "X-Database: obs" \
  -H "X-Table: otel_logs" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @- <<'EOF'
{"timestamp":"2026-04-20T14:23:01Z","service.name":"payments-svc","severity_text":"ERROR","message":"card declined","attributes":{"error.code":"CARD_DECLINED"}}
{"timestamp":"2026-04-20T14:23:04Z","service.name":"payments-svc","severity_text":"ERROR","message":"card declined","attributes":{"error.code":"CARD_DECLINED"}}
EOF

# KQL query
curl -fsS -X POST http://localhost:8080/v1/query \
  -H "X-Database: obs" \
  -H "Content-Type: application/x-kql" \
  --data-binary 'otel_logs | where severity_text == "ERROR" | summarize n = count() by error_code = tostring(attributes["error.code"])'
```

Verification gate (this is what "done" means for this spec):

- `/health` returns OK (HTTP 2xx, non-error body).
- Ingest POST returns 2xx.
- Query POST returns a result row where `error_code == "CARD_DECLINED"` and `n == 2`.

If any of these fail, the engine isn't actually set up — claim failure, not success, regardless of how far the prior steps got.

---

## 5. Risks & mitigations

| Risk                                                                            | Mitigation                                                                                                   |
|---------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------|
| First `cargo build` takes 10–20 minutes; user thinks it's hung                   | Stream logs, surface progress; do **debug** build first (faster) and only switch to `--release` if desired   |
| Port collision on 5433 / 9000 / 9001 / 9092                                     | Pre-flight `lsof -i :<port>` checks; stop and ask user before clobbering anything                            |
| `gh repo clone` auth fails                                                      | Already authenticated as `shakedaskayo` (verified during brainstorming); stop and ask if this regresses     |
| Compose-file env vars accidentally copied verbatim (`postgres`/`minio` hostnames)| §4.4 lists the **localhost-mapped** versions explicitly; that section is the source of truth, not the compose file |
| `cargo` not on PATH after `rustup show` (no default toolchain set)              | Run `rustup default stable` if needed; verify `cargo --version` before attempting build                      |
| MinIO buckets not created (minio-init race or failure)                          | Check `docker compose logs minio-init` for `kyma buckets ready`; recreate manually with `mc mb` if needed    |
| Build fails for an unrelated reason (missing system lib, etc.)                  | Stop, paste error, ask user before trying random workarounds                                                 |

---

## 6. Out of scope (explicit, so they don't sneak in during execution)

- Editing any kyma source code.
- Adding launchd / systemd unit files.
- Touching `~/.kyma/`, dotfiles, or any global config.
- Enabling OTLP, Kafka ingest, or file-drop ingest.
- Creating extra databases / tables beyond what the smoke test implies.
- Performance tuning, retention policies, compaction tweaks.
- Configuring the kyma CLI (`kyma-cli`) — not needed for the smoke test.

---

## 7. Done definition (single sentence)

The KQL summarize query in §4.5 returns a row with `error_code = "CARD_DECLINED"` and `n = 2`, with the kyma server running natively on the host against docker-compose-managed Postgres and MinIO.
