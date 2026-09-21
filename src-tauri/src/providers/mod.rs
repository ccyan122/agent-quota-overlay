mod antigravity;
pub(crate) mod chatgpt;
mod claude;

pub use antigravity::AntigravityProvider;
pub use chatgpt::ChatgptProvider;
pub use claude::ClaudeProvider;

use std::time::Duration;
use chrono::{DateTime, Utc};
use crate::quota::ProviderQuota;

pub const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("credentials unavailable")]
    CredentialsUnavailable,
    #[error("credentials unreadable")]
    CredentialsUnreadable,
    #[error("credentials malformed")]
    CredentialsMalformed,
    #[error("authentication rejected ({0})")]
    AuthenticationRejected(u16),
    #[error("rate limited; retry after {retry_after_seconds:?} seconds")]
    RateLimited { retry_after_seconds: Option<u64> },
    #[error("remote service failure ({0})")]
    ServiceFailure(u16),
    #[error("network request failed")]
    Network,
    #[error("quota response was malformed")]
    MalformedResponse,
}

impl From<crate::auth::AuthError> for ProviderError {
    fn from(value: crate::auth::AuthError) -> Self {
        match value {
            crate::auth::AuthError::Missing => Self::CredentialsUnavailable,
            crate::auth::AuthError::Unreadable => Self::CredentialsUnreadable,
            crate::auth::AuthError::Malformed => Self::CredentialsMalformed,
        }
    }
}

pub trait QuotaProvider: Send + Sync {
    fn fetch(&self) -> Result<ProviderQuota, ProviderError>;
}

fn client() -> Result<reqwest::blocking::Client, ProviderError> {
    // Quota endpoints are fixed first-party HTTPS hosts. Do not forward a
    // bearer credential to an unexpected redirect destination.
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ProviderError::Network)
}

fn status_error(response: &reqwest::blocking::Response) -> ProviderError {
    let code = response.status().as_u16();
    match code {
        401 | 403 => ProviderError::AuthenticationRejected(code),
        429 => ProviderError::RateLimited { retry_after_seconds: response.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(parse_retry_after) },
        500..=599 => ProviderError::ServiceFailure(code),
        _ => ProviderError::Network,
    }
}

/// RFC 9110 permits either a delta in seconds or an HTTP date.  Dates in the
/// past mean the caller may retry now. Invalid values deliberately do not turn
/// into a made-up delay.
pub(crate) fn parse_retry_after(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.trim().parse::<u64>() { return Some(seconds); }
    let retry_at = DateTime::parse_from_rfc2822(value.trim()).ok()?.with_timezone(&Utc);
    (retry_at - Utc::now()).to_std().ok().map(|delay| delay.as_secs())
}

/// Per-provider failures remain isolated. The caller keeps successful results and prior cache entries.
pub fn fetch_all() -> Vec<(crate::quota::ProviderId, Result<ProviderQuota, ProviderError>)> {
    fetch_selected(&[
        crate::quota::ProviderId::ChatgptWork,
        crate::quota::ProviderId::ClaudeCode,
        crate::quota::ProviderId::Antigravity,
    ])
}

/// Fetch only providers whose retry policy currently permits a network request.
/// This lets 401/403 and 429 backoff for one provider leave healthy providers polling.
pub fn fetch_selected(eligible: &[crate::quota::ProviderId]) -> Vec<(crate::quota::ProviderId, Result<ProviderQuota, ProviderError>)> {
    let providers: Vec<(crate::quota::ProviderId, Box<dyn QuotaProvider>)> = vec![
        (crate::quota::ProviderId::ChatgptWork, Box::new(ChatgptProvider) as Box<dyn QuotaProvider>),
        (crate::quota::ProviderId::ClaudeCode, Box::new(ClaudeProvider) as Box<dyn QuotaProvider>),
        (crate::quota::ProviderId::Antigravity, Box::new(AntigravityProvider) as Box<dyn QuotaProvider>),
    ].into_iter().filter(|(id, _)| eligible.contains(id)).collect();
    run_provider_jobs(providers)
}

/// Shared production runner. Each provider has its own worker so a slow or
/// failing request cannot prevent the other providers from completing.
fn run_provider_jobs(providers: Vec<(crate::quota::ProviderId, Box<dyn QuotaProvider>)>) -> Vec<(crate::quota::ProviderId, Result<ProviderQuota, ProviderError>)> {
    let jobs: Vec<_> = providers.into_iter().map(|(id, provider)| std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.fetch()))
            .unwrap_or(Err(ProviderError::Network));
        (id, outcome)
    })).collect();
    jobs.into_iter().map(|job| job.join().unwrap_or((crate::quota::ProviderId::ChatgptWork, Err(ProviderError::Network)))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Success; impl QuotaProvider for Success { fn fetch(&self) -> Result<ProviderQuota, ProviderError> { Ok(ProviderQuota::default()) } }
    struct Failure; impl QuotaProvider for Failure { fn fetch(&self) -> Result<ProviderQuota, ProviderError> { Err(ProviderError::Network) } }
    #[test] fn an_independent_provider_failure_does_not_cancel_success() {
        let values = run_provider_jobs(vec![
            (crate::quota::ProviderId::ChatgptWork, Box::new(Success)),
            (crate::quota::ProviderId::ClaudeCode, Box::new(Failure)),
        ]);
        assert!(values[0].1.is_ok()); assert!(values[1].1.is_err());
    }
    #[test] fn retry_after_accepts_delta_seconds() { assert_eq!(parse_retry_after("120"), Some(120)); }
    #[test] fn retry_after_accepts_http_date() {
        let value = (Utc::now() + chrono::Duration::seconds(90)).to_rfc2822();
        assert!(parse_retry_after(&value).is_some_and(|seconds| (88..=90).contains(&seconds)));
    }
    #[test] fn retry_after_rejects_garbage() { assert_eq!(parse_retry_after("later"), None); }
    #[test] fn classifies_real_http_statuses_and_retry_after_header() {
        use std::{io::{Read, Write}, net::TcpListener, thread};
        fn response(status: &str, retry_after: Option<&str>) -> reqwest::blocking::Response {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let status = status.to_owned();
            let retry_after = retry_after.map(str::to_owned);
            thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                let header = retry_after.map(|value| format!("Retry-After: {value}\r\n")).unwrap_or_default();
                write!(stream, "HTTP/1.1 {status}\r\n{header}Content-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            client().unwrap().get(format!("http://{address}/usage")).send().unwrap()
        }
        assert!(matches!(status_error(&response("401 Unauthorized", None)), ProviderError::AuthenticationRejected(401)));
        assert!(matches!(status_error(&response("403 Forbidden", None)), ProviderError::AuthenticationRejected(403)));
        assert!(matches!(status_error(&response("429 Too Many Requests", Some("12"))), ProviderError::RateLimited { retry_after_seconds: Some(12) }));
        assert!(matches!(status_error(&response("503 Service Unavailable", None)), ProviderError::ServiceFailure(503)));
    }
}
