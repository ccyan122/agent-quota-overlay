use std::{collections::HashMap, sync::{Mutex, OnceLock}, time::{Duration, Instant}};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use crate::{auth::codex, providers::{client, status_error, ProviderError, QuotaProvider}, quota::{ProviderId, ProviderQuota, QuotaPool, QuotaWindow, ResetCredit, ResetCredits}};

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// The only source that carries each reset credit's expiry. The usage body
/// carries the count alone, so this call is made best effort and its failure
/// never fails a quota refresh.
const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const FIVE_HOURS_SECONDS: i64 = 18_000;
const WEEK_SECONDS: i64 = 604_800;
/// The detail list is an optional, UI-triggered enhancement.  Keep it out of
/// the normal 60-second usage path and avoid hammering it while the Work card
/// is opened repeatedly.
const RESET_DETAIL_TTL: Duration = Duration::from_secs(5 * 60);
const RESET_DETAIL_FAILURE_BACKOFF: Duration = Duration::from_secs(60);

pub struct ChatgptProvider;
pub struct ResetCreditDetails {
    pub account_context: String,
    pub credits: ResetCredits,
}

#[derive(Deserialize)] struct UsageResponse {
    #[serde(alias = "rateLimit")] rate_limit: Option<RateLimit>,
    #[serde(alias = "rateLimitResetCredits")] rate_limit_reset_credits: Option<ResetCreditsResponse>,
}
#[derive(Deserialize)] struct RateLimit {
    #[serde(alias = "primaryWindow")] primary_window: Option<Window>,
    #[serde(alias = "secondaryWindow")] secondary_window: Option<Window>,
}
#[derive(Deserialize)] struct Window {
    #[serde(alias = "usedPercent")] used_percent: Option<f64>,
    #[serde(alias = "resetAt")] reset_at: Option<i64>,
    #[serde(alias = "limitWindowSeconds")] limit_window_seconds: Option<i64>,
}

/// Shared by the usage body's embedded object and the dedicated endpoint. Only
/// the latter populates `credits`.
#[derive(Deserialize)] struct ResetCreditsResponse {
    #[serde(alias = "availableCount")]
    available_count: Option<f64>,
    #[serde(default)] credits: Vec<CreditEntry>,
}
/// `status` is optional upstream and `expires_at` arrives either as an RFC 3339
/// string or as epoch seconds.
#[derive(Deserialize)] struct CreditEntry {
    status: Option<String>,
    #[serde(alias = "expiresAt")] expires_at: Option<serde_json::Value>,
}

enum ResetDetailCacheEntry {
    Ready { credits: ResetCredits, fresh_until: Instant },
    RetryAt(Instant),
    InFlight,
}

enum ResetDetailCacheDecision {
    Fetch,
    Cached(ResetCredits),
    Suppressed,
}

/// Account IDs are used only as an in-memory scope key; neither these keys nor
/// any credential material is serialized, logged, or sent to the renderer.
static RESET_DETAIL_CACHE: OnceLock<Mutex<HashMap<String, ResetDetailCacheEntry>>> = OnceLock::new();

fn reset_detail_cache() -> &'static Mutex<HashMap<String, ResetDetailCacheEntry>> {
    RESET_DETAIL_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn reserve_reset_detail(cache: &mut HashMap<String, ResetDetailCacheEntry>, account_scope: &str, now: Instant) -> ResetDetailCacheDecision {
    match cache.get(account_scope) {
        Some(ResetDetailCacheEntry::Ready { credits, fresh_until }) if *fresh_until > now => ResetDetailCacheDecision::Cached(credits.clone()),
        Some(ResetDetailCacheEntry::RetryAt(retry_at)) if *retry_at > now => ResetDetailCacheDecision::Suppressed,
        Some(ResetDetailCacheEntry::InFlight) => ResetDetailCacheDecision::Suppressed,
        _ => {
            cache.insert(account_scope.to_owned(), ResetDetailCacheEntry::InFlight);
            ResetDetailCacheDecision::Fetch
        }
    }
}

fn store_reset_detail_result(cache: &mut HashMap<String, ResetDetailCacheEntry>, account_scope: String, result: &Result<ResetCredits, ProviderError>, now: Instant) {
    let next = match result {
        Ok(credits) => ResetDetailCacheEntry::Ready { credits: credits.clone(), fresh_until: now + RESET_DETAIL_TTL },
        Err(ProviderError::RateLimited { retry_after_seconds }) => ResetDetailCacheEntry::RetryAt(now + Duration::from_secs(retry_after_seconds.unwrap_or(60).max(1))),
        Err(_) => ResetDetailCacheEntry::RetryAt(now + RESET_DETAIL_FAILURE_BACKOFF),
    };
    cache.insert(account_scope, next);
}

impl QuotaProvider for ChatgptProvider {
    fn fetch(&self) -> Result<ProviderQuota, ProviderError> {
        let credentials = codex::read()?;
        let account_context = codex::account_context(&credentials);
        let client = client()?;
        let authorize = |url: &str| {
            let mut request = client.get(url).bearer_auth(&credentials.access_token).header(reqwest::header::USER_AGENT, "codex-cli");
            if let Some(account) = credentials.account_id.as_deref() { request = request.header("ChatGPT-Account-Id", account); }
            request
        };
        let response = authorize(USAGE_URL).send().map_err(|_| ProviderError::Network)?;
        if !response.status().is_success() { return Err(status_error(&response)); }
        let usage: UsageResponse = response.json().map_err(|_| ProviderError::MalformedResponse)?;
        let mut quota = parse_response(usage)?;
        quota.account_context = Some(account_context);
        Ok(quota)
    }
}

impl ChatgptProvider {
    /// Gets reset-credit expiries only after the Work detail panel is opened.
    /// A failed detail request never changes the already-fresh usage summary.
    pub fn fetch_reset_credit_details() -> Result<Option<ResetCreditDetails>, ProviderError> {
        let credentials = codex::read()?;
        let account_scope = codex::account_context(&credentials);
        let now = Instant::now();
        {
            let mut cache = reset_detail_cache().lock().map_err(|_| ProviderError::Network)?;
            match reserve_reset_detail(&mut cache, &account_scope, now) {
                ResetDetailCacheDecision::Cached(credits) => return Ok(Some(ResetCreditDetails { account_context: account_scope, credits })),
                ResetDetailCacheDecision::Suppressed => return Ok(None),
                ResetDetailCacheDecision::Fetch => {}
            }
        }

        let result = (|| {
            let client = client()?;
            let mut request = client.get(RESET_CREDITS_URL)
                .bearer_auth(&credentials.access_token)
                .header(reqwest::header::USER_AGENT, "codex-cli");
            if let Some(account) = credentials.account_id.as_deref() {
                request = request.header("ChatGPT-Account-Id", account);
            }
            let response = request.send().map_err(|_| ProviderError::Network)?;
            if !response.status().is_success() { return Err(status_error(&response)); }
            let payload: serde_json::Value = response.json().map_err(|_| ProviderError::MalformedResponse)?;
            parse_reset_credit_detail_payload(&payload).ok_or(ProviderError::MalformedResponse)
        })();

        if let Ok(mut cache) = reset_detail_cache().lock() {
            store_reset_detail_result(&mut cache, account_scope.clone(), &result, Instant::now());
        }
        result.map(|credits| Some(ResetCreditDetails { account_context: account_scope, credits }))
    }
}

fn parse_response(usage: UsageResponse) -> Result<ProviderQuota, ProviderError> {
        let reset_credits = usage.rate_limit_reset_credits.as_ref().and_then(|source| normalize_reset_credits(source, false));
        let rate_limit = usage.rate_limit.ok_or(ProviderError::MalformedResponse)?;
        let mut pool = QuotaPool::default();
        for (window, default_weekly) in [(rate_limit.primary_window, false), (rate_limit.secondary_window, true)] {
            let Some(window) = window else { continue };
            let reset = window.reset_at.and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0));
            let quota = QuotaWindow::from_used_percent(window.used_percent, reset);
            match window.limit_window_seconds {
                Some(FIVE_HOURS_SECONDS) => pool.five_hour = quota,
                Some(WEEK_SECONDS) => pool.weekly = quota,
                // Legacy upstream slots are accepted only if duration is absent.
                None if default_weekly => pool.weekly = quota,
                None => pool.five_hour = quota,
                // Unknown durations are intentionally withheld instead of mislabelled.
                Some(_) => continue,
            }
        }
        if pool.five_hour.remaining_percent.is_none() && pool.weekly.remaining_percent.is_none() { return Err(ProviderError::MalformedResponse); }
        Ok(ProviderQuota { provider: ProviderId::ChatgptWork, pools: vec![pool], reset_credits, account_context: None })
}

/// The dedicated payload replaces the embedded count only when it carries a
/// usable count of its own. A `null` count there must not erase the usage
/// body's answer, which is the only one some accounts get.
#[cfg(test)]
fn apply_dedicated_reset_credits(quota: &mut ProviderQuota, dedicated: Option<ResetCreditsResponse>) {
    if let Some(credits) = dedicated.as_ref().and_then(|source| normalize_reset_credits(source, true)) {
        quota.reset_credits = Some(credits);
    }
}

fn normalize_reset_credits(source: &ResetCreditsResponse, expiries_known: bool) -> Option<ResetCredits> {
    let count = source.available_count.filter(|value| value.is_finite() && *value >= 0.0)?;
    let mut credits: Vec<ResetCredit> = source.credits.iter()
        // A credit is kept when it is explicitly available or carries no status
        // at all; only an explicitly spent or expired one is dropped. Requiring
        // the field would blank the whole list for responses that omit it.
        .filter(|entry| entry.status.as_deref().is_none_or(|status| status == "available"))
        .map(|entry| ResetCredit { expires_at: entry.expires_at.as_ref().and_then(parse_expiry) })
        .collect();
    // Unknown expiries sort last so the timeline still leads with real dates.
    credits.sort_by_key(|credit| (credit.expires_at.is_none(), credit.expires_at));
    Some(ResetCredits { available_count: count.floor().min(f64::from(u32::MAX)) as u32, credits, expiries_known })
}

/// The dedicated endpoint has used both a direct reset-credit object and a
/// wrapper matching the usage response. Accept both without leaking its raw
/// schema outside this provider.
fn parse_reset_credit_detail_payload(payload: &serde_json::Value) -> Option<ResetCredits> {
    let parse = |value: &serde_json::Value| serde_json::from_value::<ResetCreditsResponse>(value.clone()).ok()
        .and_then(|response| normalize_reset_credits(&response, true));
    parse(payload).or_else(|| payload.get("rate_limit_reset_credits")
        .or_else(|| payload.get("rateLimitResetCredits"))
        .and_then(parse))
}

fn parse_expiry(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    match value {
        serde_json::Value::String(text) => DateTime::parse_from_rfc3339(text).ok().map(|value| value.with_timezone(&Utc)),
        serde_json::Value::Number(number) => number.as_f64().filter(|value| value.is_finite()).and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds as i64, 0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn usage(json: &str) -> UsageResponse { serde_json::from_str(json).unwrap() }

    #[test] fn decodes_reversed_windows_by_explicit_duration() {
        let value = usage(r#"{"rate_limit":{"primary_window":{"used_percent":25,"reset_at":1800000000,"limit_window_seconds":604800},"secondary_window":{"used_percent":10,"reset_at":1700000000,"limit_window_seconds":18000}}}"#);
        let pool = parse_response(value).unwrap().pools.remove(0);
        assert_eq!(pool.five_hour.remaining_percent, Some(90.0));
        assert_eq!(pool.weekly.remaining_percent, Some(75.0));
    }
    #[test] fn rejects_unknown_duration_without_fabricating_weekly() {
        let value = usage(r#"{"rate_limit":{"primary_window":{"used_percent":20,"limit_window_seconds":86400}}}"#);
        assert!(matches!(parse_response(value), Err(ProviderError::MalformedResponse)));
    }
    #[test] fn rejects_actual_response_without_rate_limit() {
        assert!(matches!(parse_response(usage("{}")), Err(ProviderError::MalformedResponse)));
    }

    const WINDOWS: &str = r#""rate_limit":{"primary_window":{"used_percent":10,"limit_window_seconds":18000}}"#;

    #[test] fn the_usage_body_supplies_a_count_without_claiming_expiries() {
        let value = usage(&format!(r#"{{{WINDOWS},"rate_limit_reset_credits":{{"available_count":2}}}}"#));
        let credits = parse_response(value).unwrap().reset_credits.unwrap();
        assert_eq!(credits.available_count, 2);
        assert!(credits.credits.is_empty());
        assert!(!credits.expiries_known);
    }
    #[test] fn a_missing_or_null_count_reports_no_reset_credits_rather_than_zero() {
        for body in [format!("{{{WINDOWS}}}"), format!(r#"{{{WINDOWS},"rate_limit_reset_credits":{{"available_count":null}}}}"#)] {
            assert_eq!(parse_response(usage(&body)).unwrap().reset_credits, None);
        }
    }
    #[test] fn zero_available_is_a_real_answer() {
        let value = usage(&format!(r#"{{{WINDOWS},"rate_limit_reset_credits":{{"available_count":0}}}}"#));
        assert_eq!(parse_response(value).unwrap().reset_credits.unwrap().available_count, 0);
    }

    fn dedicated(json: &str) -> Option<ResetCreditsResponse> { serde_json::from_str(json).ok() }
    fn base() -> ProviderQuota { parse_response(usage(&format!(r#"{{{WINDOWS},"rate_limit_reset_credits":{{"available_count":1}}}}"#))).unwrap() }

    #[test] fn the_dedicated_endpoint_adds_expiries_sorted_soonest_first() {
        let mut quota = base();
        apply_dedicated_reset_credits(&mut quota, dedicated(r#"{"available_count":2,"credits":[
            {"status":"available","expires_at":"2026-02-20T19:00:00.000Z"},
            {"expires_at":"2026-02-20T17:30:00.000Z"},
            {"status":"consumed","expires_at":"2026-02-20T16:10:00.000Z"}]}"#));
        let credits = quota.reset_credits.unwrap();
        assert_eq!(credits.available_count, 2);
        assert!(credits.expiries_known);
        assert_eq!(credits.credits.len(), 2);
        assert_eq!(credits.credits[0].expires_at, DateTime::parse_from_rfc3339("2026-02-20T17:30:00Z").ok().map(|v| v.with_timezone(&Utc)));
        assert_eq!(credits.credits[1].expires_at, DateTime::parse_from_rfc3339("2026-02-20T19:00:00Z").ok().map(|v| v.with_timezone(&Utc)));
    }
    #[test] fn epoch_expiries_are_accepted_too() {
        let mut quota = base();
        apply_dedicated_reset_credits(&mut quota, dedicated(r#"{"available_count":1,"credits":[{"expires_at":1800000000}]}"#));
        assert_eq!(quota.reset_credits.unwrap().credits[0].expires_at, DateTime::<Utc>::from_timestamp(1_800_000_000, 0));
    }
    #[test] fn detail_payload_accepts_direct_and_wrapped_camel_case_schemas() {
        let direct = serde_json::json!({"availableCount": 1, "credits": [{"expiresAt": "2026-02-20T19:00:00Z"}]});
        let wrapped = serde_json::json!({"rateLimitResetCredits": {"availableCount": 2, "credits": []}});
        assert_eq!(parse_reset_credit_detail_payload(&direct).unwrap().available_count, 1);
        assert_eq!(parse_reset_credit_detail_payload(&wrapped).unwrap().available_count, 2);
    }
    #[test] fn detail_cache_coalesces_requests_honors_ttl_and_rate_limit_backoff() {
        let now = Instant::now();
        let scope = "test-account";
        let mut cache = HashMap::new();
        assert!(matches!(reserve_reset_detail(&mut cache, scope, now), ResetDetailCacheDecision::Fetch));
        assert!(matches!(reserve_reset_detail(&mut cache, scope, now), ResetDetailCacheDecision::Suppressed));
        let credits = ResetCredits { available_count: 1, ..Default::default() };
        store_reset_detail_result(&mut cache, scope.to_owned(), &Ok(credits.clone()), now);
        assert!(matches!(reserve_reset_detail(&mut cache, scope, now + Duration::from_secs(1)), ResetDetailCacheDecision::Cached(value) if value == credits));
        assert!(matches!(reserve_reset_detail(&mut cache, scope, now + RESET_DETAIL_TTL), ResetDetailCacheDecision::Fetch));
        let limited = Err(ProviderError::RateLimited { retry_after_seconds: Some(90) });
        store_reset_detail_result(&mut cache, scope.to_owned(), &limited, now);
        assert!(matches!(reserve_reset_detail(&mut cache, scope, now + Duration::from_secs(89)), ResetDetailCacheDecision::Suppressed));
        assert!(matches!(reserve_reset_detail(&mut cache, scope, now + Duration::from_secs(90)), ResetDetailCacheDecision::Fetch));
    }
    #[test] fn an_unusable_dedicated_payload_keeps_the_embedded_count() {
        for payload in [r#"{"available_count":null}"#, r#"{"credits":[]}"#, r#"{"available_count":"two"}"#] {
            let mut quota = base();
            apply_dedicated_reset_credits(&mut quota, dedicated(payload));
            let credits = quota.reset_credits.unwrap();
            assert_eq!(credits.available_count, 1);
            assert!(!credits.expiries_known);
        }
    }
    #[test] fn a_failed_dedicated_call_keeps_the_embedded_count() {
        let mut quota = base();
        apply_dedicated_reset_credits(&mut quota, None);
        assert_eq!(quota.reset_credits.unwrap().available_count, 1);
    }
    #[test] fn an_unparseable_expiry_is_kept_as_an_unknown_date_and_sorts_last() {
        let mut quota = base();
        apply_dedicated_reset_credits(&mut quota, dedicated(r#"{"available_count":2,"credits":[{"expires_at":"soon"},{"expires_at":"2026-02-20T19:00:00.000Z"}]}"#));
        let credits = quota.reset_credits.unwrap().credits;
        assert!(credits[0].expires_at.is_some());
        assert_eq!(credits[1].expires_at, None);
    }
}
