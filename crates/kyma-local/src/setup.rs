//! `kyma setup <agent>` — wire a coding agent to this binary over stdio
//! MCP, so it gets the full context-engine toolset (memory + data + graph) with
//! zero infra. One-liner onboarding, the engram-style `setup <agent>`.
//!
//! Writes (merging, never clobbering other servers) the agent's MCP config with
//! a `kyma` server pointing at `<this binary> mcp`. Agents whose config format
//! we don't write are handled by `--print` / the generic snippet.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

/// A known agent and where its stdio-MCP config lives.
struct Target {
    key: &'static str,
    label: &'static str,
    /// `true` → path is relative to the current project (cwd); `false` → under `$HOME`.
    project: bool,
    rel: &'static str,
}

/// Agents that use the conventional `{"mcpServers": {...}}` stdio config.
const TARGETS: &[Target] = &[
    Target { key: "claude-code", label: "Claude Code", project: true, rel: ".mcp.json" },
    Target { key: "cursor", label: "Cursor", project: true, rel: ".cursor/mcp.json" },
    Target {
        key: "windsurf",
        label: "Windsurf",
        project: false,
        rel: ".codeium/windsurf/mcp_config.json",
    },
];

fn supported() -> String {
    TARGETS.iter().map(|t| t.key).collect::<Vec<_>>().join(", ")
}

/// Configure `agent` to use `kyma mcp` over stdio. `print` previews the
/// resulting config instead of writing it.
pub fn run(agent: &str, print: bool) -> Result<()> {
    if agent.eq_ignore_ascii_case("list") {
        eprintln!("kyma setup <agent> — supported: {}", supported());
        eprintln!("Any other agent: `kyma setup <agent> --print` emits a stdio MCP snippet to paste.");
        return Ok(());
    }

    let exe = std::env::current_exe()
        .context("resolving the kyma binary path")?
        .to_string_lossy()
        .to_string();
    // The `kyma` MCP server entry — `<this binary> mcp` over stdio.
    let server = json!({ "type": "stdio", "command": exe, "args": ["mcp"] });

    let Some(t) = TARGETS.iter().find(|t| t.key.eq_ignore_ascii_case(agent)) else {
        // Unknown agent: emit the snippet for the user to paste into its config.
        let snippet = json!({ "mcpServers": { "kyma": server } });
        eprintln!("'{agent}' has no auto-writer (supported: {}).", supported());
        eprintln!("Add this stdio MCP server to its config:");
        println!("{}", serde_json::to_string_pretty(&snippet)?);
        return Ok(());
    };

    let path: PathBuf = if t.project {
        PathBuf::from(t.rel)
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(t.rel)
    };

    // Merge into an existing `{"mcpServers": {...}}` (preserving other servers)
    // or create it fresh.
    let mut root: Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().expect("root is an object");
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    servers
        .as_object_mut()
        .expect("mcpServers is an object")
        .insert("kyma".into(), server);

    let pretty = serde_json::to_string_pretty(&root)?;
    if print {
        eprintln!("# {} → {}", t.label, path.display());
        println!("{pretty}");
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&path, format!("{pretty}\n"))
        .with_context(|| format!("writing {}", path.display()))?;
    eprintln!("✓ Configured {} → {}", t.label, path.display());
    eprintln!("  MCP server 'kyma' → {} mcp (stdio)", "kyma");
    eprintln!("  Restart {} to pick it up; then memory + data + graph tools are available.", t.label);
    Ok(())
}
