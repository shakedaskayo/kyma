//! Push-ingest planning: turn a `git diff` between the pre- and post-push
//! branch tips into memory operations. Pure — the server glue reads blobs,
//! applies the ops through `MemoryWriter`, and records the run.

use std::collections::BTreeMap;

use crate::notes::{self, ParsedNote};
use crate::registry::{BrainConfig, RealmSelector, VaultLayout};
use crate::gitbin::ChangeKind;

/// One memory operation derived from a pushed change.
#[derive(Debug, Clone, PartialEq)]
pub enum IngestOp {
    /// Note carries a `pensieve_memory_id` from a previous export → update that
    /// memory in place (new version, same id).
    UpdateExisting { memory_id: String, rel_path: String, note: ParsedNote },
    /// New note (no id) → create a memory keyed by `brain:<name>:<path>`.
    CreateNew { topic_key: String, realm: String, rel_path: String, note: ParsedNote },
    /// Exported note deleted → archive the memory (never destroy).
    ArchiveDeleted { memory_id: String, rel_path: String },
}

/// The plan plus per-file warnings (parse failures etc. — a push never
/// fails because of note content).
#[derive(Debug, Clone, Default)]
pub struct IngestPlan {
    pub ops: Vec<IngestOp>,
    pub warnings: Vec<String>,
}

/// Derive the realm of a new note from its path (multi-realm layout puts
/// realm as the folder under `notes/`), falling back to the brain default.
fn realm_for_path(cfg: &BrainConfig, rel_path: &str) -> String {
    if cfg.layout == VaultLayout::ByRealm {
        if let Some(rest) = rel_path.strip_prefix("notes/") {
            if let Some((realm, _)) = rest.split_once('/') {
                if realm != "index.md" && !realm.is_empty() {
                    return realm.to_string();
                }
            }
        }
    }
    match &cfg.realms {
        RealmSelector::Realms(r) => r.first().cloned().unwrap_or_else(|| "default".to_string()),
        RealmSelector::All => "default".to_string(),
    }
}

fn is_note_path(path: &str) -> bool {
    path.ends_with(".md")
        && !path.starts_with(".pensieve/")
        && !path.starts_with(".obsidian/")
        && path != "README.md"
        && path != "CONTRIBUTING.md"
        && path != "index.md"
        && path != "inbox/README.md"
        && !path.ends_with("/index.md")
}

/// Plan the ingest for one push. `changes` is the old..new diff;
/// `read_new(path)` returns the blob at the new tip; `prior_ids` maps
/// exported paths to memory ids (from the pre-push manifest).
pub fn plan_push_ingest(
    cfg: &BrainConfig,
    changes: &[(ChangeKind, String)],
    read_new: impl Fn(&str) -> Option<Vec<u8>>,
    prior_ids: &BTreeMap<String, String>,
) -> IngestPlan {
    let mut plan = IngestPlan::default();

    for (kind, path) in changes {
        if !is_note_path(path) {
            // Non-note files ride along in git (preserved by the exporter)
            // but never touch memory. Pushes that try to rewrite the
            // manifest are ignored the same way — the next export rewrites it.
            continue;
        }
        match kind {
            ChangeKind::Deleted => {
                if let Some(id) = prior_ids.get(path) {
                    plan.ops.push(IngestOp::ArchiveDeleted {
                        memory_id: id.clone(),
                        rel_path: path.clone(),
                    });
                }
                // Deleting a never-exported file (e.g. an inbox note pushed
                // and removed in a later push before an export ran) has no
                // memory to archive when it's not in the manifest; the
                // brain: topic_key memory (if one was created) stays —
                // acceptable and rare; surfaced as a warning.
                else {
                    plan.warnings.push(format!("{path}: deleted file had no exported memory"));
                }
            }
            ChangeKind::Added | ChangeKind::Modified => {
                let Some(bytes) = read_new(path) else {
                    plan.warnings.push(format!("{path}: unreadable at new tip, skipped"));
                    continue;
                };
                let Ok(text) = String::from_utf8(bytes) else {
                    plan.warnings.push(format!("{path}: not UTF-8, skipped"));
                    continue;
                };
                let note = notes::parse_note(&text);
                let claimed_id = note.pensieve_memory_id.clone();
                let manifest_id = prior_ids.get(path).cloned();

                match (claimed_id, manifest_id) {
                    // Normal edit of an exported note.
                    (Some(claimed), Some(known)) if claimed == known => {
                        plan.ops.push(IngestOp::UpdateExisting {
                            memory_id: known,
                            rel_path: path.clone(),
                            note,
                        });
                    }
                    // Forged/foreign id on an exported path — refuse the
                    // claim, treat as edit of the note that lives there.
                    (Some(_), Some(known)) => {
                        plan.warnings.push(format!(
                            "{path}: pensieve_memory_id does not match the exported note; keeping original identity"
                        ));
                        plan.ops.push(IngestOp::UpdateExisting {
                            memory_id: known,
                            rel_path: path.clone(),
                            note,
                        });
                    }
                    // Id on a new path: someone copied a note (or moved it).
                    // Trust the id — updating the same memory is the
                    // rename-safe behavior.
                    (Some(claimed), None) => {
                        plan.ops.push(IngestOp::UpdateExisting {
                            memory_id: claimed,
                            rel_path: path.clone(),
                            note,
                        });
                    }
                    // Plain new note.
                    (None, None) => {
                        let realm = note
                            .realm
                            .clone()
                            .unwrap_or_else(|| realm_for_path(cfg, path));
                        plan.ops.push(IngestOp::CreateNew {
                            topic_key: crate::topic_key(&cfg.name, path),
                            realm,
                            rel_path: path.clone(),
                            note,
                        });
                    }
                    // Exported note whose id line was stripped: keep identity.
                    (None, Some(known)) => {
                        plan.warnings.push(format!(
                            "{path}: pensieve_memory_id removed by edit; keeping original identity"
                        ));
                        plan.ops.push(IngestOp::UpdateExisting {
                            memory_id: known,
                            rel_path: path.clone(),
                            note,
                        });
                    }
                }
            }
        }
    }
    plan
}
