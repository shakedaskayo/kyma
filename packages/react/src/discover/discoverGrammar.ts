import type { Pill } from "./types";

export function parseSearch(input: string): Pill[] {
  const out: Pill[] = [];
  let i = 0;
  while (i < input.length) {
    while (i < input.length && /\s/.test(input[i]!)) i++;
    if (i >= input.length) break;
    const negated = input[i] === "-";
    if (negated) i++;
    let raw = "";
    while (i < input.length && !/\s/.test(input[i]!)) {
      if (input[i] === '"') {
        i++;
        let closed = false;
        while (i < input.length) {
          if (input[i] === '"') { closed = true; i++; break; }
          raw += input[i++];
        }
        if (!closed) throw new Error("unterminated quoted string");
        continue;
      }
      raw += input[i++];
    }
    if (!raw) {
      if (negated) throw new Error("dangling '-' with no following token");
      continue;
    }
    out.push(tokenToPill(raw, negated));
  }
  return out;
}

function tokenToPill(token: string, negated: boolean): Pill {
  const c = token.indexOf(":");
  if (c <= 0) {
    if (negated) throw new Error("negation requires a field — substrings cannot be negated");
    return { kind: "substring", value: token };
  }
  const field = token.slice(0, c);
  const value = token.slice(c + 1);
  if (value === "*") return { kind: "exists", field };
  if (value.startsWith(">=")) return { kind: "cmp", field, op: "ge", value: value.slice(2) };
  if (value.startsWith("<=")) return { kind: "cmp", field, op: "le", value: value.slice(2) };
  if (value.startsWith(">"))  return { kind: "cmp", field, op: "gt", value: value.slice(1) };
  if (value.startsWith("<"))  return { kind: "cmp", field, op: "lt", value: value.slice(1) };
  return negated
    ? { kind: "neq", field, value }
    : { kind: "eq", field, value };
}

export function serializePills(pills: Pill[]): string {
  return pills.map(pillToToken).join(" ");
}

function pillToToken(p: Pill): string {
  switch (p.kind) {
    case "substring": return needsQuotes(p.value) ? `"${p.value}"` : p.value;
    case "eq":  return `${p.field}:${quoteIfNeeded(p.value)}`;
    case "neq": return `-${p.field}:${quoteIfNeeded(p.value)}`;
    case "exists": return `${p.field}:*`;
    case "cmp": {
      const op = { gt: ">", ge: ">=", lt: "<", le: "<=" }[p.op];
      return `${p.field}:${op}${p.value}`;
    }
  }
}

function needsQuotes(s: string) { return /\s/.test(s); }
function quoteIfNeeded(s: string) { return needsQuotes(s) ? `"${s}"` : s; }
