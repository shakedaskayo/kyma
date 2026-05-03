---
title: Ingest something custom
description: Take an arbitrary JSON event source — a webhook, a cron output, your own metric — into kyma in a way that prunes well from day one.
---

# Ingest something custom

## The problem

You have a JSON event source that isn't OTLP and isn't already in
Postgres. Maybe a webhook from GitHub Actions, a cron job dumping its
status, your own application metric, a Stripe event. You want it in
kyma so you can query it alongside the rest of your telemetry — but
done badly, custom ingest produces tables that prune poorly and need
re-ingesting six months later.

This recipe gets you to a healthy table on the first commit.

## The schema

You're creating a table from scratch. Think about what queries you'll
run before defining columns. The default auto-create path
([REST / NDJSON](/ingest/rest-ndjson)) will infer types, but inference
is conservative — explicit beats implicit when you know what you want.

For this recipe, the data is GitHub Actions workflow runs. The shape
you'd query: "which workflows are failing on which repos in the last
hour?" That implies four typed columns and one `dynamic` overflow:

| Column         | Type        | Why                                                            |
| -------------- | ----------- | -------------------------------------------------------------- |
| `_timestamp`   | `timestamp` | Time bound on every query. Without this, no pruning at stage 1. |
| `repo`         | `string`    | Equality predicate; token-indexed.                             |
| `workflow`     | `string`    | Equality predicate.                                            |
| `conclusion`   | `string`    | `success`, `failure`, `cancelled`. Equality / `in` predicate.  |
| `attributes`   | `dynamic`   | Branch, actor, run URL, head SHA — useful but not predicates.  |

Provision it:

```bash
kyma-cli create-table \
  --db default \
  --name github_runs \
  --schema "_timestamp:timestamp,repo:string,workflow:string,conclusion:string,attributes:dynamic"
```

The deliberate choices:

- **`_timestamp` is mandatory.** Every query you write will bound on it.
- **String columns for predicates only.** `repo`, `workflow`,
  `conclusion` — anything you'd put in a `WHERE` clause. The string-
  column token index makes equality and `contains` cheap.
- **Everything else in `dynamic`.** Branch names, run URLs, actor
  handles. You can read them in `project`; you usually won't filter on
  them. If you start filtering on a path frequently, [promote it to a
  typed column](/concepts/dynamic-and-vectors).

## Sending the data

A GitHub workflow can `POST` directly to your kyma at the end of a
job. Replace `$KYMA_URL` and `$KYMA_TOKEN` with your environment.

```yaml
# .github/workflows/report-to-kyma.yml
name: Report to kyma

on:
  workflow_run:
    workflows: ["*"]
    types: [completed]

jobs:
  report:
    runs-on: ubuntu-latest
    steps:
      - name: POST run record
        env:
          KYMA_URL:   ${{ secrets.KYMA_URL }}
          KYMA_TOKEN: ${{ secrets.KYMA_TOKEN }}
        run: |
          curl -sS -X POST "$KYMA_URL/v1/ingest" \
            -H "Authorization: Bearer $KYMA_TOKEN" \
            -H "X-Database: default" \
            -H "X-Table: github_runs" \
            -H "X-Idempotency-Key: gh-run-${{ github.event.workflow_run.id }}" \
            -H "Content-Type: application/x-ndjson" \
            -d "$(jq -cn \
              --arg ts "${{ github.event.workflow_run.updated_at }}" \
              --arg repo "${{ github.repository }}" \
              --arg wf "${{ github.event.workflow_run.name }}" \
              --arg c "${{ github.event.workflow_run.conclusion }}" \
              --arg branch "${{ github.event.workflow_run.head_branch }}" \
              --arg actor "${{ github.event.workflow_run.actor.login }}" \
              --arg url "${{ github.event.workflow_run.html_url }}" \
              --arg sha "${{ github.event.workflow_run.head_sha }}" \
              '{
                _timestamp: $ts,
                repo: $repo,
                workflow: $wf,
                conclusion: $c,
                attributes: { branch: $branch, actor: $actor, url: $url, head_sha: $sha }
              }')"
```

Two things to notice:

- `X-Idempotency-Key: gh-run-<id>`. GitHub fires `workflow_run.completed`
  reliably-but-not-uniquely; the workflow_run id is stable, so a replay
  is a no-op at kyma's catalog boundary. See
  [Idempotency and coercion](/ingest/idempotency-and-coercion).
- The non-predicate fields nest under `attributes`. They're real data;
  they're just not what you filter on, so they belong in `dynamic`.

## Querying it

Latest failures across all repos:

```kql
github_runs
| where _timestamp > ago(1h)
| where conclusion == "failure"
| project _timestamp, repo, workflow, attributes["branch"], attributes["url"]
| order by _timestamp desc
| take 20
```

Failure rate by workflow over the last day:

```kql
github_runs
| where _timestamp > ago(24h)
| summarize
    total = count(),
    failed = countif(conclusion == "failure")
  by workflow
| extend rate = todouble(failed) / todouble(total)
| order by rate desc
| take 10
```

(`countif` may not exist in your KQL build yet — fall back to
`count(case when conclusion == "failure" then 1 else null end)` in SQL,
or filter then count in two passes in KQL. Check
[/reference/kql-functions](/reference/) for the current list.)

## What you should see

The latest-failures query, on a healthy weekday:

| _timestamp           | repo            | workflow      | branch  | url (truncated)                   |
| -------------------- | --------------- | ------------- | ------- | --------------------------------- |
| 2026-05-03T14:32:11Z | shaked/engine   | ci            | feat/x  | github.com/shaked/engine/actions… |
| 2026-05-03T14:18:04Z | shaked/web      | deploy-stage  | main    | github.com/shaked/web/actions/…   |
| 2026-05-03T13:51:59Z | shaked/engine   | bench-nightly | main    | github.com/shaked/engine/actions… |

## Variations

- **Webhook receiver instead:** point GitHub's webhook delivery at a
  small server that translates the payload to the same NDJSON shape.
  Same idempotency story — webhook delivery id as the key.
- **Different source:** Stripe events, deploy logs, CI test summaries.
  Same shape: typed columns for predicates, `dynamic` for everything
  else, time-bounded queries. The recipe is the recipe.
- **Promote a `dynamic` field:** if you find yourself running
  `where attributes["branch"] == "main"` constantly, promote `branch`
  to a typed column. See
  [Dynamic and vectors](/concepts/dynamic-and-vectors).
- **Compute a metric on ingest:** if you want a derived field at write
  time (length of body, hash of payload), compute it in your sender.
  kyma ingest doesn't run user code; expressivity belongs upstream.
