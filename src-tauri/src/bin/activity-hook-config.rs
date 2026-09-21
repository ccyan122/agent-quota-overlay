//! Explicit, review-first activity hook configuration utility.
//!
//! This is deliberately separate from the lifecycle helper. It never changes
//! Codex hook trust; users still approve Codex entries through `/hooks`.

use std::{env, fs, path::PathBuf, process::ExitCode};

use llm_quota_overlay::activity::hooks::{apply_plan, helper_command, plan_install, plan_remove, HookPlan, HookTarget};
use serde_json::json;

fn usage() -> &'static str {
    "Usage: activity-hook-config (--install-hooks|--remove-hooks) <codex|claude|antigravity> <config-path> <helper-path> [--apply]"
}

fn target(value: &str) -> Option<HookTarget> {
    match value {
        "codex" => Some(HookTarget::Codex),
        "claude" => Some(HookTarget::Claude),
        "antigravity" => Some(HookTarget::Antigravity),
        _ => None,
    }
}

fn provider(target: HookTarget) -> &'static str {
    match target { HookTarget::Codex => "chatgpt_work", HookTarget::Claude => "claude_code", HookTarget::Antigravity => "antigravity" }
}

fn is_final_event(target: HookTarget, event: &str) -> bool {
    matches!(event, "Stop") || matches!((target, event),
        (HookTarget::Codex, "Interrupt" | "SessionEnd") | (HookTarget::Claude, "StopFailure" | "SessionEnd"))
}

fn own_commands(target: HookTarget, helper: &PathBuf, events: &[&str]) -> Vec<String> {
    events.iter().map(|event| helper_command(target, helper, provider(target), if is_final_event(target, event) { "idle" } else { "active" })).collect()
}

fn print_plan(plan: &HookPlan, helper: &PathBuf) {
    // Never serialize the merged document: it may contain a user's unrelated
    // commands, environment variables, or credentials. Review only our changes.
    println!("{}", serde_json::to_string_pretty(&json!({
        "path": plan.path,
        "target": format!("{:?}", plan.target),
        "added_events": plan.added_events,
        "removed_events": plan.removed_events,
        "inserted_commands": own_commands(plan.target, helper, &plan.added_events),
        "removed_commands": own_commands(plan.target, helper, &plan.removed_events),
        "apply_note": "Re-run the same command with --apply after review. Codex entries still require /hooks approval."
    })).expect("serializable plan"));
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(operation) = args.first().map(String::as_str) else { return Err(usage().to_owned()); };
    if operation != "--install-hooks" && operation != "--remove-hooks" { return Err(usage().to_owned()); }
    if args.len() != 4 && args.len() != 5 { return Err(usage().to_owned()); }
    let target = target(&args[1]).ok_or_else(|| usage().to_owned())?;
    let path = PathBuf::from(&args[2]);
    let helper = PathBuf::from(&args[3]);
    let apply = args.get(4).is_some_and(|arg| arg == "--apply");
    if args.len() == 5 && !apply { return Err(usage().to_owned()); }

    let existing = fs::read_to_string(&path).ok();
    let plan = match operation {
        "--install-hooks" => plan_install(existing.as_deref(), path, target, &helper),
        "--remove-hooks" => plan_remove(existing.as_deref().ok_or_else(|| "cannot remove hooks: configuration file does not exist".to_owned())?, path, target, &helper),
        _ => unreachable!(),
    }?;
    print_plan(&plan, &helper);
    if apply { apply_plan(&plan).map_err(|error| error.to_string())?; }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => { eprintln!("{message}"); ExitCode::from(2) }
    }
}
