//! Claude Code memory-file format — the pure library behind kyma's
//! Claude Code file-memory sync.
//!
//! Claude Code keeps a per-project file memory at
//! `~/.claude/projects/<path-slug>/memory/`: a `MEMORY.md` index that is
//! loaded into context every session, plus one `.md` file per memory with
//! YAML frontmatter (`name`, `description`, `metadata.type`,
//! `metadata.originSessionId`) and `[[wikilinks]]` between memories.
//!
//! This crate owns that on-disk format and nothing else: parsing/rendering
//! frontmatter ([`frontmatter`]), the `MEMORY.md` index with kyma's managed
//! region ([`index`]), wikilink extraction ([`wikilink`]), Claude Code path
//! slugs ([`slug`]), and normalized content hashing ([`hash`]). No async, no
//! database, no filesystem — both `kyma-local` (ingest + writeback) and
//! `kyma-server` (curation decisions) depend on it.

#![forbid(unsafe_code)]

pub mod frontmatter;

#[cfg(test)]
mod frontmatter_unit_tests;

pub mod index;

#[cfg(test)]
mod index_unit_tests;

pub mod wikilink;

#[cfg(test)]
mod wikilink_unit_tests;

pub mod slug;

#[cfg(test)]
mod slug_unit_tests;

pub mod hash;

#[cfg(test)]
mod hash_unit_tests;

/// Value of `metadata.source` stamped into files kyma writes (promotion /
/// archive tombstones). The ingester treats files carrying this marker as
/// kyma-authored: skipped when the on-disk hash matches the recorded one,
/// ingested as an UPDATE to the original node when the user edited them.
pub const KYMA_SOURCE_MARKER: &str = "kyma";

/// `provenance.source` value on memory nodes ingested from Claude Code files.
pub const CC_PROVENANCE_SOURCE: &str = "claude-code-file";

/// Prefix of the deterministic upsert key for file-born memories:
/// `claude-md:<path-slug>/<name>`.
pub const TOPIC_KEY_PREFIX: &str = "claude-md:";

/// Opening marker of the kyma-managed region inside `MEMORY.md`.
pub const MANAGED_BEGIN: &str = "<!-- BEGIN kyma-managed -->";

/// Closing marker of the kyma-managed region inside `MEMORY.md`.
pub const MANAGED_END: &str = "<!-- END kyma-managed -->";

/// Subdirectory of `memory/` where curation archives files (never deletes).
pub const ARCHIVE_DIR: &str = "archive";

/// The index file Claude Code loads into context every session.
pub const MEMORY_INDEX_FILE: &str = "MEMORY.md";

/// Build the deterministic upsert key for a file-born memory.
pub fn topic_key(path_slug: &str, name: &str) -> String {
    format!("{TOPIC_KEY_PREFIX}{path_slug}/{name}")
}
