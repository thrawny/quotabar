# CodexBar Reference

Notes from studying [CodexBar](~/code/CodexBar), a macOS menu bar app for monitoring AI coding tool quotas.

## Claude Data Sources

CodexBar supports three independent data paths for Claude usage, tried in order:

### 1. OAuth API (preferred)

- **Endpoint:** `GET https://api.anthropic.com/api/oauth/usage`
- **Headers:**
  - `Authorization: Bearer <access_token>`
  - `anthropic-beta: oauth-2025-04-20`
  - `User-Agent: claude-code/<version>`
- **Credentials:** Keychain (`Claude Code-credentials`) or file fallback (`~/.claude/.credentials.json`)
- **Requires:** `user:profile` scope (tokens with only `user:inference` cannot call usage)
- **Response fields:**
  - `five_hour` -> session window
  - `seven_day` -> weekly window
  - `seven_day_sonnet` / `seven_day_opus` -> model-specific weekly window
  - `extra_usage` -> monthly spend/limit
- **Plan inference:** `rate_limit_tier` from credentials maps to Max/Pro/Team/Enterprise
- **Timeout:** 30 seconds

### 2. CLI PTY (fallback)

- Runs `claude` in a PTY session (`ClaudeCLISession` actor, persistent and reused between probes)
- Starts CLI with `--allowed-tools ""` (no tools)
- Auto-responds to first-run prompts (trust files, workspace, telemetry)
- Sends `/usage`, waits for rendered panel; sends Enter retries every 0.8s
- Optionally sends `/status` to extract identity fields
- Waits 2s after startup before sending commands (TUI needs time to initialize)
- **Stop conditions:** output contains "Current session", "Current week", or "Failed to load usage data"
- **Settle time:** 2s after stop condition (let the panel finish rendering)
- **Parsing (`ClaudeStatusProbe`):**
  - Strips ANSI via regex: `\u001B\[[0-?]*[ -/]*[@-~]`
  - Trims to last "Settings:" header to isolate usage panel from status bar
  - Label matching is normalized (lowercase + strip whitespace) for robustness
  - Extracts percent from lines near labels; fallback to ordered collection of all percentages
  - Parses `Account:` and `Org:` lines when present
  - Surfaces CLI errors (e.g. token expired) directly
- **Cleanup:** sends `/exit`, then SIGTERM, then SIGKILL with timeouts
- **Timeout:** 20 seconds (configurable)

### 3. Web API (browser cookies)

- Uses `sessionKey` cookie (prefix `sk-ant-`) extracted from browsers
- **Cookie sources (in order):**
  1. Safari: `~/Library/Cookies/Cookies.binarycookies`
  2. Chrome/Chromium: `~/Library/Application Support/Google/Chrome/*/Cookies`
  3. Firefox: `~/Library/Application Support/Firefox/Profiles/*/cookies.sqlite`
- On Linux, equivalent paths would be:
  - Chrome: `~/.config/google-chrome/*/Cookies` (SQLite, encrypted)
  - Firefox: `~/.mozilla/firefox/*/cookies.sqlite`
- **Domain:** `claude.ai`
- **Cached cookies:** Keychain cache with source label + timestamp, reused before re-importing
- **API calls** (all include `Cookie: sessionKey=<value>`):
  - `GET https://claude.ai/api/organizations` -> org UUID
  - `GET https://claude.ai/api/organizations/{orgId}/usage` -> session/weekly/opus percentages + reset times
  - `GET https://claude.ai/api/organizations/{orgId}/overage_spend_limit` -> extra usage spend/limit
  - `GET https://claude.ai/api/account` -> email + plan hints
- **Timeout:** 15 seconds
- **Extra usage fetch is best-effort:** failure doesn't fail the main fetch

## Fallback Strategy

- **Auto mode** tries sources in order: OAuth -> CLI -> Web
- If a source fails, automatically tries the next without user intervention
- `shouldFallback()` method determines when to move to next strategy
- User can force a specific source via Preferences

## Rate Limiting & Resilience

- **No retry with exponential backoff** for 429s — the fallback pipeline replaces retrying
- Detects `"rate_limit_error"` or `"rate limited"` strings in CLI output
- **Credential refresh with backoff gating:** temporary suppression after OAuth refresh failures to avoid retry storms
- **Keychain prompt policy:** prevents keychain prompt storms (Never / Only on user action / Always allow)

## Cost Usage (Local Log Scan)

Separate from the API — scans local Claude Code JSONL logs for token usage:

- **Source roots:** `$CLAUDE_CONFIG_DIR/projects` or `~/.config/claude/projects` / `~/.claude/projects`
- **Files:** `**/*.jsonl`
- **Parsing:** lines with `type: "assistant"` and `message.usage`, per-model token counts
- **Deduplication:** by `message.id + requestId` (usage is cumulative per streaming chunk)

## Key Files in CodexBar

| File | Purpose |
|------|---------|
| `ClaudeOAuthUsageFetcher.swift` | OAuth endpoint calls + response parsing |
| `ClaudeWebAPIFetcher.swift` | Browser cookie-based web API calls |
| `ClaudeProviderDescriptor.swift` | Strategy orchestration & fallback logic |
| `ClaudeOAuthCredentials.swift` | Token storage, refresh, caching |
| `ClaudeStatusProbe.swift` | CLI PTY parsing + rate limit detection |
| `ClaudeCLISession.swift` | PTY session management (persistent actor) |
| `CostUsageFetcher.swift` | Local JSONL log scanning for cost data |
| `TextParsing.swift` | ANSI stripping utility |
