//! GitHub release update detection.
//!
//! This is read-only on purpose: it compares this build's version with the
//! repository's latest published release and reports the result. It never
//! downloads, replaces, or executes anything. Opening the release page stays a
//! deliberate user action from the context menu or the overlay's update badge.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The GitHub repository releases are published to, written as `owner/repo`.
///
/// An empty value switches update checking off, which is the correct state
/// before the repository exists. After publishing, set this one line; no other
/// file needs to change. Releases must be tagged `vMAJOR.MINOR.PATCH` so the tag
/// matches the version in `Cargo.toml` and `tauri.conf.json`.
pub const GITHUB_REPO: &str = "ccyan122/agent-quota-overlay";

const API_ORIGIN: &str = "https://api.github.com";
/// The only origin a detected release link may point at. A response is
/// untrusted input, so a link outside this origin is discarded rather than
/// stored and later handed to the shell.
const RELEASE_ORIGIN: &str = "https://github.com/";
const USER_AGENT: &str = concat!("llm-quota-overlay/", env!("CARGO_PKG_VERSION"));

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long a successful check stays good. Unauthenticated GitHub requests are
/// rate limited per address, and a HUD has no reason to poll releases often.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// No repository is configured, so nothing was requested.
    #[default]
    Unconfigured,
    /// Not checked yet in this run.
    Pending,
    UpToDate,
    Available,
    /// The last check could not complete. Any previous result is kept.
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub state: CheckState,
    pub current_version: String,
    pub latest_version: Option<String>,
    /// Always a `https://github.com/` link for the configured repository, or none.
    pub release_url: Option<String>,
    pub checked_at: Option<DateTime<Utc>>,
}

impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            state: if repository().is_some() { CheckState::Pending } else { CheckState::Unconfigured },
            current_version: CURRENT_VERSION.to_owned(),
            latest_version: None,
            release_url: None,
            checked_at: None,
        }
    }
}

impl UpdateStatus {
    pub fn update_available(&self) -> bool { self.state == CheckState::Available }
}

#[derive(Deserialize)]
struct Release {
    tag_name: Option<String>,
    html_url: Option<String>,
}

/// `Some` only for a well-formed `owner/repo`. Validating the constant here
/// keeps a typo from turning into a request to an unintended URL.
pub fn repository() -> Option<&'static str> { validate_slug(GITHUB_REPO) }

fn validate_slug(value: &'static str) -> Option<&'static str> {
    let value = value.trim();
    let mut parts = value.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    let segment_ok = |segment: &str| !segment.is_empty() && segment.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    (parts.next().is_none() && segment_ok(owner) && segment_ok(name)).then_some(value)
}

/// The releases page for the configured repository, used when a response
/// carries no usable link of its own.
pub fn releases_page() -> Option<String> { repository().map(|repo| format!("{RELEASE_ORIGIN}{repo}/releases")) }

/// Blocking; callers run it off the UI thread. A failure keeps the previously
/// known version rather than reporting an "up to date" it cannot prove.
pub fn check(previous: &UpdateStatus) -> UpdateStatus {
    let Some(repo) = repository() else { return UpdateStatus::default() };
    let failed = || UpdateStatus { state: CheckState::Failed, checked_at: Some(Utc::now()), ..previous.clone() };
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else { return failed() };
    let response = client
        .get(format!("{API_ORIGIN}/repos/{repo}/releases/latest"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send();
    let Ok(response) = response else { return failed() };
    if !response.status().is_success() { return failed() }
    let Ok(release) = response.json::<Release>() else { return failed() };
    resolve(release, previous)
}

fn resolve(release: Release, previous: &UpdateStatus) -> UpdateStatus {
    let Some(tag) = release.tag_name.as_deref().map(str::trim).filter(|tag| !tag.is_empty()) else {
        return UpdateStatus { state: CheckState::Failed, checked_at: Some(Utc::now()), ..previous.clone() };
    };
    let newer = is_newer(tag, CURRENT_VERSION);
    UpdateStatus {
        state: if newer { CheckState::Available } else { CheckState::UpToDate },
        current_version: CURRENT_VERSION.to_owned(),
        latest_version: Some(normalize(tag).to_owned()),
        release_url: release.html_url.as_deref().and_then(safe_release_url).or_else(releases_page),
        checked_at: Some(Utc::now()),
    }
}

/// Only a GitHub link under the configured repository is kept. Anything else in
/// the response is dropped instead of being opened later.
fn safe_release_url(value: &str) -> Option<String> {
    let repo = repository()?;
    let expected = format!("{RELEASE_ORIGIN}{repo}/");
    value.starts_with(&expected).then(|| value.to_owned())
}

fn normalize(tag: &str) -> &str { tag.trim().trim_start_matches(['v', 'V']) }

/// True only when `latest` is provably greater. An unparseable version never
/// claims an update: a wrong "new version available" badge is worse than a
/// missed one, because the user cannot verify it from the overlay.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(latest), Some(current)) = (parse(latest), parse(current)) else { return false };
    match latest.0.cmp(&current.0) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // Equal release numbers: a final release outranks any pre-release of it,
        // and two pre-releases fall back to their own ordering.
        std::cmp::Ordering::Equal => match (latest.1, current.1) {
            (None, Some(_)) => true,
            (Some(_), None) | (None, None) => false,
            (Some(left), Some(right)) => left > right,
        },
    }
}

/// `1.2.3` and `v1.2` parse; missing components are zero. A pre-release suffix
/// is kept separately so `1.0.0-rc.1` sorts below `1.0.0`.
fn parse(value: &str) -> Option<([u64; 3], Option<String>)> {
    let value = normalize(value);
    let (core, pre) = match value.split_once(['-', '+']) {
        Some((core, pre)) => (core, Some(pre.to_owned())),
        None => (value, None),
    };
    if core.trim().is_empty() { return None }
    let mut numbers = [0_u64; 3];
    let mut parts = core.split('.');
    for slot in numbers.iter_mut() {
        let Some(part) = parts.next() else { break };
        *slot = part.trim().parse().ok()?;
    }
    if parts.next().is_some() { return None }
    Some((numbers, pre))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn only_a_well_formed_slug_is_accepted() {
        assert_eq!(validate_slug("owner/repo"), Some("owner/repo"));
        assert_eq!(validate_slug("  owner/repo  "), Some("owner/repo"));
        assert_eq!(validate_slug("owner/repo/extra"), None);
        assert_eq!(validate_slug("owner repo"), None);
        assert_eq!(validate_slug("/repo"), None);
        assert_eq!(validate_slug(""), None);
    }

    #[test] fn newer_versions_are_detected_and_older_ones_are_not() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
    }

    #[test] fn a_prerelease_never_outranks_its_final_release() {
        assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(is_newer("1.0.0-rc.2", "1.0.0-rc.1"));
    }

    #[test] fn short_and_padded_versions_compare_by_value() {
        assert!(is_newer("v1.2", "1.1.9"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(is_newer("1.10.0", "1.9.0"));
    }

    #[test] fn unparseable_versions_never_claim_an_update() {
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("1.2.3.4", "0.1.0"));
        assert!(!is_newer("v..", "0.1.0"));
    }

    #[test] fn a_failed_check_keeps_the_previous_result() {
        let previous = UpdateStatus { state: CheckState::Available, latest_version: Some("0.9.0".into()), ..UpdateStatus::default() };
        let status = resolve(Release { tag_name: None, html_url: None }, &previous);
        assert_eq!(status.state, CheckState::Failed);
        assert_eq!(status.latest_version.as_deref(), Some("0.9.0"));
    }

    #[test] fn a_link_outside_the_configured_repository_is_discarded() {
        assert_eq!(safe_release_url("https://example.invalid/owner/repo/releases/tag/v9"), None);
        assert_eq!(safe_release_url("file:///C:/Windows/System32/cmd.exe"), None);
        assert_eq!(safe_release_url("https://github.com.evil.invalid/x/y/"), None);
    }
}
