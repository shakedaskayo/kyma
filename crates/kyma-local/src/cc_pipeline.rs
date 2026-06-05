//! The full Claude Code file-memory pass: ingest → curate → apply.
//!
//! One call runs the whole loop for every project (or one): `cc_sync`
//! ingests memory files into the engine, `cc_curate` (in `kyma-server`)
//! plans promotions/archives/index updates per project, and `cc_writeback`
//! applies them atomically. Wired into `kyma sync`, the MCP-startup
//! opportunistic sync, the `--watch` loop, and the optional `kyma serve`
//! tick.

use anyhow::Result;
use kyma_memory::MemoryWriter;
use kyma_server::agent::cc_curate::{
    llm_curation_pass, plan_curation, CurationConfig, CurationInput, CurationOutcome,
    LlmCurationConfig,
};
use kyma_server::agent::{AgentState, SharedToolCtx};

use crate::cc_sync::{self, CcSyncOptions, CcSyncReport};
use crate::cc_writeback::{self, ApplyReport, WritebackConfig};
use crate::Engine;

/// Everything one pass needs, injectable for tests (env resolution happens
/// in `lib.rs`).
pub(crate) struct CcPipelineOptions {
    pub sync: CcSyncOptions,
    /// Run the curation + writeback half (ingest always runs).
    pub curate: bool,
    pub promote_cfg: CurationConfig,
    pub llm_cfg: LlmCurationConfig,
    pub writeback: WritebackConfig,
}

/// Per-project curation outcome + what was applied to disk.
pub(crate) struct CuratedProject {
    pub realm: String,
    pub outcome: CurationOutcome,
    pub applied: ApplyReport,
}

pub(crate) struct CcPipelineReport {
    pub sync: CcSyncReport,
    pub curated: Vec<CuratedProject>,
}

/// Run one full pass. `agent` enables the LLM curation pass when a usable
/// engine is configured (local CLI passes `None`).
pub(crate) async fn run_pass(
    engine: &Engine,
    writer: &MemoryWriter,
    agent: Option<&AgentState>,
    opts: &CcPipelineOptions,
) -> Result<CcPipelineReport> {
    let sync = cc_sync::run_once(engine, writer, &opts.sync).await?;
    let mut curated = Vec::new();
    if opts.curate {
        let shared = SharedToolCtx {
            catalog: engine.catalog.clone(),
            format: engine.format.clone(),
            pool: None,
        };
        let now = chrono::Utc::now().to_rfc3339();
        // Dry-run is decided once (writeback) and forced onto the planners,
        // so a dry pass leaves no stamps behind and a later real pass
        // replans identically.
        let mut promote_cfg = opts.promote_cfg.clone();
        promote_cfg.dry_run |= opts.writeback.dry_run;
        let mut llm_cfg = opts.llm_cfg.clone();
        llm_cfg.dry_run |= opts.writeback.dry_run;
        for p in &sync.projects {
            let input = CurationInput {
                realm: &p.realm,
                path_slug: &p.slug,
                now: &now,
            };
            let (mut actions, mut outcome) =
                plan_curation(&shared, writer, &input, &promote_cfg).await?;
            llm_curation_pass(
                &shared,
                writer,
                agent,
                &input,
                &llm_cfg,
                &mut actions,
                &mut outcome,
            )
            .await?;
            let applied = cc_writeback::apply_actions(
                &p.dir.join("memory"),
                p.project_path.as_deref(),
                &actions,
                &opts.writeback,
                &now,
            )?;
            curated.push(CuratedProject {
                realm: p.realm.clone(),
                outcome,
                applied,
            });
        }
    }
    Ok(CcPipelineReport { sync, curated })
}
