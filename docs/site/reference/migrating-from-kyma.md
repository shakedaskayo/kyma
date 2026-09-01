# Migrating from kyma

Pensieve was called **kyma** through `0.0.22`. The rename landed in `0.1.0` as a
clean break: nothing reads the old names, and there is no compatibility shim to
turn off later. This page is the only one that still uses the old name.

If you are installing fresh, you can ignore all of this.

## The short version

```bash
# 1. remove the old install (keeps ~/.kyma so you can copy anything out)
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash -s -- --uninstall

# 2. install pensieve
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash

# 3. re-wire your coding agent (rewrites the MCP entry)
pensieve setup claude-code

# 4. once you are happy, drop the old data
rm -rf ~/.kyma
```

Ingested data does not carry over — see [Stored data](#stored-data) for why.

## Command and package names

| Was | Now |
| --- | --- |
| `kyma` (binary) | `pensieve` |
| `kyma-cli` (crate + binary) | `pensieve-cli` |
| `kyma-*` crates (19 published) | `pensieve-*` |
| `@kyma-ai/client`, `@kyma-ai/react` | `@pensieve-ai/client`, `@pensieve-ai/react` |
| `kyma-engine` Helm chart / image | `pensieve-engine` |
| `/kyma-recall`, `/kyma-remember`, … | `/pensieve-recall`, `/pensieve-remember`, … |

The old crates and npm packages stay published — crates.io has no rename — but
they are deprecated and will not get further releases.

## Environment variables

Every `KYMA_*` variable is now `PENSIEVE_*`, with no fallback: the old name is
simply not read. There are 236 of them, and the rename is purely the prefix, so
a mechanical pass over your own config works:

```bash
sed -i '' 's/\bKYMA_/PENSIEVE_/g' .env        # and your CI, compose, Helm values
```

The full list lives in [Environment variables](/reference/env).

## Paths and services

| Was | Now |
| --- | --- |
| `~/.kyma` | `~/.pensieve` |
| `KYMA_HOME` | `PENSIEVE_HOME` |
| `~/.claude/skills/kyma-memory` | `~/.claude/skills/pensieve-memory` |
| `dev.getkyma.kyma-server` (launchd) | `dev.getpensieve.pensieve-server` |
| `dev.getkyma.kyma-sync` (launchd) | `dev.getpensieve.pensieve-sync` |
| `kyma-server.service` (systemd) | `pensieve-server.service` |

Run the old installer's `--uninstall` **before** installing pensieve so the old
service units are removed; otherwise they linger and restart a binary that is
no longer there.

One thing the uninstaller cannot fix retroactively: your shell rc file has a
PATH block marked `# kyma`. The new installer writes and removes a `# pensieve`
block, so delete the old two-line stanza by hand.

## Stored data

**Ingested data does not carry over.** Extents are written with a 4-byte magic
header that changed with the rename:

| Was | Now |
| --- | --- |
| `KYMA\x01` / `KYMA\x02` | `PNSV\x01` / `PNSV\x02` |
| `extents/<id>.kyma` | `extents/<id>.pnsv` |
| object-store prefix `kyma/` | `pensieve/` |
| FTS tokenizer `kyma-word-v1` | `pensieve-word-v1` |
| promoted memory files `kyma-*.md` | `pensieve-*.md` |

The reader validates magic, so old extents fail with a clear format error
rather than being silently misread. Re-ingest from source.

If you are running against Postgres, migration `035` moves the stored
`source = 'kyma'` marker on `agent_sessions` to `'pensieve'` and updates the
CHECK constraint. Migrations `001`–`034` are unchanged, so an existing database
passes sqlx's checksum validation and upgrades in place.

## Things that break silently

These change with no error — worth checking before you assume the upgrade was
clean.

**Prometheus metrics.** All 75 `kyma_*` metric names are now `pensieve_*`
(`kyma_query_*`, `kyma_queue_*`, `kyma_scan_*`, `kyma_ingest_*`, …). Every
dashboard panel and alert rule referencing them goes blank rather than erroring.

**JWT claims.** If you authenticate through OIDC, the claims your IdP mints are
now `pensieve_role`, `pensieve_databases` and `pensieve_realms`. Rather than
changing your IdP, you can point pensieve back at the old claim names:

```bash
PENSIEVE_OIDC_ROLE_CLAIM=kyma_role
PENSIEVE_OIDC_DATABASES_CLAIM=kyma_databases
PENSIEVE_OIDC_REALMS_CLAIM=kyma_realms
```

**MCP registration.** The server key is now `pensieve`, so tools arrive as
`mcp__pensieve__*`. Any agent config still naming `kyma` points at a binary that
no longer exists — re-run `pensieve setup <agent>`.

**HTTP headers.** `x-kyma-max-wall-clock-ms` and `x-kyma-max-memory-bytes` are
now `x-pensieve-*`. The old headers are ignored, so per-query limits silently
fall back to the server defaults.

**Browser state.** The web UI's stored preferences moved from `kyma:ui` /
`kyma:theme` to `pensieve:*`, so your theme and layout reset once.

## Embedding the React SDK

`@pensieve-ai/react` is a breaking release. Component and hook names follow the
product:

```diff
-import { KymaProvider, KymaGraph, useKymaQuery } from "@kyma-ai/react";
+import { PensieveProvider, PensieveGraph, usePensieveQuery } from "@pensieve-ai/react";
```

Two things beyond the names:

- The Tailwind utility prefix is `pv-` instead of `ky-`, and the scope
  sentinels are `.pensieve-root` / `.pensieve-dark`.
- Theme tokens are `--pensieve-*` instead of `--kyma-*`. If you override them,
  see [Theming](/embed/theming).

## Cloud infrastructure

Deployments provisioned by `kyma deploy` keep their existing S3 buckets, RDS
instances and IAM roles — those names are yours, and renaming a bucket means
copying its contents. New deployments default to `pensieve`-prefixed names.
Set `project_name` explicitly if you want to keep the old ones.
