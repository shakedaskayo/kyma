---
title: Coding agents
description: pensieve is coding-agent-agnostic — any coding agent connects via MCP, CLI, or plugin. This page covers the agent registry, what each agent gets today, and how to add one.
---

# Coding agents

pensieve is **agent-agnostic**: the context engine — memory, graph, live data — doesn't
care which coding tool is on the other end. Claude Code, Cursor, Windsurf, Codex, Aider,
Continue, or anything that speaks MCP or can shell out all connect to the same engine.

Three first-class paths, any combination:

- **MCP** — the universal backbone. `pensieve setup <agent>` writes a stdio `pensieve mcp`
  entry for any known agent; the HTTP transport (`/mcp/v1`) works for any MCP-capable
  client. The full tool surface — memory, graph, live data, curation — is available to
  every agent this way. See [Connect your agent](/agent/connect) and the
  [MCP reference](/reference/mcp).
- **CLI + skill** — any agent that can shell out. `pensieve install-skill` writes a `SKILL.md`
  that teaches the agent when to call `pensieve recall` / `pensieve remember` / `pensieve query`. See
  [Connect via CLI](/agent/connect-from-cli).
- **Plugin** — deep automatic capture + recall with no tool call. Today only Claude Code
  has a plugin; it wires Stop/PostToolUse hooks to ingest sessions and
  SessionStart/UserPromptSubmit hooks to inject recalled context into every prompt.

## The agent registry

pensieve probes the local machine for known agents at startup and on each node sync cycle.
The registry is an `AgentDetector` trait:

```rust
// simplified shape
trait AgentDetector {
    fn kind(&self) -> &str;
    fn detect(&self) -> Option<DetectedAgent>;  // is this agent installed?
    fn presence(&self) -> Vec<PresenceSession>; // what sessions are live right now?
}

struct DetectedAgent {
    kind:   String,   // e.g. "claude-code"
    name:   String,
    roots:  Vec<PathBuf>, // transcript / session directories
    realms: Vec<String>,
}
```

Each registered detector advertises `source:<kind>` as a capability. The node daemon
(`pensieve worker run`) uses this registry to decide what to sync — see
[Workers & nodes](/agent/workers).

Four detectors are registered today: `claude-code`, `cursor`, `windsurf`, `codex`. Any
agent not in the list connects over MCP using the generic path below.

## Supported agents

| Agent | Detected | Ingest pipeline | `pensieve setup` writes | Plugin |
|---|---|---|---|---|
| Claude Code | yes — `~/.claude/projects/<realm>/*.jsonl` | Full: transcripts → `claude_code_events` firehose | `.mcp.json` (project) | `pensieve install-plugin` (auto-capture + recall hooks + slash commands) |
| Cursor | yes — `~/.cursor` | Detection only (roadmap) | `.cursor/mcp.json` (project) | — |
| Windsurf | yes — `~/.codeium/windsurf` | Detection only (roadmap) | `~/.codeium/windsurf/mcp_config.json` (home) | — |
| Codex | yes — `~/.codex/sessions` | Detection only (roadmap) | None — use `pensieve setup codex --print` snippet | — |
| Any other agent | via `--print` | — | paste-ready stdio snippet | — |

**What this means today**: Claude Code is the only agent with a full ingest pipeline — its
transcripts are streamed into the firehose and made queryable. Cursor, Windsurf, and Codex
are detected and presence-reported, and all three connect over MCP right now; their
source-sync pipelines are roadmap items. Any other agent connects over MCP using the
generic snippet path.

## Connecting any agent

For any agent in the registry, `pensieve setup` writes the MCP config directly:

```bash
pensieve setup cursor       # writes .cursor/mcp.json in the current project
pensieve setup windsurf     # writes ~/.codeium/windsurf/mcp_config.json
pensieve setup claude-code  # writes .mcp.json in the current project
```

`pensieve setup list` shows the agents pensieve knows about. For an unlisted agent, `--print`
emits a paste-ready stdio snippet without writing any file:

```bash
pensieve setup myagent --print
```

Output shape:

```json
{
  "mcpServers": {
    "pensieve": {
      "type": "stdio",
      "command": "<pensieve binary>",
      "args": ["mcp"]
    }
  }
}
```

Paste that into the agent's MCP config, restart it, and it has the full tool surface.

## Claude Code: the full integration

Claude Code is the most complete integration today. `pensieve install-plugin` wires four hooks:

- **Stop** and **PostToolUse** — capture each turn into the `claude_code_events` firehose.
- **SessionStart** and **UserPromptSubmit** — recall relevant memories and inject them
  into the prompt automatically, with no tool call.

It also installs `/pensieve-recall`, `/pensieve-remember`, `/pensieve-ask`, `/pensieve-ingest`, and
`/pensieve-status` as slash commands.

```bash
pensieve connect <pensieve-url>   # or a local pensieve serve
pensieve install-plugin       # hooks + slash commands land in ~/.claude
```

Full details: [Claude Code plugin](/agent/claude-code-plugin),
[Claude Code engine](/agent/claude-cli).

## Adding an agent

Conceptually: implement `AgentDetector` for a new `kind`, register it in `detectors()`,
and (optionally) write a `source_sync` pipeline that turns that agent's session files into
firehose events. In the meantime, any agent connects over MCP today — no registry entry
required.

## See also

- [Connect your agent](/agent/connect) — choosing a path, fastest MCP setup
- [Connect via CLI](/agent/connect-from-cli) — the full `pensieve recall` / `remember` / `query` surface
- [Claude Code plugin](/agent/claude-code-plugin) — hooks, slash commands, configuration
- [Claude Code engine](/agent/claude-cli) — using the Claude CLI as the pensieve agent engine
- [Workers & nodes](/agent/workers) — how the node daemon uses the registry for source sync
- [Skills](/agent/skills) — the `SKILL.md` format that teaches any agent when to call pensieve
