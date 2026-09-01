/**
 * Consumer identity → visual mapping. Agent-agnostic registry with a sensible
 * default for unknown kinds, mirroring graph-icons/graph-style. Pure (no React).
 */
import type { ComponentType } from "react";
import { Bot, Brain } from "lucide-react";
import {
  SiClaude,
  SiCursor,
  SiModelcontextprotocol,
  SiWindsurf,
} from "@icons-pack/react-simple-icons";
import type { ConsumerKind, ConsumerVerb } from "./consumer-types";

/** Common shape accepted by both lucide-react and react-simple-icons. Kept
 *  loose (`any` props) because the two libraries' prop/propTypes signatures
 *  don't unify under a stricter shared type. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type ConsumerIcon = ComponentType<any>;

/** Real vendor marks where they exist (Claude, Cursor, Windsurf, the MCP logo);
 *  a brain for Pensieve's own agent and a neutral bot for unknown clients. */
const ICON: Record<ConsumerKind, ConsumerIcon> = {
  claude_code: SiClaude,
  cursor: SiCursor,
  windsurf: SiWindsurf,
  pensieve_agent: Brain,
  mcp: SiModelcontextprotocol,
  unknown: Bot,
};

/** Per-kind accent colour (hex) — drives the beam/marker hue and the dock
 *  thread. Claude clay, cursor sky, windsurf cyan, pensieve amber, mcp violet,
 *  unknown slate. */
const COLOR: Record<ConsumerKind, string> = {
  claude_code: "#d97757",
  cursor: "#60a5fa",
  windsurf: "#22d3ee",
  pensieve_agent: "#fbbf24",
  mcp: "#a78bfa",
  unknown: "#94a3b8",
};

/** Map a raw backend source string to a normalized kind. The backend sends the
 *  MCP client name when known (`claude-code`, `cursor`, …) else the transport
 *  source (`local-serve`, `server`, `mcp-stdio`, `pensieve`). */
export function resolveConsumerKind(source: string): ConsumerKind {
  const s = source.toLowerCase();
  if (s.includes("claude")) return "claude_code";
  if (s.includes("cursor")) return "cursor";
  if (s.includes("windsurf")) return "windsurf";
  // pensieve's own agent (interactive ask / dreaming) shows up as the transport.
  if (s === "pensieve" || s.includes("serve") || s === "server" || s.includes("dream"))
    return "pensieve_agent";
  if (s.includes("mcp")) return "mcp";
  return "unknown";
}

export function consumerIcon(kind: ConsumerKind): ConsumerIcon {
  return ICON[kind] ?? Bot;
}

export function consumerColor(kind: ConsumerKind): string {
  return COLOR[kind] ?? COLOR.unknown;
}

/** Verb → short present-tense label for the dock ("reading" / "writing"). */
export function verbLabel(verb: ConsumerVerb): string {
  switch (verb) {
    case "remember":
      return "writing";
    case "search":
      return "searching";
    default:
      return "reading";
  }
}

/** Writes get a warmer tint than reads in the beam/pulse, regardless of kind. */
export function verbIsWrite(verb: ConsumerVerb): boolean {
  return verb === "remember";
}
