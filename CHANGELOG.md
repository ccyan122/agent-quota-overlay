# Changelog

This file records user-visible changes to Agent Quota Overlay. Release tags use `vMAJOR.MINOR.PATCH`.

## 0.2.0 — 2026-09-21

- Refreshes ChatGPT Work and Claude Code quota directly from their remote usage APIs using backend-only local credentials, without requiring the CLIs to stay open.
- Keeps Antigravity's local language-server path when available and falls back to its signed-in remote quota API.
- Replaces permanent provider disablement after authentication failures with bounded 1, 2, and 4 minute retries while retaining the last successful quota as stale.
- Adds a read-only ChatGPT Work long-press panel for banked Codex reset credits and expiry times.
- Fetches reset-credit details only when the panel opens, with a five-minute account-scoped cache, single-flight request coalescing, and `Retry-After` handling.
- Prevents a delayed reset-credit response from one account from being applied after the active account changes.
- Adds GitHub release detection for `ccyan122/agent-quota-overlay`; update notifications link to the release page and never install files automatically.

## 0.1.0 — 2026-09-19

- Introduces the always-on-top Windows quota HUD for ChatGPT Work, Claude Code, and Antigravity.
- Normalizes five-hour and weekly quota windows, reset countdowns, and Antigravity's separate Gemini and Claude/GPT pools.
- Isolates provider failures, persists normalized last-known quota values, and restores cached data on startup without storing credentials.
- Adds activity hooks with independent provider indicators, overlap-safe session tracking, and watchdog recovery.
- Adds drag persistence, multi-monitor and mixed-DPI handling, opacity controls, autostart, and a native context menu.
- Ships portable and MSI-compatible Windows builds with bundled hook helpers and a read-only quota diagnostic.
