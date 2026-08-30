//! Generated MOC ("map of content") pages: `README.md`, `CONTRIBUTING.md`,
//! the global `index.md`, and per-realm indexes. All pure functions of the
//! included rows — the freshness stamp is the max `updated_at`, never a
//! clock, so an unchanged brain renders byte-identical pages.

use std::collections::BTreeMap;

use crate::registry::{BrainConfig, RealmSelector, VaultLayout};
use crate::types::NoteRow;
use crate::vault::type_folder;

const RECENT_CAP: usize = 10;

fn realms_label(cfg: &BrainConfig) -> String {
    match &cfg.realms {
        RealmSelector::All => "all realms".to_string(),
        RealmSelector::Realms(r) => r.join(", "),
    }
}

fn link(path: &str, title: &str) -> String {
    format!("[[{}|{}]]", path.trim_end_matches(".md"), title)
}

/// Generated `README.md` — what this repo is and the three commands that
/// matter. Deterministic (config-only).
pub fn render_readme(cfg: &BrainConfig) -> String {
    format!(
        "# {name}\n\n\
        This repository is a pensieve **brain**: an Obsidian vault rendered from pensieve's \
        agentic memory (realms: {realms}), regenerated automatically. Clone it, read it, \
        grep it, open it in Obsidian — and push edits back.\n\n\
        ```\n\
        git clone <server>/git/{name}.git    # password = your pensieve API token\n\
        git pull                             # pick up new exports\n\
        git push                             # edits flow back into pensieve memory\n\
        ```\n\n\
        Start at [[index]]. Editing rules live in [[CONTRIBUTING]].\n\n\
        Pushed files are normalized on the next export (canonical frontmatter and \
        formatting), and notes are addressed by the `pensieve_memory_id` in their \
        frontmatter — not by filename. Git history is the changelog and the archive.\n",
        name = cfg.name,
        realms = realms_label(cfg),
    )
}

/// Generated `CONTRIBUTING.md` — the editing contract for humans and agents.
pub fn render_contributing(cfg: &BrainConfig) -> String {
    format!(
        "# Contributing to {name}\n\n\
        ## Edit a note\n\
        Edit the body (and `tags` / `importance` frontmatter) of any note under \
        `notes/`, `entities/`, or `wiki/`, then `git push`. The edit updates the same \
        pensieve memory (keyed by `pensieve_memory_id`); the next export re-renders the note \
        canonically.\n\n\
        ## Add a note\n\
        Drop a markdown file in `inbox/` — frontmatter optional (`title`, `type`, \
        `realm`, `tags`). On push it becomes a new memory; the next export re-files it \
        under `notes/`.\n\n\
        ## Delete a note\n\
        `git rm` + push archives the memory in pensieve (never destroys it) and the file \
        stays gone.\n\n\
        ## Hands off\n\
        Do not edit `index.md`, realm indexes, `.pensieve/`, `pensieve_memory_id` fields, or \
        the `<!-- pensieve:related -->` blocks — they are regenerated every export.\n\n\
        `wiki/` pages are co-owned: the gardener agent curates them, your edits are \
        kept and merged.\n",
        name = cfg.name,
    )
}

/// The global `index.md`: freshness line, counts by type, wiki links,
/// recently updated notes, realm section links.
pub fn render_index(
    cfg: &BrainConfig,
    rows: &[&NoteRow],
    paths: &BTreeMap<String, String>,
) -> String {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for r in rows {
        *counts.entry(type_folder(&r.memory_type)).or_default() += 1;
    }
    let freshest = rows.iter().map(|r| r.updated_at.as_str()).max().unwrap_or("-");

    let mut out = format!("# {}\n\n", cfg.name);
    out.push_str(&format!(
        "> {} notes · realms: {} · memory last updated: {}\n",
        rows.len(),
        realms_label(cfg),
        freshest
    ));
    if !counts.is_empty() {
        let stats: Vec<String> = counts.iter().map(|(k, v)| format!("{k} {v}")).collect();
        out.push_str(&format!("> {}\n", stats.join(" · ")));
    }

    // Wiki pages (gardener MOCs) are the curated entry points.
    let mut wiki: Vec<(&str, &str)> = rows
        .iter()
        .filter_map(|r| {
            let p = paths.get(&r.id)?;
            p.starts_with("wiki/").then_some((p.as_str(), r.title.as_str()))
        })
        .collect();
    wiki.sort_unstable();
    if !wiki.is_empty() {
        out.push_str("\n## Start here\n");
        for (p, title) in wiki {
            out.push_str(&format!("- {}\n", link(p, title)));
        }
    }

    let mut recent: Vec<&&NoteRow> = rows.iter().collect();
    recent.sort_by(|a, b| (&b.updated_at, &b.id).cmp(&(&a.updated_at, &a.id)));
    recent.truncate(RECENT_CAP);
    if !recent.is_empty() {
        out.push_str("\n## Recently updated\n");
        for r in recent {
            if let Some(p) = paths.get(&r.id) {
                out.push_str(&format!("- {} — {}\n", link(p, &r.title), r.updated_at));
            }
        }
    }

    if cfg.layout == VaultLayout::ByRealm {
        let mut realms: Vec<&str> = rows.iter().map(|r| r.realm.as_str()).collect();
        realms.sort_unstable();
        realms.dedup();
        if !realms.is_empty() {
            out.push_str("\n## Realms\n");
            for realm in realms {
                out.push_str(&format!("- [[notes/{realm}/index|{realm}]]\n"));
            }
        }
    }
    out
}

/// Per-realm `notes/<realm>/index.md` pages (multi-realm layout only):
/// notes grouped by type, sorted by title.
pub fn render_realm_indexes(
    cfg: &BrainConfig,
    rows: &[&NoteRow],
    paths: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    if cfg.layout != VaultLayout::ByRealm {
        return Vec::new();
    }
    let mut by_realm: BTreeMap<&str, Vec<&&NoteRow>> = BTreeMap::new();
    for r in rows {
        if r.memory_type != "entity" && paths.get(&r.id).is_some_and(|p| p.starts_with("notes/")) {
            by_realm.entry(r.realm.as_str()).or_default().push(r);
        }
    }
    by_realm
        .into_iter()
        .map(|(realm, mut notes)| {
            notes.sort_by(|a, b| {
                (type_folder(&a.memory_type), &a.title, &a.id)
                    .cmp(&(type_folder(&b.memory_type), &b.title, &b.id))
            });
            let mut out = format!("# {realm}\n");
            let mut current = "";
            for r in notes {
                let folder = type_folder(&r.memory_type);
                if folder != current {
                    out.push_str(&format!("\n## {folder}\n"));
                    current = folder;
                }
                if let Some(p) = paths.get(&r.id) {
                    out.push_str(&format!("- {}\n", link(p, &r.title)));
                }
            }
            (realm.to_string(), out)
        })
        .collect()
}
