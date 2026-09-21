//! One-click hook setup for the overlay menu: locate each agent's user-level hook
//! file, report whether this app's entries are present, and install or remove
//! them through the same reviewed-plan path as `activity-hook-config`.

use std::{fs, path::{Path, PathBuf}};

use super::hooks::{apply_plan, plan_install, plan_remove, HookTarget};

pub const TARGETS: [HookTarget; 3] = [HookTarget::Claude, HookTarget::Antigravity, HookTarget::Codex];

pub fn display_name(target: HookTarget) -> &'static str {
    match target { HookTarget::Claude => "Claude Code", HookTarget::Antigravity => "Antigravity", HookTarget::Codex => "Codex (ChatGPT Work)" }
}

pub fn menu_id(target: HookTarget) -> &'static str {
    match target { HookTarget::Claude => "hooks:claude", HookTarget::Antigravity => "hooks:antigravity", HookTarget::Codex => "hooks:codex" }
}

pub fn target_for_menu_id(id: &str) -> Option<HookTarget> { TARGETS.into_iter().find(|target| menu_id(*target) == id) }

/// The agent's home directory; its existence is what counts as "installed on this PC".
fn agent_dir(target: HookTarget) -> Option<PathBuf> {
    let from_env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    match target {
        HookTarget::Claude => from_env("CLAUDE_CONFIG_DIR").or_else(|| home.map(|h| h.join(".claude"))),
        HookTarget::Codex => from_env("CODEX_HOME").or_else(|| home.map(|h| h.join(".codex"))),
        HookTarget::Antigravity => home.map(|h| h.join(".gemini")),
    }
}

/// The hook file this app writes. Antigravity CLI 1.2.7 loads only the global
/// `~/.gemini/config/hooks.json`, not a workspace `.agents/hooks.json`.
pub fn config_path(target: HookTarget) -> Option<PathBuf> {
    let dir = agent_dir(target)?;
    Some(match target {
        HookTarget::Claude => dir.join("settings.json"),
        HookTarget::Codex => dir.join("hooks.json"),
        HookTarget::Antigravity => dir.join("config").join("hooks.json"),
    })
}

pub fn detected(target: HookTarget) -> bool { agent_dir(target).is_some_and(|dir| dir.is_dir()) }

/// Installed means every entry this app would add is already present.
pub fn is_installed_in(existing: Option<&str>, target: HookTarget, helper: &Path) -> bool {
    existing.is_some() && plan_install(existing, PathBuf::from("hooks.json"), target, helper).is_ok_and(|plan| plan.added_events.is_empty())
}

pub fn is_installed(target: HookTarget, helper: &Path) -> bool {
    config_path(target).is_some_and(|path| is_installed_in(fs::read_to_string(path).ok().as_deref(), target, helper))
}

/// Installs or removes this app's entries, backing up the original file first.
pub fn set_installed(target: HookTarget, helper: &Path, install: bool, backup_dir: &Path) -> Result<PathBuf, String> {
    let path = config_path(target).ok_or_else(|| "user profile directory is unavailable".to_owned())?;
    let existing = fs::read_to_string(&path).ok();
    let plan = if install {
        plan_install(existing.as_deref(), path.clone(), target, helper)?
    } else {
        plan_remove(existing.as_deref().ok_or_else(|| "hook configuration file does not exist".to_owned())?, path.clone(), target, helper)?
    };
    if let Some(original) = &existing {
        fs::create_dir_all(backup_dir).map_err(|_| "backup directory could not be created".to_owned())?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = format!("{stamp}-{}-{}", menu_id(target).trim_start_matches("hooks:"), path.file_name().and_then(|n| n.to_str()).unwrap_or("hooks.json"));
        fs::write(backup_dir.join(name), original).map_err(|_| "backup could not be written".to_owned())?;
    }
    apply_plan(&plan).map_err(|error| error.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_only_when_every_entry_is_present() {
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        assert!(!is_installed_in(None, HookTarget::Claude, helper));
        assert!(!is_installed_in(Some("{}"), HookTarget::Claude, helper));
        let full = plan_install(Some("{}"), PathBuf::from("settings.json"), HookTarget::Claude, helper).unwrap().updated_document.to_string();
        assert!(is_installed_in(Some(&full), HookTarget::Claude, helper));
        // A different helper location (e.g. portable vs installed) is not "installed".
        assert!(!is_installed_in(Some(&full), HookTarget::Claude, Path::new("D:/other/llm-overlay-hook.exe")));
    }

    #[test]
    fn menu_ids_round_trip() {
        for target in TARGETS { assert_eq!(target_for_menu_id(menu_id(target)), Some(target)); }
        assert_eq!(target_for_menu_id("hooks:unknown"), None);
    }

    #[test]
    fn install_then_remove_round_trip_keeps_unrelated_settings() {
        let root = std::env::temp_dir().join(format!("llm-overlay-setup-test-{}", std::process::id()));
        let file = root.join("settings.json");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, r#"{"theme":"dark"}"#).unwrap();
        let helper = Path::new("C:/overlay/llm-overlay-hook.exe");
        let plan = plan_install(Some(r#"{"theme":"dark"}"#), file.clone(), HookTarget::Claude, helper).unwrap();
        apply_plan(&plan).unwrap();
        assert!(is_installed_in(fs::read_to_string(&file).ok().as_deref(), HookTarget::Claude, helper));
        let removed = plan_remove(&fs::read_to_string(&file).unwrap(), file.clone(), HookTarget::Claude, helper).unwrap();
        apply_plan(&removed).unwrap();
        let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after["theme"], "dark");
        assert!(!is_installed_in(fs::read_to_string(&file).ok().as_deref(), HookTarget::Claude, helper));
        let _ = fs::remove_dir_all(root);
    }
}
