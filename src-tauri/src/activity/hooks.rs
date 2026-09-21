//! Non-destructive hook configuration planning.
//!
//! Callers show the returned plan to the user and only call `apply_plan` after
//! explicit user action. This module never changes hook trust state.

use std::{fs, io, path::{Path, PathBuf}};

use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookTarget { Codex, Claude, Antigravity }

#[derive(Debug, Clone)]
pub struct HookPlan {
    pub path: PathBuf,
    pub target: HookTarget,
    pub updated_document: Value,
    pub added_events: Vec<&'static str>,
    pub removed_events: Vec<&'static str>,
    original_document: Option<String>,
}

/// The exact command this app writes for `target`. Inputs are fixed literals only.
pub fn helper_command(target: HookTarget, helper: &Path, provider: &str, event: &str) -> String {
    match target {
        // Antigravity passes the command to `cmd /c` as one Go-escaped argument, which
        // turns `"` into `\"`; cmd cannot parse that (observed with agy 1.2.7). Use a
        // path with no spaces instead of quoting it.
        HookTarget::Antigravity => format!("{} {} {}", unquoted_program(helper), provider, event),
        // Other hook hosts run the string through a shell: quote so a path containing
        // spaces stays a single argument.
        HookTarget::Codex | HookTarget::Claude => quoted_command(helper, provider, event),
    }
}

fn quoted_command(helper: &Path, provider: &str, event: &str) -> String {
    format!("\"{}\" {} {}", helper.display(), provider, event)
}

fn unquoted_program(helper: &Path) -> String {
    let display = helper.display().to_string();
    if !display.contains(char::is_whitespace) { return display; }
    short_path(helper).filter(|short| !short.contains(char::is_whitespace)).unwrap_or_else(|| format!("\"{display}\""))
}

#[cfg(windows)]
fn short_path(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 1024];
    let length = unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    (length > 0 && length < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..length]))
}
#[cfg(not(windows))]
fn short_path(_: &Path) -> Option<String> { None }

fn handler(command: String) -> Value {
    // An upper bound only; the helper itself exits within ~150 ms. agy 1.2.7 runs the
    // Stop hook while the turn is shutting down, and a cold `cmd.exe` start there
    // exceeded a 1 s limit and was killed before the helper ran.
    json!({"type": "command", "command": command, "timeout": 5})
}

fn claude_group(command: String) -> Value {
    json!({"matcher": "", "hooks": [handler(command)]})
}

fn event_names(target: HookTarget) -> (&'static str, &'static [&'static str], &'static str) {
    match target {
        HookTarget::Codex => ("chatgpt_work", &["UserPromptSubmit", "PreToolUse", "PostToolUse"], "Stop"),
        HookTarget::Claude => ("claude_code", &["UserPromptSubmit", "PreToolUse", "PostToolUse"], "Stop"),
        HookTarget::Antigravity => ("antigravity", &["PreInvocation", "PreToolUse", "PostToolUse"], "Stop"),
    }
}

/// Merges only our exact command entries into an in-memory JSON configuration.
/// Existing hooks, matchers, and unknown fields remain untouched.
pub fn plan_install(existing: Option<&str>, path: PathBuf, target: HookTarget, helper: &Path) -> Result<HookPlan, String> {
    let original_document = existing.map(str::to_owned);
    let mut document = match existing.filter(|s| !s.trim().is_empty()) {
        Some(text) => serde_json::from_str(text).map_err(|_| "existing hook configuration is not valid JSON".to_owned())?,
        None => json!({}),
    };
    let root = document.as_object_mut().ok_or_else(|| "hook configuration root must be a JSON object".to_owned())?;
    let (provider, starts, stop) = event_names(target);
    let mut added = Vec::new();

    match target {
        HookTarget::Claude | HookTarget::Codex => {
            let hooks = object_field(root, "hooks")?;
            for event in starts.iter().copied().map(|event| (event, "active")).chain(std::iter::once((stop, "idle"))) {
                let command = helper_command(target, helper, provider, event.1);
                if append_unique(hooks, event.0, claude_group(command.clone()), &command)? { added.push(event.0); }
            }
            // Final events only. Tool and model events remain active heartbeats.
            if target == HookTarget::Claude || target == HookTarget::Codex {
                let final_events: &[&str] = if target == HookTarget::Claude {
                    &["StopFailure", "SessionEnd"]
                } else {
                    &["Interrupt", "SessionEnd"]
                };
                for final_event in final_events {
                    let command = helper_command(target, helper, provider, "idle");
                    if append_unique(hooks, final_event, claude_group(command.clone()), &command)? { added.push(final_event); }
                }
            }
        }
        HookTarget::Antigravity => {
            // Antigravity documents named hook sets: retain all other named sets.
            let set = object_field(root, "llm-quota-overlay")?;
            for (event, kind) in starts.iter().copied().map(|event| (event, "active")).chain(std::iter::once((stop, "idle"))) {
                let command = helper_command(target, helper, provider, kind);
                if append_unique(set, event, handler(command.clone()), &command)? { added.push(event); }
            }
        }
    }
    Ok(HookPlan { path, target, updated_document: document, added_events: added, removed_events: vec![], original_document })
}

/// Removes only entries whose command exactly matches this app's helper command.
/// If an event becomes empty it is removed; other tools' entries survive unchanged.
pub fn plan_remove(existing: &str, path: PathBuf, target: HookTarget, helper: &Path) -> Result<HookPlan, String> {
    let mut document: Value = serde_json::from_str(existing).map_err(|_| "existing hook configuration is not valid JSON".to_owned())?;
    let root = document.as_object_mut().ok_or_else(|| "hook configuration root must be a JSON object".to_owned())?;
    let (provider, starts, stop) = event_names(target);
    let mut removed = Vec::new();
    let mut entries: Vec<(&str, &str)> = starts.iter().map(|event| (*event, "active")).collect();
    entries.push((stop, "idle"));
    match target {
        HookTarget::Claude => entries.extend([("StopFailure", "idle"), ("SessionEnd", "idle")]),
        HookTarget::Codex => entries.extend([("Interrupt", "idle"), ("SessionEnd", "idle")]),
        HookTarget::Antigravity => {}
    }
    let container = match target {
        HookTarget::Codex | HookTarget::Claude => root.get_mut("hooks").and_then(Value::as_object_mut),
        HookTarget::Antigravity => root.get_mut("llm-quota-overlay").and_then(Value::as_object_mut),
    };
    if let Some(container) = container {
        for (event, kind) in entries {
            let command = helper_command(target, helper, provider, kind);
            // Also clean up the quoted form earlier builds wrote for every target.
            let legacy = quoted_command(helper, provider, kind);
            let removed_current = remove_exact(container, event, &command);
            let removed_legacy = legacy != command && remove_exact(container, event, &legacy);
            if removed_current || removed_legacy { removed.push(event); }
        }
        if target == HookTarget::Antigravity && container.is_empty() { root.remove("llm-quota-overlay"); }
    }
    Ok(HookPlan { path, target, updated_document: document, added_events: vec![], removed_events: removed, original_document: Some(existing.to_owned()) })
}

/// Writes a previously reviewed plan with a same-directory temporary file + rename.
pub fn apply_plan(plan: &HookPlan) -> io::Result<()> {
    let current = fs::read_to_string(&plan.path).ok();
    if current != plan.original_document {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, "hook configuration changed after this plan was reviewed"));
    }
    let rendered = serde_json::to_vec_pretty(&plan.updated_document).expect("serializable hook document");
    if let Some(parent) = plan.path.parent() { fs::create_dir_all(parent)?; }
    let file_name = plan.path.file_name().and_then(|name| name.to_str()).unwrap_or("hooks.json");
    let temporary = plan.path.with_file_name(format!(".{file_name}.llm-overlay-{}.tmp", std::process::id()));
    fs::write(&temporary, rendered)?;
    replace_same_directory(&temporary, &plan.path)
}

/// Same-directory replacement prevents partially-written hook JSON from becoming
/// visible. Windows explicitly replaces an existing destination.
#[cfg(windows)]
fn replace_same_directory(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{Foundation::GetLastError, Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH}};
    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(temporary.as_ptr(), destination.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(io::Error::from_raw_os_error(unsafe { GetLastError() } as i32));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_same_directory(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

fn object_field<'a>(root: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Map<String, Value>, String> {
    let value = root.entry(key.to_owned()).or_insert_with(|| json!({}));
    value.as_object_mut().ok_or_else(|| format!("{key} must be a JSON object"))
}

fn append_unique(container: &mut Map<String, Value>, event: &str, entry: Value, command: &str) -> Result<bool, String> {
    let values = container.entry(event.to_owned()).or_insert_with(|| json!([]));
    let Some(values) = values.as_array_mut() else { return Err(format!("{event} must be an array")); };
    if values.iter().any(|value| contains_command(value, command)) { return Ok(false); }
    values.push(entry);
    Ok(true)
}

fn remove_exact(container: &mut Map<String, Value>, event: &str, command: &str) -> bool {
    let Some(values) = container.get_mut(event).and_then(Value::as_array_mut) else { return false; };
    let before = values.len();
    values.retain_mut(|value| prune_exact_command(value, command));
    let changed = values.len() != before;
    if values.is_empty() { container.remove(event); }
    changed
}

/// Returns whether this handler/group still contains a handler after pruning.
fn prune_exact_command(value: &mut Value, command: &str) -> bool {
    if value.get("command").and_then(Value::as_str) == Some(command) { return false; }
    if let Some(hooks) = value.get_mut("hooks").and_then(Value::as_array_mut) {
        hooks.retain_mut(|hook| prune_exact_command(hook, command));
        return !hooks.is_empty();
    }
    true
}

fn contains_command(value: &Value, command: &str) -> bool {
    value.get("command").and_then(Value::as_str) == Some(command)
        || value.get("hooks").and_then(Value::as_array).is_some_and(|hooks| hooks.iter().any(|hook| contains_command(hook, command)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn install_and_removal_keep_a_neighbour_hook() {
        let existing = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#;
        let path = PathBuf::from("hooks.json");
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        let installed = plan_install(Some(existing), path.clone(), HookTarget::Claude, helper).unwrap();
        assert!(installed.added_events.contains(&"UserPromptSubmit"));
        let removed = plan_remove(&installed.updated_document.to_string(), path, HookTarget::Claude, helper).unwrap();
        assert!(removed.removed_events.contains(&"Stop"));
        assert!(removed.updated_document.to_string().contains("other-tool"));
    }

    #[test]
    fn removal_prunes_only_our_handler_from_a_shared_group() {
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        let ours = helper_command(HookTarget::Claude, helper, "claude_code", "idle");
        let existing = json!({"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": ours},
            {"type": "command", "command": "other-tool"}
        ]}]}}).to_string();
        let removed = plan_remove(&existing, PathBuf::from("hooks.json"), HookTarget::Claude, helper).unwrap();
        let stop = &removed.updated_document["hooks"]["Stop"][0]["hooks"];
        assert_eq!(stop.as_array().unwrap().len(), 1);
        assert_eq!(stop[0]["command"], "other-tool");
    }

    #[test]
    fn antigravity_command_is_unquoted_for_cmd_while_others_stay_quoted() {
        let helper = Path::new("D:/llm-usage-ui/release/llm-overlay-hook.exe");
        assert_eq!(helper_command(HookTarget::Antigravity, helper, "antigravity", "active"), "D:/llm-usage-ui/release/llm-overlay-hook.exe antigravity active");
        assert_eq!(helper_command(HookTarget::Claude, helper, "claude_code", "idle"), "\"D:/llm-usage-ui/release/llm-overlay-hook.exe\" claude_code idle");
    }

    #[test]
    fn antigravity_removal_also_cleans_the_earlier_quoted_form() {
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        let existing = json!({"llm-quota-overlay": {
            "PreInvocation": [{"type": "command", "command": "\"C:/overlay/llm-overlay-hook.exe\" antigravity active"}],
            "Stop": [{"type": "command", "command": "C:/overlay/llm-overlay-hook.exe antigravity idle"}]
        }, "other": {"Stop": [{"type": "command", "command": "other-tool"}]}}).to_string();
        let removed = plan_remove(&existing, PathBuf::from("hooks.json"), HookTarget::Antigravity, helper).unwrap();
        assert!(removed.updated_document.get("llm-quota-overlay").is_none());
        assert_eq!(removed.updated_document["other"]["Stop"][0]["command"], "other-tool");
    }

    #[test]
    fn install_is_idempotent_and_rejects_malformed_event_arrays() {
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        let once = plan_install(None, PathBuf::from("hooks.json"), HookTarget::Codex, helper).unwrap();
        let twice = plan_install(Some(&once.updated_document.to_string()), PathBuf::from("hooks.json"), HookTarget::Codex, helper).unwrap();
        assert!(twice.added_events.is_empty());
        assert!(plan_install(Some(r#"{"hooks":{"Stop":{}}}"#), PathBuf::from("hooks.json"), HookTarget::Codex, helper).is_err());
    }

    #[test]
    fn reviewed_plan_refuses_a_concurrent_edit() {
        let root = std::env::temp_dir().join(format!("llm-overlay-hooks-test-{}", std::process::id()));
        let path = root.join("nested/hooks.json");
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        let plan = plan_install(None, path.clone(), HookTarget::Claude, helper).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{}").unwrap();
        assert_eq!(apply_plan(&plan).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reviewed_plan_creates_a_missing_parent() {
        let root = std::env::temp_dir().join(format!("llm-overlay-hooks-create-{}", std::process::id()));
        let path = root.join("nested/hooks.json");
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        let plan = plan_install(None, path.clone(), HookTarget::Antigravity, helper).unwrap();
        apply_plan(&plan).unwrap();
        assert!(path.exists());
        let _ = fs::remove_dir_all(root);
    }
}
