# Validation ledger

The complete acceptance criteria are in [REQUIREMENTS.ko.md](REQUIREMENTS.ko.md). This file records observed results; implemented behavior is not automatically treated as a successful live test.

## Environment observed on 2026-09-18

- Windows x64, OS build 10.0.26200.0.
- Codex CLI 0.154.0; desktop-bundled CLI 0.155.0-alpha.2.6.
- Claude Code CLI 2.1.270.
- Antigravity installed; its current Electron package and language server were located. Version verification pending.
- Existing Codex and Claude credential files are present. Their contents are not included in this ledger.
- MSVC 2022 Build Tools, Windows SDK 10.0.26100.0, and WebView2 153.0.4234.32 are installed.
- Rust 1.98.1 was installed under this workspace's `.tools` directory without modifying the system PATH.

## Required evidence

| Area | Automated evidence required | Live evidence required | Status |
| --- | --- | --- | --- |
| Quota | Remaining conversion, clamping, malformed/missing data, 401/403/429/5xx, failure isolation, cache | Official provider requests using existing local sign-in | Pass (all three live; Antigravity OAuth fallback still 401) |
| Antigravity | Separate Gemini and Claude/GPT pools; explicit 5h/weekly semantics | Current installed provider response compatibility | Pass via local language server; requires Antigravity running |
| Countdown | Days/hours/minutes/seconds, expired/negative time, refresh deduplication | Visible countdown with real reset times | Pass (live truncation observed 2026-09-19) |
| Activity | Start/stop, overlapping sessions/providers, unknown, watchdog, helper failure | Real lifecycle hook delivery where available | Claude and Antigravity pass (E2E + live pulse); Codex pending (`/hooks` approval) |
| Hook configuration | Merge/remove preserves other entries, concurrent-edit protection | Installation against current tool schemas; trust requirements recorded | Formats verified live; installed into real user config 2026-09-19 (Codex awaiting `/hooks` approval) |
| Overlay | Geometry and persistence; reduced-motion behavior | Frameless, topmost, taskbar exclusion, drag, opacity, context menu, monitor recovery | Pass except monitor-disconnect recovery (pending) |
| Autostart | Opt-in setting and command construction | Toggle writes/removes only this app's registration | Pass (unpackaged launch) |
| Security | No credentials or conversation data in frontend, logs, IPC or cache | Sanitized diagnostic output and same-user local IPC | In progress |
| Performance | No per-second network requests, isolated countdown rendering | Release process CPU/RAM measurement | Pass (0% idle CPU; 211 MB private incl. WebView2) |
| Distribution | Production frontend and Rust build | Launch portable executable and create installer where possible | Portable pass; MSI built and installed on this PC 2026-09-19 |

## Live quota probe, 2026-09-19 KST

The user explicitly approved authenticated requests to the official quota services after automatic approval review initially blocked them. The current source was rebuilt and `quota-probe` completed successfully outside the network-restricted sandbox. Only normalized quota values and sanitized failure categories were printed.

| Provider | 5h remaining | Weekly remaining | Observed result |
| --- | --- | --- | --- |
| ChatGPT Work | 62% | 38% | Both real reset timestamps received |
| Claude Code | 98% | 77% | Both real reset timestamps received |
| Antigravity | Unavailable | Unavailable | Existing OAuth credential was rejected with HTTP 401; local language-server compatibility remains in progress |

These values are an observation at the time of the probe, not current usage guarantees. A sandboxed Credential Manager miss was not treated as proof that the signed-in user's credential was absent.

## Review decisions

- Direct implementation is assigned to GPT 5.6 Terra with medium reasoning. The parent coordinates research and validation.
- Provider endpoints remain inside Rust adapters. Missing provider windows remain unavailable instead of being inferred from unrelated limits.
- User credentials are read in place; token copies, refresh/reissue operations, and prompt-based quota probes are not part of quota retrieval.
- New Codex hooks require review in the tool's hook trust interface. The implementation must not bypass or silently change that trust state.
- Synthetic hook events validate transport/state behavior only. They do not establish that an installed agent emits the expected lifecycle events.

## Additional live and automated evidence, 2026-09-19

The Terra provider agent rebuilt and ran the current quota probe with desktop permissions. The running Antigravity language server returned Gemini 5h 87.37218%, weekly 97.89536%, and Claude/GPT 5h and weekly 100%, with separate reset timestamps for all four windows. These historical observations supersede the earlier unavailable Antigravity result above; the OAuth fallback still returned 401.

Parent verification: frontend tests passed (2 tests) and the production web bundle built. The initial bundle reported a missing Pretendard asset, which remains a release gate until bundled. The Rust suite passed 31 library tests, 3 helper tests, and one spawned-helper named-pipe integration test. These counts are test cases, not a claim that all 35 acceptance scenarios have been verified.

Review follow-ups: bound malformed/oversized/no-EOF hook input; omit existing user settings from hook-plan output; constrain Antigravity process discovery to the installed app and bound discovery time; verify polling backoff, request deduplication, native UI, and release artifacts.

## Native overlay and performance, 2026-09-19 19:35–19:45 KST

Measured on the release portable build (`release/LLM-Quota-Overlay/llm-quota-overlay.exe`) with `scripts/validate-native-overlay.ps1` and `scripts/measure-overlay-performance.ps1`, plus UI Automation and screen captures. Both scripts are read-only.

**Launch context caveat.** The instance that was already running (PID 8284) had been started from the Codex desktop app. Codex is an MSIX package, so Windows redirected that overlay's `%LOCALAPPDATA%` writes to `%LOCALAPPDATA%\Packages\OpenAI.Codex_*\LocalCache\Local\LLMQuotaOverlay`. Its settings recorded `autostartEnabled: true`, but no real `HKCU\...\Run` entry existed, because the registry write was virtualized as well. The code is not at fault. Persistence and autostart must be validated from an unpackaged launch, such as Explorer, a shortcut, or a Run key. The overlay was restarted through `explorer.exe` (PID 2300) for the checks below.

| Check | Result | Evidence |
| --- | --- | --- |
| Always-on-top | Pass | `WS_EX_TOPMOST` set (ExStyle `0x00040118`) |
| Frameless | Pass (visual) | No title bar in capture. `WS_CAPTION` bits remain set because tao keeps them for snap and animation and hides them through non-client handling, so the style bit is only advisory. |
| Taskbar exclusion | Pass | UI Automation lists 27 taskbar buttons and none belongs to the overlay. `WS_EX_APPWINDOW` is present, but tao removes the tab with `ITaskbarList::DeleteTab`. |
| Window size | Pass | 328×332 px at 100% scaling on the primary monitor, inside the 280–360 px target |
| Multi-monitor restore | Pass (restore only) | PID 8284 restored on `\\.\DISPLAY2` (portrait, 125% scaling, negative X) from its saved `monitorName` and `monitorScaleFactor` |
| Persistence path | Pass | Unpackaged launch writes `%LOCALAPPDATA%\LLMQuotaOverlay\quota-cache.json` within 3 s of start |
| 60 s polling and cache | Pass | Redirected cache `fetchedAt` advanced in step with the polling interval. `stale:false` for all three providers. |
| Live quota display | Pass | ChatGPT Work 0%/84%, Claude Code 96%/73%, Antigravity Gemini 91%/91%, Claude/GPT 100%/100%. Every reset countdown is present. |
| Countdown truncation (live) | Pass | Gemini 5h changed from `1시간` to `59분` between captures |
| Idle CPU | Pass | 0.000% across all 7 processes (overlay and WebView2) over a 60 s sample |
| Memory | Recorded | Working set 442 MB and private bytes 211 MB, including WebView2 child processes |

Still pending live evidence, all of which needs user input: drag and position save after drag, Ctrl+wheel opacity and its persistence, context menu items, the autostart toggle writing and removing this app's Run entry from an unpackaged instance, and returning to a visible area after a monitor is disconnected.

### User-driven checks, 2026-09-19 19:43 KST (PID 2300, unpackaged)

| Check | Result | Evidence |
| --- | --- | --- |
| Drag and position save | Pass | Dragged from primary (182,182) to `\\.\DISPLAY2`; `settings.json` stored monitor name and 1.25 scale; native rect now (-707,129) |
| Autostart toggle (enable) | Pass | Real `HKCU\...\Run` value `LLM Quota Overlay` = release exe path; `autostartEnabled: true` |
| Opacity persistence | Pass | The default is 88. The user changed opacity with Ctrl+wheel and ended at 100, and `settings.json` recorded 100. |
| Autostart toggle (disable) | Pass | After the user turned it off at 20:01 on the rebuilt DPI-fix build, the `HKCU\...\Run` value `LLM Quota Overlay` was removed and `autostartEnabled` became false |
| Monitor removal recovery | Pending | Needs a physical disconnect |

### Cross-DPI drag bug, found and fixed 2026-09-19 19:45–19:58 KST

Symptom: while the user dragged between the 100% primary monitor and the 125% secondary monitor, the overlay shrank at each crossing (the user once saw it at 9×9 px), stuttered at the boundary, and returned to full size only later. (The first native-check reading of 328 px on DISPLAY2 was an artifact of a DPI-unaware measuring process, not the bug.)

Cause, from an env-gated trace build: at the boundary, `WM_DPICHANGED` alternated between 96 and 120 DPI about every 15 ms, 30–180 times per drag. For each 1.25→1.0 change, tao resized the window from its stale size (328 → 262 → 210 → … → 35 px). That intermediate size moved most of the window back onto the other monitor, which triggered the next DPI change. A queued `set_size` correction could not keep up, and correcting with an extra `SetWindowPos` inside the handler caused its own ping-pong.

Fix (`src-tauri/src/window.rs`, `dpi_guard`): a window subclass sets a target rect for the duration of tao's `WM_DPICHANGED` handling: Windows' suggested top-left with the design size × new DPI. It rewrites tao's own resize in `WM_WINDOWPOSCHANGING` to that rect, so a wrong intermediate size is never applied. `minWidth` and `maxWidth` were removed from `tauri.conf.json`: the window is not resizable, and tao converts those limits with a possibly stale scale.

Verification: in the user's drag test (5 round trips), each crossing produced exactly one DPI change, and every size was correct (328×332 at 100%, 410×415 at 125%). The user confirmed there was no shrinking or stutter. 50 library tests pass. The release folder and portable zip were rebuilt at 19:58.

## Live activity lifecycle E2E, 2026-09-19 20:03–20:13 KST

Each test runs one minimal real turn ("Reply with exactly OK.") and sends hook events to a test-only named pipe. The user's real hook configuration was not modified.

| Provider | Result | Evidence |
| --- | --- | --- |
| Claude Code 2.1.278 | Pass | `docs/test-results/claude-lifecycle-e2e.json`: 2 transitions (active → idle), 14.6 s |
| Antigravity CLI (agy) 1.2.7 | Pass | `docs/test-results/antigravity-lifecycle-e2e.json`: 2 transitions (active → idle), 23.8 s |

Root causes fixed along the way:

- **Claude, previously "inconclusive" at a 90 s timeout.** `claude --print` waits for stdin when stdin is not a terminal, and the inherited test-runner stdin never closed. The test now passes `Stdio::null()`, and a manual run completes in about 5 s. This affected only the test; real hooks are unaffected.
- **Antigravity hook file location.** agy 1.2.7 logged `loaded 0 named hooks from 0 hooks.json file(s)` for a workspace `.agents/hooks.json`, even with the workspace in `trustedWorkspaces`. Only the global `~/.gemini/config/hooks.json` loaded. Install Antigravity hooks at that global path. The E2E now uses an isolated `USERPROFILE`/`HOME` that contains only that file, and sign-in still works through the Windows keyring.
- **Antigravity command quoting.** agy runs hook commands through `cmd /c` as a single Go-escaped argument, so `"C:\...\llm-overlay-hook.exe"` became `\"...\"` and cmd reported that the program was not recognized. Antigravity commands are now written unquoted. If the path contains spaces, the 8.3 short path is used, falling back to quotes only when no short path exists. Removal also deletes the earlier quoted form. Claude and Codex keep quoted commands.
- **Antigravity Stop timeout.** The Stop hook runs while the turn shuts down. With `timeout: 1`, it failed with exit status 1 before the helper started, while `timeout: 5` succeeds. Installed hooks now use a 5 s upper bound; the helper itself exits in about 150 ms.
- Note: agy parses hook stdout as JSON, so a hook that prints non-JSON text fails. The helper prints nothing.

Still pending: installing hooks into the user's real configuration (`~/.claude/settings.json`, `~/.gemini/config/hooks.json`, and Codex with `/hooks` approval) and observing the overlay pulse during real work.

## Hook installation into real user configuration, 2026-09-19 20:17 KST

Installed with the user's approval after reviewing the plan-only output of `activity-hook-config --install-hooks` for each target, using the helper at `release\LLM-Quota-Overlay\llm-overlay-hook.exe`. Originals were backed up to `%LOCALAPPDATA%\LLMQuotaOverlay\hook-backups\<timestamp>\`.

| Target | File | Result |
| --- | --- | --- |
| Claude Code | `~/.claude/settings.json` | Valid JSON. Only the `hooks` key changed. Six events added (UserPromptSubmit, PreToolUse, PostToolUse → active; Stop, StopFailure, SessionEnd → idle), each with a single entry from this app. |
| Antigravity | `~/.gemini/config/hooks.json` (created) | Named set `llm-quota-overlay` with PreInvocation, PreToolUse, PostToolUse, and Stop, using unquoted commands and `timeout: 5` |
| Codex | `~/.codex/hooks.json` (was `{}`) | Six events added. The user must still approve them in Codex `/hooks`. |

Pending live check: the overlay indicator should pulse during real work in each agent and stop at turn end.

### Live pulse check, 2026-09-19 KST (user-observed)

| Agent | Result |
| --- | --- |
| Claude Code (new session) | Pass: the user saw the indicator pulse during real work and stop at turn end |
| Antigravity | Pass: the user saw the indicator pulse during real work and stop at turn end |
| Codex / ChatGPT Work | Pending: needs `/hooks` approval in Codex; deferred until the user's Codex 5h quota resets |
| Concurrent agents (#21) | Pass: with multiple agents working at once, the user saw each indicator pulse independently |

## MSI installer, 2026-09-19 20:22 KST

Built with `npx tauri build --bundles msi --features desktop` (tauri-cli 2.11.4, WiX 3.14 downloaded automatically) as `release/LLM Quota Overlay_0.1.0_x64_ko-KR.msi` (9.1 MB). The Tauri bundler includes every Cargo `[[bin]]` automatically, so listing the helper exes under `bundle.resources` caused ICE30 duplicate-component errors. Only the license and readme files are declared as resources.

Contents, verified by administrative extraction (`msiexec /a`, which installs nothing), under `Program Files\LLM Quota Overlay\`: `llm-quota-overlay.exe`, `llm-overlay-hook.exe`, `activity-hook-config.exe`, `quota-probe.exe` (read-only diagnostic, not in the portable zip), `README.md`, `THIRD_PARTY_NOTICES`, and `Pretendard-OFL.txt`.

The MSI is per-machine and needs UAC to install. The installed helper path contains spaces: Claude and Codex commands stay quoted, and Antigravity uses the 8.3 short path. After installing, re-point the hooks from the portable helper to the installed one. An actual install and uninstall have not been run yet.

## In-app hook setup and MSI install, 2026-09-19 20:31–20:35 KST

A new context-menu submenu **Activity hooks** lists Claude Code, Antigravity, and Codex (ChatGPT Work) as check items (`src-tauri/src/activity/setup.rs`, `app.rs`). An item is checked when this app's entries for the helper next to the running exe are present. It is disabled when the agent's home folder is missing. Clicking an item shows a confirmation dialog with the file, the helper, and the backup location, then installs or removes the entries. The original file is backed up to `%LOCALAPPDATA%\LLMQuotaOverlay\hook-backups\`, and Codex users are reminded to approve the entries in `/hooks`.

Test on this PC: the portable-path hooks were removed with `activity-hook-config --remove-hooks` (leaving only an empty `hooks: {}`), and the rebuilt MSI was installed (`msiexec` exit 0, per-machine, to `C:\Program Files\LLM Quota Overlay\`). The user installed all three agents' hooks from the installed app's menu and confirmed it works.

| Target | Written command | Check |
| --- | --- | --- |
| Claude Code | `"C:\Program Files\LLM Quota Overlay\llm-overlay-hook.exe" claude_code active/idle`, timeout 5 | Six events present |
| Codex | Same quoted path with `chatgpt_work` | Six events present; `/hooks` approval pending |
| Antigravity | `C:\PROGRA~1\LLMQUO~1\LLM-OV~1.EXE antigravity active/idle` (8.3 short path), timeout 5 | Four events present; `cmd /c` runs the command with exit 0 |

55 library tests pass, including new ones for install status and menu ids.
