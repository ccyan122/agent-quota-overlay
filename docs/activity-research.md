# Activity lifecycle research

This application treats **actual lifecycle events** as the sole evidence of activity.
The presence of a process is never sufficient: a terminal waiting at a prompt is idle,
and a provider without a verified lifecycle event remains `unknown`.

## Local evidence collected on 2026-09-18

| Provider | Installed version / local evidence | Supported lifecycle mapping used |
| --- | --- | --- |
| Codex / ChatGPT Work | Codex CLI `0.154.0`; desktop-bundled `0.155.0-alpha.2.6`; `%USERPROFILE%\\.codex\\hooks.json` exists and is an empty JSON container | `UserPromptSubmit` -> active; `PreToolUse` and `PostToolUse` -> active heartbeat; `Stop`, `Interrupt`, `SessionEnd` -> idle |
| Claude Code | Claude Code `2.1.270`; `%USERPROFILE%\\.claude\\settings.json` contains no configured hook keys | `UserPromptSubmit` -> active; `PreToolUse` and `PostToolUse` -> active heartbeat; `Stop`, `StopFailure`, `SessionEnd` -> idle |
| Google Antigravity | local `%USERPROFILE%\\.gemini\\antigravity` runtime data and `bin` were found | `PreInvocation` -> active; `PreToolUse` and `PostToolUse` -> active heartbeat; `Stop` -> idle |

The no-config findings are sanitized presence/schema checks only. Credential files, transcript files,
and hook payload contents were not read.

## Primary sources checked

- OpenAI Codex hooks documentation: `https://learn.chatgpt.com/docs/hooks` (checked 2026-09-18). It documents `UserPromptSubmit`, `Stop`, `Interrupt`, and `SessionEnd`, and requires user review in `/hooks` for untrusted hooks.
- OpenAI Codex current source, `codex-rs/hooks/src/lib.rs` (checked 2026-09-18), lists `PreToolUse`, `PostToolUse`, `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `Stop`, and `Interrupt` among supported names.
- Anthropic Claude Code hooks documentation (checked 2026-09-18) documents `UserPromptSubmit`, tool lifecycle events, `Stop`, `StopFailure`, and `SessionEnd`.
- Google Antigravity Hooks documentation: `https://antigravity.google/docs/hooks` (checked 2026-09-18). It documents named hook sets in `.agents/hooks.json` or global customization locations, with `PreInvocation`, `PostInvocation`, `PreToolUse`, `PostToolUse`, and `Stop`. Observed 2026-09-19 with agy 1.2.7: only the global `~/.gemini/config/hooks.json` loaded. Commands run through `cmd /c` with Go argument escaping, so a quoted program path breaks and must be written unquoted. Hook stdout is parsed as JSON. See VALIDATION.md.

`PostInvocation` and `PostToolUse` never make a provider idle: a single turn can continue with
additional tool/model work. They refresh the watchdog instead. Only a documented final event
changes an observed provider to idle.

## IPC boundary

`llm-overlay-hook.exe` accepts only fixed `provider` and `active`/`idle` arguments, plus an
optional opaque `session_id` restricted to 128 ASCII alphanumeric, `_`, and `-` characters. When
a hook host provides the session id only on stdin, the helper reads at most 256 KiB for at most
50 ms, extracts only the `session_id`, `sessionId`, or `conversationId` lifecycle field, then
discards the input. This accommodates normal hook envelopes which also contain prompt and tool
fields without retaining or relaying them. It sends one reduced bounded JSON line (maximum 512
bytes) to `\\.\\pipe\\llm-quota-overlay`, waits only up to its 150 ms total best-effort budget for
the server acknowledgement needed for PID/SID validation, then exits successfully even if the
server is unavailable.

The server uses `PIPE_REJECT_REMOTE_CLIENTS`, limits concurrent connections, bounds read size and
duration, rejects unknown JSON fields, ignores client timestamps, and compares the connecting
process's Windows token SID with the overlay process SID. A failed identity check is rejected.

## Watchdog and concurrency

An active event creates or refreshes an opaque session slot. A final event clears only that slot,
so a parallel session stays active. Providers with no session identifier use a provider-scoped slot.
If an active slot receives no lifecycle heartbeat for 10 minutes, it becomes idle. The timeout is a
crash/failure escape hatch, not process detection.

## Installation and removal

The hook module creates **plans**, not automatic configuration changes. A caller must present its
path, events, and exact helper command to the user, then explicitly apply the reviewed plan.

- Existing JSON keys, hook events, matchers, and handlers are preserved.
- Installation appends only an exact helper entry when it is absent.
- Removal prunes only the helper command and keeps other commands even when they share the same hook group.
- The apply step detects a changed configuration between planning and writing, creates a missing parent directory, and writes a same-directory temporary file before rename.
- Codex installation never enables hidden feature flags, bypasses hook trust, or edits trust state. The user must approve the new entry through Codex `/hooks` when Codex asks.

No real user-level agent configuration has been changed by this module.
