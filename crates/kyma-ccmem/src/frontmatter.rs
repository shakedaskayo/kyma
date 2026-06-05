//! Parse and render the YAML frontmatter of a Claude Code memory file.
//!
//! Tolerant on the way in (malformed YAML or missing fence → `None`, callers
//! skip the file), deterministic on the way out (`render` is a pure function
//! of [`MemoryFile`], so re-renders are byte-identical — the idempotency
//! anchor for curation).

use serde_yaml::{Mapping, Value};

/// The frontmatter fields kyma reads and writes. Claude Code's own schema is
/// `name`/`description`/`metadata.{type, originSessionId}`; the `kyma_*`,
/// `source`, `content_hash` and `archived_*` fields are kyma's additions on
/// promoted files and archive tombstones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    /// Stable identity of the memory (kebab slug). Claude Code always writes
    /// it; tolerated as `None` for hand-made files (callers fall back to the
    /// file stem).
    pub name: Option<String>,
    /// One-line summary; becomes the kyma memory title.
    pub description: Option<String>,
    /// `metadata.type`: user | feedback | project | reference.
    pub cc_type: Option<String>,
    /// `metadata.originSessionId`.
    pub origin_session_id: Option<String>,
    /// `metadata.source` — [`crate::KYMA_SOURCE_MARKER`] on kyma-authored files.
    pub source: Option<String>,
    /// `metadata.kyma_memory_id` — back-pointer (`memory:<uuid>`) on promoted files.
    pub kyma_memory_id: Option<String>,
    /// `metadata.content_hash` — hash of the body as last written by kyma;
    /// a mismatch on disk means the user edited the file.
    pub content_hash: Option<String>,
    /// `metadata.archived_at` — set on archive tombstones.
    pub archived_at: Option<String>,
    /// `metadata.archived_reason` — set on archive tombstones.
    pub archived_reason: Option<String>,
}

/// A parsed memory file: frontmatter + markdown body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryFile {
    pub front: Frontmatter,
    /// Markdown body after the closing `---` fence, leading newline trimmed.
    pub body: String,
}

impl MemoryFile {
    /// True when the file was written by kyma (promotion or tombstone).
    pub fn is_kyma_authored(&self) -> bool {
        self.front.source.as_deref() == Some(crate::KYMA_SOURCE_MARKER)
    }
}

fn get_str(map: &Mapping, key: &str) -> Option<String> {
    map.get(Value::String(key.to_string()))?
        .as_str()
        .map(str::to_string)
}

/// Parse a raw memory file. Returns `None` when there is no frontmatter
/// fence or the YAML is malformed — callers log and skip.
pub fn parse(raw: &str) -> Option<MemoryFile> {
    let rest = raw.strip_prefix("---\n")?;
    let (yaml, body) = if let Some(i) = rest.find("\n---\n") {
        (&rest[..i], &rest[i + 5..])
    } else {
        let i = rest.find("\n---")?;
        if !rest[i + 4..].trim().is_empty() {
            return None;
        }
        (&rest[..i], "")
    };

    let val: Value = serde_yaml::from_str(yaml).ok()?;
    let map = val.as_mapping()?;
    let meta = map
        .get(Value::String("metadata".to_string()))
        .and_then(Value::as_mapping);

    let from_meta = |key: &str| meta.and_then(|m| get_str(m, key));

    Some(MemoryFile {
        front: Frontmatter {
            name: get_str(map, "name"),
            description: get_str(map, "description"),
            cc_type: from_meta("type"),
            origin_session_id: from_meta("originSessionId"),
            source: from_meta("source"),
            kyma_memory_id: from_meta("kyma_memory_id"),
            content_hash: from_meta("content_hash"),
            archived_at: from_meta("archived_at"),
            archived_reason: from_meta("archived_reason"),
        },
        body: body.trim_start_matches('\n').to_string(),
    })
}

/// Render a memory file deterministically (fixed field order, absent fields
/// omitted). `parse(render(f))` round-trips.
pub fn render(file: &MemoryFile) -> String {
    let f = &file.front;
    let mut top = Mapping::new();
    let put = |m: &mut Mapping, key: &str, v: &Option<String>| {
        if let Some(v) = v {
            m.insert(
                Value::String(key.to_string()),
                Value::String(v.clone()),
            );
        }
    };
    put(&mut top, "name", &f.name);
    put(&mut top, "description", &f.description);

    let mut meta = Mapping::new();
    put(&mut meta, "type", &f.cc_type);
    put(&mut meta, "originSessionId", &f.origin_session_id);
    put(&mut meta, "source", &f.source);
    put(&mut meta, "kyma_memory_id", &f.kyma_memory_id);
    put(&mut meta, "content_hash", &f.content_hash);
    put(&mut meta, "archived_at", &f.archived_at);
    put(&mut meta, "archived_reason", &f.archived_reason);
    if !meta.is_empty() {
        top.insert(
            Value::String("metadata".to_string()),
            Value::Mapping(meta),
        );
    }

    let yaml = serde_yaml::to_string(&Value::Mapping(top)).unwrap_or_default();
    format!("---\n{yaml}---\n\n{}", file.body)
}
