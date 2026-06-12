# Golden retrieval set

Judged query fixtures that gate retrieval quality. Every S1 retrieval change
(ANN indexes, BM25, rerankers, fusion tweaks) must hold NDCG@10 and MRR on
this set within the tolerance committed in `baseline.json`.

## Layout

- `*.json` — one fixture per query (see format below). Files prefixed
  `example-` are synthetic wiring examples, not mined queries.
- `baseline.json` — committed aggregate floor:
  `{ "mean_ndcg_at_10": .., "mean_mrr": .., "tolerance": .., "notes": .. }`.
  A run regresses when an aggregate drops more than `tolerance` below its
  baseline value. Improvements never fail; re-capture the baseline when a
  deliberate quality win lands.

## Fixture format

```json
{
  "query": "natural-language query as a user would type it",
  "mode": "data" | "memory" | "graph",
  "judged": [
    { "key": "stable-result-key", "grade": 3 },
    { "key": "another-key",       "grade": 0 }
  ],
  "notes": "provenance + judging notes"
}
```

- `key` is whatever stable identifier the engine returns for that mode
  (memory id, `db/table` row key, graph node id). The runner matches keys
  exactly.
- `grade` is graded relevance: 3 = perfect answer, 2 = highly relevant,
  1 = marginally relevant, 0 = irrelevant. Results the engine returns that
  carry no judgment score as grade 0.

## How queries get here

1. **Mine** real queries from Ask-Kyma traces (`otel_traces` /
   `agent_sessions`): frequent, diverse, anonymized. No synthetic phrasing —
   the set must reflect what users actually ask.
2. **Collect candidates** per query from the current engine (`/v1/search`,
   top ~20 per mode) so judgments cover what retrieval can actually surface.
3. **Judge** with the local-only `judge_calibrate` bin (LLM judge via
   `ANTHROPIC_API_KEY`), then **freeze**: a human spot-checks a sample, the
   calibration stats (exact-match rate, MAE vs human labels) are recorded in
   the fixture `notes`, and the JSON is committed. After that the fixture is
   immutable data — the judge never runs in CI, and re-judging an existing
   fixture requires re-calibration.
4. **Baseline**: run the golden runner against the current engine and commit
   the aggregates to `baseline.json`.

## Running

```sh
cargo test -p kyma-retrieval-eval          # unit + wiring tests (CI-safe)
cargo run -p kyma-retrieval-eval --bin judge_calibrate -- --help   # local only
```

The runner itself lives in `kyma_retrieval_eval::golden` —
`load_fixtures` → `evaluate` → `check_regression`.
