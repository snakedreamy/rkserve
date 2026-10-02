use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::domain::CoreMask;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesiredPluginState {
    pub plugin_id: String,
    pub enabled: bool,
    pub core_mask: CoreMask,
    pub last_error: Option<String>,
    pub updated_at_unix_ms: u64,
    #[serde(default)]
    pub configuration: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedState {
    #[serde(default)]
    plugins: BTreeMap<String, DesiredPluginState>,
}

struct StateStoreInner {
    path: PathBuf,
    state: PersistedState,
}

#[derive(Clone)]
pub struct StateStore {
    inner: Arc<Mutex<StateStoreInner>>,
    commit_lock: Arc<AsyncMutex<()>>,
}

impl StateStore {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create state directory {}", parent.display()))?;
        }
        let state = if path.is_file() {
            let raw = fs::read(&path).with_context(|| format!("read state {}", path.display()))?;
            match serde_json::from_slice(&raw) {
                Ok(state) => state,
                Err(_) => {
                    let quarantine = path.with_extension(format!("json.corrupt-{}", now_ms()));
                    fs::rename(&path, &quarantine).with_context(|| format!("quarantine corrupt state {}", path.display()))?;
                    PersistedState::default()
                }
            }
        } else {
            PersistedState::default()
        };
        Ok(Self {
            inner: Arc::new(Mutex::new(StateStoreInner { path, state })),
            commit_lock: Arc::new(AsyncMutex::new(())),
        })
    }

    pub fn desired(&self) -> Vec<DesiredPluginState> {
        self.inner.lock().expect("state mutex poisoned").state.plugins.values().cloned().collect()
    }

    pub fn get(&self, plugin_id: &str) -> Option<DesiredPluginState> {
        self.inner.lock().expect("state mutex poisoned").state.plugins.get(plugin_id).cloned()
    }

    pub async fn enable(&self, plugin_id: &str, core_mask: CoreMask) -> Result<()> {
        self.update(plugin_id, true, core_mask, None, None).await
    }

    pub async fn disable(&self, plugin_id: &str, core_mask: CoreMask) -> Result<()> {
        self.update(plugin_id, false, core_mask, None, None).await
    }

    pub async fn fail(
        &self,
        plugin_id: &str,
        core_mask: CoreMask,
        error: String,
    ) -> Result<()> {
        self.update(plugin_id, false, core_mask, Some(error), None)
            .await
    }

    pub async fn set_configuration(
        &self,
        plugin_id: &str,
        core_mask: CoreMask,
        configuration: BTreeMap<String, String>,
    ) -> Result<()> {
        self.commit(|state| {
            let existing = state.plugins.get(plugin_id);
            let enabled = existing.is_some_and(|state| state.enabled);
            let core_mask = existing.map_or(core_mask, |state| state.core_mask);
            let last_error = existing.and_then(|state| state.last_error.clone());
            state.plugins.insert(plugin_id.to_owned(), DesiredPluginState {
                plugin_id: plugin_id.to_owned(), enabled, core_mask, last_error,
                updated_at_unix_ms: now_ms(), configuration,
            });
        })
        .await
    }

    async fn update(
        &self,
        plugin_id: &str,
        enabled: bool,
        core_mask: CoreMask,
        last_error: Option<String>,
        configuration: Option<BTreeMap<String, String>>,
    ) -> Result<()> {
        self.commit(|state| {
            // None preserves the existing values; Some replaces them explicitly.
            let configuration = configuration
                .or_else(|| state.plugins.get(plugin_id).map(|state| state.configuration.clone()))
                .unwrap_or_default();
            state.plugins.insert(plugin_id.to_owned(), DesiredPluginState {
                plugin_id: plugin_id.to_owned(), enabled, core_mask, last_error,
                updated_at_unix_ms: now_ms(), configuration,
            });
        })
        .await
    }

    async fn commit(&self, update: impl FnOnce(&mut PersistedState)) -> Result<()> {
        let _commit = self.commit_lock.lock().await;
        let (path, mut next) = {
            let inner = self.inner.lock().expect("state mutex poisoned");
            (inner.path.clone(), inner.state.clone())
        };
        update(&mut next);
        let next = tokio::task::spawn_blocking(move || {
            persist(&path, &next)?;
            Ok::<_, anyhow::Error>(next)
        })
        .await
        .context("join desired state persistence")??;
        self.inner.lock().expect("state mutex poisoned").state = next;
        Ok(())
    }
}

fn persist(path: &PathBuf, state: &PersistedState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state).context("serialize desired state")?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes).with_context(|| format!("write state {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("replace state {}", path.display()))?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX)
}
