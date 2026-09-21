use chrono::{DateTime, Utc};
use serde::Deserialize;
use crate::{auth::claude, providers::{client, status_error, ProviderError, QuotaProvider}, quota::{ProviderId, ProviderQuota, QuotaPool, QuotaWindow}};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";

pub struct ClaudeProvider;
#[derive(Deserialize)] struct UsageResponse { five_hour: Option<Bucket>, seven_day: Option<Bucket> }
#[derive(Deserialize)] struct Bucket { utilization: Option<f64>, resets_at: Option<String> }

impl QuotaProvider for ClaudeProvider {
    fn fetch(&self) -> Result<ProviderQuota, ProviderError> {
        let credentials = claude::read()?;
        let response = client()?.get(USAGE_URL).bearer_auth(&credentials.access_token).header("anthropic-beta", OAUTH_BETA).header(reqwest::header::USER_AGENT, "claude-code").send().map_err(|_| ProviderError::Network)?;
        if !response.status().is_success() { return Err(status_error(&response)); }
        parse_response(response.json().map_err(|_| ProviderError::MalformedResponse)?)
    }
}

fn parse_time(value: Option<String>) -> Option<DateTime<Utc>> { value.and_then(|time| DateTime::parse_from_rfc3339(&time).ok()).map(|time| time.with_timezone(&Utc)) }

fn parse_response(usage: UsageResponse) -> Result<ProviderQuota, ProviderError> {
    let map = |bucket: Option<Bucket>| bucket.map(|bucket| QuotaWindow::from_used_percent(bucket.utilization, parse_time(bucket.resets_at))).unwrap_or_default();
    let pool = QuotaPool { name: None, five_hour: map(usage.five_hour), weekly: map(usage.seven_day) };
    if pool.five_hour.remaining_percent.is_none() && pool.weekly.remaining_percent.is_none() { return Err(ProviderError::MalformedResponse); }
    Ok(ProviderQuota { provider: ProviderId::ClaudeCode, pools: vec![pool], ..Default::default() })
}

#[cfg(test)] mod tests { use super::*; #[test] fn rejects_invalid_timestamp() { assert!(parse_time(Some("not-a-date".into())).is_none()); }
#[test] fn rejects_actual_empty_usage_dto() { let usage: UsageResponse = serde_json::from_str(r#"{}"#).unwrap(); assert!(matches!(parse_response(usage), Err(ProviderError::MalformedResponse))); }
#[test] fn maps_actual_usage_dto() { let usage: UsageResponse = serde_json::from_str(r#"{"five_hour":{"utilization":2.0,"resets_at":"2026-09-20T00:00:00Z"},"seven_day":{"utilization":25.0,"resets_at":"2026-09-25T00:00:00Z"}}"#).unwrap(); let pool = parse_response(usage).unwrap().pools.remove(0); assert_eq!(pool.five_hour.remaining_percent, Some(98.0)); assert_eq!(pool.weekly.remaining_percent, Some(75.0)); } }
