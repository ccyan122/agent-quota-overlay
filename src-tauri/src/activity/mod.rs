//! Hook-driven activity detection. Process existence is deliberately never activity.

pub mod ipc;
pub mod hooks;
pub mod setup;
pub mod state;

pub use state::{ActivityProvider, ActivityRegistry, AgentActivity, ProviderActivity};

use std::{sync::Arc, time::{Duration, Instant}};
use tokio::sync::{watch, Mutex};

/// Owns hook-derived activity state for the app lifetime. Consumers receive only
/// provider/state snapshots, never hook payloads or session identifiers.
#[derive(Clone)]
pub struct ActivityService {
    registry: Arc<Mutex<ActivityRegistry>>,
    updates: watch::Sender<Vec<ProviderActivity>>,
}

impl ActivityService {
    pub fn new(timeout: Duration) -> (Self, watch::Receiver<Vec<ProviderActivity>>) {
        let registry = Arc::new(Mutex::new(ActivityRegistry::new(timeout)));
        let initial = [ActivityProvider::ChatgptWork, ActivityProvider::ClaudeCode, ActivityProvider::Antigravity]
            .into_iter()
            .map(|provider| ProviderActivity { provider, state: AgentActivity::Unknown })
            .collect();
        let (updates, receiver) = watch::channel(initial);
        (Self { registry, updates }, receiver)
    }

    pub fn with_default_timeout() -> (Self, watch::Receiver<Vec<ProviderActivity>>) {
        Self::new(Duration::from_secs(10 * 60))
    }

    pub fn registry(&self) -> Arc<Mutex<ActivityRegistry>> { Arc::clone(&self.registry) }

    pub async fn snapshot(&self) -> Vec<ProviderActivity> { self.registry.lock().await.snapshot() }

    pub fn snapshot_now(&self) -> Vec<ProviderActivity> {
        self.registry.try_lock().map(|registry| registry.snapshot()).unwrap_or_default()
    }

    pub fn spawn(&self) {
        let registry = self.registry();
        let updates = self.updates.clone();
        let notify = Arc::new(move || {
            let registry = Arc::clone(&registry);
            let updates = updates.clone();
            tokio::spawn(async move { let _ = updates.send(registry.lock().await.snapshot()); });
        });
        let server_registry = self.registry();
        tokio::spawn(async move { let _ = ipc::run_named_pipe_with_listener(server_registry, notify).await; });

        let registry = self.registry();
        let updates = self.updates.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let mut state = registry.lock().await;
                if state.expire(Instant::now()) { let _ = updates.send(state.snapshot()); }
            }
        });
    }
}
