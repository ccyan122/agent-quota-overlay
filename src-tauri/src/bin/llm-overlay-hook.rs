//! Tiny best-effort lifecycle helper. It only extracts a bounded opaque session
//! identifier from hook stdin and never logs.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use llm_quota_overlay::activity::ipc::{parse_event, PIPE_NAME};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_STDIN_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
struct HookStdin {
    #[serde(alias = "sessionId")]
    session_id: Option<String>,
    #[serde(alias = "conversationId")]
    conversation_id: Option<String>,
}

fn parse_stdin_session(input: &[u8]) -> Result<Option<String>, ()> {
    if input.is_empty() || input.len() > MAX_STDIN_BYTES { return Err(()); }
    let envelope = serde_json::from_slice::<HookStdin>(input).map_err(|_| ())?;
    // Hook hosts include prompt/tool fields alongside lifecycle metadata. Serde
    // ignores them here: only these two opaque identifiers are retained, and
    // neither the original input nor any ignored field enters the pipe.
    Ok(envelope.session_id.or(envelope.conversation_id))
}

async fn stdin_session_id() -> Result<Option<String>, ()> {
    // Claude and other hook hosts supply their opaque lifecycle session id on stdin.
    // Read a tiny bounded envelope, discard all other fields, and never forward it.
    let mut bytes = Vec::with_capacity(1024);
    let mut input = tokio::io::stdin().take((MAX_STDIN_BYTES + 1) as u64);
    let read = tokio::time::timeout(Duration::from_millis(50), input.read_to_end(&mut bytes)).await.map_err(|_| ())?.map_err(|_| ())?;
    if read > MAX_STDIN_BYTES { return Err(()); }
    parse_stdin_session(&bytes)
}

fn reduced_event(provider: String, event: String, session_id: Option<String>) -> Option<Vec<u8>> {
    let mut value = serde_json::json!({"provider": provider, "event": event});
    if let Some(session_id) = session_id { value["session_id"] = serde_json::Value::String(session_id); }
    let value = serde_json::to_vec(&value).ok()?;
    parse_event(&value).map(|_| value)
}

async fn usage_event() -> Option<Vec<u8>> {
    let mut args = std::env::args().skip(1);
    let provider = args.next()?;
    let event = args.next()?;
    // A supplied opaque session id is useful for an explicit synthetic E2E run.
    // In real hook configuration there are only two arguments, so the helper
    // reads the bounded typed stdin instead. Never read stdin when an explicit
    // session was supplied; this keeps synthetic invocations fail-fast.
    let session_id = args.next();
    if args.next().is_some() { return None; }
    let session_id = match session_id {
        Some(id) => Some(id),
        // A valid hook envelope without a documented session field is allowed to
        // use the provider slot. Invalid, oversized, unreadable, or unclosed
        // input is dropped so it cannot create an unmatched anonymous active.
        None => stdin_session_id().await.ok()?,
    };
    reduced_event(provider, event, session_id)
}

#[cfg(windows)]
async fn notify_to(pipe_name: &str, event: Vec<u8>) {
    use tokio::net::windows::named_pipe::ClientOptions;
    // All failures are deliberately silent and successful from the hook's perspective.
    let _ = tokio::time::timeout(Duration::from_millis(150), async move {
        loop {
            match ClientOptions::new().open(pipe_name) {
                Ok(mut client) => {
                    client.write_all(&event).await?;
                    client.write_all(b"\n").await?;
                    // Keep this short-lived process alive until the server has
                    // obtained and validated its PID/SID. Without the ACK the
                    // process can exit before that check and drop real events.
                    let mut acknowledgement = [0; 3];
                    client.read_exact(&mut acknowledgement).await?;
                    return if acknowledgement == *b"ok\n" {
                        Ok(())
                    } else {
                        Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "unexpected pipe acknowledgement"))
                    };
                }
                // ERROR_PIPE_BUSY while the server rotates to its next instance.
                Err(error) if error.raw_os_error() == Some(231) => tokio::time::sleep(Duration::from_millis(5)).await,
                Err(error) => return Err(error),
            }
        }
    }).await;
}

#[cfg(windows)]
async fn notify(event: Vec<u8>) {
    // Test harnesses can isolate a pipe when an overlay is already running. Hook
    // installation never sets this variable, so production always uses PIPE_NAME.
    let pipe_name = std::env::var("LLM_OVERLAY_PIPE_NAME").unwrap_or_else(|_| PIPE_NAME.to_owned());
    notify_to(&pipe_name, event).await;
}

#[cfg(not(windows))]
async fn notify(_event: Vec<u8>) {}

async fn run() {
    if let Some(event) = usage_event().await { notify(event).await; }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("hook runtime");
    runtime.block_on(run());
    // Tokio's stdin reader may have an outstanding blocking read when a broken
    // hook host keeps stdin open. Do not let its worker delay hook completion.
    runtime.shutdown_timeout(Duration::from_millis(10));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_payload_is_reduced_before_pipe_delivery() {
        let parsed = serde_json::from_slice::<HookStdin>(br#"{"session_id":"turn_9","prompt":"secret","tool_input":{"api_key":"nope"}}"#).unwrap();
        assert_eq!(parsed.session_id.as_deref(), Some("turn_9"));
        let event = reduced_event("claude_code".to_owned(), "active".to_owned(), parsed.session_id.or(parsed.conversation_id)).unwrap();
        assert_eq!(event, br#"{"event":"active","provider":"claude_code","session_id":"turn_9"}"#);
        assert!(!String::from_utf8(event).unwrap().contains("secret"));
    }

    #[test]
    fn long_hook_payload_keeps_the_same_opaque_session_for_start_and_stop() {
        let payload = format!(r#"{{"session_id":"turn_9","prompt":"{}","tool_input":{{"secret":"discard"}}}}"#, "x".repeat(8 * 1024));
        let session = parse_stdin_session(payload.as_bytes()).unwrap();
        let start = reduced_event("claude_code".to_owned(), "active".to_owned(), session.clone()).unwrap();
        let stop = reduced_event("claude_code".to_owned(), "idle".to_owned(), session).unwrap();
        assert!(String::from_utf8(start).unwrap().contains("turn_9"));
        assert!(String::from_utf8(stop).unwrap().contains("turn_9"));
    }

    #[test]
    fn malformed_and_oversized_stdin_are_dropped() {
        assert!(parse_stdin_session(br#"{"session_id":"unterminated"#).is_err());
        assert!(parse_stdin_session(&vec![b'x'; MAX_STDIN_BYTES + 1]).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn absent_pipe_returns_within_best_effort_budget() {
        let start = std::time::Instant::now();
        notify_to(r"\\.\pipe\llm-quota-overlay-no-server-test", br#"{"provider":"claude_code","event":"active"}"#.to_vec()).await;
        assert!(start.elapsed() < Duration::from_millis(200));
    }
}
