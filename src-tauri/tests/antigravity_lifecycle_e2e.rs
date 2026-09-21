#![cfg(windows)]

use std::{fs, path::Path, sync::{atomic::{AtomicUsize, Ordering}, Arc}, time::{Duration, Instant}};

use llm_quota_overlay::activity::{hooks::{plan_install, HookTarget}, ipc::run_named_pipe_at, ActivityProvider, ActivityRegistry, AgentActivity};
use tokio::sync::Mutex;
use tokio::process::Command;

/// Runs an authenticated local Antigravity CLI turn against a throwaway home
/// directory. agy 1.2.7 loads hooks only from the global
/// `~/.gemini/config/hooks.json` (a workspace `.agents/hooks.json` is not loaded),
/// so the test points USERPROFILE/HOME at a temporary directory holding only that
/// file. The CLI signs in through the Windows keyring, so the user's real
/// configuration is never read or changed.
#[tokio::test]
#[ignore = "requires an authenticated local Antigravity CLI turn"]
async fn installed_antigravity_emits_active_then_final_idle_from_global_hooks() {
    assert_eq!(std::env::var("LLM_OVERLAY_RUN_ANTIGRAVITY_E2E").as_deref(), Ok("1"), "set LLM_OVERLAY_RUN_ANTIGRAVITY_E2E=1 to authorize the live turn");
    let home = std::env::temp_dir().join(format!("llm-overlay-antigravity-e2e-{}", std::process::id()));
    let workspace = home.join("ws");
    fs::create_dir_all(&workspace).expect("temporary workspace");
    // The same planner the installer uses, so this also covers the written command format.
    let plan = plan_install(None, home.join(".gemini").join("config").join("hooks.json"), HookTarget::Antigravity, Path::new(env!("CARGO_BIN_EXE_llm-overlay-hook"))).expect("hook plan");
    llm_quota_overlay::activity::hooks::apply_plan(&plan).expect("temporary global hooks");

    let registry = Arc::new(Mutex::new(ActivityRegistry::default()));
    let changes = Arc::new(AtomicUsize::new(0));
    let pipe_name = format!(r"\\.\pipe\llm-quota-overlay-antigravity-e2e-{}", std::process::id());
    let listener_changes = Arc::clone(&changes);
    let server = tokio::spawn(run_named_pipe_at(Arc::clone(&registry), Arc::new(move || { listener_changes.fetch_add(1, Ordering::Relaxed); }), pipe_name.clone()));
    tokio::time::sleep(Duration::from_millis(20)).await;

    let started = Instant::now();
    let mut child = Command::new("agy")
        .current_dir(&workspace)
        .env("LLM_OVERLAY_PIPE_NAME", &pipe_name)
        .env("USERPROFILE", &home)
        .env("HOME", &home)
        .args(["--mode", "plan", "--print-timeout", "45s", "--print", "Reply with exactly OK."])
        .stdin(std::process::Stdio::null())
        .spawn()
        .expect("installed Antigravity CLI starts");
    let status = match tokio::time::timeout(Duration::from_secs(60), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => panic!("Antigravity CLI wait failed: {error}"),
        Err(_) => { let _ = child.kill().await; panic!("minimal Antigravity turn exceeded 60 seconds"); }
    };
    assert!(status.success(), "minimal Antigravity turn completed");

    for _ in 0..50 {
        if changes.load(Ordering::Relaxed) >= 2 && registry.lock().await.activity(ActivityProvider::Antigravity) == AgentActivity::Idle { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(changes.load(Ordering::Relaxed) >= 2, "received active and final idle lifecycle transitions");
    assert_eq!(registry.lock().await.activity(ActivityProvider::Antigravity), AgentActivity::Idle);
    let report_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("test-results");
    fs::create_dir_all(&report_dir).expect("sanitized report directory");
    fs::write(
        report_dir.join("antigravity-lifecycle-e2e.json"),
        format!(r#"{{"provider":"antigravity","result":"pass","stage":"final_idle_observed","hooksLocation":"global ~/.gemini/config/hooks.json (isolated home)","lifecycleTransitions":{},"finalState":"idle","durationMs":{},"capturedAt":"{}"}}"#, changes.load(Ordering::Relaxed), started.elapsed().as_millis(), chrono::Utc::now().to_rfc3339()),
    ).expect("sanitized report");
    server.abort();
    let _ = fs::remove_dir_all(home);
}
