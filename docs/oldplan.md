# CodexBar Linux/Wayland Port (Rust)

Fork of CodexBar rewritten in Rust for Linux/Wayland, targeting tiling WM users (niri, Sway, Hyprland).

## Stack

```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "process"] }
gtk4 = "0.9"
gtk4-layer-shell = "0.7"          # Wayland popups
notify-rust = "4"                  # Desktop notifications
reqwest = { version = "0.12", features = ["json", "cookies"] }
keyring = "3"                      # Credential storage (libsecret)
decrypt-cookies = "0.6"            # Chrome cookie decryption
rusqlite = "0.32"                  # Firefox cookies
portable-pty = "0.8"               # CLI PTY fallback
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## Architecture

No daemon needed. Waybar polls on interval, keeps cache fresh. Popup reads cache for instant display.

```
codexbar waybar          # Fetches live, writes cache, prints JSON
    ↓
~/.cache/codexbar/state.json
    ↓
codexbar popup           # Reads cache (instant), background refresh if stale
```

### CLI Commands

```bash
codexbar waybar              # Fetch, cache, print JSON (Waybar runs this on interval)
codexbar popup               # Show layer-shell popup (reads cache, refreshes in background)
codexbar status              # Print all provider status to terminal
codexbar fetch               # Force fetch and update cache (manual refresh)
```

### Waybar Config

```jsonc
{
    "custom/codexbar": {
        "exec": "codexbar waybar",
        "return-type": "json",
        "interval": 60,
        "on-click": "codexbar popup"
    }
}
```

JSON output: `{"text": "󰧑 72%", "tooltip": "Claude: 72% │ Codex: 85%", "class": ["warning"]}`

Shows lowest quota across enabled providers. Popup has tab switcher for all providers.

## Providers (Initial Scope)

### Claude

Strategies (in order):
1. **Credentials file** → `~/.claude/.credentials.json` → `GET api.anthropic.com/api/oauth/usage`
2. **Browser cookies** → claude.ai `sessionKey` cookie → Web API
3. **CLI PTY** → Run `claude`, send `/usage`, parse output

No OAuth flow - reads tokens created by `claude` CLI.

### Codex

Strategies (in order):
1. **Credentials file** → `~/.codex/auth.json` → OpenAI usage API
2. **Web dashboard** → platform.openai.com cookies → scrape HTML
3. **CLI PTY** → Run `codex`, send `/usage`, parse output

No OAuth flow - reads tokens created by `codex` CLI.

### OpenCode

Single strategy:
- **Browser cookies** → `opencode.ai` auth cookie → `opencode.ai/_server/...` API

Tracks OpenCode's own quota (5-hour rolling + weekly), separate from Claude/Codex.

## Browser Cookie Extraction

```
Firefox:  ~/.mozilla/firefox/<profile>/cookies.sqlite  (plain SQLite)
Chrome:   ~/.config/google-chrome/Default/Cookies      (encrypted, use decrypt-cookies)
```

## Notifications

Via `notify-rust` → D-Bus → mako/dunst/swaync:
- Session depleted: "Claude session depleted - 0% left"
- Session restored: "Claude session restored"

## Dev Workflow

```bash
cargo run -- popup --mock       # Run with mock data (no credentials needed)
cargo run -- popup              # Reads cache, refreshes in background
cargo run -- fetch              # Force live fetch to populate cache
cargo run -- waybar             # Test JSON output
```

Iterate on popup UI first with `--mock`. Once UI is solid, implement providers and test with real data via `fetch`.

## Phases

1. **Core** - Traits, config, cache, mock data, `cargo run -- popup --mock` works
2. **Claude** - All 3 strategies, `cargo run -- popup` shows real Claude data
3. **Waybar** - JSON output mode with caching
4. **Codex** - Reuse cookie/PTY infra from Claude
5. **OpenCode** - Cookie-based fetch (optional, lower priority)

## Reference: Original Swift Implementation

Base repo: https://github.com/steipete/CodexBar

Key files to reference:
- [UsageSnapshot.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/UsageSnapshot.swift) - Core data model
- [ProviderDescriptor.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/ProviderDescriptor.swift) - Provider trait pattern

Claude:
- [ClaudeOAuthCredentials.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/Claude/ClaudeOAuth/ClaudeOAuthCredentials.swift) - Credential file parsing
- [ClaudeOAuthUsageFetcher.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/Claude/ClaudeOAuth/ClaudeOAuthUsageFetcher.swift) - API calls
- [ClaudeStatusProbe.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/Claude/ClaudeStatusProbe.swift) - PTY/CLI parsing

Codex:
- [CodexOAuthCredentials.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/Codex/CodexOAuth/CodexOAuthCredentials.swift)
- [CodexStatusProbe.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/Codex/CodexStatusProbe.swift)

OpenCode:
- [OpenCodeUsageFetcher.swift](https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/OpenCode/OpenCodeUsageFetcher.swift)

## Config

```
~/.config/codexbar/config.toml
~/.cache/codexbar/state.json
~/.local/share/codexbar/logs/
```

```toml
[general]
refresh_interval = "5m"

[notifications]
enabled = true
on_depleted = true

[providers.claude]
enabled = true

[providers.codex]
enabled = true

[providers.opencode]
enabled = false
```

## Open Questions

1. **Name** - "quotabar" is clearer (CodexBar implies Codex-specific)
2. **License** - MIT (same as original)
