use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Stable, frontend-safe provider identifiers. `ChatGPT Work` is a display concern.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId { ChatgptWork, ClaudeCode, Antigravity }

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// Always remaining percentage, clamped to 0..=100. Null means unavailable.
    pub remaining_percent: Option<f64>,
    /// RFC 3339 UTC timestamp only when the provider reported one.
    pub reset_at: Option<DateTime<Utc>>,
}

impl QuotaWindow {
    pub fn from_used_percent(used_percent: Option<f64>, reset_at: Option<DateTime<Utc>>) -> Self {
        Self { remaining_percent: used_percent.filter(|used| used.is_finite()).map(|used| clamp_percent(100.0 - used)), reset_at }
    }

    pub fn from_remaining_fraction(remaining: Option<f64>, reset_at: Option<DateTime<Utc>>) -> Self {
        Self { remaining_percent: remaining.filter(|value| value.is_finite()).map(|value| clamp_percent(value * 100.0)), reset_at }
    }
}

pub fn clamp_percent(value: f64) -> f64 { value.clamp(0.0, 100.0) }

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaPool {
    /// Antigravity uses `Gemini` and `Claude/GPT`; the other providers use null.
    pub name: Option<String>,
    pub five_hour: QuotaWindow,
    pub weekly: QuotaWindow,
}

/// One Codex on-demand rate-limit reset credit. Spending one immediately clears
/// the 5-hour and weekly windows; this app only ever reports them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredit {
    /// RFC 3339 UTC, only when the provider reported an expiry for this credit.
    pub expires_at: Option<DateTime<Utc>>,
}

/// Codex reset credits. The count is always available; the per-credit expiry
/// list comes from a separate endpoint, so `expiries_known` distinguishes
/// "no expiries reported" from "expiries were never fetched".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredits {
    pub available_count: u32,
    /// Still-available credits only, sorted soonest expiry first.
    pub credits: Vec<ResetCredit>,
    pub expiries_known: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuota {
    pub provider: ProviderId,
    pub pools: Vec<QuotaPool>,
    /// Codex only. Absent for every other provider, and for cache records
    /// written before reset credits were read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits: Option<ResetCredits>,
    /// Backend-only authentication context for preventing detail data from an
    /// in-flight previous account being applied after an account switch.
    #[serde(skip)]
    pub(crate) account_context: Option<String>,
}

impl Default for ProviderId { fn default() -> Self { Self::ChatgptWork } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn used_becomes_remaining_and_clamps() {
        assert_eq!(QuotaWindow::from_used_percent(Some(28.0), None).remaining_percent, Some(72.0));
        assert_eq!(QuotaWindow::from_used_percent(Some(-2.0), None).remaining_percent, Some(100.0));
        assert_eq!(QuotaWindow::from_used_percent(Some(300.0), None).remaining_percent, Some(0.0));
    }
    #[test] fn malformed_number_becomes_unavailable() { assert_eq!(QuotaWindow::from_used_percent(Some(f64::NAN), None).remaining_percent, None); }
}
