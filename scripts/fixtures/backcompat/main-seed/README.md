# main-seed back-compat fixture

Captured against `main` HEAD at F1 implementation time, before any
`v1.0.0-pre.N` tag exists. Pinned dataset: see `seed.ndjson` in this
directory.

This is the synthetic seed fixture. It establishes the back-compat
machinery before real tags exist. Once `v1.0.0-pre.1` is cut, the
workflow grows a per-tag fixture next to this one and continues to
include this seed.

Do **not** edit the captured files (`expected-hashes.txt`,
`catalog-schema.sql`, `manifest.json`). They are the contract.
