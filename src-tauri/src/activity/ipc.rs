//! Content-free local IPC protocol for lifecycle hook helpers.

use std::{sync::Arc, time::{Duration, Instant}};

use serde::Deserialize;
use tokio::{io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader}, sync::{Mutex, Semaphore}};

use super::state::{ActivityEventKind, ActivityProvider, ActivityRegistry};

pub const PIPE_NAME: &str = r"\\.\pipe\llm-quota-overlay";
pub const MAX_EVENT_BYTES: usize = 512;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    provider: ActivityProvider,
    event: WireEventKind,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireEventKind { Active, Idle }

impl WireEvent {
    fn validate(self) -> Option<(ActivityProvider, ActivityEventKind, Option<String>)> {
        let session_id = self.session_id;
        // An opaque bounded identifier supports concurrent turns, but excludes free text.
        if let Some(id) = &session_id {
            if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                return None;
            }
        }
        Some((self.provider, match self.event { WireEventKind::Active => ActivityEventKind::Active, WireEventKind::Idle => ActivityEventKind::Idle }, session_id))
    }
}

pub fn parse_event(input: &[u8]) -> Option<(ActivityProvider, ActivityEventKind, Option<String>)> {
    if input.len() > MAX_EVENT_BYTES { return None; }
    let parsed: WireEvent = serde_json::from_slice(input).ok()?;
    parsed.validate()
}

/// Runs the Windows pipe loop. `reject_remote_clients` sets PIPE_REJECT_REMOTE_CLIENTS,
/// and every connection must prove it has the same Windows user SID as this process.
/// No client-supplied timestamps are accepted, and malformed data receives no acknowledgement.
#[cfg(windows)]
pub async fn run_named_pipe(registry: Arc<Mutex<ActivityRegistry>>) -> std::io::Result<()> {
    run_named_pipe_with_listener(registry, Arc::new(|| {})).await
}

/// Same as [`run_named_pipe`], with a content-free notification after an accepted
/// lifecycle event. The listener must obtain the public snapshot from the registry;
/// raw hook envelopes never leave this module.
#[cfg(windows)]
pub async fn run_named_pipe_with_listener(
    registry: Arc<Mutex<ActivityRegistry>>,
    listener: Arc<dyn Fn() + Send + Sync>,
) -> std::io::Result<()> {
    run_named_pipe_at(registry, listener, PIPE_NAME.to_owned()).await
}

/// Internal/testable pipe runner. Production callers always use [`PIPE_NAME`];
/// a distinct name lets tests avoid attaching to a concurrently running overlay.
#[cfg(windows)]
pub async fn run_named_pipe_at(
    registry: Arc<Mutex<ActivityRegistry>>,
    listener: Arc<dyn Fn() + Send + Sync>,
    pipe_name: String,
) -> std::io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;
    use std::os::windows::io::AsRawHandle;

    let permits = Arc::new(Semaphore::new(8));
    let mut first_instance = true;
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(first_instance)
            .reject_remote_clients(true)
            .create(&pipe_name)?;
        first_instance = false;
        server.connect().await?;
        // A local pipe name is not an authorization boundary. Fail closed if the
        // client cannot be tied to the app's current Windows user token.
        if !same_windows_user(server.as_raw_handle()) { continue; }
        let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else { continue; };
        let registry = Arc::clone(&registry);
        let listener = Arc::clone(&listener);
        tokio::spawn(async move {
            let _permit = permit;
            let (reader, mut writer) = tokio::io::split(server);
            let reader = BufReader::new(reader);
            let mut line = Vec::with_capacity(MAX_EVENT_BYTES);
            // A helper sends exactly one newline-delimited envelope and exits.
            let mut bounded = reader.take((MAX_EVENT_BYTES + 1) as u64);
            if tokio::time::timeout(Duration::from_millis(250), bounded.read_until(b'\n', &mut line)).await.ok().and_then(Result::ok).is_some() {
                if let Some((provider, event, session)) = parse_event(line.strip_suffix(b"\n").unwrap_or(&line)) {
                    if registry.lock().await.ingest(provider, event, session.as_deref(), Instant::now()) {
                        listener();
                    }
                    let _ = writer.write_all(b"ok\n").await;
                }
            }
        });
    }
}

#[cfg(windows)]
fn same_windows_user(pipe_handle: std::os::windows::io::RawHandle) -> bool {
    use std::{ffi::c_void, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::{Pipes::GetNamedPipeClientProcessId, Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION}},
    };

    unsafe fn user_sid(process: HANDLE) -> Option<(HANDLE, Vec<u8>)> {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 { return None; }
        let mut bytes = 0;
        let _ = GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes);
        if bytes == 0 { let _ = CloseHandle(token); return None; }
        let mut buffer = vec![0u8; bytes as usize];
        if GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast::<c_void>(), bytes, &mut bytes) == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        Some((token, buffer))
    }

    unsafe {
        let mut client_pid = 0u32;
        if GetNamedPipeClientProcessId(pipe_handle.cast(), &mut client_pid) == 0 || client_pid == 0 { return false; }
        let client_process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, client_pid);
        if client_process.is_null() { return false; }
        let Some((client_token, client_buffer)) = user_sid(client_process) else { let _ = CloseHandle(client_process); return false; };
        let Some((self_token, self_buffer)) = user_sid(GetCurrentProcess()) else {
            let _ = CloseHandle(client_token); let _ = CloseHandle(client_process); return false;
        };
        let client_sid = (*(client_buffer.as_ptr() as *const TOKEN_USER)).User.Sid;
        let self_sid = (*(self_buffer.as_ptr() as *const TOKEN_USER)).User.Sid;
        let same = !client_sid.is_null() && !self_sid.is_null() && EqualSid(client_sid, self_sid) != 0;
        let _ = CloseHandle(self_token);
        let _ = CloseHandle(client_token);
        let _ = CloseHandle(client_process);
        same
    }
}

#[cfg(not(windows))]
pub async fn run_named_pipe(_registry: Arc<Mutex<ActivityRegistry>>) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "activity pipe is Windows-only"))
}

#[cfg(not(windows))]
pub async fn run_named_pipe_with_listener(
    registry: Arc<Mutex<ActivityRegistry>>,
    _listener: Arc<dyn Fn() + Send + Sync>,
) -> std::io::Result<()> {
    run_named_pipe(registry).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::AgentActivity;
    #[test]
    fn parser_accepts_only_the_reduced_schema() {
        let event = parse_event(br#"{"provider":"claude_code","event":"active","session_id":"turn_7"}"#).unwrap();
        assert_eq!(event.0, ActivityProvider::ClaudeCode);
        assert!(parse_event(br#"{"provider":"claude_code","event":"active","session_id":"x","prompt":"secret"}"#).is_none());
        assert!(parse_event(br#"{"provider":"claude_code","event":"active","session_id":"contains spaces"}"#).is_none());
    }

    #[test]
    fn parser_rejects_nested_or_oversized_input() {
        assert!(parse_event(br#"{"provider":"claude_code","event":"active","session_id":{"value":"x"}}"#).is_none());
        assert!(parse_event(&vec![b'x'; MAX_EVENT_BYTES + 1]).is_none());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn local_named_pipe_delivers_a_reduced_event() {
        use tokio::net::windows::named_pipe::ClientOptions;
        let registry = Arc::new(Mutex::new(ActivityRegistry::default()));
        let pipe_name = format!(r"\\.\pipe\llm-quota-overlay-test-{}", std::process::id());
        let server = tokio::spawn(run_named_pipe_at(Arc::clone(&registry), Arc::new(|| {}), pipe_name.clone()));
        let mut client = None;
        for _ in 0..25 {
            if let Ok(pipe) = ClientOptions::new().open(&pipe_name) { client = Some(pipe); break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut client = client.expect("local pipe server became available");
        client.write_all(b"{\"provider\":\"claude_code\",\"event\":\"active\",\"session_id\":\"e2e_turn\"}\n").await.unwrap();
        let mut acknowledgement = [0; 3];
        tokio::time::timeout(Duration::from_millis(250), client.read_exact(&mut acknowledgement)).await.unwrap().unwrap();
        assert_eq!(&acknowledgement, b"ok\n");
        assert_eq!(registry.lock().await.activity(ActivityProvider::ClaudeCode), AgentActivity::Active);
        server.abort();
    }
}
