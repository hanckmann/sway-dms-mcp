# Verification

- Run commands through `nix develop --command ...` when Cargo is not on PATH.
- `nix develop --command just check` runs formatting, Clippy, and the default tests. For strict linting use `cargo clippy --locked --all-targets -- -D warnings` inside the dev shell.
- The subprocess timeout regression intentionally takes 30 seconds.
- Default tests do not need a desktop. Read-only live checks: `cargo test --locked --test mcp_stdio --test dms_stdio live_ -- --ignored --nocapture` with the session's PATH, SWAYSOCK, and XDG_RUNTIME_DIR.
- Isolated integration: set SWAY_TEST_BIN and DMS_TEST_BIN to matching channel package binaries, then run `cargo test --locked --test headless -- --ignored --nocapture`. This launches temporary headless Sway and DMS sessions, needs dbus-run-session and kill on PATH, and uses temporary home/config/cache/data/runtime directories.
- Verified channel revisions: NixOS 26.05 `1f607aa79b6356ed45d3cd3c8079c196e79efa2c` provides Sway 1.12 and dms-shell 1.4.6; unstable `3ec636362e1816bc2f0888fa0fa2c69f1bc9e504` provides Sway 1.12 and dux-shell 2.0.0 (dms-shell is an alias). The package search service's stable alias/index lagged these channel sources; evaluate exact revisions when checking compatibility.
- DMS 1.4.6 needs `dms ipc --help`, not bare `dms ipc`, for reliable configuration resolution during target discovery.
- LLVM coverage: use cargo-llvm-cov with LLVM_COV and LLVM_PROFDATA pointing to LLVM 21 tools. The pinned unstable Rust 1.99 toolchain crashed in initializeValueProfRuntimeRecord at process exit; Rust 1.95 from the stable revision above worked. Supply cargo, rustc, and gcc from that revision via nix shell, and set CARGO_TARGET_DIR=target/stable-coverage. Invoke cargo-llvm-cov directly if Cargo incorrectly selects a broken ~/.cargo/bin shim. Use `cargo-llvm-cov llvm-cov --locked --show-missing-lines --fail-under-lines 90`.
- Stdio test clients close stdin and allow graceful server exit before force-killing, so subprocess coverage profiles are flushed.
- Preserve Cargo.lock. The servers intentionally resolve the session's swaymsg/dms through PATH rather than bundling a different compositor/shell version.
