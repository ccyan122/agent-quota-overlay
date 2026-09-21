//! Application shell: window lifecycle, credential-free settings, and native menu.

use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::{Duration, Instant}};

use tauri::{menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu}, AppHandle, Emitter, LogicalSize, Manager, PhysicalSize, Size, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt;

use crate::{activity::{setup as hook_setup, ActivityService}, providers::{self, ProviderError}, quota::{cache::{self, CachedProviderQuota, CachedQuotaSnapshot}, ProviderId, ProviderQuota}, settings::{self, Settings}, update::{self, UpdateStatus}, window};

const SETTINGS_FILE: &str = "settings.json";
pub const OPACITY_CHANGED: &str = "overlay://opacity";
pub const REFRESH_REQUESTED: &str = "quota://refresh-requested";
pub const QUOTA_UPDATED: &str = "quota://updated";
pub const ACTIVITY_UPDATED: &str = "activity://updated";
pub const UPDATE_STATUS: &str = "update://status";

/// The overlay window and the Codex reset-credit panel.
const OVERLAY_WINDOW: &str = "overlay";
const PANEL_WINDOW: &str = "resets";
/// Panel geometry in logical pixels. The height grows with the number of
/// credits whose expiry is known and stops at a height that still fits beside
/// the overlay on a small display; the panel body scrolls beyond that.
const PANEL_WIDTH: f64 = 252.0;
const PANEL_BASE_HEIGHT: f64 = 112.0;
/// The divider and padding the expiry list adds before its first row.
const PANEL_LIST_HEADER: f64 = 15.0;
const PANEL_CREDIT_ROW_HEIGHT: f64 = 22.0;
const PANEL_MAX_HEIGHT: f64 = 300.0;

#[derive(Clone, Copy)]
enum RetryState { Ready, InFlight, RetryAt(Instant) }

struct QuotaState {
    cache_path: PathBuf,
    values: Mutex<Vec<CachedProviderQuota>>,
    retries: Mutex<HashMap<ProviderId, RetryState>>,
    failures: Mutex<HashMap<ProviderId, u8>>,
    /// Never persisted: cache values may render immediately, but only a fresh
    /// usage response proves which account owns their reset summary.
    work_account_context: Mutex<Option<String>>,
}

impl QuotaState {
    fn new() -> Self {
        let cache_path = settings::app_data_dir().join("quota-cache.json");
        Self::new_at_path(cache_path)
    }
    fn new_at_path(cache_path: PathBuf) -> Self {
        let mut values = cache::load(&cache_path).unwrap_or_default().providers;
        let stale_after = chrono::Utc::now() - chrono::Duration::minutes(5);
        for cached in &mut values { if cached.fetched_at < stale_after { cached.stale = true; } }
        Self { cache_path, values: Mutex::new(values), retries: Mutex::new(HashMap::new()), failures: Mutex::new(HashMap::new()), work_account_context: Mutex::new(None) }
    }
    fn snapshot(&self) -> Vec<ProviderQuota> {
        self.values.lock().map(|items| items.iter().map(|item| item.quota.clone()).collect()).unwrap_or_default()
    }
    /// Detail expiry data is independent from the regular usage poll.  It may
    /// enrich an existing Work summary, but must never invent a quota record or
    /// replace other providers after a best-effort detail request.
    fn apply_reset_credit_details(&self, details: crate::providers::chatgpt::ResetCreditDetails) -> bool {
        let context = match self.work_account_context.lock() { Ok(value) => value.clone(), Err(_) => return false };
        if context.as_deref() != Some(details.account_context.as_str()) { return false; }
        let mut values = match self.values.lock() { Ok(items) => items, Err(_) => return false };
        let Some(work) = values.iter_mut().find(|item| item.provider == ProviderId::ChatgptWork) else { return false };
        let Some(summary) = work.quota.reset_credits.as_mut() else { return false };
        *summary = details.credits;
        let _ = cache::save(&self.cache_path, &CachedQuotaSnapshot { providers: values.clone() });
        true
    }
    fn reserve(&self, force: bool, requested: Option<ProviderId>) -> Vec<ProviderId> { self.reserve_at(force, requested, Instant::now()) }
    fn reserve_at(&self, _force: bool, requested: Option<ProviderId>, now: Instant) -> Vec<ProviderId> {
        let mut retries = match self.retries.lock() { Ok(value) => value, Err(_) => return Vec::new() };
        [ProviderId::ChatgptWork, ProviderId::ClaudeCode, ProviderId::Antigravity].into_iter().filter(|provider| requested.is_none_or(|only| only == *provider)).filter(|provider| {
            let eligible = match retries.get(provider).copied() {
                Some(RetryState::InFlight) => false,
                Some(RetryState::RetryAt(at)) => at <= now,
                _ => true,
            };
            if eligible { retries.insert(*provider, RetryState::InFlight); }
            eligible
        }).collect()
    }
    fn apply(&self, results: Vec<(ProviderId, Result<ProviderQuota, ProviderError>)>) { self.apply_at(results, chrono::Utc::now(), Instant::now()); }
    fn apply_at(&self, results: Vec<(ProviderId, Result<ProviderQuota, ProviderError>)>, now: chrono::DateTime<chrono::Utc>, monotonic_now: Instant) {
        let mut values = match self.values.lock() { Ok(items) => items, Err(_) => return };
        let mut retries = match self.retries.lock() { Ok(items) => items, Err(_) => return };
        let mut failures = match self.failures.lock() { Ok(items) => items, Err(_) => return };
        for (provider, result) in results {
            match result {
                Ok(quota) => {
                    if provider == ProviderId::ChatgptWork {
                        if let Ok(mut context) = self.work_account_context.lock() { *context = quota.account_context.clone(); }
                    }
                    if let Some(existing) = values.iter_mut().find(|item| item.provider == provider) {
                        existing.quota = quota; existing.fetched_at = now; existing.stale = false;
                    } else { values.push(CachedProviderQuota { provider, fetched_at: now, stale: false, quota }); }
                    retries.insert(provider, RetryState::Ready);
                    failures.remove(&provider);
                }
                Err(error) => {
                    if let Some(existing) = values.iter_mut().find(|item| item.provider == provider) { existing.stale = true; }
                    let next = match error {
                        ProviderError::RateLimited { retry_after_seconds } => RetryState::RetryAt(monotonic_now + Duration::from_secs(retry_after_seconds.unwrap_or(60).max(1))),
                        _ => {
                            // Authentication and credential errors can recover without an app
                            // restart when a provider refreshes or recreates its credential file.
                            // Keep probing at a bounded rate instead of disabling the provider
                            // for the remainder of this process.
                            let count = failures.entry(provider).and_modify(|n| *n = n.saturating_add(1)).or_insert(1);
                            let exponent = (*count).saturating_sub(1).min(2);
                            RetryState::RetryAt(monotonic_now + Duration::from_secs(60 * 2_u64.pow(exponent as u32)))
                        },
                    };
                    retries.insert(provider, next);
                }
            }
        }
        let _ = cache::save(&self.cache_path, &CachedQuotaSnapshot { providers: values.clone() });
    }
}

pub struct AppState {
    settings_path: PathBuf,
    settings: Mutex<Settings>,
}

async fn refresh_shared(app: AppHandle, force: bool) -> Vec<ProviderQuota> { refresh_selected(app, force, None).await }

async fn refresh_selected(app: AppHandle, force: bool, requested: Option<ProviderId>) -> Vec<ProviderQuota> {
    let state = app.state::<QuotaState>();
    let eligible = state.reserve(force, requested);
    let mut jobs = tokio::task::JoinSet::new();
    for provider in eligible {
        jobs.spawn_blocking(move || providers::fetch_selected(&[provider]).into_iter().next().unwrap_or((provider, Err(ProviderError::Network))));
    }
    while let Some(Ok(result)) = jobs.join_next().await {
        state.apply(vec![result]);
        let _ = app.emit(QUOTA_UPDATED, state.snapshot());
    }
    let snapshot = state.snapshot();
    let _ = app.emit(QUOTA_UPDATED, snapshot.clone());
    refit_open_panel(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn get_quota_snapshot(state: State<'_, QuotaState>) -> Vec<ProviderQuota> { state.snapshot() }

#[tauri::command]
async fn refresh_quotas(app: AppHandle, manual: bool, provider: Option<ProviderId>) -> Vec<ProviderQuota> {
    if let Some(provider) = provider {
        refresh_selected(app, manual, Some(provider)).await
    } else { refresh_shared(app, manual).await }
}

#[tauri::command]
fn get_activity_snapshot(state: State<'_, ActivityService>) -> Vec<crate::activity::ProviderActivity> { state.snapshot_now() }

/// Codex reset-credit panel. The webview decides no layout policy, so the
/// panel's size and the side it opens on are decided here and applied before it
/// becomes visible. Returns whether the panel is now open, so a long press acts
/// as a toggle.
#[tauri::command]
fn toggle_reset_panel(app: AppHandle, state: State<'_, QuotaState>) -> Result<bool, String> {
    let panel = app.get_webview_window(PANEL_WINDOW).ok_or_else(|| "reset panel unavailable".to_owned())?;
    if panel.is_visible().unwrap_or(false) {
        panel.hide().map_err(|error| error.to_string())?;
        return Ok(false);
    }
    apply_panel_geometry(&app, &panel, &state.snapshot())?;
    panel.set_always_on_top(true).map_err(|error| error.to_string())?;
    panel.show().map_err(|error| error.to_string())?;
    refresh_reset_credit_details(app.clone());
    Ok(true)
}

/// Details are fetched after the panel is visible. This keeps a slow or
/// rate-limited expiry endpoint from delaying the Work interaction or harming
/// the main usage poll; the provider itself supplies the account-scoped TTL
/// and Retry-After backoff.
fn refresh_reset_credit_details(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let details = tokio::task::spawn_blocking(crate::providers::ChatgptProvider::fetch_reset_credit_details)
            .await.ok().and_then(Result::ok).flatten();
        let Some(details) = details else { return };
        let state = app.state::<QuotaState>();
        if state.apply_reset_credit_details(details) {
            let snapshot = state.snapshot();
            let _ = app.emit(QUOTA_UPDATED, snapshot.clone());
            refit_open_panel(&app, &snapshot);
        }
    });
}

fn apply_panel_geometry(app: &AppHandle, panel: &WebviewWindow, snapshot: &[ProviderQuota]) -> Result<(), String> {
    let overlay = app.get_webview_window(OVERLAY_WINDOW).ok_or_else(|| "overlay unavailable".to_owned())?;
    let size = LogicalSize::new(PANEL_WIDTH, panel_height(snapshot));
    panel.set_size(Size::Logical(size)).map_err(|error| error.to_string())?;
    let physical: PhysicalSize<u32> = size.to_physical(overlay.scale_factor().unwrap_or(1.0));
    window::place_beside(panel, &overlay, physical).map_err(|error| error.to_string())
}

/// Keeps an open panel fitted to the credit list it is showing, so a refresh
/// that changes the count does not leave the window the wrong size.
fn refit_open_panel(app: &AppHandle, snapshot: &[ProviderQuota]) {
    let Some(panel) = app.get_webview_window(PANEL_WINDOW) else { return };
    if !panel.is_visible().unwrap_or(false) { return }
    let _ = apply_panel_geometry(app, &panel, snapshot);
}

#[tauri::command]
fn hide_reset_panel(app: AppHandle) -> Result<(), String> {
    match app.get_webview_window(PANEL_WINDOW) {
        Some(panel) => panel.hide().map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// Grows with the credits whose expiry is known, so a short list leaves no dead
/// space and a long one still fits beside the overlay.
fn panel_height(snapshot: &[ProviderQuota]) -> f64 {
    let rows = snapshot.iter()
        .find(|quota| quota.provider == ProviderId::ChatgptWork)
        .and_then(|quota| quota.reset_credits.as_ref())
        .map_or(0, |credits| credits.credits.len());
    if rows == 0 { return PANEL_BASE_HEIGHT }
    (PANEL_BASE_HEIGHT + PANEL_LIST_HEADER + rows as f64 * PANEL_CREDIT_ROW_HEIGHT).min(PANEL_MAX_HEIGHT)
}

/// Last known result of the GitHub release check. Nothing here is persisted:
/// a version comparison is cheap to redo and a stale one is worse than none.
struct UpdateState { status: Mutex<UpdateStatus> }

impl UpdateState {
    fn new() -> Self { Self { status: Mutex::new(UpdateStatus::default()) } }
    fn snapshot(&self) -> UpdateStatus { self.status.lock().map(|value| value.clone()).unwrap_or_default() }
}

#[tauri::command]
fn get_update_status(state: State<'_, UpdateState>) -> UpdateStatus { state.snapshot() }

async fn run_update_check(app: AppHandle) -> UpdateStatus {
    let previous = app.state::<UpdateState>().snapshot();
    let result = tokio::task::spawn_blocking(move || update::check(&previous)).await.unwrap_or_default();
    if let Some(state) = app.try_state::<UpdateState>() {
        if let Ok(mut current) = state.status.lock() { *current = result.clone(); }
    }
    let _ = app.emit(UPDATE_STATUS, result.clone());
    result
}

/// Opens the release page in the user's browser. Nothing is downloaded or
/// installed by this app; the link is the whole action.
#[tauri::command]
fn open_release_page(app: AppHandle) -> Result<(), String> {
    let url = app.try_state::<UpdateState>()
        .and_then(|state| state.snapshot().release_url)
        .or_else(update::releases_page)
        .ok_or_else(|| "no release page is configured".to_owned())?;
    open_external(&url)
}

/// Only a GitHub release link is ever handed to the shell. The URL originates
/// from a network response, so the check is repeated here rather than trusted
/// from where it was stored.
fn open_external(url: &str) -> Result<(), String> {
    if !url.starts_with("https://github.com/") { return Err("refusing to open a non-GitHub link".to_owned()) }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (operation, target) = (wide("open"), wide(url));
        unsafe { ShellExecuteW(std::ptr::null_mut(), operation.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL); }
    }
    Ok(())
}

impl AppState {
    fn new() -> Self {
        let settings_path = settings::app_data_dir().join(SETTINGS_FILE);
        Self { settings: Mutex::new(settings::load(&settings_path)), settings_path }
    }

    fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        let mut settings = self.settings.lock().map_err(|_| "settings unavailable".to_owned())?;
        change(&mut settings);
        settings.normalize();
        settings::save(&self.settings_path, &settings).map_err(|_| "settings could not be saved".to_owned())?;
        Ok(settings.clone())
    }
}

#[tauri::command]
fn get_overlay_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state.settings.lock().map(|value| value.clone()).map_err(|_| "settings unavailable".to_owned())
}

#[tauri::command]
fn set_overlay_opacity(app: AppHandle, state: State<'_, AppState>, opacity: u8) -> Result<u8, String> {
    let updated = state.update(|settings| settings.opacity = opacity)?;
    let _ = app.emit(OPACITY_CHANGED, updated.opacity);
    Ok(updated.opacity)
}

#[tauri::command]
fn persist_overlay_position(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let window = app.get_webview_window(OVERLAY_WINDOW).ok_or_else(|| "overlay unavailable".to_owned())?;
    let position = window::remembered_position(&window).ok_or_else(|| "window position unavailable".to_owned())?;
    state.update(|settings| settings.position = Some(position)).map(|_| ())
}

#[tauri::command]
fn show_overlay_context_menu(app: AppHandle) -> Result<(), String> {
    let window = app.get_webview_window(OVERLAY_WINDOW).ok_or_else(|| "overlay unavailable".to_owned())?;
    let menu = context_menu(&app).map_err(|error| error.to_string())?;
    window.popup_menu(&menu).map_err(|error| error.to_string())
}

fn set_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if enabled { app.autolaunch().enable() } else { app.autolaunch().disable() }.map_err(|_| "autostart could not be updated".to_owned())
}

fn refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { let _ = refresh_shared(app, true).await; });
}

fn context_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let refresh = MenuItem::with_id(app, "refresh", "Refresh Now", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, "autostart", "Start with Windows", true, app.autolaunch().is_enabled().unwrap_or(false), None::<&str>)?;
    let thirty = MenuItem::with_id(app, "opacity:30", "30%", true, None::<&str>)?;
    let fifty = MenuItem::with_id(app, "opacity:50", "50%", true, None::<&str>)?;
    let seventy = MenuItem::with_id(app, "opacity:70", "70%", true, None::<&str>)?;
    let ninety = MenuItem::with_id(app, "opacity:90", "90%", true, None::<&str>)?;
    let hundred = MenuItem::with_id(app, "opacity:100", "100%", true, None::<&str>)?;
    let opacity = Submenu::with_items(app, "Opacity", true, &[&thirty, &fifty, &seventy, &ninety, &hundred])?;
    let hooks = hooks_submenu(app)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let update = update_menu_item(app)?;
    let mut entries: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&refresh, &opacity, &autostart, &hooks];
    if let Some(item) = update.as_ref() { entries.push(item); }
    entries.push(&separator);
    entries.push(&quit);
    Menu::with_items(app, &entries)
}

/// Absent until a repository is configured, so a build that has no releases to
/// compare against shows no update entry at all.
fn update_menu_item(app: &AppHandle) -> tauri::Result<Option<MenuItem<tauri::Wry>>> {
    if update::repository().is_none() { return Ok(None) }
    let status = app.try_state::<UpdateState>().map(|state| state.snapshot()).unwrap_or_default();
    let label = match (status.update_available(), status.latest_version.as_deref()) {
        (true, Some(version)) => format!("Get Update (v{version})"),
        _ => "Check for Updates".to_owned(),
    };
    MenuItem::with_id(app, "update", label, true, None::<&str>).map(Some)
}

/// A known update opens its release page; otherwise this checks and reports the
/// outcome, because a silent check leaves the user unable to tell it happened.
fn update_action(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if app.try_state::<UpdateState>().is_some_and(|state| state.snapshot().update_available()) {
            let _ = open_release_page(app.clone());
            return;
        }
        let status = run_update_check(app).await;
        let message = match (status.state, status.latest_version.as_deref()) {
            (crate::update::CheckState::Available, Some(version)) => format!("Version {version} is available.

This copy is {}.", status.current_version),
            (crate::update::CheckState::UpToDate, _) => format!("This is the latest version ({}).", status.current_version),
            _ => "Could not reach GitHub to check for updates.".to_owned(),
        };
        // MessageBoxW blocks its thread, so it never runs on the async runtime.
        std::thread::spawn(move || dialog::info("LLM Quota Overlay", &message));
    });
}

/// The hook helper shipped next to this executable (MSI install or portable folder).
fn bundled_helper() -> Option<std::path::PathBuf> {
    let helper = std::env::current_exe().ok()?.parent()?.join("llm-overlay-hook.exe");
    helper.is_file().then_some(helper)
}

/// One check item per agent: checked when this app's hooks are present for the
/// helper next to this executable; disabled when the agent is not on this PC.
fn hooks_submenu(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let helper = bundled_helper();
    let items = hook_setup::TARGETS.into_iter().map(|target| {
        let enabled = helper.is_some() && hook_setup::detected(target);
        let checked = helper.as_deref().is_some_and(|helper| hook_setup::is_installed(target, helper));
        CheckMenuItem::with_id(app, hook_setup::menu_id(target), hook_setup::display_name(target), enabled, checked, None::<&str>)
    }).collect::<tauri::Result<Vec<_>>>()?;
    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = items.iter().map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>).collect();
    Submenu::with_items(app, "Activity hooks", true, &refs)
}

/// Confirms, then installs or removes one agent's hooks. Runs off the event loop so
/// the modal dialogs never stall the overlay.
fn toggle_hooks(target: crate::activity::hooks::HookTarget) {
    std::thread::spawn(move || {
        let Some(helper) = bundled_helper() else { return };
        let Some(path) = hook_setup::config_path(target) else { return };
        let install = !hook_setup::is_installed(target, &helper);
        let name = hook_setup::display_name(target);
        let backups = settings::app_data_dir().join("hook-backups");
        let prompt = if install {
            let approval = if target == crate::activity::hooks::HookTarget::Codex { "

Codex asks you to approve new hooks in /hooks before they run." } else { "" };
            format!("Add activity hooks for {name}?

File: {}
Helper: {}

Only this app's entries are added. Existing settings are kept, and the original file is backed up to:
{}{approval}", path.display(), helper.display(), backups.display())
        } else {
            format!("Remove this app's activity hooks for {name}?

File: {}

Other hooks and settings are kept. The original file is backed up first.", path.display())
        };
        if !dialog::confirm("LLM Quota Overlay", &prompt) { return; }
        match hook_setup::set_installed(target, &helper, install, &backups) {
            Ok(_) if install => {
                let next = match target {
                    crate::activity::hooks::HookTarget::Codex => "Approve the new entries in Codex /hooks, then start a new session.",
                    _ => "Start a new session of the agent for the hooks to take effect.",
                };
                dialog::info("LLM Quota Overlay", &format!("Activity hooks added for {name}.

{next}"));
            }
            Ok(_) => dialog::info("LLM Quota Overlay", &format!("Activity hooks removed for {name}.")),
            Err(error) => dialog::warn("LLM Quota Overlay", &format!("Could not update hooks for {name}:
{error}

No changes were made.")),
        }
    });
}

#[cfg(windows)]
mod dialog {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO};
    fn show(title: &str, text: &str, flags: u32) -> i32 {
        let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), flags | MB_TOPMOST | MB_SETFOREGROUND) }
    }
    pub fn confirm(title: &str, text: &str) -> bool { show(title, text, MB_YESNO | MB_ICONQUESTION) == IDYES }
    pub fn info(title: &str, text: &str) { show(title, text, MB_OK | MB_ICONINFORMATION); }
    pub fn warn(title: &str, text: &str) { show(title, text, MB_OK | MB_ICONWARNING); }
}
#[cfg(not(windows))]
mod dialog {
    pub fn confirm(_: &str, _: &str) -> bool { false }
    pub fn info(_: &str, _: &str) {}
    pub fn warn(_: &str, _: &str) {}
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "refresh" => refresh(app),
        "update" => update_action(app),
        "quit" => app.exit(0),
        "autostart" => {
            let enabled = app.autolaunch().is_enabled().unwrap_or(false);
            if set_autostart(app, !enabled).is_ok() {
                if let Some(state) = app.try_state::<AppState>() { let _ = state.update(|settings| settings.autostart_enabled = !enabled); }
            }
        }
        value if hook_setup::target_for_menu_id(value).is_some() => {
            if let Some(target) = hook_setup::target_for_menu_id(value) { toggle_hooks(target); }
        }
        value if value.starts_with("opacity:") => {
            if let Ok(opacity) = value.trim_start_matches("opacity:").parse::<u8>() {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(updated) = state.update(|settings| settings.opacity = opacity) { let _ = app.emit(OPACITY_CHANGED, updated.opacity); }
                }
            }
        }
        _ => {}
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) { let _ = window.show(); }
        }))
        .manage(AppState::new())
        .manage(QuotaState::new())
        .manage(UpdateState::new())
        .setup(|app| {
            let state = app.state::<AppState>();
            let settings = state.settings.lock().map_err(|_| "settings unavailable")?.clone();
            if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) { window::configure(&window, &settings)?; }
            window::attach_monitor_guard(app.handle().clone());
            // The panel is a popup, not a document: closing it hides it so the
            // next long press can show the same warm webview.
            if let Some(panel) = app.get_webview_window(PANEL_WINDOW) {
                let hidden = panel.clone();
                panel.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event { api.prevent_close(); let _ = hidden.hide(); }
                });
            }
            let (activity, mut activity_updates) = ActivityService::new(Duration::from_secs(settings.activity_timeout_seconds));
            app.manage(activity.clone());
            let activity_for_runtime = activity.clone();
            tauri::async_runtime::spawn(async move { activity_for_runtime.spawn(); });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if activity_updates.changed().await.is_err() { break; }
                    let _ = handle.emit(ACTIVITY_UPDATED, activity_updates.borrow().clone());
                }
            });
            let handle = app.handle().clone();
            let refresh_interval = Duration::from_secs(settings.refresh_interval_seconds);
            tauri::async_runtime::spawn(async move {
                let _ = refresh_shared(handle.clone(), false).await;
                let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + refresh_interval, refresh_interval);
                loop { ticker.tick().await; let _ = refresh_shared(handle.clone(), false).await; }
            });
            // Release checks start after the overlay is up and stay rare: this
            // is an unauthenticated public endpoint, and a HUD gains nothing
            // from polling it often.
            if update::repository().is_some() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    loop {
                        let _ = run_update_check(handle.clone()).await;
                        tokio::time::sleep(update::CHECK_INTERVAL).await;
                    }
                });
            }
            Ok(())
        })
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .invoke_handler(tauri::generate_handler![get_overlay_settings, set_overlay_opacity, persist_overlay_position, show_overlay_context_menu, get_quota_snapshot, refresh_quotas, get_activity_snapshot, toggle_reset_panel, hide_reset_panel, get_update_status, open_release_page])
        .run(tauri::generate_context!())
        .expect("LLM Quota Overlay failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> QuotaState { QuotaState::new_at_path(std::env::temp_dir().join(format!("quota-state-{}.json", std::process::id()))) }
    fn quota(id: ProviderId) -> ProviderQuota { ProviderQuota { provider: id, pools: vec![], ..Default::default() } }
    fn work_quota(context: &str, count: u32) -> ProviderQuota {
        ProviderQuota {
            provider: ProviderId::ChatgptWork,
            pools: vec![],
            reset_credits: Some(crate::quota::ResetCredits { available_count: count, ..Default::default() }),
            account_context: Some(context.to_owned()),
        }
    }
    #[test]
    fn settings_path_is_not_a_credential_path() {
        assert_eq!(AppState::new().settings_path.file_name().and_then(|value| value.to_str()), Some(SETTINGS_FILE));
    }
    #[test]
    fn reservations_prevent_duplicate_inflight_and_allow_targeting() {
        let state = state();
        let now = Instant::now();
        assert_eq!(state.reserve_at(false, Some(ProviderId::ClaudeCode), now), vec![ProviderId::ClaudeCode]);
        assert!(state.reserve_at(true, Some(ProviderId::ClaudeCode), now).is_empty());
        assert_eq!(state.reserve_at(false, Some(ProviderId::ChatgptWork), now), vec![ProviderId::ChatgptWork]);
    }
    #[test]
    fn auth_failure_retries_automatically_and_success_recovers() {
        let state = state(); let now = Instant::now();
        let _ = state.reserve_at(false, Some(ProviderId::ClaudeCode), now);
        state.apply_at(vec![(ProviderId::ClaudeCode, Err(ProviderError::AuthenticationRejected(401)))], chrono::Utc::now(), now);
        assert!(state.reserve_at(false, Some(ProviderId::ClaudeCode), now + Duration::from_secs(59)).is_empty());
        assert_eq!(state.reserve_at(false, Some(ProviderId::ClaudeCode), now + Duration::from_secs(60)), vec![ProviderId::ClaudeCode]);
        state.apply_at(vec![(ProviderId::ClaudeCode, Ok(quota(ProviderId::ClaudeCode)))], chrono::Utc::now(), now + Duration::from_secs(60));
        assert_eq!(state.reserve_at(false, Some(ProviderId::ClaudeCode), now + Duration::from_secs(60)), vec![ProviderId::ClaudeCode]);
    }
    #[test]
    fn retry_after_and_exponential_backoff_are_enforced_and_success_resets() {
        let state = state(); let now = Instant::now();
        let id = ProviderId::ChatgptWork;
        for (index, seconds) in [60, 120, 240, 240].into_iter().enumerate() {
            let _ = state.reserve_at(false, Some(id), now + Duration::from_secs(index as u64 * 300));
            let at = now + Duration::from_secs(index as u64 * 300);
            state.apply_at(vec![(id, Err(ProviderError::Network))], chrono::Utc::now(), at);
            assert!(state.reserve_at(false, Some(id), at + Duration::from_secs(seconds - 1)).is_empty());
            assert_eq!(state.reserve_at(false, Some(id), at + Duration::from_secs(seconds)), vec![id]);
        }
        state.apply_at(vec![(id, Ok(quota(id)))], chrono::Utc::now(), now + Duration::from_secs(2000));
        assert_eq!(state.reserve_at(false, Some(id), now + Duration::from_secs(2000)), vec![id]);
    }
    #[test]
    fn failure_marks_last_good_stale_without_erasing_it() {
        let state = state(); let now = Instant::now(); let id = ProviderId::Antigravity;
        let _ = state.reserve_at(false, Some(id), now);
        state.apply_at(vec![(id, Ok(quota(id)))], chrono::Utc::now(), now);
        let _ = state.reserve_at(false, Some(id), now);
        state.apply_at(vec![(id, Err(ProviderError::Network))], chrono::Utc::now(), now);
        assert_eq!(state.snapshot(), vec![quota(id)]);
        assert!(state.values.lock().unwrap()[0].stale);
    }
    #[test]
    fn reset_details_apply_only_to_the_matching_fresh_work_account() {
        let state = state();
        state.apply(vec![(ProviderId::ChatgptWork, Ok(work_quota("account-b", 2)))]);
        let old_account = crate::providers::chatgpt::ResetCreditDetails {
            account_context: "account-a".to_owned(),
            credits: crate::quota::ResetCredits { available_count: 9, ..Default::default() },
        };
        assert!(!state.apply_reset_credit_details(old_account));
        assert_eq!(state.snapshot()[0].reset_credits.as_ref().unwrap().available_count, 2);
        let current_account = crate::providers::chatgpt::ResetCreditDetails {
            account_context: "account-b".to_owned(),
            credits: crate::quota::ResetCredits { available_count: 3, ..Default::default() },
        };
        assert!(state.apply_reset_credit_details(current_account));
        assert_eq!(state.snapshot()[0].reset_credits.as_ref().unwrap().available_count, 3);
    }
}
