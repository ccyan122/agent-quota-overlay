//! Only normalized quota values are eligible for persistence; credentials never are.
use std::{fs, io, path::Path};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::quota::{ProviderId, ProviderQuota};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedProviderQuota {
    pub provider: ProviderId,
    pub fetched_at: DateTime<Utc>,
    /// Internal only; frontends receive `quota` and never credentials or account fields.
    pub stale: bool,
    pub quota: ProviderQuota,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedQuotaSnapshot {
    /// Every provider has an independent freshness timestamp. A failed
    /// provider never makes a healthy provider appear stale, and vice versa.
    pub providers: Vec<CachedProviderQuota>,
}

pub fn load(path: &Path) -> io::Result<CachedQuotaSnapshot> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub fn save(path: &Path, snapshot: &CachedQuotaSnapshot) -> io::Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let bytes = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    // `rename` maps to a replace-existing move on Windows.  Do not delete the
    // old cache first: a crash or power loss must leave a complete last-good
    // file available. Credentials never enter this directory or this payload.
    fs::rename(temporary, path)
}

/// Merge one refresh pass without throwing away a provider's last successful
/// quota. Failed entries remain available and are explicitly marked stale.
pub fn retain_last_good(
    previous: &CachedQuotaSnapshot,
    outcomes: Vec<(ProviderId, Result<ProviderQuota, crate::providers::ProviderError>)>,
    fetched_at: DateTime<Utc>,
) -> CachedQuotaSnapshot {
    let mut providers = previous.providers.clone();
    for (provider, outcome) in outcomes {
        match outcome {
            Ok(quota) => {
                if let Some(entry) = providers.iter_mut().find(|entry| entry.provider == provider) {
                    *entry = CachedProviderQuota { provider, fetched_at, stale: false, quota };
                } else { providers.push(CachedProviderQuota { provider, fetched_at, stale: false, quota }); }
            }
            Err(_) => {
                if let Some(entry) = providers.iter_mut().find(|entry| entry.provider == provider) { entry.stale = true; }
            }
        }
    }
    CachedQuotaSnapshot { providers }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::QuotaPool;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn cache_keeps_provider_freshness_independent() {
        let first = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let second = DateTime::<Utc>::from_timestamp(1_700_000_060, 0).unwrap();
        let snapshot = CachedQuotaSnapshot { providers: vec![
            CachedProviderQuota { provider: ProviderId::ChatgptWork, fetched_at: first, stale: false, quota: ProviderQuota { provider: ProviderId::ChatgptWork, pools: vec![], ..Default::default() } },
            CachedProviderQuota { provider: ProviderId::ClaudeCode, fetched_at: second, stale: true, quota: ProviderQuota { provider: ProviderId::ClaudeCode, pools: vec![], ..Default::default() } },
        ]};
        assert_eq!(snapshot.providers[0].fetched_at, first);
        assert!(snapshot.providers[1].stale);
    }

    #[test]
    fn replacing_cache_leaves_a_complete_snapshot() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("llm-quota-cache-{suffix}"));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("quota.json");
        let first = CachedQuotaSnapshot { providers: vec![CachedProviderQuota { provider: ProviderId::ChatgptWork, fetched_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap(), stale: false, quota: ProviderQuota::default() }] };
        let second = CachedQuotaSnapshot { providers: vec![CachedProviderQuota { provider: ProviderId::ClaudeCode, fetched_at: DateTime::<Utc>::from_timestamp(1_700_000_060, 0).unwrap(), stale: false, quota: ProviderQuota::default() }] };
        save(&path, &first).unwrap();
        save(&path, &second).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.providers.len(), 1);
        assert_eq!(loaded.providers[0].provider, ProviderId::ClaudeCode);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_provider_keeps_its_last_good_value() {
        let saved = ProviderQuota { provider: ProviderId::ClaudeCode, pools: vec![QuotaPool::default()], ..Default::default() };
        let previous = CachedQuotaSnapshot { providers: vec![CachedProviderQuota { provider: ProviderId::ClaudeCode, fetched_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap(), stale: false, quota: saved.clone() }] };
        let refreshed = retain_last_good(&previous, vec![(ProviderId::ClaudeCode, Err(crate::providers::ProviderError::Network))], DateTime::<Utc>::from_timestamp(1_700_000_060, 0).unwrap());
        assert_eq!(refreshed.providers[0].quota, saved);
        assert!(refreshed.providers[0].stale);
        assert_eq!(refreshed.providers[0].fetched_at, previous.providers[0].fetched_at);
    }
}
