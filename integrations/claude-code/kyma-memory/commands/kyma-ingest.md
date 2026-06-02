---
description: Ingest into Kyma on demand — pull from a connector, or create virtual graph entities wired to memory and existing resources.
argument-hint: [connector/source to pull from | resources & relationships to add]
allowed-tools: Bash(kyma connector:*), Bash(kyma ingest:*), Bash(kyma status:*)
---

Ingest into Kyma's context engine. Request: **$ARGUMENTS**

Connectors configured on this server (name · type · enabled · last run):

!`kyma connector list 2>/dev/null || echo "no kyma connection — run: kyma connect <url>"`

Recent ingestion runs:

!`kyma ingest status 2>/dev/null || true`

Pick the mode that fits the request.

## Mode A — pull from a connector (data ingestion)

Use when `$ARGUMENTS` names or implies a source (github / gitlab / bitbucket /
prometheus / s3 / postgres / notion / slack / jira / confluence / gmail / gdrive), or is
**empty** — then summarize the connectors listed above and ask which to pull from.

1. Pick the matching connector from the list. If it isn't configured yet, add it with
   `kyma connector add <source> …` (run `kyma connector add --help` for the source's flags) —
   adding triggers a first run.
2. Trigger an on-demand run now: `kyma connector trigger <name-or-id>`.
3. Confirm it ran: `kyma ingest status --connector <name-or-id>` (or
   `kyma ingest tail --connector <name-or-id>` to follow).
4. Report what was pulled and point the user at the graph view (Graph page, select the
   connector's database) or `/kyma-recall`. **Continuous** background ingestion is a server
   feature — the scheduler ticks connectors on their interval; this command is on-demand.

## Mode B — dynamic entities (enrich the graph)

Use when `$ARGUMENTS` describes resources/relationships, or to capture the structure you've
learned this session — services, repos, tables, people, configs, concepts, and how they
relate — as **virtual resources** on the graph.

1. **Find what to wire to first** (don't duplicate existing nodes):
   - `recall_memory` / `memory_search` (MCP server `kyma`) for related memories;
   - `find_references_to` / `graph_traverse` for existing graph node ids (e.g. the GitHub
     `repo:owner/name`, a service or table node).
2. For each resource, call **`ingest_entity`** (MCP server `kyma`) with:
   - `name`, `kind` (`service|repo|table|person|file|config|concept`), optional `properties`
     (e.g. `{"language":"rust","owner":"team-pay"}`);
   - `links` — wire it to what you found:
     - a catalog/connector resource:
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
