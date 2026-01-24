use crate::models::{Provider, UsageSnapshot};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheState {
    pub snapshots: HashMap<Provider, UsageSnapshot>,
    pub updated_at: DateTime<Utc>,
}

impl CacheState {
    pub fn cache_path() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("quotabar")
            .join("state.json")
    }

    pub fn load() -> Result<Option<Self>> {
        let path = Self::cache_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let state: CacheState = serde_json::from_str(&content)?;
            Ok(Some(state))
        } else {
            Ok(None)
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::cache_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Atomic write: write to temp file, then rename
        let temp_path = path.with_extension("tmp");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&temp_path, content)?;
        std::fs::rename(&temp_path, &path)?;

        Ok(())
    }

    pub fn get(&self, provider: Provider) -> Option<&UsageSnapshot> {
        self.snapshots.get(&provider)
    }
}
