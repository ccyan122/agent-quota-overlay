#![cfg(windows)]

use std::{ffi::c_void, fs, os::windows::io::AsRawHandle, sync::{atomic::{AtomicUsize, Ordering}, Arc}, time::{Duration, Instant}};

use llm_quota_overlay::activity::{ipc::run_named_pipe_at, ActivityProvider, ActivityRegistry, AgentActivity};
use tokio::sync::Mutex;

struct KillOnCloseJob(windows_sys::Win32::Foundation::HANDLE);

impl KillOnCloseJob {
    fn assign(child: &std::process::Child) -> Result<Self, String> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE}};
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() { return Err("CreateJobObjectW failed".to_owned()); }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(handle, JobObjectExtendedLimitInformation, (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(), std::mem::size_of_val(&limits) as u32) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle().cast()) == 0 {
                let _ = CloseHandle(handle);
                return Err("unable to assign test child to kill-on-close job".to_owned());
            }
            Ok(Self(handle))
        }
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) { unsafe { let _ = windows_sys::Win32::Foundation::CloseHandle(self.0); } }
}

async fn bounded_status(mut command: std::process::Command, timeout: Duration) -> Result<std::process::ExitStatus, String> {
    let mut child = command.spawn().map_err(|error| format!("start failed: {error}"))?;
    let job = KillOnCloseJob::assign(&child)?;
    let wait = tokio::task::spawn_blocking(move || child.wait());
    match tokio::time::timeout(timeout, wait).await {
        Ok(Ok(Ok(status))) => { drop(job); Ok(status) }
        Ok(Ok(Err(error))) => { drop(job); Err(format!("wait failed: {error}")) }
        Ok(Err(error)) => { drop(job); Err(format!("wait task failed: {error}")) }
        Err(_) => { drop(job); Err(format!("timed out after {} seconds; only the test job was terminated", timeout.as_secs())) }
    }
}

/// Uses a throwaway `--settings` file, a throwaway working directory, and a
/// minimal print turn. It never reads user settings, transcript contents, or
/// changes global hook configuration. Run deliberately with:
/// `LLM_OVERLAY_RUN_CLAUDE_E2E=1 cargo test --test claude_lifecycle_e2e -- --ignored`.
#[tokio::test]
#[ignore = "requires an authenticated local Claude Code turn"]
async fn installed_claude_emits_active_then_final_idle_through_temporary_settings() {
    assert_eq!(std::env::var("LLM_OVERLAY_RUN_CLAUDE_E2E").as_deref(), Ok("1"), "set LLM_OVERLAY_RUN_CLAUDE_E2E=1 to authorize the live turn");
    let root = std::env::temp_dir().join(format!("llm-overlay-claude-e2e-{}", std::process::id()));
    fs::create_dir_all(&root).expect("temporary workspace");
    let helper = env!("CARGO_BIN_EXE_llm-overlay-hook").replace('\\', "\\\\");
    let settings = root.join("settings.json");
    fs::write(&settings, format!(r#"{{"hooks":{{"UserPromptSubmit":[{{"matcher":"","hooks":[{{"type":"command","command":"\"{helper}\" claude_code active","timeout":1}}]}}],"Stop":[{{"matcher":"","hooks":[{{"type":"command","command":"\"{helper}\" claude_code idle","timeout":1}}]}}],"StopFailure":[{{"matcher":"","hooks":[{{"type":"command","command":"\"{helper}\" claude_code idle","timeout":1}}]}}],"SessionEnd":[{{"matcher":"","hooks":[{{"type":"command","command":"\"{helper}\" claude_code idle","timeout":1}}]}}]}}}}"#)).expect("temporary settings");

    let registry = Arc::new(Mutex::new(ActivityRegistry::default()));
    let changes = Arc::new(AtomicUsize::new(0));
    let pipe_name = format!(r"\\.\pipe\llm-quota-overlay-claude-e2e-{}", std::process::id());
    let listener_changes = Arc::clone(&changes);
    let server = tokio::spawn(run_named_pipe_at(Arc::clone(&registry), Arc::new(move || { listener_changes.fetch_add(1, Ordering::Relaxed); }), pipe_name.clone()));
    tokio::time::sleep(Duration::from_millis(20)).await;

    let started = Instant::now();
    let mut command = std::process::Command::new("claude");
    command
        .current_dir(&root)
        .env("LLM_OVERLAY_PIPE_NAME", &pipe_name)
        .args(["--settings", settings.to_str().expect("utf8 path"), "--setting-sources", "project", "--no-session-persistence", "--permission-mode", "plan", "--print", "Reply with exactly OK."])
        // `--print` waits for piped stdin when it is not a terminal; the inherited
        // test-runner stdin never closes, which previously stalled the turn to timeout.
        .stdin(std::process::Stdio::null());
    let status = match bounded_status(command, Duration::from_secs(90)).await {
        Ok(status) => status,
        Err(error) => {
            let report_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("test-results");
            let _ = fs::create_dir_all(&report_dir);
            let current_state = registry.lock().await.activity(ActivityProvider::ClaudeCode);
            let state = match current_state { AgentActivity::Active => "active", AgentActivity::Idle => "idle", AgentActivity::Unknown => "unknown" };
            let _ = fs::write(report_dir.join("claude-lifecycle-e2e.json"), format!(r#"{{"provider":"claude_code","result":"inconclusive","stage":"bounded_process_timeout","lifecycleTransitions":{},"lastObservedState":"{}","durationMs":{}}}"#, changes.load(Ordering::Relaxed), state, started.elapsed().as_millis()));
            server.abort();
            panic!("bounded Claude Code turn: {error}");
        }
    };
    assert!(status.success(), "minimal Claude turn completed");

    for _ in 0..50 {
        if changes.load(Ordering::Relaxed) >= 2 { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(changes.load(Ordering::Relaxed) >= 2, "received active and final idle lifecycle transitions");
    assert_eq!(registry.lock().await.activity(ActivityProvider::ClaudeCode), AgentActivity::Idle);
    let report_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("docs").join("test-results");
    fs::create_dir_all(&report_dir).expect("sanitized report directory");
    fs::write(
        report_dir.join("claude-lifecycle-e2e.json"),
        format!(r#"{{"provider":"claude_code","result":"pass","stage":"final_idle_observed","lifecycleTransitions":{},"finalState":"idle","durationMs":{},"capturedAt":"{}"}}"#, changes.load(Ordering::Relaxed), started.elapsed().as_millis(), chrono::Utc::now().to_rfc3339()),
    ).expect("sanitized report");
    server.abort();
    let _ = fs::remove_dir_all(root);
}
