#!/usr/bin/env node
// Mirrors docs/{architecture,benchmarks}.md into docs/site/architecture/ at build time.
// Source-of-truth lives outside the site root so engineers edit it where they expect.

import fs from 'node:fs/promises'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
const docsRoot = path.resolve(root, '..')
const out = path.join(root, 'architecture')

const map = [
  { from: 'architecture.md', to: 'architecture.md',
    frontmatter: '---\ntitle: Architecture\ndescription: How kyma stays correct, fast, and distributable.\n---\n\n' },
  { from: 'benchmarks.md', to: 'benchmarks.md',
    frontmatter: '---\ntitle: Benchmarks\ndescription: Performance budgets and measurements.\n---\n\n' },
]

await fs.mkdir(out, { recursive: true })
for (const { from, to, frontmatter } of map) {
  const src = path.join(docsRoot, from)
  const dst = path.join(out, to)
  const body = await fs.readFile(src, 'utf8')
  // Strip leading H1 line if present (page already has frontmatter title).
  const stripped = body.replace(/^# .+\n+/, '')
  await fs.writeFile(dst, frontmatter + stripped, 'utf8')
}
console.log('synced included markdown into docs/site/architecture/')
