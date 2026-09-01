//! `MEMORY.md` index with a surgical pensieve-managed region.
//!
//! Claude Code loads this file into context every session, and humans (and
//! Claude itself) write entries to it. pensieve therefore only ever rewrites the
//! region between [`crate::MANAGED_BEGIN`] and [`crate::MANAGED_END`];
//! everything outside round-trips byte-for-byte. When no markers exist yet,
//! the managed block is appended at the end — user content is never
//! reordered.

use crate::{MANAGED_BEGIN, MANAGED_END};

/// One bullet inside the managed region: `- [title](file) — hook`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedEntry {
    pub title: String,
    /// Relative filename, e.g. `pensieve-auth-model.md`.
    pub file: String,
    /// Short description after the em-dash.
    pub hook: String,
}

/// A parsed `MEMORY.md`: user content before/after the managed region plus
/// the managed entries themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryIndex {
    before: String,
    managed: Vec<ManagedEntry>,
    after: String,
    had_markers: bool,
}

fn parse_entry(line: &str) -> Option<ManagedEntry> {
    let rest = line.trim().strip_prefix("- [")?;
    let (title, rest) = rest.split_once("](")?;
    let (file, rest) = rest.split_once(')')?;
    let hook = rest
        .trim_start()
        .strip_prefix('—')
        .map_or_else(|| rest.trim(), str::trim_start);
    Some(ManagedEntry {
        title: title.to_string(),
        file: file.to_string(),
        hook: hook.to_string(),
    })
}

impl MemoryIndex {
    /// An index for a project that has no `MEMORY.md` yet.
    pub fn new_empty() -> Self {
        MemoryIndex {
            before: "# Memory index\n".to_string(),
            managed: Vec::new(),
            after: String::new(),
            had_markers: false,
        }
    }

    /// Parse an existing `MEMORY.md`. Never fails: content with no markers
    /// becomes `before` verbatim.
    pub fn parse(raw: &str) -> Self {
        match (raw.find(MANAGED_BEGIN), raw.find(MANAGED_END)) {
            (Some(b), Some(e)) if b < e => {
                let inner = &raw[b + MANAGED_BEGIN.len()..e];
                MemoryIndex {
                    before: raw[..b].to_string(),
                    managed: inner.lines().filter_map(parse_entry).collect(),
                    after: raw[e + MANAGED_END.len()..].to_string(),
                    had_markers: true,
                }
            }
            _ => MemoryIndex {
                before: raw.to_string(),
                managed: Vec::new(),
                after: String::new(),
                had_markers: false,
            },
        }
    }

    /// Replace the managed region's entries.
    pub fn set_managed(&mut self, entries: Vec<ManagedEntry>) {
        self.managed = entries;
    }

    /// Current managed entries.
    pub fn managed(&self) -> &[ManagedEntry] {
        &self.managed
    }

    /// Filenames referenced by bullets in the *user-owned* part of the index
    /// (outside the managed region). Used to defer to user entries instead of
    /// double-listing a file pensieve manages.
    pub fn user_files(&self) -> Vec<String> {
        self.before
            .lines()
            .chain(self.after.lines())
            .filter_map(parse_entry)
            .map(|e| e.file)
            .collect()
    }

    /// Render the full file. Pure function of the parsed state: rendering a
    /// parsed file with unchanged entries is byte-identical to the input.
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        if self.had_markers {
            out.push_str(&self.before);
        } else {
            let trimmed = self.before.trim_end();
            if trimmed.is_empty() {
                out.push_str("# Memory index");
            } else {
                out.push_str(trimmed);
            }
            out.push_str("\n\n");
        }
        out.push_str(MANAGED_BEGIN);
        out.push('\n');
        for e in &self.managed {
            let _ = writeln!(out, "- [{}]({}) — {}", e.title, e.file, e.hook);
        }
        out.push_str(MANAGED_END);
        if self.had_markers {
            out.push_str(&self.after);
        } else {
            out.push('\n');
        }
        out
    }
}
