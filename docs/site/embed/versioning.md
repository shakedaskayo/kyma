---
title: Versioning and release — @pensieve-ai/react
description: Changesets release flow, semver policy, and npm publishing for @pensieve-ai/react and @pensieve-ai/client.
---

# Versioning and release

## Packages

| Package | npm | Description |
|---|---|---|
| `@pensieve-ai/react` | `npm install @pensieve-ai/react` | Embeddable React components and hooks |
| `@pensieve-ai/client` | `npm install @pensieve-ai/client` | Framework-agnostic TypeScript client (peer dep of react) |

Both packages are MIT-compatible (Apache-2.0) and published with
`"access": "public"` on the `@pensieve-ai` npm org.

## Changesets release flow

The monorepo uses [Changesets](https://github.com/changesets/changesets) to
track unreleased changes and automate version bumps.

**Adding a changeset for a PR:**

```bash
pnpm changeset add
# Follow the interactive prompts to select packages and bump type.
```

This creates a file in `.changeset/` that describes the change. Commit it with
your PR. Multiple changesets can accumulate before a release.

**The release CI pipeline** (`release.yml`) runs on every push to `main`:

1. `pnpm install --frozen-lockfile`
2. `pnpm build:packages` — builds `@pensieve-ai/client` and `@pensieve-ai/react`
3. `pnpm test:packages` — runs both test suites
4. `changesets/action@v1` — either:
   - Opens / updates a "Release @pensieve-ai packages" version-bump PR if changesets
     are present (bumps `package.json` versions, updates `CHANGELOG.md` files),
     **or**
   - Publishes to npm (`pnpm changeset publish`) if that PR is merged.

No manual `npm publish` is needed. Merging the version-bump PR triggers the
publish step automatically on the next push.

## Semver policy

| Change type | Bump | Examples |
|---|---|---|
| New optional prop (backward-compatible) | **minor** | Adding `onSearchChange` to `PensieveDiscover` |
| New hook | **minor** | Adding `usePensieveCapabilities` |
| Bug fix, internal refactor | **patch** | Fixing proactive JWT refresh, fixing store isolation |
| Required prop added, prop removed, prop renamed | **major** | Removing `toolbar` from `PensieveGraph` |
| Peer dependency major bump | **major** | Requiring React 19 |

As a general rule: if existing call sites continue to compile and behave
correctly without any changes, it is a minor or patch. If existing call sites
break or need updates, it is a major.

## Building packages locally

```bash
# From the repo root
pnpm build:packages
# equivalent to:
pnpm --filter @pensieve-ai/client build
pnpm --filter @pensieve-ai/react build
```

## Running package tests

```bash
pnpm test:packages
# equivalent to:
pnpm --filter @pensieve-ai/client test
pnpm --filter @pensieve-ai/react test
```

## Storybook

```bash
pnpm --filter @pensieve-ai/react storybook        # dev server at :6006
pnpm --filter @pensieve-ai/react build-storybook  # static output to packages/react/storybook-static
```
