# quotabar

Monitor API quota/usage for AI coding tools in Waybar.

A Linux port of [CodexBar](https://github.com/steipete/CodexBar) for Wayland compositors.

![Waybar modules](docs/waybar.png)

![Screenshot](docs/screenshot.png)

## Supported Providers

- Claude (Anthropic)
- Codex (OpenAI)

## Installation

```bash
cargo install --path .
```

## Usage

Add one module per provider to your Waybar config. The text shows the
session percentage with the weekly percentage dimmed next to it; the
`warning` (≥75%) and `critical` (≥90%) classes apply per provider.

```json
{
  "custom/quotabar-claude": {
    "exec": "quotabar waybar --provider claude",
    "return-type": "json",
    "interval": 60,
    "on-click": "quotabar popup"
  },
  "custom/quotabar-codex": {
    "exec": "quotabar waybar --provider codex",
    "return-type": "json",
    "interval": 60,
    "on-click": "quotabar popup"
  }
}
```

Provider logos are embedded in the binary and written to
`~/.local/share/quotabar/` on each run, so Waybar CSS can use them as
background images:

```css
#custom-quotabar-claude,
#custom-quotabar-codex {
  background-repeat: no-repeat;
  background-position: 8px center;
  background-size: 14px 14px;
  padding: 0 8px 0 28px;
}

#custom-quotabar-claude {
  background-image: url("/home/USER/.local/share/quotabar/claude.svg");
}

#custom-quotabar-codex {
  background-image: url("/home/USER/.local/share/quotabar/openai.svg");
}
```

Running `quotabar waybar` without `--provider` outputs a single combined
module with a generic icon, falling back across providers.

### Structured snapshots

`quotabar snapshot` prints one JSON object for both Claude and Codex. It uses the
same cache intervals and reset-credit notifications as Waybar. Failed refreshes
retain the last successful provider snapshot and report an error. Each provider
refresh has a 45-second deadline; providers refresh concurrently.

The version 1 contract has `schema_version`, `generated_at`, `cache_updated_at`
and a `providers` array in Claude/Codex order. Each provider includes identity,
usage URL, availability, stale/error state, last successful update, plain-text
summaries, quota windows, cost/budget and reset credits. Windows include the raw
rate-window fields plus backend-computed expiry, severity, reset text, weekly
pace text and expected usage. Credit records include their complete lifecycle
and current availability. Frontends should reject unsupported schema versions,
keep their last valid snapshot on process failure, and show stale/error state.

Poll every 30 seconds. The backend fetches every five minutes normally and every
30 seconds when usage is high. `quotabar snapshot --mock` emits fixtures without
reading credentials/cache, contacting providers, or sending notifications.

### Expiring Codex reset credits

When an available Codex reset credit is less than six hours from expiry,
quotabar sends a critical desktop notification once per hour. During the final
hour it repeats every 15 minutes. Notifications are deduplicated across
multiple Waybar modules and can be disabled in
`~/.config/quotabar/config.toml`:

```toml
[notifications]
enabled = false
```

## License

MIT - see [LICENSE](LICENSE) for details.

Inspired by [CodexBar](https://github.com/steipete/CodexBar) by Peter Steinberger.

Provider icons from [LobeHub Icons](https://github.com/lobehub/lobe-icons) (MIT License).
