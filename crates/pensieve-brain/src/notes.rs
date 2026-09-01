//! Per-memory note rendering and parsing — the brain vault's on-disk note
//! format.
//!
//! Flat, Obsidian-native YAML frontmatter (deterministic key order, absent
//! fields omitted), an H1 title, the memory content, and a pensieve-managed
//! `Related` block generated from graph edges. Everything outside managed
//! blocks is the *editable region*; `content_hash` covers it so push-ingest
//! can tell real edits from formatting churn.

use serde_yaml::Value;

use crate::types::NoteRow;

/// Opening marker of the generated Related block.
pub const RELATED_BEGIN: &str = "<!-- pensieve:related:begin -->";
/// Closing marker of the generated Related block.
pub const RELATED_END: &str = "<!-- pensieve:related:end -->";

/// One wikilink target in the Related block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedLink {
    /// Repo-relative path without the `.md` extension (Obsidian link target).
    pub target: String,
    /// Display text (the related note's current title).
    pub title: String,
    /// Edge type, e.g. `RELATES_TO`.
    pub edge_type: String,
}

/// A parsed note (from a pushed file). Unknown frontmatter keys are ignored.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedNote {
    pub title: Option<String>,
    pub pensieve_memory_id: Option<String>,
    pub memory_type: Option<String>,
    pub realm: Option<String>,
    pub tags: Vec<String>,
    pub importance: Option<f64>,
    pub content_hash: Option<String>,
    /// Editable body: everything after the frontmatter, managed blocks
    /// removed, H1 title line removed, trimmed.
    pub body: String,
    /// Wikilink targets found in the editable body (normalized note stems).
    pub links: Vec<String>,
}

/// Strip `<private>…</private>` spans (including the tags) from content.
/// Unclosed spans are stripped to the end.
pub fn strip_private_spans(s: &str) -> String {
    const OPEN: &str = "<private>";
    const CLOSE: &str = "</private>";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(OPEN) {
        out.push_str(&rest[..i]);
        rest = &rest[i + OPEN.len()..];
        match rest.find(CLOSE) {
            Some(j) => rest = &rest[j + CLOSE.len()..],
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

/// Kebab slug of a title (no `pensieve-` prefix, unlike ccmem's
/// `memory_filename` — the whole repo is pensieve-managed).
pub fn title_slug(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut prev_dash = true;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { "note".to_string() } else { slug.to_string() }
}

/// Windows-reserved device names (clones land on Windows machines even
/// though the server is Unix).
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Filename stem for a note: `<title-slug>-<uuid-first-8>`. Minted once —
/// callers keep the prior path from the manifest when the memory already
/// has one. Truncated so path components stay portable.
pub fn note_stem(title: &str, memory_id: &str) -> String {
    let mut slug = title_slug(title);
    slug.truncate(80);
    let slug = slug.trim_end_matches('-');
    let slug = if WINDOWS_RESERVED.contains(&slug) { "note" } else { slug };
    let id8: String = memory_id.chars().filter(|c| *c != '-').take(8).collect();
    let id8 = if id8.is_empty() { "00000000".to_string() } else { id8 };
    format!("{slug}-{id8}")
}

/// Hash of the editable region (title + body without managed blocks).
/// Push-ingest compares it to the frontmatter value to detect real edits.
pub fn content_hash(title: &str, editable_body: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(title.as_bytes());
    hasher.update(b"\n");
    hasher.update(normalize_body(editable_body).as_bytes());
    hasher.finalize().to_hex()[..16].to_string()
}

fn normalize_body(s: &str) -> String {
    let joined: Vec<&str> = s.lines().map(str::trim_end).collect();
    joined.join("\n").trim().to_string()
}

/// Remove every managed block (`RELATED_BEGIN…RELATED_END`) from a body.
pub fn strip_managed_blocks(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find(RELATED_BEGIN) {
        out.push_str(&rest[..i]);
        rest = &rest[i + RELATED_BEGIN.len()..];
        match rest.find(RELATED_END) {
            Some(j) => rest = &rest[j + RELATED_END.len()..],
            None => rest = "",
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

fn yaml_scalar(s: &str) -> String {
    let needs_quoting = s.is_empty()
        || s.contains(|c: char| ":#{}[]&*!|>'\"%@`,\n\t".contains(c))
        || s.starts_with(|c: char| c.is_whitespace() || "-?".contains(c))
        || s.ends_with(char::is_whitespace)
        || matches!(s, "true" | "false" | "null" | "yes" | "no" | "on" | "off")
        || s.parse::<f64>().is_ok();
    if needs_quoting {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn yaml_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| yaml_scalar(s)).collect();
    format!("[{}]", quoted.join(", "))
}

fn format_importance(v: f64) -> String {
    // Fixed two decimals: deterministic and round-trip stable.
    format!("{v:.2}")
}

/// Render a note deterministically. `related` must already be sorted by the
/// caller (the vault planner sorts by edge type, then target).
pub fn render_note(row: &NoteRow, related: &[RelatedLink], redact_private: bool) -> String {
    let content = if redact_private {
        strip_private_spans(&row.content)
    } else {
        row.content.clone()
    };
    let content = content.trim();
    let tags: Vec<String> = row
        .tags
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();

    let editable = content;
    let hash = content_hash(&row.title, editable);

    let mut fm = String::new();
    fm.push_str(&format!("title: {}\n", yaml_scalar(&row.title)));
    fm.push_str(&format!("pensieve_memory_id: {}\n", row.id));
    fm.push_str(&format!("type: {}\n", row.memory_type));
    fm.push_str(&format!("realm: {}\n", yaml_scalar(&row.realm)));
    if !tags.is_empty() {
        fm.push_str(&format!("tags: {}\n", yaml_list(&tags)));
    }
    fm.push_str(&format!("importance: {}\n", format_importance(row.importance)));
    fm.push_str(&format!("status: {}\n", row.status));
    fm.push_str(&format!("created: {}\n", row.created_at));
    fm.push_str(&format!("updated: {}\n", row.updated_at));
    if let Some(v) = &row.valid_at {
        fm.push_str(&format!("valid_from: {v}\n"));
    }
    if let Some(v) = &row.invalid_at {
        fm.push_str(&format!("valid_until: {v}\n"));
    }
    fm.push_str(&format!("aliases: {}\n", yaml_list(std::slice::from_ref(&row.title))));
    fm.push_str(&format!("content_hash: {hash}\n"));

    let mut out = format!("---\n{fm}---\n\n# {}\n\n{editable}\n", row.title);
    if !related.is_empty() {
        out.push_str(&format!("\n{RELATED_BEGIN}\n## Related\n"));
        for link in related {
            out.push_str(&format!(
                "- [[{}|{}]] — {}\n",
                link.target, link.title, link.edge_type
            ));
        }
        out.push_str(&format!("{RELATED_END}\n"));
    }
    out
}

fn value_str(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    match map.get(Value::String(key.to_string()))? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Parse a pushed note file. Returns `None` when there is no frontmatter
/// fence at all (plain markdown is still ingestable — callers treat the
/// whole file as body with defaults); malformed YAML inside a fence also
/// yields `None` from the frontmatter but keeps the body.
pub fn parse_note(raw: &str) -> ParsedNote {
    let (front, body_raw) = split_frontmatter(raw);
    let mut note = ParsedNote::default();

    if let Some(yaml) = front {
        if let Ok(Value::Mapping(map)) = serde_yaml::from_str::<Value>(&yaml) {
            note.title = value_str(&map, "title").or_else(|| value_str(&map, "name"));
            note.pensieve_memory_id = value_str(&map, "pensieve_memory_id");
            note.memory_type = value_str(&map, "type");
            note.realm = value_str(&map, "realm");
            note.content_hash = value_str(&map, "content_hash");
            note.importance = map
                .get(Value::String("importance".into()))
                .and_then(Value::as_f64);
            if let Some(Value::Sequence(seq)) = map.get(Value::String("tags".into())) {
                note.tags = seq
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
            } else if let Some(Value::String(s)) = map.get(Value::String("tags".into())) {
                note.tags = s.split(',').map(str::trim).map(str::to_string).collect();
            }
        }
    }

    let stripped = strip_managed_blocks(body_raw);
    // Drop a leading H1 that duplicates the title.
    let mut lines = stripped.lines().peekable();
    let mut body_lines: Vec<&str> = Vec::new();
    if let Some(first) = lines.peek() {
        if let Some(h1) = first.strip_prefix("# ") {
            if note.title.is_none() {
                note.title = Some(h1.trim().to_string());
            }
            if note.title.as_deref() == Some(h1.trim()) {
                lines.next();
            }
        }
    }
    body_lines.extend(lines);
    note.body = body_lines.join("\n").trim().to_string();
    note.links = pensieve_ccmem::wikilink::extract_normalized(&note.body);
    note
}

fn split_frontmatter(raw: &str) -> (Option<String>, &str) {
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some(i) = rest.find("\n---\n") {
            return (Some(rest[..i].to_string()), &rest[i + 5..]);
        }
        if let Some(i) = rest.find("\n---") {
            if rest[i + 4..].trim().is_empty() {
                return (Some(rest[..i].to_string()), "");
            }
        }
    }
    (None, raw)
}
