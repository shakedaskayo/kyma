---
description: Ingest into Pensieve on demand — pull from a data source, or create virtual graph entities wired to memory and existing resources.
argument-hint: [data source to pull from | resources & relationships to add]
allowed-tools: Bash(pensieve datasource:*), Bash(pensieve ingest:*), Bash(pensieve status:*)
---

Ingest into Pensieve's context engine. Request: **$ARGUMENTS**

Data sources configured on this server (name · type · enabled · last run):

!`pensieve datasource list 2>/dev/null || echo "no pensieve connection — run: pensieve connect <url>"`

Recent ingestion runs:

!`pensieve ingest status 2>/dev/null || true`

Pick the mode that fits the request.

## Mode A — pull from a data source (data ingestion)

Use when `$ARGUMENTS` names or implies a source (github / gitlab / bitbucket /
prometheus / s3 / postgres / notion / slack / jira / confluence / gmail / gdrive), or is
**empty** — then summarize the data sources listed above and ask which to pull from.

1. Pick the matching data source from the list. If it isn't configured yet, add it with
   `pensieve datasource add <source> …` (run `pensieve datasource add --help` for the source's flags) —
   adding triggers a first run.
2. Trigger an on-demand run now: `pensieve datasource trigger <name-or-id>`.
3. Confirm it ran: `pensieve ingest status --datasource <name-or-id>` (or
   `pensieve ingest tail --datasource <name-or-id>` to follow).
4. Report what was pulled and point the user at the graph view (Graph page, select the
   data source's database) or `/pensieve-recall`. **Continuous** background ingestion is a server
   feature — the scheduler ticks data sources on their interval; this command is on-demand.

## Mode B — dynamic entities (enrich the graph)

Use when `$ARGUMENTS` describes resources/relationships, or to capture the structure you've
learned this session — services, repos, tables, people, configs, concepts, and how they
relate — as **virtual resources** on the graph.

1. **Find what to wire to first** (don't duplicate existing nodes):
   - `recall_memory` / `memory_search` (MCP server `pensieve`) for related memories;
   - `find_references_to` / `graph_traverse` for existing graph node ids (e.g. the GitHub
     `repo:owner/name`, a service or table node).
2. For each resource, call **`ingest_entity`** (MCP server `pensieve`) with:
   - `name`, `kind` (`service|repo|table|person|file|config|concept`), optional `properties`
     (e.g. `{"language":"rust","owner":"team-pay"}`);
   - `links` — wire it to what you found:
     - a catalog/data-source resource:
       `{ "target_node_id": "repo:owner/name", "target_namespace": "github", "relationship_type": "LIVES_IN" }`
     - a memory:
       `{ "target_node_id": "memory:<uuid>", "relationship_type": "DOCUMENTED_BY" }`
     - another entity you just created:
       `{ "target_node_id": "memory:<uuid>", "relationship_type": "OWNS" }` (also `DEPENDS_ON`, `PART_OF`, …).
   `ingest_entity` is **idempotent** per `(realm, kind, name)` — re-running updates the entity
   in place instead of duplicating.
3. Report the entities + edges created and how they connect to existing resources/memory.

Prefer linking to real resources over re-describing them; keep entities durable and meaningful
(skip transient detail). Use the project realm (working-directory basename) unless told
otherwise.
