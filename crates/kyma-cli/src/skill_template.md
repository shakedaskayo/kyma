---
name: kyma
description: The user's Kyma context engine — durable memory + live data + a knowledge graph — via the `kyma` CLI. RECALL prior decisions/preferences/conventions before answering, REMEMBER durable facts the user states, QUERY their live data (logs, traces, tables, code/graph) in plain English, and ENRICH the graph with entities. Use whenever a request may depend on past context or on the user's actual systems/data.
---

# Kyma — the user's context engine

This machine is wired to **Kyma**: the user's durable memory **+** live data **+** the
knowledge graph that links them, reachable through the `kyma` CLI. Treat it as your
long-term memory of this user and project, *and* your window into their real systems.

**Setup check (once per machine):** run `kyma status`. If it reports "no server
configured", ask the user to run `kyma connect <their-server-url>`, then continue.

## Recall — before you answer

Whenever a request could depend on prior context — the user's preferences, past
decisions, project conventions, "how we did X last time", or anything about their
systems — **recall first** and ground your answer in what comes back:

```bash
kyma recall "how do we handle database migrations?"
```

It returns ranked, citation-rich context (memories + the resources they link to). Don't
invent memories or claim to remember something `recall` didn't return.

## Remember — after the user states something durable

When the user makes a decision, states a preference, or teaches a non-obvious,
load-bearing fact worth keeping across sessions, save it:

```bash
kyma remember "We deploy by tagging a release, then running make ship" --type procedure
kyma remember "Prefer KQL over SQL in examples" --type preference --realm global
```

Types: `fact | decision | preference | learning | procedure`. Use `--topic-key <stable/key>`
for things that get revised — re-saving with the same key **updates in place** instead of
duplicating. Save the durable signal, never transient scratch, secrets, or tokens.

## Query — the user's live data

Ask their Kyma deployment anything about their data, in plain English; it streams the answer:

```bash
kyma query "How many 500s on payments-svc in the last hour?"
kyma query "What columns does the orders table have?"
kyma query "Which services call auth-service?"
```

This runs live queries over the user's *actual* data — far better than guessing from
context. `kyma query` is **read-only**. Add `--json` for the raw event stream.

## Enrich — add entities to the graph

When you learn how the user's resources relate, record it as a graph entity wired to real
nodes and memories (idempotent — re-running updates in place):

```bash
kyma entity "payments service" --kind service --prop owner=team-pay \
  --link "repo:owner/name|github|LIVES_IN" --link "memory:<uuid>||DOCUMENTED_BY"
```

## When NOT to use

- General programming or documentation questions — answer those directly.
- Pure file edits in the repo — use your normal file tools.
- Anything requiring a write to the user's systems — `kyma query` only reads.

## Troubleshooting

- `no server configured` → the user hasn't run `kyma connect <url>` yet.
- `401` / `403` → the saved token expired: `kyma connect <url> --token <new-token>`.
- Probe timeouts → the server is down or a model is loading; retry once `kyma status` is OK.
