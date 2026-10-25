use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;

use serde_json::{Value, json};

/// Serializes tests that write-then-exec a fake binary: while one thread
/// holds a script open for writing, another thread's fork() would inherit
/// the write fd and trigger ETXTBSY when the first script is exec'd.
pub static SCRIPT_LOCK: Mutex<()> = Mutex::new(());

pub struct McpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: std::sync::mpsc::Receiver<String>,
    next_id: u64,
    pub init_response: Value,
}

impl McpClient {
    /// Spawn an MCP server binary and complete the initialize handshake.
    /// `bin` is usually `env!("CARGO_BIN_EXE_<name>")`.
    pub fn spawn(bin: &str, env: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(bin);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Isolate from the host environment so tests are deterministic.
        cmd.env_remove("SWAYSOCK");
        cmd.env_remove("XDG_RUNTIME_DIR");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("failed to spawn MCP server");
        let stdin = child.stdin.take().unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        let (sender, stdout) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 0,
            init_response: Value::Null,
        };
        client.initialize();
        client
    }

    fn send(&mut self, msg: &Value) {
        let mut line = serde_json::to_string(msg).unwrap();
        line.push('\n');
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&mut self, id: u64) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        loop {
            let line = self
                .stdout
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap_or_else(|error| panic!("no response for id {id}: {error}"));
            let msg: Value = serde_json::from_str(line.trim()).unwrap();
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
        }
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        self.recv(id)
    }

    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
    }

    fn initialize(&mut self) {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.0.0"},
            },
        }));
        self.init_response = self.recv(id);
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }));
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.stdin.take();
        for _ in 0..100 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Parsed JSON body inside a CallToolResult's first text block.
pub fn tool_body(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("missing text content");
    serde_json::from_str(text).expect("tool result is not JSON")
}

/// Write an executable script named `name` into `dir`.
pub fn write_script(dir: &Path, name: &str, script: &str) {
    let path = dir.join(name);
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
}

/// A PATH value with `dir` first (where fake binaries were written).
pub fn path_with(dir: &Path) -> String {
    format!("{}:{}", dir.display(), std::env::var("PATH").unwrap())
}
