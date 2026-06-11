---
title: Dynamic and vectors
description: How kyma stores and queries the two non-relational column types — dynamic (CBOR with a token index) and vector(N) (fixed-dimension embeddings with distance UDFs).
---

# Dynamic and vectors

Kyma is column-aware first. Most data fits the typed columns from the
[schema model](/concepts/schema-model). Two column types handle what
doesn't: `dynamic` for arbitrary structured data, and `vector(N)` for
embeddings.

## The `dynamic` column

`dynamic` is the catch-all. CBOR-encoded values per row, with two
catalog-side indices that make queries against them fast.

Why CBOR (and not JSON): smaller, faster to parse, native binary support
without base64. Every primitive type maps to a CBOR major type; nested
maps and arrays are first-class.

### What goes in dynamic

Anything that's structurally not a single column:

- OTLP resource attributes — a map of arbitrary keys per log line.
- Mongo documents synced via the data source framework. Top-level fields
  flatten to dotted columns up to `flatten_depth`; deeper nesting and
  polymorphic fields land in `dynamic`.
- Postgres `jsonb` columns — the entire document as one `dynamic` value.
- Application-specific attributes that haven't stabilized into typed
  columns yet.

### Path access in queries

You query `dynamic` with bracketed path syntax:

```kql
otel_logs
| where attributes["http.method"] == "POST"
| where attributes["http.status_code"] >= 500
| project _timestamp, attributes["http.url"], attributes["error.code"]
```

The same in SQL:

```sql
SELECT _timestamp,
       attributes ->> 'http.url' AS url,
       attributes ->> 'error.code' AS error_code
  FROM otel_logs
 WHERE attributes ->> 'http.method' = 'POST'
   AND CAST(attributes ->> 'http.status_code' AS INTEGER) >= 500
```

KQL is the more ergonomic surface for `dynamic` access; SQL works but
gets verbose with casts.

### Why dynamic queries stay fast

Two indices, both at the extent level:

- **Path bitmap** records which paths were written to this extent. A
  query referencing a path the extent never saw skips it without reading
  any block bytes.
- **Token index** is a posting list over leaf strings. A predicate like
  `attributes["error.code"] == "ECONNRESET"` plans as a posting-list
  intersection, not a substring scan.

For a tour of how these fit into the broader pipeline, see
[The pruning cascade](/concepts/the-pruning-cascade).

### When to promote out of dynamic

A field that's appeared in ≥ 100 events with one consistent type within
a 1000-event window is a candidate for promotion to a typed column.
Manual promotion via `kyma-cli`:

```bash
kyma-cli alter-table otel_logs add-column \
  --name "service_name" \
  --type "string" \
  --from-dynamic "attributes.service.name"
```

After promotion, new writes go to the typed column; old data stays in
`dynamic`. Reads union the two via `coalesce()`. Data sources in sync
mode do this promotion automatically.

## The `vector(N)` column

A fixed-dimension `Float32` embedding column. The dimension `N` is set
at table creation time and never changes.

```bash
kyma-cli create-table embeddings \
  --schema '_timestamp:timestamp, doc_id:string, body:string, embedding:vector(384)'
```

### Storage

Vectors are stored as Arrow `FixedSizeList<Float32, N>`. Per-extent
column statistics include centroid, bounding box, and (when an ANN
index is built — see roadmap) HNSW or IVF metadata.

### Distance UDFs

Three distance functions registered in DataFusion:

```sql
SELECT doc_id,
       cosine_distance(embedding, $query_vec) AS d
  FROM embeddings
 ORDER BY d ASC
 LIMIT 5
```

Available UDFs:

| UDF                         | Distance                             |
| --------------------------- | ------------------------------------ |
| `cosine_distance(a, b)`     | `1 - (a · b) / (‖a‖ ‖b‖)`            |
| `l2_distance(a, b)`         | `√(Σ (aᵢ − bᵢ)²)`                    |
| `inner_product(a, b)`       | `−(a · b)` (for ranking)             |

Dimensions are checked at query time; mismatches fail loudly.

### Without an ANN index

Today, vector search is exact: every candidate row gets a distance
calculation. With time-range and metadata filters, this is usually
fast enough — pruning eliminates most extents before any vector math
runs.

For tables with millions of vectors and no good prefilter, exact search
becomes the bottleneck. ANN indices (HNSW) land in a later milestone;
the trait surface (`SegmentFormat::vector_index`) is already in place.

### Loading vectors

Two paths to populate a vector column:

- **Compute outside, ingest as values.** Generate embeddings with your
  model of choice; send them as Arrow `FixedSizeList<Float32, 384>` over
  the REST or OTLP path.
- **Compute inside, on ingest.** Configure an [embedding backend][embed]
  (`fastembed`, `ollama`, OpenAI-compatible, Gemini) on the table; the
  ingest path runs `body` through the backend and writes the result to
  `embedding` automatically.

[embed]: /concepts/the-agent-loop

## Where to go next

- The agent endpoint, which uses vectors for schema RAG: [The agent loop](/concepts/the-agent-loop).
- KQL syntax for `dynamic`: [Query](/query/).
- Schema evolution rules: [Schema model](/concepts/schema-model).
