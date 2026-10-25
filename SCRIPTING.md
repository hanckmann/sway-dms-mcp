# Scripting the desktop through the MCP servers

The `sway-mcp` and `dms-mcp` binaries speak MCP (JSON-RPC 2.0) over stdio —
newline-delimited JSON, no framing headers. That makes them usable as a
desktop-control backend from any language, e.g. behind a speech-to-text
pipeline that turns utterances into tool calls.

## Lifecycle

1. Spawn the binary once and keep it running — do **not** respawn per command.
2. Send `initialize`, then the `notifications/initialized` notification.
3. Send `tools/call` requests; match responses to requests by `id`.

Every tool result is `{content: [{type: "text", text: "<json>"}]}` where the
embedded JSON is `{"success": true, "data": ...}` or
`{"success": false, "error": "..."}`. `isError: true` marks failures.

## Minimal Python client

```python
import json
import subprocess


class McpServer:
    """One long-lived MCP stdio server process."""

    def __init__(self, binary: str):
        self.proc = subprocess.Popen(
            [binary],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,  # server logs go to stderr
            text=True,
            bufsize=1,
        )
        self._next_id = 0
        self._request("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "stt-backend", "version": "1.0"},
        })
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _send(self, msg: dict):
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def _request(self, method: str, params: dict) -> dict:
        self._next_id += 1
        self._send({
            "jsonrpc": "2.0",
            "id": self._next_id,
            "method": method,
            "params": params,
        })
        # read lines until the response with our id arrives
        while True:
            msg = json.loads(self.proc.stdout.readline())
            if msg.get("id") == self._next_id:
                if "error" in msg:
                    raise RuntimeError(msg["error"])
                return msg["result"]

    def call(self, tool: str, arguments: dict | None = None) -> dict:
        """Call a tool; returns the inner {success, data|error} body."""
        result = self._request("tools/call", {
            "name": tool,
            "arguments": arguments or {},
        })
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        return json.loads(result["content"][0]["text"])

    def close(self):
        self.proc.terminate()
```

## Voice-command backend sketch

```python
sway = McpServer("./target/release/sway-mcp")
dms = McpServer("./target/release/dms-mcp")


def handle_transcript(text: str):
    words = text.strip().lower()

    if words.startswith("workspace"):
        number = words.split()[-1]
        sway.call("sway_command", {"command": f"workspace {number}"})

    elif words == "mute microphone":
        dms.call("dms_ipc_call", {"target": "mic", "function": "mute"})

    elif words == "volume up":
        dms.call("dms_ipc_call", {"target": "audio", "function": "increment"})

    elif words == "toggle notifications":
        dms.call("dms_ipc_call", {"target": "notifications", "function": "toggle"})

    elif words == "night light":
        dms.call("dms_ipc_call", {"target": "night", "function": "toggle"})

    elif words == "close window":
        sway.call("sway_command", {"command": "kill"})

    elif words == "lock screen":
        dms.call("dms_ipc_call", {"target": "lock", "function": "lock"})
```

Discover what's callable at runtime: `dms.call("dms_help")` returns the full
DMS target/function table, and `sway.call("sway_help")` returns the Sway
command reference — useful for building or validating your intent map.

## One-liner from a shell script

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sway_command","arguments":{"command":"workspace 3"}}}' \
  | ./target/release/sway-mcp | tail -1
```

## Honest alternatives and caveats

- **Skip the MCP layer if you don't need it.** These servers are thin wrappers;
  a pure script can call `swaymsg -t command 'workspace 3'` or
  `dms ipc call audio increment` directly with less overhead. The MCP layer
  pays off when an LLM/agent is in the loop (schema-driven tool discovery) or
  when you want one uniform protocol for both Sway and DMS.
- **`sway_command` can run arbitrary commands** (`exec`, `kill`). If an
  always-listening STT pipeline feeds it, dispatch through a whitelist of
  intents like the sketch above rather than passing raw transcripts through.
- Responses arrive in request order on a single connection; if you ever send
  concurrent requests, match on `id` instead of assuming ordering.
