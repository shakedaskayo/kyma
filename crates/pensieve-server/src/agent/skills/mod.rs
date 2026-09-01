//! Skill upstream registry — discovers markdown skill files on the host and
//! tracks which ones the user has enabled for agent injection.
//!
//! See docs/superpowers/specs/2026-05-28-agent-engine-and-skills-design.md
//! §3 (Phase A3) for the design. v1 ships local discovery + Postgres-backed
//! enabled-set storage; remote/git sources arrive in A4.

use serde::Serialize;

pub mod local;
pub mod store;

pub use local::LocalSkillSource;
pub use store::{EnabledSkillsStore, PgEnabledSkillsStore};

/// One discovered skill — frontmatter + body, plus the path it came from so
/// the UI can distinguish "user-installed" from "plugin" from "project".
#[derive(Debug, Clone, Serialize)]
pub struct SkillSpec {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: SkillSource,
    /// Filesystem path the skill was loaded from (for the UI tooltip and
    /// the future "reveal in Finder" affordance).
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// `${CWD}/.claude/skills/`
    Project,
    /// `$HOME/.claude/skills/`
    User,
    /// `$HOME/.claude/plugins/cache/<plugin>/<version>/skills/`
    Plugin,
}

impl SkillSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Plugin => "plugin",
        }
    }
}

/// Build the v1 set of skills available to the agent: every skill the
/// discovery walker found, deduplicated by name. First-match-wins on
/// collision (so a project skill shadows a user skill of the same name).
pub fn discover_all() -> Vec<SkillSpec> {
    LocalSkillSource::new().discover()
}
