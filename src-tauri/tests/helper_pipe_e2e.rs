#![cfg(windows)]

use std::{io::Write, process::{Command, Stdio}, sync::Arc, time::{Duration, Instant}};

use llm_quota_overlay::activity::{ipc::run_named_pipe_at, ActivityProvider, ActivityRegistry, AgentActivity};
use tokio::sync::Mutex;

/// Exercises the shipped process, including the server's PID/SID validation and
/// acknowledgement. This prevents a helper from exiting before authorization.
#[tokio::test]
async fn helper_process_delivers_an_event_to_the_local_pipe() {
    let registry = Arc::new(Mutex::new(ActivityRegistry::default()));
    let pipe_name = format!(r"\\.\pipe\llm-quota-overlay-helper-e2e-{}", std::process::id());
    let server = tokio::spawn(run_named_pipe_at(Arc::clone(&registry), Arc::new(|| {}), pipe_name.clone()));
    tokio::time::sleep(Duration::from_millis(20)).await;

    let status = Command::new(env!("CARGO_BIN_EXE_llm-overlay-hook"))
        .args(["claude_code", "active", "helper_process_e2e"])
        .env("LLM_OVERLAY_PIPE_NAME", &pipe_name)
        .status()
        .expect("helper starts");
    assert!(status.success());

    for _ in 0..20 {
        if registry.lock().await.activity(ActivityProvider::ClaudeCode) == AgentActivity::Active { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(registry.lock().await.activity(ActivityProvider::ClaudeCode), AgentActivity::Active);
    server.abort();
}

#[tokio::test]
async fn helper_exits_when_hook_stdin_never_reaches_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_llm-overlay-hook"))
        .args(["claude_code", "active"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("helper starts");
    let _open_stdin = child.stdin.take().expect("stdin pipe");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("process status") {
            assert!(status.success());
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(1), "helper waited for stdin EOF");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn assert_invalid_stdin_is_not_delivered(input: Vec<u8>) {
    let registry = Arc::new(Mutex::new(ActivityRegistry::default()));
    let pipe_name = format!(r"\\.\pipe\llm-quota-overlay-invalid-e2e-{}", std::process::id());
    let server = tokio::spawn(run_named_pipe_at(Arc::clone(&registry), Arc::new(|| {}), pipe_name.clone()));
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_llm-overlay-hook"))
        .args(["claude_code", "active"])
        .env("LLM_OVERLAY_PIPE_NAME", &pipe_name)
        .stdin(Stdio::piped())
        .spawn()
        .expect("helper starts");
    let mut stdin = child.stdin.take().expect("stdin");
    // The helper may close its bounded reader as soon as it has enough bytes;
    // a broken pipe after that point is the expected oversized-input outcome.
    let _ = stdin.write_all(&input);
    drop(stdin);
    let status = child.wait().expect("helper exits");
    assert!(status.success());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(registry.lock().await.activity(ActivityProvider::ClaudeCode), AgentActivity::Unknown);
    server.abort();
}

#[tokio::test]
async fn malformed_hook_stdin_is_not_delivered() {
    assert_invalid_stdin_is_not_delivered(br#"{"session_id":"unfinished"#.to_vec()).await;
}

#[tokio::test]
async fn oversized_hook_stdin_is_not_delivered() {
    assert_invalid_stdin_is_not_delivered(vec![b'x'; 256 * 1024 + 1]).await;
}
