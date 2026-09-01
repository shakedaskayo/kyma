use super::index::{ManagedEntry, MemoryIndex};
use super::{MANAGED_BEGIN, MANAGED_END};

fn entry(title: &str, file: &str, hook: &str) -> ManagedEntry {
    ManagedEntry {
        title: title.into(),
        file: file.into(),
        hook: hook.into(),
    }
}

const USER_ONLY: &str = "# Memory index\n\n- [Agentic memory overhaul](agentic-memory-overhaul.md) — pensieve memory stack rebuilt.\n- [Binary topology](pensieve-local-build-cmake.md) — one pensieve CLI.\n";

#[test]
fn no_markers_preserves_user_content_and_appends_block() {
    let mut idx = MemoryIndex::parse(USER_ONLY);
    assert!(idx.managed().is_empty());
    idx.set_managed(vec![entry("Auth model", "pensieve-auth-model.md", "why tokens")]);
    let out = idx.render();
    // User content intact, in order, before the managed block.
    assert!(out.starts_with(USER_ONLY.trim_end()));
    let begin = out.find(MANAGED_BEGIN).expect("begin marker");
    let end = out.find(MANAGED_END).expect("end marker");
    assert!(begin < end);
    assert!(out.contains("- [Auth model](pensieve-auth-model.md) — why tokens"));
}

#[test]
fn with_markers_round_trips_byte_identical() {
    let raw = format!(
        "# Memory index\n\n- [User entry](user.md) — keep me\n\n{MANAGED_BEGIN}\n- [A](pensieve-a.md) — a hook\n- [B](pensieve-b.md) — b hook\n{MANAGED_END}\n\n- [Trailing user entry](tail.md) — also keep\n"
    );
    let idx = MemoryIndex::parse(&raw);
    assert_eq!(idx.managed().len(), 2);
    assert_eq!(idx.managed()[0], entry("A", "pensieve-a.md", "a hook"));
    assert_eq!(idx.render(), raw);
}

#[test]
fn set_managed_replaces_only_the_block() {
    let raw = format!(
        "# Memory index\n\n- [User entry](user.md) — keep me\n\n{MANAGED_BEGIN}\n- [Old](pensieve-old.md) — stale\n{MANAGED_END}\n"
    );
    let mut idx = MemoryIndex::parse(&raw);
    idx.set_managed(vec![entry("New", "pensieve-new.md", "fresh")]);
    let out = idx.render();
    assert!(out.contains("- [User entry](user.md) — keep me"));
    assert!(!out.contains("pensieve-old.md"));
    assert!(out.contains("- [New](pensieve-new.md) — fresh"));
}

#[test]
fn user_files_lists_only_unmanaged_bullets() {
    let raw = format!(
        "# Memory index\n\n- [User entry](user.md) — keep me\n\n{MANAGED_BEGIN}\n- [A](pensieve-a.md) — managed\n{MANAGED_END}\n"
    );
    let idx = MemoryIndex::parse(&raw);
    assert_eq!(idx.user_files(), vec!["user.md".to_string()]);
}

#[test]
fn empty_index_renders_header_and_block() {
    let mut idx = MemoryIndex::new_empty();
    idx.set_managed(vec![entry("A", "pensieve-a.md", "hook")]);
    let out = idx.render();
    assert!(out.starts_with("# Memory index"));
    assert!(out.contains(MANAGED_BEGIN));
    assert!(out.contains("- [A](pensieve-a.md) — hook"));
    assert!(out.contains(MANAGED_END));
}

#[test]
fn render_is_idempotent_through_parse() {
    let mut idx = MemoryIndex::new_empty();
    idx.set_managed(vec![
        entry("A", "pensieve-a.md", "a"),
        entry("B", "pensieve-b.md", "b"),
    ]);
    let once = idx.render();
    let twice = MemoryIndex::parse(&once).render();
    assert_eq!(once, twice);
}
