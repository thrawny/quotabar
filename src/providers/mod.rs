pub mod claude;
pub mod codex;

use crate::models::UsageSnapshot;
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait ProviderFetcher: Send + Sync {
    async fn fetch(&self) -> Result<UsageSnapshot>;
    fn name(&self) -> &'static str;
}
