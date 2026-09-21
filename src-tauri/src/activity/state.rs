use std::{collections::HashMap, time::{Duration, Instant}};

use serde::{Deserialize, Serialize};

pub use crate::quota::ProviderId as ActivityProvider;

/// The only activity information exposed to the webview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivity {
    Active,
    Idle,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderActivity {
    pub provider: ActivityProvider,
    pub state: AgentActivity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityEventKind {
    Active,
    /// An explicit final-turn stop. Intermediate model/tool events must not map here.
    Idle,
}

#[derive(Debug, Clone)]
struct ActiveSession {
    last_seen: Instant,
}

#[derive(Debug, Default)]
struct ProviderState {
    /// False means no verified lifecycle signal has ever arrived.
    observed: bool,
    active_sessions: HashMap<String, ActiveSession>,
}

/// Reducer for hook events. It tracks opaque session identifiers so concurrent turns
/// for one provider cannot clear one another. It never inspects process existence.
#[derive(Debug)]
pub struct ActivityRegistry {
    providers: HashMap<ActivityProvider, ProviderState>,
    timeout: Duration,
}

impl Default for ActivityRegistry {
    fn default() -> Self {
        Self::new(Duration::from_secs(10 * 60))
    }
}

impl ActivityRegistry {
    pub fn new(timeout: Duration) -> Self {
        Self { providers: HashMap::new(), timeout }
    }

    pub fn timeout(&self) -> Duration { self.timeout }

    /// Returns true only when the public activity state changed. Heartbeats still
    /// refresh their session timestamp but do not cause a webview-wide update.
    pub fn ingest(&mut self, provider: ActivityProvider, event: ActivityEventKind, session_id: Option<&str>, now: Instant) -> bool {
        let state = self.providers.entry(provider).or_default();
        let was_observed = state.observed;
        let was_active = !state.active_sessions.is_empty();
        state.observed = true;
        // Hooks which cannot supply a session receive one provider-scoped slot.
        // A matching stop only clears that slot; it cannot clear known parallel sessions.
        let key = session_id.unwrap_or("_provider_scope_").to_owned();
        match event {
            ActivityEventKind::Active => { state.active_sessions.insert(key, ActiveSession { last_seen: now }); }
            ActivityEventKind::Idle => { state.active_sessions.remove(&key); }
        }
        !was_observed || was_active != !state.active_sessions.is_empty()
    }

    /// A repeated active signal is a heartbeat. Stale active sessions become idle,
    /// which makes crashes and failed stop hooks harmless after the watchdog period.
    pub fn expire(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for state in self.providers.values_mut() {
            let before = state.active_sessions.len();
            state.active_sessions.retain(|_, session| now.saturating_duration_since(session.last_seen) < self.timeout);
            changed |= before != state.active_sessions.len();
        }
        changed
    }

    pub fn activity(&self, provider: ActivityProvider) -> AgentActivity {
        match self.providers.get(&provider) {
            None | Some(ProviderState { observed: false, .. }) => AgentActivity::Unknown,
            Some(state) if state.active_sessions.is_empty() => AgentActivity::Idle,
            Some(_) => AgentActivity::Active,
        }
    }

    pub fn snapshot(&self) -> Vec<ProviderActivity> {
        [ActivityProvider::ChatgptWork, ActivityProvider::ClaudeCode, ActivityProvider::Antigravity]
            .into_iter()
            .map(|provider| ProviderActivity { provider, state: self.activity(provider) })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_until_a_lifecycle_event_arrives() {
        assert_eq!(ActivityRegistry::default().activity(ActivityProvider::ClaudeCode), AgentActivity::Unknown);
    }

    #[test]
    fn sessions_overlap_without_clearing_each_other() {
        let start = Instant::now();
        let mut registry = ActivityRegistry::default();
        registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Active, Some("a"), start);
        registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Active, Some("b"), start);
        registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Idle, Some("a"), start);
        assert_eq!(registry.activity(ActivityProvider::ClaudeCode), AgentActivity::Active);
        registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Idle, Some("b"), start);
        assert_eq!(registry.activity(ActivityProvider::ClaudeCode), AgentActivity::Idle);
    }

    #[test]
    fn watchdog_clears_a_missing_stop() {
        let start = Instant::now();
        let mut registry = ActivityRegistry::new(Duration::from_secs(600));
        registry.ingest(ActivityProvider::Antigravity, ActivityEventKind::Active, Some("opaque"), start);
        assert!(registry.expire(start + Duration::from_secs(600)));
        assert_eq!(registry.activity(ActivityProvider::Antigravity), AgentActivity::Idle);
    }

    #[test]
    fn provider_scoped_stop_does_not_clear_an_opaque_parallel_session() {
        let now = Instant::now();
        let mut registry = ActivityRegistry::default();
        registry.ingest(ActivityProvider::ChatgptWork, ActivityEventKind::Active, None, now);
        registry.ingest(ActivityProvider::ChatgptWork, ActivityEventKind::Active, Some("turn_9"), now);
        registry.ingest(ActivityProvider::ChatgptWork, ActivityEventKind::Idle, None, now);
        assert_eq!(registry.activity(ActivityProvider::ChatgptWork), AgentActivity::Active);
    }

    #[test]
    fn providers_are_independently_active_at_the_same_time() {
        let now = Instant::now();
        let mut registry = ActivityRegistry::default();
        registry.ingest(ActivityProvider::ChatgptWork, ActivityEventKind::Active, Some("codex_turn"), now);
        registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Active, Some("claude_turn"), now);
        registry.ingest(ActivityProvider::Antigravity, ActivityEventKind::Active, Some("gravity_turn"), now);
        for provider in [ActivityProvider::ChatgptWork, ActivityProvider::ClaudeCode, ActivityProvider::Antigravity] {
            assert_eq!(registry.activity(provider), AgentActivity::Active);
        }

        registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Idle, Some("claude_turn"), now);
        assert_eq!(registry.activity(ActivityProvider::ClaudeCode), AgentActivity::Idle);
        assert_eq!(registry.activity(ActivityProvider::ChatgptWork), AgentActivity::Active);
        assert_eq!(registry.activity(ActivityProvider::Antigravity), AgentActivity::Active);
    }

    #[test]
    fn heartbeat_refreshes_without_publishing_a_duplicate_state() {
        let now = Instant::now();
        let mut registry = ActivityRegistry::default();
        assert!(registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Active, Some("turn"), now));
        assert!(!registry.ingest(ActivityProvider::ClaudeCode, ActivityEventKind::Active, Some("turn"), now + Duration::from_secs(1)));
    }
}
