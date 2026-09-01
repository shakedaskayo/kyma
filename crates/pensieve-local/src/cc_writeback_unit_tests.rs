use std::path::Path;
use std::time::Duration;

use crate::cc_writeback::{apply_actions, WritebackConfig};
use pensieve_server::agent::cc_curate::{FileAction, IndexEntry};

fn write(p: &Path, content: &str) {
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, content).expect("write");
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// A rendered pensieve file whose stamp matches its body (untouched on disk).
fn promoted_file(name: &str, body: &str) -> (String, String) {
    let hash = pensieve_ccmem::hash::content_hash(name, Some("project"), body);
    let content = pensieve_ccmem::frontmatter::render(&pensieve_ccmem::frontmatter::MemoryFile {
        front: pensieve_ccmem::frontmatter::Frontmatter {
            name: Some(name.to_string()),
            description: Some("desc".to_string()),
            cc_type: Some("project".to_string()),
            source: Some("pensieve".to_string()),
            pensieve_memory_id: Some("memory:abc".to_string()),
            content_hash: Some(hash.clone()),
            ..pensieve_ccmem::frontmatter::Frontmatter::default()
        },
        body: body.to_string(),
    });
    (content, hash)
}

fn write_action(file: &str, content: &str, hash: &str) -> FileAction {
    FileAction::WriteMemoryFile {
        file: file.to_string(),
        content: content.to_string(),
        node_id: "memory:abc".to_string(),
        content_hash: hash.to_string(),
    }
}

const NOW: &str = "2026-06-05T12:00:00+00:00";

#[test]
fn writes_archives_and_reindexes_idempotently() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    write(
        &mem.join("MEMORY.md"),
        "# Memory index\n\n- [User entry](user-note.md) — keep me\n",
    );
    write(
        &mem.join("old-note.md"),
        "---\nname: old-note\nmetadata:\n  type: project\n---\n\nThis was true once.\n",
    );

    let (content, hash) = promoted_file("pensieve-auth", "Tokens win.\n");
    let actions = vec![
        write_action("pensieve-auth.md", &content, &hash),
        FileAction::ArchiveFile {
            file: "old-note.md".to_string(),
            reason: "superseded in pensieve".to_string(),
            node_id: None,
        },
        FileAction::SetIndex {
            entries: vec![IndexEntry {
                title: "Auth".to_string(),
                file: "pensieve-auth.md".to_string(),
                hook: "tokens".to_string(),
            }],
        },
    ];
    let cfg = WritebackConfig::default();
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply");
    assert_eq!(report.written, 1);
    assert_eq!(report.archived, 1);
    assert!(report.index_updated);

    // Promoted file landed.
    assert_eq!(read(&mem.join("pensieve-auth.md")), content);
    // Archive is a move with a tombstone — body fully preserved, original gone.
    assert!(!mem.join("old-note.md").exists());
    let tomb = pensieve_ccmem::frontmatter::parse(&read(&mem.join("archive/old-note.md")))
        .expect("tombstone parses");
    assert_eq!(tomb.front.archived_at.as_deref(), Some(NOW));
    assert_eq!(tomb.front.archived_reason.as_deref(), Some("superseded in pensieve"));
    assert!(tomb.body.contains("This was true once."));
    // Index: user line intact, managed region appended.
    let idx = read(&mem.join("MEMORY.md"));
    assert!(idx.contains("- [User entry](user-note.md) — keep me"));
    assert!(idx.contains(pensieve_ccmem::MANAGED_BEGIN));
    assert!(idx.contains("- [Auth](pensieve-auth.md) — tokens"));

    // Re-applying the same plan changes nothing.
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("re-apply");
    assert_eq!(report.written, 0, "identical content → no rewrite");
    assert_eq!(report.archived, 0, "file already archived");
    assert!(!report.index_updated, "index byte-identical");
}

#[test]
fn user_edited_pensieve_file_is_never_overwritten() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    // On disk: a pensieve file whose body was hand-edited (stamp ≠ body hash).
    let (content, _) = promoted_file("pensieve-auth", "Original body.\n");
    let edited = content.replace("Original body.", "USER IMPROVED THIS.");
    write(&mem.join("pensieve-auth.md"), &edited);

    let (new_content, new_hash) = promoted_file("pensieve-auth", "Refreshed by pensieve.\n");
    let actions = vec![write_action("pensieve-auth.md", &new_content, &new_hash)];
    let report =
        apply_actions(&mem, None, &actions, &WritebackConfig::default(), NOW).expect("apply");
    assert_eq!(report.skipped_user_edited, 1);
    assert_eq!(report.written, 0);
    assert!(read(&mem.join("pensieve-auth.md")).contains("USER IMPROVED THIS."));
}

#[test]
fn foreign_file_at_target_path_is_never_overwritten() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    // A user file (not pensieve-authored) occupies the target name.
    write(
        &mem.join("pensieve-auth.md"),
        "---\nname: pensieve-auth\nmetadata:\n  type: user\n---\n\nMine, hands off.\n",
    );
    let (content, hash) = promoted_file("pensieve-auth", "Pensieve wants this spot.\n");
    let report = apply_actions(
        &mem,
        None,
        &[write_action("pensieve-auth.md", &content, &hash)],
        &WritebackConfig::default(),
        NOW,
    )
    .expect("apply");
    assert_eq!(report.skipped_user_edited, 1);
    assert!(read(&mem.join("pensieve-auth.md")).contains("Mine, hands off."));
}

#[test]
fn fresh_lock_blocks_stale_lock_is_reclaimed() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).expect("mkdir");
    let lock = mem.join(".pensieve-curate.lock");
    std::fs::write(&lock, "pid").expect("lock");

    let (content, hash) = promoted_file("pensieve-auth", "Body.\n");
    let actions = vec![write_action("pensieve-auth.md", &content, &hash)];
    let cfg = WritebackConfig::default();
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply");
    assert!(report.skipped_locked);
    assert!(!mem.join("pensieve-auth.md").exists());

    // Make the lock stale → reclaimed, pass proceeds, lock released after.
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    let f = std::fs::File::options().write(true).open(&lock).expect("open lock");
    f.set_times(std::fs::FileTimes::new().set_modified(old)).expect("set mtime");
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply 2");
    assert!(!report.skipped_locked);
    assert_eq!(report.written, 1);
    assert!(!lock.exists(), "lock released on completion");
}

#[test]
fn active_session_defers_writeback() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).expect("mkdir");
    let sessions = tmp.path().join("sessions");
    let project = tmp.path().join("the-project");
    #[allow(clippy::cast_possible_truncation)]
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_millis() as i64;
    write(
        &sessions.join("123.json"),
        &format!(
            r#"{{"pid":123,"cwd":"{}","updatedAt":{now_ms},"status":"busy"}}"#,
            project.display()
        ),
    );

    let (content, hash) = promoted_file("pensieve-auth", "Body.\n");
    let actions = vec![write_action("pensieve-auth.md", &content, &hash)];
    let cfg = WritebackConfig {
        sessions_dir: Some(sessions.clone()),
        ..WritebackConfig::default()
    };
    let report = apply_actions(&mem, Some(&project), &actions, &cfg, NOW).expect("apply");
    assert!(report.skipped_quiet);
    assert!(!mem.join("pensieve-auth.md").exists());

    // A long-idle session does not block.
    write(
        &sessions.join("123.json"),
        &format!(
            r#"{{"pid":123,"cwd":"{}","updatedAt":{},"status":"busy"}}"#,
            project.display(),
            now_ms - 3_600_000
        ),
    );
    let report = apply_actions(&mem, Some(&project), &actions, &cfg, NOW).expect("apply 2");
    assert!(!report.skipped_quiet);
    assert_eq!(report.written, 1);
}

#[test]
fn dry_run_touches_nothing_but_audits() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).expect("mkdir");
    let audit = tmp.path().join("cc-curation.log");

    let (content, hash) = promoted_file("pensieve-auth", "Body.\n");
    let actions = vec![write_action("pensieve-auth.md", &content, &hash)];
    let cfg = WritebackConfig {
        dry_run: true,
        audit_log: Some(audit.clone()),
        ..WritebackConfig::default()
    };
    let report = apply_actions(&mem, None, &actions, &cfg, NOW).expect("apply");
    assert_eq!(report.written, 1, "dry run reports what it would do");
    assert!(!mem.join("pensieve-auth.md").exists(), "but writes nothing");

    let log = read(&audit);
    let line: serde_json::Value =
        serde_json::from_str(log.lines().next().expect("one line")).expect("json");
    assert_eq!(line["dry_run"], true);
    assert_eq!(line["actions"][0]["action"], "write_memory_file");
}

#[test]
fn empty_index_with_no_markers_writes_nothing() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    let original = "# Memory index\n\n- [Mine](mine.md) — untouched\n";
    write(&mem.join("MEMORY.md"), original);

    // Nothing to manage → MEMORY.md must stay byte-identical (no empty
    // marker block littered into every project).
    let actions = vec![FileAction::SetIndex { entries: vec![] }];
    let report =
        apply_actions(&mem, None, &actions, &WritebackConfig::default(), NOW).expect("apply");
    assert!(!report.index_updated);
    assert_eq!(read(&mem.join("MEMORY.md")), original);

    // And a project with no MEMORY.md at all doesn't get one invented.
    let mem2 = tmp.path().join("memory2");
    std::fs::create_dir_all(&mem2).expect("mkdir");
    apply_actions(&mem2, None, &actions, &WritebackConfig::default(), NOW).expect("apply 2");
    assert!(!mem2.join("MEMORY.md").exists());
}

#[test]
fn index_drops_entries_for_missing_files() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    // Only one of the two indexed files exists on disk (the user deleted the
    // other promoted file by hand — respect that).
    let (content, _) = promoted_file("pensieve-real", "Exists.\n");
    write(&mem.join("pensieve-real.md"), &content);
    let actions = vec![FileAction::SetIndex {
        entries: vec![
            IndexEntry {
                title: "Real".to_string(),
                file: "pensieve-real.md".to_string(),
                hook: "exists".to_string(),
            },
            IndexEntry {
                title: "Ghost".to_string(),
                file: "pensieve-ghost.md".to_string(),
                hook: "deleted by user".to_string(),
            },
        ],
    }];
    apply_actions(&mem, None, &actions, &WritebackConfig::default(), NOW).expect("apply");
    let idx = read(&mem.join("MEMORY.md"));
    assert!(idx.contains("pensieve-real.md"));
    assert!(!idx.contains("pensieve-ghost.md"), "no dead links in the index");
}

#[test]
fn index_defers_to_user_entries() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mem = tmp.path().join("memory");
    // The user already lists pensieve-auth.md themselves; both files exist.
    write(
        &mem.join("MEMORY.md"),
        "# Memory index\n\n- [My own pointer](pensieve-auth.md) — user's words\n",
    );
    let (auth, _) = promoted_file("pensieve-auth", "Auth.\n");
    write(&mem.join("pensieve-auth.md"), &auth);
    let (other, _) = promoted_file("pensieve-other", "Other.\n");
    write(&mem.join("pensieve-other.md"), &other);
    let actions = vec![FileAction::SetIndex {
        entries: vec![
            IndexEntry {
                title: "Auth".to_string(),
                file: "pensieve-auth.md".to_string(),
                hook: "tokens".to_string(),
            },
            IndexEntry {
                title: "Other".to_string(),
                file: "pensieve-other.md".to_string(),
                hook: "other".to_string(),
            },
        ],
    }];
    apply_actions(&mem, None, &actions, &WritebackConfig::default(), NOW).expect("apply");
    let idx = read(&mem.join("MEMORY.md"));
    assert!(idx.contains("- [My own pointer](pensieve-auth.md) — user's words"));
    assert!(idx.contains("- [Other](pensieve-other.md) — other"));
    let managed_region = idx
        .split(pensieve_ccmem::MANAGED_BEGIN)
        .nth(1)
        .expect("managed region");
    assert!(
        !managed_region.contains("pensieve-auth.md"),
        "user-listed file must not be double-indexed"
    );
}
