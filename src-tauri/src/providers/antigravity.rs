use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{process::{Command, Stdio}, time::{Duration, Instant}};
use crate::{auth::antigravity, providers::{client, status_error, ProviderError, QuotaProvider}, quota::{ProviderId, ProviderQuota, QuotaPool, QuotaWindow}};

const ENDPOINTS: &[&str] = &["https://daily-cloudcode-pa.googleapis.com", "https://daily-cloudcode-pa.sandbox.googleapis.com", "https://cloudcode-pa.googleapis.com"];

pub struct AntigravityProvider;
#[derive(Deserialize)] struct LoadResponse { #[serde(rename = "cloudaicompanionProject")] project: Option<String> }
#[derive(Deserialize)] struct Summary { groups: Option<Vec<Group>> }
#[derive(Deserialize)] struct Group { #[serde(rename = "displayName")] display_name: Option<String>, description: Option<String>, buckets: Option<Vec<Bucket>> }
#[derive(Deserialize)] struct Bucket { #[serde(rename = "bucketId")] bucket_id: Option<String>, #[serde(rename = "displayName")] bucket_display_name: Option<String>, window: Option<String>, #[serde(rename = "remainingFraction")] remaining_fraction: Option<f64>, #[serde(rename = "resetTime")] reset_time: Option<String> }

impl QuotaProvider for AntigravityProvider {
    fn fetch(&self) -> Result<ProviderQuota, ProviderError> {
        // The desktop language server is authoritative while Antigravity is
        // open: it reports the same two quota groups the application shows.
        // Its short-lived CSRF value remains process-local and is never
        // persisted, logged, or returned to the frontend.
        if let Ok(quota) = fetch_local_language_server() { return Ok(quota); }
        let credentials = antigravity::read()?;
        let mut last = ProviderError::Network;
        for endpoint in ENDPOINTS {
            match fetch_endpoint(endpoint, &credentials.access_token) { Ok(quota) => return Ok(quota), Err(error @ ProviderError::AuthenticationRejected(_)) => return Err(error), Err(error) => last = error }
        }
        Err(last)
    }
}

const LOCAL_QUOTA_RPC: &str = "/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary";

fn fetch_local_language_server() -> Result<ProviderQuota, ProviderError> {
    let candidates = language_server_candidates()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .build().map_err(|_| ProviderError::Network)?;
    let mut last = ProviderError::Network;
    for candidate in candidates {
        let url = format!("https://127.0.0.1:{}{LOCAL_QUOTA_RPC}", candidate.port);
        let response = match client.post(url).header("X-Codeium-Csrf-Token", candidate.csrf).header("Connect-Protocol-Version", "1").json(&serde_json::json!({})).send() { Ok(response) => response, Err(_) => continue };
        if !response.status().is_success() { last = status_error(&response); continue; }
        match map_summary(response.json().map_err(|_| ProviderError::MalformedResponse)?) { Ok(quota) => return Ok(quota), Err(error) => last = error }
    }
    Err(last)
}

#[derive(Deserialize)]
struct LocalLanguageServerCandidate { port: u16, csrf: String }

#[cfg(windows)]
fn language_server_candidates() -> Result<Vec<LocalLanguageServerCandidate>, ProviderError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // WMI reduces each matching command line inside the child process. Only
    // paired localhost port/CSRF values cross the process boundary, in memory.
    let script = "$session=(Get-Process -Id $PID).SessionId;$sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$root=Join-Path $env:LOCALAPPDATA 'Programs\\antigravity\\resources\\bin\\language_server.exe';Get-CimInstance Win32_Process -Filter \"Name='language_server.exe'\"|Where-Object {$owner=Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid -ErrorAction SilentlyContinue;$_.SessionId -eq $session -and $_.ExecutablePath -ieq $root -and $owner.Sid -eq $sid}|ForEach-Object {$p=[regex]::Match($_.CommandLine,'--https_server_port(?:=|\\s+)(?<v>\"[^\"]+\"|\\S+)');$c=[regex]::Match($_.CommandLine,'--csrf_token(?:=|\\s+)(?<v>\"[^\"]+\"|\\S+)');[UInt16]$port=0;if($p.Success){[void][UInt16]::TryParse($p.Groups['v'].Value.Trim('\"'),[ref]$port)};if($port -gt 0 -and $c.Success){[pscustomobject]@{port=$port;csrf=$c.Groups['v'].Value.Trim('\"')}|ConvertTo-Json -Compress}}";
    let mut child = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", script]).creation_flags(CREATE_NO_WINDOW).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_| ProviderError::CredentialsUnavailable)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while child.try_wait().map_err(|_| ProviderError::CredentialsUnavailable)?.is_none() {
        if Instant::now() >= deadline { let _ = child.kill(); let _ = child.wait(); return Err(ProviderError::CredentialsUnavailable); }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().map_err(|_| ProviderError::CredentialsUnavailable)?;
    if !output.status.success() { return Err(ProviderError::CredentialsUnavailable); }
    parse_candidates(&output.stdout)
}

#[cfg(not(windows))]
fn language_server_candidates() -> Result<Vec<LocalLanguageServerCandidate>, ProviderError> { Err(ProviderError::CredentialsUnavailable) }

fn parse_candidates(output: &[u8]) -> Result<Vec<LocalLanguageServerCandidate>, ProviderError> {
    let text = std::str::from_utf8(output).map_err(|_| ProviderError::CredentialsUnavailable)?;
    let candidates: Vec<_> = text.lines().filter_map(|line| serde_json::from_str::<LocalLanguageServerCandidate>(line).ok()).filter(|candidate| !candidate.csrf.is_empty()).collect();
    if candidates.is_empty() { Err(ProviderError::CredentialsUnavailable) } else { Ok(candidates) }
}

fn fetch_endpoint(endpoint: &str, token: &str) -> Result<ProviderQuota, ProviderError> {
    let client = client()?;
    let load = client.post(format!("{endpoint}/v1internal:loadCodeAssist")).bearer_auth(token).header(reqwest::header::USER_AGENT, "antigravity").json(&serde_json::json!({"metadata":{"ideType":"ANTIGRAVITY"}})).send().map_err(|_| ProviderError::Network)?;
    if !load.status().is_success() { return Err(status_error(&load)); }
    let project = load.json::<LoadResponse>().map_err(|_| ProviderError::MalformedResponse)?.project.filter(|value| !value.trim().is_empty()).ok_or(ProviderError::MalformedResponse)?;
    let response = client.post(format!("{endpoint}/v1internal:retrieveUserQuotaSummary")).bearer_auth(token).header(reqwest::header::USER_AGENT, "antigravity").json(&serde_json::json!({"project":project})).send().map_err(|_| ProviderError::Network)?;
    if response.status().is_success() { return map_summary(response.json().map_err(|_| ProviderError::MalformedResponse)?); }
    let summary_error = status_error(&response);
    if matches!(summary_error, ProviderError::AuthenticationRejected(_)) { return Err(summary_error); }
    // A model list does not prove either a shared pool or a 5-hour window.
    // Withhold it rather than relabel per-model availability as quota.
    Err(summary_error)
}

fn map_summary(summary: Summary) -> Result<ProviderQuota, ProviderError> {
    let mut gemini = QuotaPool { name: Some("Gemini".into()), ..Default::default() };
    let mut claude_gpt = QuotaPool { name: Some("Claude/GPT".into()), ..Default::default() };
    let mut seen_gemini = false;
    let mut seen_claude_gpt = false;
    for group in summary.groups.unwrap_or_default() {
        let bucket_text = group.buckets.as_ref().map(|buckets| buckets.iter().map(|bucket| format!("{} {}", bucket.bucket_id.as_deref().unwrap_or_default(), bucket.bucket_display_name.as_deref().unwrap_or_default())).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let text = format!("{} {} {bucket_text}", group.display_name.unwrap_or_default(), group.description.unwrap_or_default()).to_ascii_lowercase();
        let target = if text.contains("gemini") { seen_gemini = true; &mut gemini } else if text.contains("claude") || text.contains("gpt") { seen_claude_gpt = true; &mut claude_gpt } else { continue };
        for bucket in group.buckets.unwrap_or_default() {
            let quota = QuotaWindow::from_remaining_fraction(bucket.remaining_fraction, parse_time(bucket.reset_time));
            match bucket.window.as_deref().map(str::to_ascii_lowercase).as_deref() { Some("5h") | Some("five_hour") => target.five_hour = quota, Some("weekly") | Some("7d") => target.weekly = quota, _ => {} }
        }
    }
    let mut pools = Vec::new(); if seen_gemini { pools.push(gemini); } if seen_claude_gpt { pools.push(claude_gpt); }
    if pools.is_empty() { Err(ProviderError::MalformedResponse) } else { Ok(ProviderQuota { provider: ProviderId::Antigravity, pools, ..Default::default() }) }
}
fn parse_time(value: Option<String>) -> Option<DateTime<Utc>> { value.and_then(|time| DateTime::parse_from_rfc3339(&time).ok()).map(|time| time.with_timezone(&Utc)) }

#[cfg(test)]
mod tests { use super::*; #[test] fn accepts_only_structured_paired_candidates() { let output = b"{\"port\":41000,\"csrf\":\"first\"}\nnot-json\n{\"port\":41001,\"csrf\":\"second\"}\n"; let candidates = parse_candidates(output).unwrap(); assert_eq!(candidates.len(), 2); assert_eq!(candidates[0].port, 41000); assert_eq!(candidates[1].port, 41001); }
#[test] fn rejects_the_string_port_form_the_producer_must_not_emit() { assert!(parse_candidates(b"{\"port\":\"41000\",\"csrf\":\"secret\"}\n").is_err()); }
#[test] fn rejects_empty_or_unpaired_discovery_output() { assert!(parse_candidates(b"").is_err()); assert!(parse_candidates(b"{\"port\":41000,\"csrf\":\"\"}\n").is_err()); }
#[test] fn preserves_separate_pools() { let value = Summary { groups: Some(vec![Group { display_name: Some("Gemini".into()), description: None, buckets: Some(vec![Bucket { bucket_id: None, bucket_display_name: None, window: Some("5h".into()), remaining_fraction: Some(0.8), reset_time: None }]) }, Group { display_name: Some("Claude/GPT".into()), description: None, buckets: Some(vec![Bucket { bucket_id: None, bucket_display_name: None, window: Some("weekly".into()), remaining_fraction: Some(0.4), reset_time: None }]) }]) }; let mapped = map_summary(value).unwrap(); assert_eq!(mapped.pools.len(), 2); assert_eq!(mapped.pools[0].five_hour.remaining_percent, Some(80.0)); assert_eq!(mapped.pools[1].weekly.remaining_percent, Some(40.0)); }
}
