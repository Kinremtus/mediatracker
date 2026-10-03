//! Stub module — replaced by its dedicated implementation batch.
use crate::services::external::official_meta::OfficialMeta;
pub async fn fetch_meta(_client: &reqwest::Client, _source_id: &str) -> anyhow::Result<OfficialMeta> { Ok(OfficialMeta::default()) }
pub async fn search(_client: &reqwest::Client, _title: &str) -> anyhow::Result<Vec<crate::services::official_types::OfficialHit>> { Ok(Vec::new()) }
pub async fn search_cached(_client: &reqwest::Client, _title: &str) -> anyhow::Result<Vec<crate::services::official_types::OfficialHit>> { Ok(Vec::new()) }
