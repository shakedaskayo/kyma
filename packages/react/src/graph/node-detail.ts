/**
 * Pure helpers for the graph node inspector. No React — easy to unit test.
 * Decide which property is the renderable "content", whether a node points at a
 * fetchable source artifact (`object_path`), and how to format property values
 * for display (pretty JSON, collapsed embeddings).
 */
import type { GraphNode } from "@kyma-ai/client";

/** The node's renderable long-text content (`properties.content`) or null. */
export function nodeContent(node: GraphNode): string | null {
  const c = node.properties?.content;
  return typeof c === "string" && c.trim() !== "" ? c : null;
}

/** The object-store key of a fetchable source file (`properties.object_path`) or null. */
export function nodeSourcePath(node: GraphNode): string | null {
  const p = node.properties?.object_path;
  return typeof p === "string" && p.trim() !== "" ? p : null;
}

/** Human-readable node title: `name` → `title` → raw id. */
export function nodeName(node: GraphNode): string {
  return (node.properties?.name as string) || (node.properties?.title as string) || node.id;
}

/** Subtitle line: labels joined with the namespace, dropping empty parts. */
export function nodeSubtitle(node: GraphNode): string {
  return [...node.labels, node.namespace].filter(Boolean).join(" · ");
}

export type FormattedValue = { text: string; kind: "scalar" | "json" | "array" };

const COLLAPSE_LEN = 24; // collapse numeric arrays/embeddings longer than this

function allNumeric(parts: string[]): boolean {
  return parts.every((p) => p.trim() !== "" && !Number.isNaN(Number(p)));
}

/**
 * Format a property value for display:
 * - large numeric arrays / comma-joined number strings (embeddings) → `float[N]`
 * - objects and JSON-looking strings → pretty-printed JSON
 * - everything else → `String(v)`
 */
export function formatValue(v: unknown): FormattedValue {
  if (Array.isArray(v)) {
    if (v.length > COLLAPSE_LEN && v.every((x) => typeof x === "number")) {
      return { text: `float[${v.length}]`, kind: "array" };
    }
    return { text: JSON.stringify(v, null, 2), kind: "json" };
  }
  if (v !== null && typeof v === "object") {
    return { text: JSON.stringify(v, null, 2), kind: "json" };
  }
  if (typeof v === "string") {
    const t = v.trim();
    // Embedding rendered as a bare comma-separated number list.
    const commaParts = t.split(",");
    if (commaParts.length > COLLAPSE_LEN && allNumeric(commaParts)) {
      return { text: `float[${commaParts.length}]`, kind: "array" };
    }
    // JSON-looking string (e.g. provenance is stored as a JSON string).
    if ((t.startsWith("{") && t.endsWith("}")) || (t.startsWith("[") && t.endsWith("]"))) {
      try {
        const parsed = JSON.parse(t);
        if (Array.isArray(parsed) && parsed.length > COLLAPSE_LEN && parsed.every((x) => typeof x === "number")) {
          return { text: `float[${parsed.length}]`, kind: "array" };
        }
        return { text: JSON.stringify(parsed, null, 2), kind: "json" };
      } catch {
        // not valid JSON — fall through to scalar
      }
    }
    return { text: v, kind: "scalar" };
  }
  return { text: String(v), kind: "scalar" };
}

/** Property entries in a stable display order: content, title, …alpha…, embedding last. */
export function orderedProps(node: GraphNode): Array<[string, unknown]> {
  const rank = (k: string): number => {
    if (k === "content") return 0;
    if (k === "title") return 1;
    if (k === "embedding") return 99;
    return 50;
  };
  return Object.entries(node.properties ?? {}).sort((a, b) => {
    const ra = rank(a[0]);
    const rb = rank(b[0]);
    return ra !== rb ? ra - rb : a[0].localeCompare(b[0]);
  });
}
