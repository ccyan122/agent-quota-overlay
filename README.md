# LLM Quota Overlay

Windows HUD for local ChatGPT Work, Claude Code, and Antigravity quota data. It reads existing local credentials directly and never sends them through a third-party service.

Run `llm-quota-overlay.exe`. Right-click the HUD to refresh, set opacity, enable autostart, turn activity hooks on or off per agent (Activity hooks), or quit. Ctrl+mouse wheel adjusts opacity. The overlay stores only its position, display preferences, and normalized last-known quota values under `%LOCALAPPDATA%\LLMQuotaOverlay`.

## Codex reset credits

Press and hold the **ChatGPT Work** section to open a separate panel listing your
Codex rate-limit reset credits (리셋권): how many are available and when each one
expires. The panel opens on whichever side of the overlay has more room on the
display it currently sits on, so an overlay parked against an edge opens inward.
Hold again, click the overlay, or move the overlay to close it.

The panel is read-only. Spending a credit resets your 5-hour and weekly windows
and cannot be undone, so that stays in Codex where you already confirm it.

## Update notifications

The overlay compares its own version with the latest GitHub release and shows a
small version badge in its top-right corner when a newer one exists; **Check for
Updates** in the right-click menu checks on demand. Clicking either opens the
release page in your browser. The check reads release metadata only; the app
does not download or install a release payload.

Releases are checked from the project repository configured in `src-tauri/src/update/mod.rs`:

```rust
pub const GITHUB_REPO: &str = "ccyan122/agent-quota-overlay";
```

Publish a full GitHub Release tagged `vMAJOR.MINOR.PATCH`, with its numeric
version matching both `Cargo.toml` and `tauri.conf.json`. A tag without a
published release is not considered, and an unparseable tag never claims an
update. See [CHANGELOG.md](CHANGELOG.md) for the release history.

Build: `npm run build`, then from `src-tauri`, build with `cargo build --release --features desktop,custom-protocol`.

MSI installer: `npx tauri build --bundles msi --features desktop,custom-protocol` (WiX is downloaded on first run). Output: `src-tauri/target/release/bundle/msi/`. It installs per machine to `C:\Program Files\LLM Quota Overlay\`. After installing, enable activity hooks from the right-click **Activity hooks** menu; they point at the helper next to the running exe. `activity-hook-config.exe` remains available for scripted installs.
