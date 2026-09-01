//! The wiki gardener's run instructions. The gardener is a dreaming run
//! with this text as its `focus` — it writes *memories* (topic_key
//! `wiki:<brain>:<slug>`), never files; the deterministic exporter renders
//! them into `wiki/` on the next pass, keeping a single git writer.

use crate::registry::{BrainConfig, RealmSelector};

/// Build the dreaming `focus` for one brain's gardener run.
pub fn gardener_focus(cfg: &BrainConfig) -> String {
    let realm = match &cfg.realms {
        RealmSelector::Realms(r) => r.first().cloned().unwrap_or_else(|| "default".into()),
        RealmSelector::All => "default".to_string(),
    };
    let scope = match &cfg.realms {
        RealmSelector::All => "all realms".to_string(),
        RealmSelector::Realms(r) => r.join(", "),
    };
    format!(
        "WIKI GARDENER for the published brain `{name}` (realms: {scope}).\n\
         This run maintains the brain's curated wiki layer instead of general housekeeping. \
         The brain is a git repo users read as an Obsidian vault; its `wiki/` folder is \
         rendered from memories whose topic_key starts with `wiki:{name}:`.\n\
         Your job:\n\
         1. Survey this brain's memories (memory_search / list_memories, realms above) and the \
         existing wiki pages (memories with topic_key prefix `wiki:{name}:`).\n\
         2. MAINTAIN a small set of stable, high-value wiki pages: a `start-here` overview and \
         one page per major topic/theme (an architecture area, a recurring project, a domain). \
         UPDATE existing pages in place rather than adding near-duplicates — save_memory with \
         the SAME topic_key (`wiki:{name}:<page-slug>`) updates the page.\n\
         3. Each page: memory_type `summary`, tag `wiki`, realm `{realm}`, title = the page \
         title, content = curated markdown — a few sentences of orientation plus a link list \
         referencing the underlying notes BY TITLE as wikilinks (e.g. `[[Auth model uses \
         stateless JWT]]` — exported notes carry their title as an Obsidian alias, so \
         title links resolve in the vault).\n\
         4. Prefer merging/refreshing over creating: never mint two pages for the same theme; \
         retire a stale page by updating it (same topic_key) into a redirect stub pointing at \
         the successor page.\n\
         5. Do NOT touch memories that are not wiki pages except to read them; normal \
         housekeeping is another run's job.",
        name = cfg.name,
    )
}
