# kyma Stability Contract

> **Status:** in force from `v1.0.0`. Until then, this document is the
> contract kyma is being hardened to meet. Any change to a "frozen" surface
> below requires the deprecation policy at the bottom of this file.

This document names every surface kyma promises not to break across minor
versions in the v1.x series, and the deprecation policy that governs
intentional changes.

If a surface is not listed here, it is not part of the v1.0 contract — it
may change in any minor release.

---

## 1. HTTP REST API

_Filled in by Task 2._

## 2. Arrow Flight gRPC API

_Filled in by Task 3._

## 3. KQL dialect

_Filled in by Task 4._

## 4. SQL dialect

_Filled in by Task 5._

## 5. MCP surface

_Filled in by Task 6._

## 6. Catalog Postgres schema

_Filled in by Task 7._

## 7. Configuration keys and environment variables

_Filled in by Task 8._

## 8. Extent on-disk format

_Filled in by Task 9. Final freeze blocked on P0 (format v1)._

## 9. Metrics, structured logs, internal traces

See [`docs/metrics-taxonomy.md`](metrics-taxonomy.md) for the rules. The
concrete metric/log/trace inventories are owned by the per-area specs
(A1–A4) and listed under each area's runbook.

## 10. Deprecation policy

_Filled in by Task 10. Mirrored in `CONTRIBUTING.md` by Task 11._
