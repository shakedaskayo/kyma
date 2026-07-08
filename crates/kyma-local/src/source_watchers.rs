//! Reconcile continuous-drive data source rows (`drive_model = 'continuous'`,
//! e.g. obsidian) into running per-vault sync loops.
//!
//! A manager task polls the `data_sources` table every ~15s and diffs the
//! enabled rows against the loops it runs ([`reconcile_plan`] — pure, unit
//! tested). Each loop watches the vault with `notify` (debounced 2s) and
//! also full-scans on the row's `schedule_ms` as a fallback, so the sync is
//! event-driven with a periodic safety net. Every pass heartbeats the
//! in-process watcher registry (`LocalWatcherStatus`, kind `"obsidian"`,
//! id `obsidian:<source_id>`) and records run success/failure on the data
//! source row — the standard sources UI shows status without new plumbing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};

use crate::datasource_catalog::SqliteDataSourceControl;
use crate::watcher_status::{self, LocalWatcherStatus};
use crate::{vault_sync, Engine};
use kyma_datasources::runner::DataSourceControl;

/// How often the manager re-reads the rows to start/stop loops.
const RECONCILE_EVERY: Duration = Duration::from_secs(15);
/// Quiet window after an fs event before syncing (coalesces editor bursts).
const DEBOUNCE: Duration = Duration::from_secs(2);
/// Floor for the periodic full-scan fallback.
const MIN_SCAN_INTERVAL: Duration = Duration::from_secs(30);

/// A git-hosted vault to clone/pull before each sync pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitVault {
    pub url: String,
    pub branch: Option<String>,
    pub token: Option<String>,
}

/// What a continuous-drive row needs to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesiredSource {
    pub id: String,
    /// Detects config edits → restart (config + schedule + name).
    pub fingerprint: String,
    /// Local folder synced into memory. For a git-hosted vault this is the
    /// managed clone dir (`${KYMA_HOME}/vaults/<id>`).
    pub vault_path: PathBuf,
    pub realm: String,
    pub scan_interval: Duration,
    pub tenant: String,
    /// `Some` when the vault is imported from a git repo.
    pub git: Option<GitVault>,
}

/// One reconciliation step.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PlanAction {
    Start(DesiredSource),
    Stop(String),
}

/// Pure diff of running loops (id → fingerprint) against desired rows.
/// A changed fingerprint stops the old loop and starts a new one.
pub(crate) fn reconcile_plan(
    running: &HashMap<String, String>,
    desired: &[DesiredSource],
) -> Vec<PlanAction> {
    let mut plan = Vec::new();
    let desired_by_id: HashMap<&str, &DesiredSource> =
        desired.iter().map(|d| (d.id.as_str(), d)).collect();
    for (id, fp) in running {
        match desired_by_id.get(id.as_str()) {
            Some(d) if d.fingerprint == *fp => {}
            _ => plan.push(PlanAction::Stop(id.clone())),
        }
    }
    for d in desired {
        if running.get(&d.id) != Some(&d.fingerprint) {
            plan.push(PlanAction::Start(d.clone()));
        }
    }
    plan
}

/// Expand a leading `~` against `$HOME`.
fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

/// `${KYMA_HOME}/vaults` — where git-hosted Obsidian imports are cloned.
fn vaults_root() -> PathBuf {
    let home = std::env::var("KYMA_HOME").unwrap_or_else(|_| {
        let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{base}/.kyma")
    });
    PathBuf::from(home).join("vaults")
}

/// Desired sources from the `data_sources` table. Unparseable rows are
/// skipped with a warning — one broken config must not stall the others.
async fn desired_sources(pool: &SqlitePool) -> Vec<DesiredSource> {
    let rows = match sqlx::query(
        "SELECT id, tenant_id, name, config_json, schedule_ms FROM data_sources
         WHERE drive_model = 'continuous' AND type = 'obsidian' AND enabled = 1",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("vault watchers: listing sources failed: {e}");
            return Vec::new();
        }
    };
    rows.into_iter()
        .filter_map(|r| {
            let id: String = r.get("id");
            let tenant: String = r.get("tenant_id");
            let name: String = r.get("name");
            let config_str: String = r.get("config_json");
            let schedule_ms: i64 = r.get("schedule_ms");
            let config: Value = serde_json::from_str(&config_str).ok()?;
            let field = |k: &str| {
                config.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
            };
            let git_url = field("git_url").map(str::to_string);
            // Git-hosted vaults are cloned into a managed dir; local vaults use
            // the given path.
            let (vault_path, git) = match &git_url {
                Some(url) => {
                    let dir = vaults_root().join(&id);
                    let git = GitVault {
                        url: url.clone(),
                        branch: field("git_branch").map(str::to_string),
                        token: field("git_token").map(str::to_string),
                    };
                    (dir, Some(git))
                }
                None => {
                    let raw_path = field("vault_path")?;
                    (expand_home(raw_path), None)
                }
            };
            let repo_name = git_url.as_deref().map(|u| {
                u.trim_end_matches('/')
                    .trim_end_matches(".git")
                    .rsplit('/')
                    .next()
                    .unwrap_or("vault")
                    .to_string()
            });
            let realm = field("vault_name")
                .map(str::to_string)
                .or(repo_name)
                .or_else(|| {
                    vault_path.file_name().and_then(|n| n.to_str()).map(str::to_string)
                })
                .unwrap_or_else(|| name.clone());
            let scan_interval = Duration::from_millis(u64::try_from(schedule_ms).unwrap_or(60_000))
                .max(MIN_SCAN_INTERVAL);
            Some(DesiredSource {
                fingerprint: format!("{config_str}|{schedule_ms}|{name}"),
                id,
                vault_path,
                realm,
                scan_interval,
                tenant,
                git,
            })
        })
        .collect()
}

/// Run the reconciler until the process exits. Spawned from `serve()`.
pub(crate) async fn run_manager(
    engine: Engine,
    status: LocalWatcherStatus,
    control: Arc<SqliteDataSourceControl>,
    pool: SqlitePool,
) {
    let mut running: HashMap<String, String> = HashMap::new();
    let mut handles: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    loop {
        let desired = desired_sources(&pool).await;
        for action in reconcile_plan(&running, &desired) {
            match action {
                PlanAction::Stop(id) => {
                    if let Some(h) = handles.remove(&id) {
                        h.abort();
                    }
                    running.remove(&id);
                    status.remove(&format!("obsidian:{id}"));
                    tracing::info!(source = %id, "vault watcher stopped");
                }
                PlanAction::Start(src) => {
                    running.insert(src.id.clone(), src.fingerprint.clone());
                    let handle = tokio::spawn(run_source_loop(
                        engine.clone(),
                        status.clone(),
                        control.clone(),
                        src.clone(),
                    ));
                    handles.insert(src.id.clone(), handle);
                    tracing::info!(source = %src.id, vault = %src.vault_path.display(), "vault watcher started");
                }
            }
        }
        tokio::time::sleep(RECONCILE_EVERY).await;
    }
}

/// One vault's loop: notify events (debounced) + periodic full scan.
async fn run_source_loop(
    engine: Engine,
    status: LocalWatcherStatus,
    control: Arc<SqliteDataSourceControl>,
    src: DesiredSource,
) {
    use notify::{RecursiveMode, Watcher};

    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);
    // The notify watcher runs callbacks on its own thread; keep the handle
    // alive for the loop's lifetime. Filter to markdown outside dot-dirs so
    // .obsidian workspace churn doesn't trigger syncs.
    let mut watcher = match notify::recommended_watcher(
        move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            let relevant = event.paths.iter().any(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("md")
                    && !p
                        .components()
                        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
            });
            if relevant {
                let _ = tx.try_send(());
            }
        },
    ) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(source = %src.id, "vault watcher: notify unavailable, polling only: {e}");
            None
        }
    };
    if let Some(w) = watcher.as_mut() {
        if let Err(e) = w.watch(&src.vault_path, RecursiveMode::Recursive) {
            tracing::warn!(source = %src.id, "vault watcher: watch failed, polling only: {e}");
            watcher = None;
        }
    }
    // Hold the watcher for the loop's lifetime (dropping it unwatches).
    let _watcher = watcher;

    let (node_host, node_id, identity) = watcher_status::node_identity();
    let started_at = chrono::Utc::now().to_rfc3339();
    let watcher_id = format!("obsidian:{}", src.id);
    let config = json!({
        "root": src.vault_path.display().to_string(),
        "poll_secs": src.scan_interval.as_secs(),
        "source_id": src.id,
        "realm": src.realm,
    });
    let tenant = kyma_core::tenant::TenantId::from_uuid(
        src.tenant.parse().unwrap_or_else(|_| uuid::Uuid::nil()),
    );
    let source_uuid = src.id.parse().unwrap_or_else(|_| uuid::Uuid::nil());
    let mut last_scan: Option<Value> = None;

    loop {
        let pass_start = std::time::Instant::now();
        let wall_start = chrono::Utc::now();
        let outcome = run_pass(&engine, &src).await;
        match outcome {
            Ok(report) => {
                let scan = report.last_scan_value(
                    u64::try_from(pass_start.elapsed().as_millis()).unwrap_or(u64::MAX),
                    wall_start,
                    &src.realm,
                );
                let processed = i64::try_from(report.upserted).unwrap_or(i64::MAX);
                if let Err(e) = control
                    .mark_run_success(tenant, source_uuid, processed)
                    .await
                {
                    tracing::warn!(source = %src.id, "vault watcher: recording success failed: {e}");
                }
                last_scan = Some(scan);
            }
            Err(e) => {
                tracing::warn!(source = %src.id, "vault sync failed: {e}");
                if let Err(e2) = control
                    .mark_run_failure(tenant, source_uuid, &e.to_string())
                    .await
                {
                    tracing::warn!(source = %src.id, "vault watcher: recording failure failed: {e2}");
                }
            }
        }
        status.upsert(watcher_status::LocalWatcher {
            id: watcher_id.clone(),
            kind: "obsidian".into(),
            node_host: node_host.clone(),
            node_id: node_id.clone(),
            identity: identity.clone(),
            config: config.clone(),
            started_at: started_at.clone(),
            last_heartbeat_at: chrono::Utc::now().to_rfc3339(),
            last_scan: last_scan.clone(),
        });

        // Wait for an fs event (then debounce + drain the burst) or the
        // periodic fallback tick, whichever comes first.
        tokio::select! {
            ev = rx.recv() => {
                if ev.is_none() {
                    // Watcher thread gone — fall back to pure polling.
                    tokio::time::sleep(src.scan_interval).await;
                } else {
                    tokio::time::sleep(DEBOUNCE).await;
                    while rx.try_recv().is_ok() {}
                }
            }
            () = tokio::time::sleep(src.scan_interval) => {}
        }
    }
}

/// One sync pass: (for git vaults) clone/pull the repo, then a fresh writer
/// (shared embedding backend) + vault_sync over the working tree.
async fn run_pass(
    engine: &Engine,
    src: &DesiredSource,
) -> anyhow::Result<vault_sync::VaultSyncReport> {
    if let Some(git) = &src.git {
        let bin = kyma_brain::gitbin::GitBin::detect()
            .await
            .ok_or_else(|| anyhow::anyhow!("git binary not found — cannot import a git-hosted vault"))?;
        let head = bin
            .clone_or_pull(&git.url, &src.vault_path, git.branch.as_deref(), git.token.as_deref())
            .await
            .map_err(|e| anyhow::anyhow!("git import: {e}"))?;
        tracing::debug!(source = %src.id, head = %head, "git vault synced to HEAD");
    }
    let embed = kyma_memory::shared_embedding()
        .await
        .map_err(|e| anyhow::anyhow!("embedding backend: {e}"))?;
    let writer =
        kyma_memory::MemoryWriter::new(engine.catalog.clone(), engine.format.clone(), embed);
    let opts = vault_sync::VaultSyncOptions {
        source_id: src.id.clone(),
        vault_path: src.vault_path.clone(),
        realm: src.realm.clone(),
    };
    vault_sync::run_once(engine, &writer, &opts).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(id: &str, fp: &str) -> DesiredSource {
        DesiredSource {
            id: id.into(),
            fingerprint: fp.into(),
            vault_path: "/v".into(),
            realm: "v".into(),
            scan_interval: Duration::from_secs(60),
            tenant: "00000000-0000-0000-0000-000000000000".into(),
            git: None,
        }
    }

    #[test]
    fn starts_new_rows() {
        let running = HashMap::new();
        let desired = vec![src("a", "f1")];
        assert_eq!(
            reconcile_plan(&running, &desired),
            vec![PlanAction::Start(src("a", "f1"))]
        );
    }

    #[test]
    fn stops_removed_or_disabled_rows() {
        let running = HashMap::from([("a".to_string(), "f1".to_string())]);
        assert_eq!(
            reconcile_plan(&running, &[]),
            vec![PlanAction::Stop("a".into())]
        );
    }

    #[test]
    fn restarts_on_fingerprint_change() {
        let running = HashMap::from([("a".to_string(), "f1".to_string())]);
        let desired = vec![src("a", "f2")];
        let plan = reconcile_plan(&running, &desired);
        assert_eq!(
            plan,
            vec![
                PlanAction::Stop("a".into()),
                PlanAction::Start(src("a", "f2"))
            ]
        );
    }

    #[test]
    fn noop_when_unchanged() {
        let running = HashMap::from([("a".to_string(), "f1".to_string())]);
        let desired = vec![src("a", "f1")];
        assert_eq!(reconcile_plan(&running, &desired), vec![]);
    }

    #[test]
    fn expand_home_handles_tilde() {
        // Don't mutate HOME (parallel tests read it) — assert against it.
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        assert_eq!(expand_home("~/Vault"), PathBuf::from(home).join("Vault"));
        assert_eq!(expand_home("/abs/Vault"), PathBuf::from("/abs/Vault"));
    }
}
