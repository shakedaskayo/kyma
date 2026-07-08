//! Local-mode brain registry: `${KYMA_HOME}/brains.json`, atomic
//! tmp+rename writes (the cc_writeback pattern), serialized by a mutex.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use kyma_brain::registry::{BrainConfig, BrainRecord, BrainRegistry, BrainRuntime};
use kyma_brain::BrainError;
use tokio::sync::Mutex;

pub struct LocalBrainRegistry {
    path: PathBuf,
    lock: Mutex<()>,
}

impl LocalBrainRegistry {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), lock: Mutex::new(()) }
    }

    async fn load(&self) -> Result<BTreeMap<String, BrainRecord>, BrainError> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| BrainError::Other(format!("parse {}: {e}", self.path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    async fn store(&self, map: &BTreeMap<String, BrainRecord>) -> Result<(), BrainError> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let tmp = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(map)?;
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

#[async_trait]
impl BrainRegistry for LocalBrainRegistry {
    async fn list(&self) -> Result<Vec<BrainRecord>, BrainError> {
        let _g = self.lock.lock().await;
        Ok(self.load().await?.into_values().collect())
    }

    async fn get(&self, name: &str) -> Result<Option<BrainRecord>, BrainError> {
        let _g = self.lock.lock().await;
        Ok(self.load().await?.remove(name))
    }

    async fn upsert_config(&self, cfg: &BrainConfig) -> Result<(), BrainError> {
        let _g = self.lock.lock().await;
        let mut map = self.load().await?;
        let entry = map.entry(cfg.name.clone()).or_insert_with(|| BrainRecord {
            config: cfg.clone(),
            runtime: BrainRuntime::default(),
        });
        entry.config = cfg.clone();
        self.store(&map).await
    }

    async fn delete(&self, name: &str) -> Result<(), BrainError> {
        let _g = self.lock.lock().await;
        let mut map = self.load().await?;
        map.remove(name);
        self.store(&map).await
    }

    async fn update_runtime(&self, name: &str, rt: &BrainRuntime) -> Result<(), BrainError> {
        let _g = self.lock.lock().await;
        let mut map = self.load().await?;
        let Some(entry) = map.get_mut(name) else {
            return Err(BrainError::Other(format!("brain `{name}` not found")));
        };
        entry.runtime = rt.clone();
        self.store(&map).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kyma_brain::registry::RealmSelector;

    #[tokio::test]
    async fn crud_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let reg = LocalBrainRegistry::new(dir.path().join("brains.json"));
        assert!(reg.list().await.unwrap().is_empty());

        let cfg = BrainConfig::new(
            "team",
            RealmSelector::Realms(vec!["kyma".into()]),
            "2026-07-08T00:00:00Z",
        )
        .unwrap();
        reg.upsert_config(&cfg).await.unwrap();
        let rec = reg.get("team").await.unwrap().unwrap();
        assert_eq!(rec.config.name, "team");

        let mut rt = rec.runtime;
        rt.last_commit = Some("abc".into());
        reg.update_runtime("team", &rt).await.unwrap();
        assert_eq!(
            reg.get("team").await.unwrap().unwrap().runtime.last_commit.as_deref(),
            Some("abc")
        );

        // Config update must not clobber runtime.
        let mut cfg2 = cfg.clone();
        cfg2.export_interval_secs = 60;
        reg.upsert_config(&cfg2).await.unwrap();
        let rec = reg.get("team").await.unwrap().unwrap();
        assert_eq!(rec.config.export_interval_secs, 60);
        assert_eq!(rec.runtime.last_commit.as_deref(), Some("abc"));

        reg.delete("team").await.unwrap();
        assert!(reg.get("team").await.unwrap().is_none());
    }
}
