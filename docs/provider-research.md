# Provider research and compatibility notes

This application reads existing local sign-ins and requests provider quota
summaries. It does not scrape browser pages, issue new OAuth tokens, refresh
tokens, or expose credential material to the frontend.

## ChatGPT Work

The existing Codex sign-in at `%CODEX_HOME%\\auth.json` (or
`%USERPROFILE%\\.codex\\auth.json`) supplies an access token and optional
account identifier to `GET https://chatgpt.com/backend-api/wham/usage`.
`rate_limit.primary_window` and `secondary_window` are classified from
`limit_window_seconds`, never their position. The recognized values are
18,000 seconds for 5h and 604,800 seconds for week. A missing duration uses
the documented legacy slot only; an unknown explicit duration is withheld.

Reset credits are read from the same sign-in. `GET
https://chatgpt.com/backend-api/wham/rate-limit-reset-credits` is the only
source that carries each credit's expiry, so it is requested best effort right
after the usage call and its failure never fails a refresh. A credit is counted
when its `status` is `available` or absent; an explicitly consumed or expired
one is dropped. `expires_at` is accepted as RFC 3339 or epoch seconds. When that
call returns nothing usable, the count falls back to the usage body's own
`rate_limit_reset_credits.available_count`, and the panel says the expiry times
were not reported rather than implying there are none. Spending a credit is
irreversible, so the consume endpoint is deliberately not called by this app.

## Claude Code

Claude credentials are read from `CLAUDE_CODE_OAUTH_TOKEN`, then
`%CLAUDE_CONFIG_DIR%\\.credentials.json`, then
`%USERPROFILE%\\.claude\\.credentials.json`. The credential JSON uses
`claudeAiOauth.accessToken`. The adapter calls the existing OAuth usage
endpoint with the `oauth-2025-04-20` beta header and maps `five_hour` and
`seven_day` separately. It does not use local token totals as an allowance
estimate and does not perform a message-generation fallback, because that
would spend user quota.

## Google Antigravity

The adapter first attempts the existing Windows Credential Manager entry
`gemini:antigravity` in memory and calls `loadCodeAssist`, then
`retrieveUserQuotaSummary` against Cloud Code service candidates. A model
list is not used as quota fallback because it does not prove either a shared
pool or a 5-hour window.

The installed Antigravity desktop app was checked on 2026-09-19. Its executable
file metadata and `--version` output contain no version string on this
installation, so this document does not claim a version number. Its
`language_server.exe` exposes a loopback Connect service while the app is
running. The adapter reads only the running server's port and per-session CSRF
argument in memory, sends one bounded HTTPS
`RetrieveUserQuotaSummary` request to `127.0.0.1`, disables redirects and
proxies for that request, and discards the command line and response after
normalization. It does not read terminal output, conversation logs, or CSRF
secrets from log files.

## Failure and refresh behavior

Providers run independently. 401/403 stops normal retry for that provider;
429 honors `Retry-After` in either seconds or HTTP-date form; 5xx and network
errors are eligible for caller-managed exponential backoff. Last-good values
are retained as per-provider cache records with their own timestamps. Cache
replacement writes a complete temporary file then replaces the destination
without a delete-first gap.

## Validation status (2026-09-19)

With explicit user authorization for read-only quota requests, `quota-probe`
returned normalized ChatGPT Work and Claude Code five-hour and weekly values.
It also returned both Antigravity language-server quota groups. The
Antigravity Credential Manager token received HTTP 401 from the current Cloud
Code endpoint; no token refresh, re-login, or reissue was attempted.
The diagnostic prints only normalized provider and quota values.

Unit coverage includes missing and malformed Codex/Claude credential shapes,
actual JSON fixtures for ChatGPT and Claude quota DTOs, malformed Antigravity
candidate records, separate Antigravity pool mapping, HTTP 401/403/429/5xx
classification, and per-provider last-good cache retention. Local language
server discovery accepts only a process in the current Windows session, owned
by the current Windows SID, at the installed Antigravity language-server path.

## Update detection

The overlay asks `GET https://api.github.com/repos/{owner}/{repo}/releases/latest`
unauthenticated, 15 seconds after start and every six hours, and only when a
repository is configured in `update::GITHUB_REPO`. It reads `tag_name` and
`html_url`, nothing else. A tag that does not parse as a dotted version never
reports an update, a pre-release never outranks its final release, and a failed
check keeps the previous result rather than reporting "up to date". The stored
link is discarded unless it sits under `https://github.com/{owner}/{repo}/`, and
that origin is checked again before the link is handed to the shell. The app
never downloads or installs a release.

## Reference handling

CodeZeno/Claude-Code-Usage-Monitor and OpenUsage were consulted for endpoint
and response-shape compatibility. Both checked-out references are MIT
licensed and are acknowledged in `THIRD_PARTY_NOTICES`. The checked-out
LimitDock reference contains no license file; it informed no copied source
code and is not included as a third-party component.
