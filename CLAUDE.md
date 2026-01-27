# quotabar

GTK4/Layer-Shell popup for monitoring AI coding tool quotas on Wayland.

## Development

```bash
just mock         # Run popup with mock data
just watch-mock   # Auto-reload on changes (requires watchexec)
just popup        # Run popup with real data
just check        # Format, lint, and test
just install      # Install to ~/.cargo/bin
```

Run `just install` after code changes (not needed for docs).

## Architecture

- `src/main.rs` - CLI entry point (popup, waybar subcommands)
- `src/popup.rs` - GTK4 layer-shell popup UI
- `src/providers/` - Provider implementations (Claude, Codex, OpenCode)
- `assets/` - SVG icons (use `currentColor` for theming)

## Future Work

- Proper CSS theming support for icon colors
