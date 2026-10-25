# sway-mcp / dms-mcp — MCP servers for Sway and DMS IPC

# List available recipes
default:
    @just --list

# Build both servers (debug)
build:
    cargo build

# Build both servers (release)
release:
    cargo build --release

install: release
    install -Dm755 target/release/sway-mcp "$HOME/.local/bin/sway-mcp"
    install -Dm755 target/release/dms-mcp "$HOME/.local/bin/dms-mcp"

# Run the full test suite
test:
    cargo test

# Run tests matching a filter, e.g. `just test-one dms`
test-one FILTER:
    cargo test {{ FILTER }}

# Format all code
format:
    cargo fmt

# Check formatting without modifying files
format-check:
    cargo fmt --check

# Lint everything with clippy
lint:
    cargo clippy --all-targets

# Full verification: format check + lint + tests
check: format-check lint test

# Run a server over stdio: `just run sway-mcp` or `just run dms-mcp`
run BIN:
    cargo run --bin {{ BIN }}

# Build via the nix flake into ./result
nix-build:
    nix build

# Debug a server with the MCP inspector, e.g. `just inspect sway-mcp`
inspect BIN:
    npx @modelcontextprotocol/inspector ./target/debug/{{ BIN }}

# Remove build artifacts
clean:
    cargo clean
