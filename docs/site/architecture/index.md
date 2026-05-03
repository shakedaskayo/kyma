---
title: Architecture
description: How kyma stays correct, fast, and distributable. The five invariants, the three-level pruning cascade, the four-slice roadmap, and the on-disk format powering all of it.
---

# Architecture

Two things make kyma's architecture interesting. The first is the
[five invariants](/concepts/the-five-invariants) — five non-negotiable
properties that turn distribution from a rewrite into a deployment
change. The second is the
[three-level pruning cascade](/concepts/the-pruning-cascade) — the
reason a query touches 0.0001 % of the bytes in the bucket on a typical
production-shaped predicate.

This section is the deep view of both. It pairs the synced
architecture document — the canonical source of truth, generated from
`docs/architecture.md` — with the slice-by-slice roadmap that says
exactly what's shipped, what's next, and what's still being decided.
The on-disk format gets its own page: `kyma-format-tlm` is one
implementation of the `SegmentFormat` trait, and the deep-dive covers
its extent layout, footer structure, block-level metadata, and the
indices that make the cascade work.

If you're evaluating kyma, start with the architecture overview and
the slice roadmap. If you're building on kyma — implementing a new
segment format, writing a connector, or planning a self-hosted
deployment — the storage format page is where the real specification
lives. The benchmarks page anchors all of it in measured numbers.

<div class="feature-grid">

<div class="feature-card">

### [Architecture overview](/architecture/architecture)

The canonical living document. Five invariants, three external
dependencies, the three-level pruning cascade, the slice roadmap,
distribution-readiness affordances. Synced from
`docs/architecture.md` at build time — this is the source of truth.

</div>

<div class="feature-card">

### [Slice roadmap](/architecture/slice-roadmap)

The four-slice plan, expanded. Slice 1 (single-node, distribution-
ready) is what ships today. Slices 2–4 (read scale-out, ingest scale-
out, federation) are committed direction with traits already in
place — not aspirational.

</div>

<div class="feature-card">

### [Storage format](/architecture/storage-format)

`kyma-format-tlm` deep dive. Extent layout, magic bytes and footer
framing, per-block stats, per-column distinct sets, token index for
text-search pruning, dynamic-column path bitmaps. The reference for
implementing a new `SegmentFormat`.

</div>

<div class="feature-card">

### [Benchmarks](/architecture/benchmarks)

Measured numbers — ingest throughput, query latency, pruning
effectiveness, storage compression. Synced from `docs/benchmarks.md`
at build time. The architectural tests in `benches/distribution/`
that enforce the five invariants live alongside.

</div>

</div>

## How the pages relate

The architecture overview is the brief. The slice roadmap is the
schedule. The storage format is the spec. The benchmarks are the
receipts.

Cross-references that come up often:

- [Extents and snapshots](/concepts/extents-and-snapshots) — the
  storage-shape concepts that the storage-format page is the
  byte-level companion to.
- [The pruning cascade](/concepts/the-pruning-cascade) — the
  query-side counterpart. Catalog pruning, extent pruning, block
  pruning are what every footer field on the storage-format page
  exists to serve.
- [Schema model](/concepts/schema-model) — the schema-only-widens
  rule referenced repeatedly across the architecture pages.
