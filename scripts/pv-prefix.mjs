#!/usr/bin/env node
/**
 * pv-prefix.mjs — Tailwind class prefix codemod
 *
 * Rewrites Tailwind utility tokens inside .ts/.tsx files so they gain the `pv-`
 * prefix required by packages/react (which sets `prefix: "pv-"` in its
 * tailwind config).
 *
 * Usage:
 *   node scripts/pv-prefix.mjs <dir-or-file> [--dry]
 *
 * The --dry flag prints unified-diff-style output without writing files.
 */

import { readFileSync, writeFileSync, statSync } from "fs";
import { readdirSync } from "fs";
import { join, extname, resolve } from "path";
import { createRequire } from "module";

// ─── Token-level rewrite ──────────────────────────────────────────────────────

/**
 * Tokens that must never be prefixed.
 * These are the pensieve-root/pensieve-dark scope sentinels, and the pv- prefix itself.
 */
const PRESERVE_EXACT = new Set(["pensieve-root", "pensieve-dark"]);

/**
 * Returns true when a token is "not class-like" and should be left alone.
 *   • Contains template/interpolation leftovers: { } $ `
 *   • Is a CSS custom-property name starting with --
 *   • Already carries the pv- prefix (idempotent)
 */
function isNonClassLike(token) {
  if (token.startsWith("--")) return true;
  if (/[{}`$]/.test(token)) return true;
  return false;
}

/**
 * Prefix a single utility segment (the part after the final colon).
 * Handles: negative (-mt-2), important (!px-2), plain, already-prefixed.
 *
 * @param {string} util  e.g. "flex", "-mt-2", "!px-2", "pv-flex"
 * @returns {string}
 */
function prefixUtil(util) {
  // Already prefixed — idempotent
  if (util.startsWith("pv-")) return util;

  // Important modifier  !px-2 → !pv-px-2
  if (util.startsWith("!")) {
    const inner = util.slice(1);
    if (inner.startsWith("pv-")) return util; // !pv-... already done
    return "!" + "pv-" + inner;
  }

  // Negative  -mt-2 → -pv-mt-2
  if (util.startsWith("-")) {
    const inner = util.slice(1);
    if (inner.startsWith("pv-")) return util; // -pv-... already done
    return "-pv-" + inner;
  }

  return "pv-" + util;
}

/**
 * Split a full token (possibly containing variant prefixes) into its parts.
 *
 * A variant chain looks like:  md:dark:[&>svg]:flex
 * The LAST colon-delimited segment is the utility; everything before is variants.
 *
 * Arbitrary VARIANTS like [&>svg]: or data-[state=open]: can contain brackets
 * and colons themselves — we must not split on the colon inside brackets.
 *
 * Strategy: scan from right to left finding the last "top-level" colon
 * (i.e. a colon not inside square brackets).
 */
function splitVariantUtil(token) {
  // Find last top-level colon
  let depth = 0;
  let lastColon = -1;
  for (let i = 0; i < token.length; i++) {
    const ch = token[i];
    if (ch === "[") depth++;
    else if (ch === "]") depth--;
    else if (ch === ":" && depth === 0) lastColon = i;
  }

  if (lastColon === -1) {
    // No variant — entire token is the utility
    return { variants: "", util: token };
  }

  return {
    variants: token.slice(0, lastColon + 1), // includes the colon
    util: token.slice(lastColon + 1),
  };
}

/**
 * Rewrite a single whitespace-separated class token.
 *
 * @param {string} token
 * @returns {string}
 */
export function prefixToken(token) {
  if (!token) return token;
  if (PRESERVE_EXACT.has(token)) return token;
  if (isNonClassLike(token)) return token;

  const { variants, util } = splitVariantUtil(token);

  // Already fully prefixed (utility part already pv-)
  const prefixed = prefixUtil(util);
  return variants + prefixed;
}

/**
 * Rewrite a whitespace-separated class string (e.g. the value of className="...").
 *
 * @param {string} s  space-separated list of class tokens (may contain newlines)
 * @returns {string}
 */
export function prefixClassList(s) {
  // Split on whitespace, preserving the whitespace gaps for round-trip fidelity
  // We split on runs of whitespace, keeping the delimiters.
  return s.replace(/(\S+)/g, (token) => prefixToken(token));
}

// ─── String-literal rewriting inside source files ────────────────────────────

/**
 * Rewrite every string literal inside a matched context region.
 *
 * We do a careful regex pass rather than a full AST parse.
 * Supported contexts:
 *   1. className="..."                    double-quoted
 *   2. className={"..."}                  JSX expression, double-quoted
 *   3. className={'...'}                  JSX expression, single-quoted
 *   4. className={`...`}                  template literal (static segments only)
 *   5. cn(...), cva(...), clsx(...), twMerge(...)  — every string arg / object key
 *
 * Returns { code: string, count: number, warnings: string[] }
 */
export function rewriteFile(source, filePath = "<input>") {
  let code = source;
  let count = 0;
  const warnings = [];

  // ── Helper: rewrite a string value and track count ──────────────────────────
  function rewriteClassString(str) {
    const rewritten = prefixClassList(str);
    if (rewritten !== str) count++;
    return rewritten;
  }

  // ── 1. className="..." (plain double-quoted attribute) ──────────────────────
  code = code.replace(
    /\bclassName=(")((?:[^"\\]|\\.)*)(")/g,
    (_, open, content, close) => {
      return `className=${open}${rewriteClassString(content)}${close}`;
    }
  );

  // ── 2. className={'...'} (single-quoted JSX expression) ─────────────────────
  code = code.replace(
    /\bclassName=\{(')((?:[^'\\]|\\.)*?)('\)?)(\})/g,
    (match, open, content, close, brace) => {
      return `className={${open}${rewriteClassString(content)}${close}${brace}`;
    }
  );
  // Simpler pass for className={'...'} (balancing the { separately)
  code = code.replace(
    /\bclassName=(\{')([^']*?)('\})/g,
    (_, open, content, close) => {
      return `className=${open}${rewriteClassString(content)}${close}`;
    }
  );
  // className={"..."}
  code = code.replace(
    /\bclassName=(\{")([^"]*?)("\})/g,
    (_, open, content, close) => {
      return `className=${open}${rewriteClassString(content)}${close}`;
    }
  );

  // ── 3. className={`...`} template literals ─────────────────────────────────
  // Find template literals in className={`...`}
  // We rewrite static segments (between ${ }) and warn if there are interpolations.
  code = code.replace(
    /\bclassName=\{(`)([\s\S]*?)(`)\}/g,
    (match, open, content, close) => {
      const hasInterp = /\$\{/.test(content);
      if (hasInterp) {
        warnings.push(
          `${filePath}: template literal with interpolation in className — review manually`
        );
      }
      // Rewrite static segments around ${ ... }
      const rewritten = content.replace(
        /((?:[^`$]|\$(?!\{))*?)(\$\{[\s\S]*?\}|$)/g,
        (seg, staticPart, interp) => {
          return rewriteClassString(staticPart) + interp;
        }
      );
      return `className={${open}${rewritten}${close}}`;
    }
  );

  // ── 4. cn(...), cva(...), clsx(...), twMerge(...) call expressions ──────────
  // We rewrite string literals inside these calls.
  // Strategy: find the call, then process its argument list.
  // This handles: string args, array elements, object keys (clsx map).
  //
  // We do a balanced-paren walk for each call site.

  const CALL_FNS = /\b(cn|cva|clsx|twMerge)\s*\(/g;
  let callMatch;
  const callReplacements = [];

  // Reset lastIndex
  CALL_FNS.lastIndex = 0;

  while ((callMatch = CALL_FNS.exec(code)) !== null) {
    const start = callMatch.index + callMatch[0].length - 1; // position of opening (
    // Walk to find matching closing paren
    let depth = 0;
    let end = start;
    for (let i = start; i < code.length; i++) {
      const ch = code[i];
      if (ch === "(") depth++;
      else if (ch === ")") {
        depth--;
        if (depth === 0) {
          end = i;
          break;
        }
      }
    }

    if (depth !== 0) continue; // unbalanced — skip

    const argsRegion = code.slice(start, end + 1); // includes ( and )
    const rewrittenArgs = rewriteCallArgs(argsRegion, filePath, warnings, count);
    count = rewrittenArgs.count;

    if (rewrittenArgs.result !== argsRegion) {
      callReplacements.push({
        start,
        end: end + 1,
        original: argsRegion,
        replacement: rewrittenArgs.result,
      });
    }
  }

  // Apply replacements in reverse order to preserve indices
  for (let i = callReplacements.length - 1; i >= 0; i--) {
    const { start, end, replacement } = callReplacements[i];
    code = code.slice(0, start) + replacement + code.slice(end);
  }

  // ── 5. Warn for className={identifier} patterns ─────────────────────────────
  const classNameIdentifier = /\bclassName=\{([A-Za-z_$][A-Za-z0-9_$.]*)\}/g;
  let idMatch;
  while ((idMatch = classNameIdentifier.exec(code)) !== null) {
    warnings.push(
      `${filePath}: className={${idMatch[1]}} — variable assignment not rewritten, review manually`
    );
  }

  return { code, count, warnings };
}

/**
 * Rewrite string literals inside a call argument region (the balanced parens).
 * Handles: plain strings, array elements, object keys (clsx maps), nested calls.
 *
 * @param {string} region  e.g. `("px-2", { "flex": cond })`
 * @returns {{ result: string, count: number }}
 */
function rewriteCallArgs(region, filePath, warnings, count) {
  let result = region;

  // Double-quoted strings:  "px-2 flex"
  result = result.replace(
    /"((?:[^"\\]|\\.)*)"/g,
    (_, content) => {
      const rewritten = prefixClassList(content);
      if (rewritten !== content) count++;
      return `"${rewritten}"`;
    }
  );

  // Single-quoted strings:  'px-2 flex'
  result = result.replace(
    /'((?:[^'\\]|\\.)*)'/g,
    (_, content) => {
      const rewritten = prefixClassList(content);
      if (rewritten !== content) count++;
      return `'${rewritten}'`;
    }
  );

  // Template literals inside calls (no nested ${} supported for simplicity)
  result = result.replace(
    /`((?:[^`$]|\$(?!\{))*)`/g,
    (_, content) => {
      const rewritten = prefixClassList(content);
      if (rewritten !== content) count++;
      return "`" + rewritten + "`";
    }
  );

  return { result, count };
}

// ─── File walking ─────────────────────────────────────────────────────────────

function collectFiles(target) {
  const stat = statSync(target);
  if (stat.isFile()) {
    const ext = extname(target);
    return ext === ".ts" || ext === ".tsx" ? [target] : [];
  }
  if (stat.isDirectory()) {
    const files = [];
    for (const entry of readdirSync(target, { withFileTypes: true })) {
      if (entry.name.startsWith(".")) continue;
      if (entry.name === "node_modules") continue;
      files.push(...collectFiles(join(target, entry.name)));
    }
    return files;
  }
  return [];
}

// ─── Unified diff (minimal, line-level) ──────────────────────────────────────

function unifiedDiff(original, updated, filePath) {
  const origLines = original.split("\n");
  const updLines = updated.split("\n");
  const lines = [];
  lines.push(`--- ${filePath}`);
  lines.push(`+++ ${filePath} (rewritten)`);

  const maxLen = Math.max(origLines.length, updLines.length);
  for (let i = 0; i < maxLen; i++) {
    const o = origLines[i];
    const u = updLines[i];
    if (o === u) {
      lines.push(` ${o ?? ""}`);
    } else {
      if (o !== undefined) lines.push(`-${o}`);
      if (u !== undefined) lines.push(`+${u}`);
    }
  }
  return lines.join("\n");
}

// ─── Main ─────────────────────────────────────────────────────────────────────

function main() {
  const args = process.argv.slice(2);
  const dry = args.includes("--dry");
  const targets = args.filter((a) => !a.startsWith("--"));

  if (targets.length === 0) {
    console.error(
      "Usage: node scripts/pv-prefix.mjs <dir-or-file> [--dry]"
    );
    process.exit(1);
  }

  let totalFiles = 0;
  let totalRewritten = 0;
  let totalWarnings = 0;

  for (const target of targets) {
    const absTarget = resolve(target);
    const files = collectFiles(absTarget);

    for (const filePath of files) {
      const source = readFileSync(filePath, "utf-8");
      const { code, count, warnings } = rewriteFile(source, filePath);

      totalFiles++;
      totalRewritten += count;
      totalWarnings += warnings.length;

      if (code !== source || warnings.length > 0) {
        if (dry) {
          if (code !== source) {
            console.log(unifiedDiff(source, code, filePath));
          }
        } else {
          if (code !== source) {
            writeFileSync(filePath, code, "utf-8");
          }
        }
        const verb = dry ? "[dry]" : "[write]";
        if (count > 0) {
          console.log(`${verb} ${filePath}: ${count} literal(s) rewritten`);
        }
      }

      for (const w of warnings) {
        console.warn(`[warn] ${w}`);
      }
    }
  }

  console.log(
    `\nDone. ${totalFiles} file(s) scanned, ${totalRewritten} literal(s) rewritten, ${totalWarnings} warning(s).`
  );
}

// Run main only when invoked directly (not when imported by tests)
const isMain =
  process.argv[1] &&
  resolve(process.argv[1]) === resolve(new URL(import.meta.url).pathname);

if (isMain) {
  main();
}
