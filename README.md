# sway-mcp / dms-mcp

Rust [MCP](https://modelcontextprotocol.io) servers (stdio transport) that expose a
Wayland desktop as tools for AI agents — and, more generally, as a scriptable
desktop-control backend.

| Binary     | Wraps          | Tools                                                                                          |
| ---------- | -------------- | ---------------------------------------------------------------------------------------------- |
| `sway-mcp` | `swaymsg`      | `sway_help`, `sway_command`, `sway_get_tree`, `sway_get_workspaces`, `sway_get_outputs`, `sway_get_marks`, `sway_get_version`, `sway_get_inputs`, `sway_get_config` |
| `dms-mcp`  | `dms ipc call` | `dms_help`, `dms_ipc_call`                                                                     |

`swaymsg` / `dms` are resolved via `PATH` at spawn time, so the servers work
against whatever session is running.

## Requirements

- A running Sway session (for `sway-mcp`) and/or DankMaterialShell (for `dms-mcp`)
- A Rust toolchain — provided automatically by the Nix flake (see below)

## Build

```bash
cd rust
direnv allow        # load cargo/rustc/clippy/just from the flake
just build          # = cargo build        -> target/debug/{sway-mcp,dms-mcp}
just release        # = cargo build --release
just nix-build      # = nix build          -> result/bin/{sway-mcp,dms-mcp}
just install        # install both binaries to ~/.local/bin
```

`just` alone lists all recipes (`test`, `lint`, `format`, `check`, `run`,
`inspect`, `install`, `clean`).

## Install on NixOS

This repo is a flake — add it as an input to your system configuration:

```nix
{
  inputs.sway-mcp.url = "github:hanckmann/sway-dms-mcp";
  # or locally: "path:/home/patrick/Projects/sway-mcp/rust";
}
```

Then pick one of:

```nix
# 1. NixOS module (installs to /run/current-system/sw/bin)
imports = [ inputs.sway-mcp.nixosModules.default ];
programs.sway-mcp.enable = true;
programs.dms-mcp.enable = true;

# 2. home-manager module (installs to ~/.nix-profile/bin)
imports = [ inputs.sway-mcp.homeManagerModules.default ];
programs.sway-mcp.enable = true;
programs.dms-mcp.enable = true;

# 3. Plain packages
environment.systemPackages = [
  inputs.sway-mcp.packages.x86_64-linux.sway-mcp
  inputs.sway-mcp.packages.x86_64-linux.dms-mcp
  # or both at once: inputs.sway-mcp.packages.x86_64-linux.default
];

# 4. Overlay
nixpkgs.overlays = [ inputs.sway-mcp.overlays.default ];
environment.systemPackages = [ pkgs.sway-mcp pkgs.dms-mcp ];
```

To try without a config change:

```bash
nix profile install github:hanckmann/sway-dms-mcp#sway-mcp
nix profile install github:hanckmann/sway-dms-mcp#dms-mcp
# or from a checkout: nix profile install .#sway-mcp
```

After installing, point your MCP client at the installed path, e.g.
`/run/current-system/sw/bin/sway-mcp` (NixOS module/packages) or
`~/.nix-profile/bin/sway-mcp` (home-manager / `nix profile`).

## Verify

```bash
just check          # cargo fmt --check + clippy --all-targets + cargo test
```

The servers log to stderr; set `RUST_LOG=debug` for verbose output. To poke a
server interactively, use the MCP inspector:

```bash
just inspect sway-mcp
```

Or send JSON-RPC directly — each request is one line on stdin:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"cli","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sway_get_version","arguments":{}}}' \
  | ./target/debug/sway-mcp
```

## Integrating with clients

### Devin Desktop / Devin CLI

Via command line (writes to `~/.config/devin/mcp_config.json` by default with `-s user`, or `.devin/mcp_config.local.json` without a flag):

```bash
devin mcp add -s user sway -- /path/to/rust/target/release/sway-mcp
devin mcp add -s user dms  -- /path/to/rust/target/release/dms-mcp
devin mcp list
```

Or by config file — `~/.config/devin/mcp_config.json` (user scope) or
`.devin/mcp_config.json` (project scope):

```json
{
  "mcpServers": {
    "sway": { "command": "/path/to/rust/target/release/sway-mcp" },
    "dms":  { "command": "/path/to/rust/target/release/dms-mcp" }
  }
}
```

To auto-approve the tools instead of prompting each call, add to your
permissions config:

```json
{ "permissions": { "allow": ["mcp__sway__*", "mcp__dms__*"] } }
```

### OpenCode

In `opencode.json` (project root) or `~/.config/opencode/opencode.json` (global):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "sway": {
      "type": "local",
      "command": ["/path/to/rust/target/release/sway-mcp"],
      "enabled": true
    },
    "dms": {
      "type": "local",
      "command": ["/path/to/rust/target/release/dms-mcp"],
      "enabled": true
    }
  }
}
```

### Claude Code

In `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "sway": { "command": "/path/to/rust/target/release/sway-mcp" },
    "dms":  { "command": "/path/to/rust/target/release/dms-mcp" }
  }
}
```

## Using it from a script (e.g. speech-to-text backend)

The servers speak plain JSON-RPC 2.0 over stdio, so any language can drive the
desktop through them — see [SCRIPTING.md](SCRIPTING.md) for a minimal client and
voice-command examples.
