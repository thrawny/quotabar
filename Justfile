# quotabar development commands
# Run `direnv allow` to auto-activate the dev shell

# Show available commands
default:
    @just --list

# Enter dev shell (flakes)
shell:
    nix develop

# Build the project
build:
    cargo build

# Build release
release:
    cargo build --release

# Run with mock data (primary dev workflow)
mock:
    cargo run -- popup --mock

# Run popup (reads cache)
popup:
    cargo run -- popup

# Run waybar output
waybar:
    cargo run -- waybar

# Force fetch and update cache
fetch:
    cargo run -- fetch

# Print status to terminal
status:
    cargo run -- status

# Run tests
test:
    cargo test

# Run clippy
lint:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting
check-fmt:
    cargo fmt -- --check

# Full check (format, lint, test)
check: check-fmt lint test

# Clean build artifacts
clean:
    cargo clean

# Watch and rebuild on changes (requires cargo-watch)
watch:
    cargo watch -x 'run -- popup --mock'

# Install locally
install:
    cargo install --path .
