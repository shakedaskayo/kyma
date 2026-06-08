---
title: Local & cluster mode
description: The same context engine runs three ways — a local single binary with zero infra, a team control plane on Postgres + object storage, and a read-scale cluster of stateless nodes over shared storage. What runs where, and when to use each.
---

# Local & cluster mode

The same engine and the same data model run from one laptop binary to a multi-node
cluster. You don't pick an architecture up front — you grow along it, and memory stays
coherent across the spectrum via [sync](/concepts/sync).

## The three ways to run

| | **Local mode** | **Control plane** | **Cluster** |
|---|---|---|---|
| Command | `kyma` (CLI / `mcp` / `serve`) | `kyma server` (Docker) | `kyma server`, N nodes |
| Catalog | embedded **SQLite** | **Postgres** | Postgres (shared) |
| Storage | local files (`~/.kyma`) | object store (S3 / MinIO) | object store (shared) |
| Infra | none | Postgres + object store | + a load balancer |
| Best for | one developer, offline, instant | a team, always-on | read scale-out, HA |
| Extras | on-demand ingest + query, local web UI | scheduled connectors, [dreaming](/agent/dreaming), full web app | everything, horizontally |

### Local mode — `kyma`

One binary per developer. No Postgres, no Docker, no sudo — the catalog is embedded
SQLite and data lives under `~/.kyma`. It gives your agent the full toolset over stdio
MCP (`kyma mcp`), an optional local web UI (`kyma serve`), and on-demand ingest + query.
This is where [Give your agent memory](/quickstart/give-your-agent-memory) starts.

### Control plane — `kyma server`

The team tier: Postgres for the catalog, an object store (S3 or MinIO) for extents.
On top of local mode it adds the things that need to be always-on and shared — scheduled
[connectors](/connectors/), background [dreaming](/agent/dreaming), and the full web app
(Graph Explorer · Memory · Discover · Agent). Stand it up with `docker compose up -d`
([Five-minute start](/quickstart/five-minute-start)) or [deploy it to production](/deploy/).

### Cluster — many nodes, one source of truth

The control plane scales out without a rewrite, because of [the five
invariants](/concepts/the-five-invariants): object storage is the only source of truth and
compute is stateless. Run N `kyma server` nodes against the same Postgres catalog and the
same object store, behind a load balancer. Any node can answer any query; ingest commits
serialize through catalog CAS. Read throughput scales with node count.

## How to choose

- **Just want your agent to remember?** Local mode. Nothing to provision.
- **A team sharing memory + live connectors?** Control plane.
- **High query volume or HA?** A cluster of control-plane nodes.

You're never locked in: start local, [sync](/concepts/sync) to a control plane when your
team needs it, add nodes when load demands it. Same binary, same data.

## Read next

- Keep machines coherent: [Sync](/concepts/sync).
- The whole picture: [How kyma works](/concepts/how-kyma-works).
- Why stateless compute scales: [The five invariants](/concepts/the-five-invariants).
- Run the control plane in production: [Deploy](/deploy/).
