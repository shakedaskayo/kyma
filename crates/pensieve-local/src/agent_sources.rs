//! Agent-source detection — a small, extensible registry of coding-agent
//! installations a node can own as local sources.
//!
//! Pensieve is a distributed context engine for *any* coding agent, so the node
//! daemon must not assume Claude Code is the only world. Each detector is a
//! cheap filesystem probe: it reports the agent's root dir(s), the realms it
//! has on disk, and which sessions look active right now. Detectors that find
//! nothing report nothing.
//!
//! Adding an agent is one entry in [`detectors`] — implement [`AgentDetector`]
//! and list it. Today only Claude Code has a real ingestion pipeline; the
//! others are detected (so they appear in node inventory + capabilities) and
//! their `source_sync` jobs no-op until a pipeline lands.

use pensieve_core::fabric::PresenceSession;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Presence cutoff: a session is "active" if its transcript was touched within
/// this window.
const PRESENCE_WINDOW: Duration = Duration::from_secs(600);
/// Cap on presence sessions reported per heartbeat.
const PRESENCE_CAP: usize = 16;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// A detected coding-agent installation on this node.
#[derive(Debug, Clone)]
pub struct DetectedAgent {
    /// Stable kind id, e.g. `"claude-code"`.
    pub kind: &'static str,
    /// Human-friendly name, e.g. `"Claude Code"`.
    pub name: &'static str,
    /// Root dir(s) this agent's state lives under.
    pub roots: Vec<PathBuf>,
    /// Realm names found on disk (sorted).
    pub realms: Vec<String>,
}

impl DetectedAgent {
    fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "name": self.name,
            "roots": self.roots.iter().map(|r| r.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "realms": self.realms,
        })
    }
}

/// A detector for one coding agent. Detection is a pure fs probe — no network,
/// no spawning. `detect` returns `None` when the agent isn't installed.
pub trait AgentDetector: Send + Sync {
    /// Stable kind id (matches [`DetectedAgent::kind`]).
    fn kind(&self) -> &'static str;
    /// Probe the filesystem; `None` if this agent isn't present.
    fn detect(&self) -> Option<DetectedAgent>;
    /// Active sessions observed for this agent right now (transcript mtimes).
    /// Default: none — only agents whose transcript layout we understand
    /// report presence.
    fn presence(&self) -> Vec<PresenceSession> {
        vec![]
    }
}

/// The detector registry. Adding an agent = one entry here.
pub fn detectors() -> Vec<Box<dyn AgentDetector>> {
    vec![
        Box::new(ClaudeCode),
        Box::new(Cursor),
        Box::new(Windsurf),
        Box::new(Codex),
    ]
}

/// Run every detector and collect the agents present on this node.
pub fn detect_all() -> Vec<DetectedAgent> {
    detectors().iter().filter_map(|d| d.detect()).collect()
}

/// Source inventory in the generic `{ "agents": [...] }` shape (plus a
/// `claude_code` compat key when Claude Code is present, for older readers).
pub fn discover_sources() -> Value {
    let agents = detect_all();
    let agents_json: Vec<Value> = agents.iter().map(DetectedAgent::to_json).collect();
    let mut out = json!({ "agents": agents_json });
    // Back-compat: legacy consumers may look for sources.claude_code.{roots,realms}.
    if let Some(cc) = agents.iter().find(|a| a.kind == ClaudeCode.kind()) {
        out["claude_code"] = json!({
            "roots": cc.roots.iter().map(|r| r.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "realms": cc.realms,
        });
    }
    out
}

/// Aggregate active sessions across every detected agent.
pub fn discover_presence() -> Vec<PresenceSession> {
    let mut out = vec![];
    for d in detectors() {
        // Only probe presence for agents that are actually installed.
        if d.detect().is_some() {
            out.extend(d.presence());
        }
    }
    out.truncate(PRESENCE_CAP);
    out
}

/// Per-kind capability advertised on registration (e.g. `source:claude-code`).
pub fn capability_for(kind: &str) -> String {
    format!("source:{kind}")
}

// ── shared helpers ───────────────────────────────────────────────────────────

/// List immediate subdirectory names under `root` (sorted) — the common
/// "one realm per directory" layout.
fn realm_dirs(root: &PathBuf) -> Vec<String> {
    let mut realms = vec![];
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    realms.push(name.to_string());
                }
            }
        }
    }
    realms.sort();
    realms
}

/// Recently-modified transcript files become presence sessions. `root` holds
/// one realm subdir per project; each realm subdir holds `*.<ext>` transcripts.
/// `kind` tags each session with its owning agent.
fn presence_from_realm_transcripts(
    root: &PathBuf,
    ext: &str,
    kind: &'static str,
) -> Vec<PresenceSession> {
    let cutoff = SystemTime::now() - PRESENCE_WINDOW;
    let mut out = vec![];
    let Ok(projects) = std::fs::read_dir(root) else {
        return out;
    };
    for proj in projects.flatten() {
        if !proj.path().is_dir() {
            continue;
        }
        let realm = proj.file_name().to_string_lossy().to_string();
        let Ok(files) = std::fs::read_dir(proj.path()) else {
            continue;
        };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().and_then(|e| e.to_str()) != Some(ext) {
                continue;
            }
            let Ok(meta) = f.metadata() else { continue };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if modified >= cutoff {
                out.push(PresenceSession {
                    session_id: path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    realm: Some(realm.clone()),
                    agent: Some(kind.to_string()),
                    last_activity: chrono::DateTime::from(modified),
                });
            }
        }
    }
    out
}

// ── detectors ──────────────────────────────────────────────────────────────

/// Claude Code — `~/.claude/projects/<realm>/*.jsonl`. The one agent with a
/// working ingestion pipeline today.
struct ClaudeCode;

impl ClaudeCode {
    fn root(&self) -> Option<PathBuf> {
        home().map(|h| h.join(".claude").join("projects"))
    }
}

impl AgentDetector for ClaudeCode {
    fn kind(&self) -> &'static str {
        "claude-code"
    }
    fn detect(&self) -> Option<DetectedAgent> {
        let root = self.root()?;
        if !root.is_dir() {
            return None;
        }
        Some(DetectedAgent {
            kind: self.kind(),
            name: "Claude Code",
            realms: realm_dirs(&root),
            roots: vec![root],
        })
    }
    fn presence(&self) -> Vec<PresenceSession> {
        let Some(root) = self.root() else {
            return vec![];
        };
        presence_from_realm_transcripts(&root, "jsonl", self.kind())
    }
}

/// Cursor — `~/.cursor` if present. Detection only (no ingest pipeline yet).
struct Cursor;

impl AgentDetector for Cursor {
    fn kind(&self) -> &'static str {
        "cursor"
    }
    fn detect(&self) -> Option<DetectedAgent> {
        let root = home()?.join(".cursor");
        if !root.is_dir() {
            return None;
        }
        Some(DetectedAgent {
            kind: self.kind(),
            name: "Cursor",
            realms: vec![],
            roots: vec![root],
        })
    }
}

/// Windsurf (Codeium) — `~/.codeium/windsurf` if present. Detection only.
struct Windsurf;

impl AgentDetector for Windsurf {
    fn kind(&self) -> &'static str {
        "windsurf"
    }
    fn detect(&self) -> Option<DetectedAgent> {
        let root = home()?.join(".codeium").join("windsurf");
        if !root.is_dir() {
            return None;
        }
        Some(DetectedAgent {
            kind: self.kind(),
            name: "Windsurf",
            realms: vec![],
            roots: vec![root],
        })
    }
}

/// Codex — `~/.codex/sessions` (preferred) or `~/.codex`. Detection only.
struct Codex;

impl AgentDetector for Codex {
    fn kind(&self) -> &'static str {
        "codex"
    }
    fn detect(&self) -> Option<DetectedAgent> {
        let base = home()?.join(".codex");
        if !base.is_dir() {
            return None;
        }
        let sessions = base.join("sessions");
        let root = if sessions.is_dir() { sessions } else { base };
        Some(DetectedAgent {
            kind: self.kind(),
            name: "Codex",
            realms: vec![],
            roots: vec![root],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_format() {
        assert_eq!(capability_for("claude-code"), "source:claude-code");
        assert_eq!(capability_for("cursor"), "source:cursor");
    }

    #[test]
    fn sources_shape_has_agents_array() {
        // Whatever is (or isn't) installed, the top-level shape is stable:
        // an object with an `agents` array.
        let v = discover_sources();
        assert!(v.get("agents").and_then(|a| a.as_array()).is_some());
    }

    #[test]
    fn detected_agent_json_shape() {
        let a = DetectedAgent {
            kind: "claude-code",
            name: "Claude Code",
            roots: vec![PathBuf::from("/home/x/.claude/projects")],
            realms: vec!["my-realm".to_string()],
        };
        let j = a.to_json();
        assert_eq!(j["kind"], "claude-code");
        assert_eq!(j["name"], "Claude Code");
        assert_eq!(j["roots"][0], "/home/x/.claude/projects");
        assert_eq!(j["realms"][0], "my-realm");
    }

    #[test]
    fn realm_dirs_lists_sorted_subdirs() {
        let tmp = std::env::temp_dir().join(format!("pensieve-realms-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join("zeta")).unwrap();
        std::fs::create_dir_all(tmp.join("alpha")).unwrap();
        std::fs::write(tmp.join("a-file"), b"x").unwrap(); // files are ignored
        let realms = realm_dirs(&tmp);
        assert_eq!(realms, vec!["alpha".to_string(), "zeta".to_string()]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn presence_picks_up_recent_transcripts() {
        let tmp = std::env::temp_dir().join(format!("pensieve-presence-{}", uuid::Uuid::new_v4()));
        let realm = tmp.join("proj-a");
        std::fs::create_dir_all(&realm).unwrap();
        std::fs::write(realm.join("sess-1.jsonl"), b"{}").unwrap();
        std::fs::write(realm.join("ignored.txt"), b"x").unwrap();
        let sessions = presence_from_realm_transcripts(&tmp, "jsonl", "claude-code");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-1");
        assert_eq!(sessions[0].realm.as_deref(), Some("proj-a"));
        assert_eq!(sessions[0].agent.as_deref(), Some("claude-code"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
