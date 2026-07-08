# Brain repos

Publish kyma's memory as a Git repository in Obsidian-vault format. Anyone (or any agent) with a kyma token can clone it, grep it, open it in Obsidian, and push edits back into memory.

```bash
git clone http://localhost:7777/git/team.git
# username: kyma (anything) · password: your kyma API token
rg "auth model" team/notes/
```

Create one:

```bash
kyma brain create team --realm kyma            # one realm, flat layout
kyma brain create org --all-realms --gardener  # every realm + agentic wiki curation
```

or in the web UI under **Intelligence → Brains**, or via the API:

```bash
curl -X POST http://localhost:7777/v1/brain \
  -H "Authorization: Bearer $KYMA_TOKEN" \
  -d '{ "name": "team", "realms": ["kyma"], "export_interval_secs": 900 }'
```

## What's in the vault

```
README.md              clone/pull/push quickstart (generated)
index.md               global index: counts, freshness, start-here links
CONTRIBUTING.md        the editing contract (generated)
notes/<type>/          one markdown note per memory: facts/ decisions/
                       preferences/ learnings/ procedures/ summaries/
entities/              entity memories with graph-edge wikilinks
wiki/                  gardener-curated overview pages (co-owned, editable)
inbox/                 drop zone for new notes you push
.obsidian/             seeded once (graph colors per type) — yours to tweak
.kyma/manifest.json    exporter-owned paths (don't edit)
```

Multi-realm brains add one level: `notes/<realm>/<type>/…`. Every note has flat, Obsidian-native frontmatter:

```markdown
---
title: Auth model uses stateless JWT
kyma_memory_id: 9f3a2b1c-…
type: decision
realm: kyma
tags: [auth, architecture]
importance: 0.80
status: active
created: 2026-05-14T09:12:00Z
updated: 2026-07-01T18:40:00Z
aliases: [Auth model uses stateless JWT]
content_hash: 4f2a91c8d3e5b7a1
---

# Auth model uses stateless JWT

Sessions are stateless JWTs signed with the server keypair.

<!-- kyma:related:begin -->
## Related
- [[notes/decisions/refresh-tokens-rotate-daily-1b2c3d4e|Refresh tokens rotate daily]] — RELATES_TO
<!-- kyma:related:end -->
```

Filenames are `<title-slug>-<memory-id-prefix>.md`, minted once and never renamed — the `kyma_memory_id` in frontmatter is the identity, not the path. Git history is the changelog and the archive: memories that get archived or filtered out disappear from the tree (recover with `git log --diff-filter=D`).

## Push edits back

The brain is bidirectional. After a `git push`, the server ingests the diff while holding the brain's lock:

- **Edit a note body** (or its `tags`/`importance`) → the same memory gets a new version, keyed by `kyma_memory_id`. Edges, topic key, and history are preserved.
- **Add a file** (drop it in `inbox/`, frontmatter optional) → a new memory with topic key `brain:<name>:<path>`; the next export re-files it canonically under `notes/`.
- **Delete a note** → the memory is archived (never destroyed) and the file stays gone.
- Non-markdown files (images, personal notes) ride along in git untouched; exports preserve them.
- Force-pushes are rejected (`receive.denyNonFastForwards`). Frontmatter parse errors never fail a push — they surface as run warnings.

Pushed files are normalized (canonical frontmatter/formatting) by the next export. Don't hand-edit `index.md`, `.kyma/`, `kyma_memory_id` values, or `<!-- kyma:related -->` blocks — they are regenerated.

## Exports and the gardener

A deterministic exporter re-renders the vault from memory on each brain's `export_interval_secs` (default 15 min; `0` = manual). Identical memory state produces a byte-identical tree, so quiet periods create no commits. Trigger manually:

```bash
kyma brain export team          # POST /v1/brain/team/export
kyma brain runs team            # export + push-ingest history
```

The **wiki gardener** (opt-in per brain) is a [dreaming](/agent/dreaming) run focused on the brain: it curates `wiki/` overview pages — updating pages in place via `wiki:<name>:<slug>` topic keys rather than appending near-duplicates. The gardener writes memories; only the exporter writes git. Kick it manually with `kyma brain garden team`; watch it under **Memory → Dreaming**.

## Auth

Git speaks HTTP Basic: the kyma API token rides as the password (username is ignored). Clone requires the brain's `visibility_role` (default `read`); push requires `write`. With auth disabled (default local mode) no credentials are needed.

`kyma brain clone team` wires a repo-local credential helper (`kyma git-credential`) so the token never appears in URLs, `.git/config`, or shell history. For CI, `kyma brain url team --with-token` prints a credential-embedded URL.

## CLI

| Command | Does |
|---|---|
| `kyma brain list` | brains, note counts, last export, head commit |
| `kyma brain create <name> --realm <r>… [--all-realms] [--schedule 15m\|1h\|daily\|manual] [--gardener]` | publish + first export |
| `kyma brain show <name>` | config, stats, clone command |
| `kyma brain export <name>` / `garden <name>` / `runs <name>` | trigger + history |
| `kyma brain clone <name> [dir]` | clone with the credential helper |
| `kyma brain url <name> [--with-token]` | print the clone URL |
| `kyma brain delete <name> [--yes]` | remove repo + registry entry (memories untouched) |

## Importing an existing vault (the other direction)

Brains *publish* memory as a vault. To go the other way — pull an existing
Obsidian vault (including a team's shared git repo) *into* memory — add an
**Obsidian data source**. It takes either a local folder or a git URL:

```bash
kyma datasource add   # → pick Obsidian, or via the web UI: Data Sources → Add
# fields: git_url = https://github.com/org/team-vault.git
#         git_token (private repos), branch, vault name (→ realm)
```

kyma clones the repo into `${KYMA_HOME}/vaults/<id>`, ingests every note
(wikilinks become graph edges, deleted notes archive), and pulls it again on
the source's schedule so the memory stays fresh. Local-folder vaults are
watched live by the file watcher instead. This is the inverse of a brain: the
imported notes become recallable memory, which a brain can then re-publish.

## Operations

- Repos live at `${KYMA_HOME}/brain/<name>.git` (local mode) or `KYMA_BRAIN_DIR` (hosted, default `/var/lib/kyma/brain` — mount a persistent volume).
- The server shells out to the `git` binary. Without it the rest of kyma is unaffected: `/v1/brain` reports `git_available: false` and repo endpoints answer 503.
- Hosted multi-replica: exactly one replica should mount the brain volume and advertise the `brain` worker capability; `brain_export` fabric jobs run there.
- Filters per brain (API/`PUT /v1/brain/<name>`): memory types, statuses (default `active` only), minimum importance. `space != public` memories are always excluded, `<private>…</private>` spans are stripped, and embeddings never leave the store.
