# Show available commands
default:
    @just --list

# Run with mock data
mock:
    cargo run -- popup --mock

# Run popup
popup:
    cargo run -- popup

# Fetch and update cache
fetch:
    cargo run -- waybar

# Install locally
install:
    cargo install --path .

# Format, lint, and test
check:
    cargo fmt
    cargo clippy --fix --allow-dirty -- -D warnings
    cargo test
