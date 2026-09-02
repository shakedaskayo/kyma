#!/usr/bin/env python3
"""Print pensieve-cli's workspace dependency closure in publish order.

crates.io requires every dependency of a crate to already be published before
that crate can be packaged, so the publish has to walk the closure bottom-up.
publish-crates.yml used to hand-maintain that list as a comment-annotated
ladder of "layers", and it went stale: pensieve-graph-topo was added to the
workspace and never added to the ladder, so a publish run died on

    error: failed to prepare local package for uploading
    Caused by: no matching package named `pensieve-graph-topo` found

after having already pushed pensieve-core to the registry — the worst kind of
failure, since a partial publish cannot be undone. The list was missing 14 of
the 33 crates that actually need publishing.

Deriving it from cargo metadata means adding a crate to the workspace cannot
silently break the release again.

Usage:
    python3 scripts/crates-publish-order.py            # one crate per line
    python3 scripts/crates-publish-order.py --layers   # grouped, for humans
"""

from __future__ import annotations

import json
import subprocess
import sys
from collections import defaultdict

ROOT = "pensieve-cli"


def main() -> int:
    md = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    members = {p["name"]: p for p in md["packages"]}
    if ROOT not in members:
        raise SystemExit(f"{ROOT} is not a workspace member; run from the repo root")

    # `publish = false` in Cargo.toml surfaces as an empty registry list.
    unpublishable = {n for n, p in members.items() if p.get("publish") == []}

    # Workspace-internal, non-dev dependencies. Dev-dependencies do not gate
    # publishing, so including them would impose a false ordering (and can
    # introduce cycles that do not really exist).
    deps: dict[str, set[str]] = defaultdict(set)
    for name, pkg in members.items():
        for d in pkg["dependencies"]:
            if d["name"] in members and d["kind"] is None:
                deps[name].add(d["name"])

    closure: set[str] = set()
    stack = [ROOT]
    while stack:
        n = stack.pop()
        if n in closure:
            continue
        closure.add(n)
        stack.extend(deps[n])
    closure -= unpublishable

    # Kahn layering; alphabetical within a layer so the output is stable.
    layers: list[list[str]] = []
    done: set[str] = set()
    while len(done) < len(closure):
        layer = sorted(
            n for n in closure if n not in done and (deps[n] & closure) <= done
        )
        if not layer:
            raise SystemExit(f"dependency cycle among: {sorted(closure - done)}")
        layers.append(layer)
        done |= set(layer)

    if "--layers" in sys.argv:
        print(f"# {len(closure)} crates in {len(layers)} layers")
        for i, layer in enumerate(layers, 1):
            print(f"# layer {i}")
            for n in layer:
                print(f"  {n}")
        if unpublishable:
            print(f"# skipped (publish = false): {' '.join(sorted(unpublishable))}")
    else:
        for layer in layers:
            for n in layer:
                print(n)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
