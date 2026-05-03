#!/usr/bin/env node
// Validates that every <Diagram name="..."> reference in markdown points to
// a real SVG file in docs/public/diagrams/, and every <image href="/icons/brand/..."/>
// reference inside those SVGs points to a real icon file.
// Exit non-zero on any unresolved reference.

import { readFile, readdir, stat } from 'node:fs/promises';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const DOCS_ROOT = resolve(__dirname, '..');
const DIAGRAMS_DIR = join(DOCS_ROOT, 'public', 'diagrams');

const errors = [];

async function* walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name.startsWith('.') || entry.name === 'superpowers') continue;
      yield* walk(full);
    } else {
      yield full;
    }
  }
}

async function fileExists(p) {
  try { await stat(p); return true; } catch { return false; }
}

// 1. Scan every .md for <Diagram name="..." />, skipping content inside code fences.
const diagramRefRe = /<Diagram\s+[^>]*name=["']([^"']+)["']/g;
for await (const file of walk(DOCS_ROOT)) {
  if (!file.endsWith('.md')) continue;
  if (file.includes('/node_modules/')) continue;
  const raw = await readFile(file, 'utf8');
  // Strip fenced ``` blocks and inline `code` so component examples in prose don't false-positive.
  const text = raw.replace(/```[\s\S]*?```/g, '').replace(/`[^`]*`/g, '');
  let m;
  while ((m = diagramRefRe.exec(text)) !== null) {
    const name = m[1];
    const svgPath = join(DIAGRAMS_DIR, `${name}.svg`);
    if (!(await fileExists(svgPath))) {
      errors.push(`${file}: <Diagram name="${name}"> → missing ${svgPath}`);
    }
  }
}

// 2. Scan every SVG in /public/diagrams/ for <image href="/icons/..."/> (also matches xlink:href).
const imageHrefRe = /<image\s+[^>]*?(?:xlink:)?href=["'](\/icons\/[^"']+)["']/g;
for await (const file of walk(DIAGRAMS_DIR)) {
  if (!file.endsWith('.svg')) continue;
  const text = await readFile(file, 'utf8');
  let m;
  while ((m = imageHrefRe.exec(text)) !== null) {
    const href = m[1]; // e.g. /icons/brand/postgresql.svg
    const iconPath = join(DOCS_ROOT, 'public', href.replace(/^\//, ''));
    if (!(await fileExists(iconPath))) {
      errors.push(`${file}: <image href="${href}"> → missing ${iconPath}`);
    }
  }
}

if (errors.length) {
  console.error(`\n✖ check-diagrams: ${errors.length} unresolved reference(s):\n`);
  for (const e of errors) console.error('  ' + e);
  process.exit(1);
}
console.log('✔ check-diagrams: all references resolve.');
