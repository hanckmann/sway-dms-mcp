use std::ffi::OsString;
use std::path::PathBuf;

use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::wrapper::Parameters, model::*, tool,
    tool_handler, tool_router,
};
use serde_json::{Value, json};
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SwayMessageType {
    Command,
    GetTree,
    GetWorkspaces,
    GetOutputs,
    GetMarks,
    GetVersion,
    GetInputs,
    GetConfig,
    SendTick,
    Subscribe,
}

impl SwayMessageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::GetTree => "get_tree",
            Self::GetWorkspaces => "get_workspaces",
            Self::GetOutputs => "get_outputs",
            Self::GetMarks => "get_marks",
            Self::GetVersion => "get_version",
            Self::GetInputs => "get_inputs",
            Self::GetConfig => "get_config",
            Self::SendTick => "send_tick",
            Self::Subscribe => "subscribe",
        }
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SwayCommandArgs {
    /// The swaymsg command to execute (without 'swaymsg' prefix)
    pub command: String,
    /// The swaymsg message type (-t flag). Defaults to 'command'
    #[serde(default)]
    pub message_type: Option<SwayMessageType>,
}

#[derive(Debug, Clone)]
pub struct SwayMcp {
    sway_socket: Option<PathBuf>,
    swaymsg: PathBuf,
}

fn uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .unwrap_or(0)
}

pub fn discover_sway_socket() -> Option<PathBuf> {
    discover_sway_socket_inner(
        std::env::var_os("SWAYSOCK"),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
    )
}

pub fn discover_sway_socket_inner(
    sway_sock: Option<OsString>,
    xdg_runtime_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use std::os::unix::net::UnixStream;

    if let Some(sock) = sway_sock.filter(|sock| !sock.is_empty()) {
        return Some(PathBuf::from(sock));
    }

    let uid = uid();
    let runtime_dir = xdg_runtime_dir
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
    let prefix = format!("sway-ipc.{uid}.");
    let mut sockets = std::fs::read_dir(&runtime_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let metadata = entry.metadata().ok()?;
            (name.starts_with(&prefix)
                && name.ends_with(".sock")
                && metadata.file_type().is_socket()
                && metadata.uid() == uid
                && UnixStream::connect(entry.path()).is_ok())
            .then(|| entry.path())
        });
    let socket = sockets.next()?;
    if sockets.next().is_some() {
        return None;
    }
    Some(socket)
}

pub fn help() -> Value {
    json!({
        "overview": "Sway is a tiling window manager for Wayland. All commands are executed via sway_command with the exact syntax shown.",
        "categories": {
            "workspaces": {
                "description": "Switch and manage workspaces",
                "commands": [
                    "workspace <number>",
                    "workspace <name>",
                    "workspace next",
                    "workspace prev",
                    "workspace next_on_output",
                    "workspace prev_on_output",
                    "move container to workspace <number|name>",
                ],
            },
            "windows": {
                "description": "Manipulate windows and containers",
                "commands": [
                    "focus left|right|up|down",
                    "focus parent|child",
                    "focus next|prev",
                    "move left|right|up|down",
                    "move position <x> <y>",
                    "resize set width <px> height <px>",
                    "resize grow|shrink width|height <px>",
                    "kill",
                    "floating toggle",
                    "floating on|off",
                ],
            },
            "layout": {
                "description": "Change window layout and split direction",
                "commands": [
                    "layout splith",
                    "layout splitv",
                    "layout tabbed",
                    "layout stacking",
                    "layout toggle split|all",
                    "split horizontal|vertical|toggle",
                ],
            },
            "fullscreen": {
                "description": "Fullscreen modes",
                "commands": [
                    "fullscreen",
                    "fullscreen global",
                    "fullscreen toggle",
                ],
            },
            "gaps": {
                "description": "Adjust gaps between windows",
                "commands": [
                    "gaps inner current set <amount>",
                    "gaps outer current set <amount>",
                    "gaps top|right|bottom|left current set <amount>",
                    "gaps horizontal|vertical current set <amount>",
                ],
            },
            "marks": {
                "description": "Mark and navigate to windows",
                "commands": [
                    "mark <name>",
                    "unmark <name>",
                    "unmark",
                    "[con_mark=<name>] focus",
                ],
            },
            "outputs": {
                "description": "Configure monitors/displays",
                "commands": [
                    "output <name> position <x> <y>",
                    "output <name> resolution <w>x<h>",
                    "output <name> enable|disable",
                    "output <name> power on|off",
                ],
            },
            "exec": {
                "description": "Launch applications",
                "commands": [
                    "exec <command>",
                    "exec_always <command>",
                ],
            },
            "other": {
                "description": "Miscellaneous",
                "commands": [
                    "reload",
                    "exit",
                    "sticky on|off|toggle",
                    "swap container with id <id>",
                    "rename workspace <old> to <new>",
                    "mode <mode-name>",
                    "opacity <value>",
                ],
            },
        },
        "queries": {
            "description": "Get state information about Sway (separate from commands)",
            "available": [
                "get_tree - Complete window hierarchy",
                "get_workspaces - All workspaces with state",
                "get_outputs - All displays with resolution/position",
                "get_marks - All named bookmarks",
                "get_version - Sway version info",
                "get_inputs - Input devices (keyboards, mice, etc)",
                "get_config - Current configuration",
            ],
        },
        "examples": {
            "Switch to workspace": "workspace 2",
            "Move window to workspace": "move container to workspace 3",
            "Toggle floating": "floating toggle",
            "Resize window": "resize set width 800 height 600",
            "Change to tabbed layout": "layout tabbed",
            "Launch app": "exec firefox",
            "Get current state": "Use sway_get_tree tool",
            "Find windows by mark": "[con_mark=mymark] focus",
        },
    })
}

fn ok_result(data: Value) -> Result<CallToolResult, McpError> {
    let body = json!({ "success": true, "data": data });
    Ok(CallToolResult::success(vec![ContentBlock::text(
        body.to_string(),
    )]))
}

fn err_result(error: impl Into<String>) -> Result<CallToolResult, McpError> {
    let body = json!({ "success": false, "error": error.into() });
    Ok(CallToolResult::error(vec![ContentBlock::text(
        body.to_string(),
    )]))
}

/// Run `program args...`, optionally overriding env vars, and wrap the
/// outcome in the shared `{success, data|error}` tool-result shape:
/// stdout is parsed as JSON when possible, otherwise returned as a string.
pub async fn run_capture(
    program: &std::path::Path,
    args: &[String],
    envs: &[(&str, &PathBuf)],
) -> Result<CallToolResult, McpError> {
    let mut cmd = Command::new(program);
    cmd.args(args).kill_on_drop(true);
    for (key, value) in envs {
        cmd.env(key, value);
    }

    let output = match tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return err_result(format!("failed to run {}: {e}", program.display()));
        }
        Err(_) => return err_result(format!("{} timed out after 30 seconds", program.display())),
    };

    if !output.status.success() {
        // swaymsg/dms report errors on stdout; stderr is usually empty
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = stderr.trim();
        let detail = if detail.is_empty() {
            stdout.trim()
        } else {
            detail
        };
        return err_result(format!(
            "{} exited with {}: {detail}",
            program.display(),
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data = serde_json::from_str(&stdout).unwrap_or_else(|_| json!(stdout.trim()));
    ok_result(data)
}

#[tool_router]
impl SwayMcp {
    pub fn new() -> Self {
        Self {
            sway_socket: discover_sway_socket(),
            swaymsg: PathBuf::from("swaymsg"),
        }
    }

    /// Constructor with an explicit swaymsg binary path and socket. Used by tests.
    #[doc(hidden)]
    pub fn with_swaymsg(swaymsg: PathBuf, sway_socket: Option<PathBuf>) -> Self {
        Self {
            sway_socket,
            swaymsg,
        }
    }

    pub async fn execute_swaymsg(
        &self,
        command: &str,
        message_type: SwayMessageType,
    ) -> Result<CallToolResult, McpError> {
        let mut args = vec!["-t".to_string(), message_type.as_str().to_string()];
        if !command.is_empty() {
            args.push("--".to_string());
            args.push(command.to_string());
        }
        let envs: &[(&str, &PathBuf)] = match &self.sway_socket {
            Some(socket) => &[("SWAYSOCK", socket)],
            None => &[],
        };
        run_capture(&self.swaymsg, &args, envs).await
    }

    #[tool(
        description = "Get comprehensive documentation of all available Sway commands and their syntax. Use this once per session to understand what's available."
    )]
    async fn sway_help(&self) -> Result<CallToolResult, McpError> {
        ok_result(help())
    }

    #[tool(
        description = "Execute a raw swaymsg command. Pass the exact command syntax as you would to swaymsg, e.g. 'workspace 1', 'floating toggle', 'move position 100 100'"
    )]
    async fn sway_command(
        &self,
        Parameters(args): Parameters<SwayCommandArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg(
            &args.command,
            args.message_type.unwrap_or(SwayMessageType::Command),
        )
        .await
    }

    #[tool(
        description = "Get the complete window tree showing all containers, workspaces, and windows"
    )]
    async fn sway_get_tree(&self) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg("", SwayMessageType::GetTree).await
    }

    #[tool(
        description = "List all workspaces with their current state, focused status, and visible outputs"
    )]
    async fn sway_get_workspaces(&self) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg("", SwayMessageType::GetWorkspaces)
            .await
    }

    #[tool(description = "List all connected displays/outputs with resolution and position info")]
    async fn sway_get_outputs(&self) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg("", SwayMessageType::GetOutputs).await
    }

    #[tool(description = "Get all marks (named bookmarks) currently set in Sway")]
    async fn sway_get_marks(&self) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg("", SwayMessageType::GetMarks).await
    }

    #[tool(description = "Get Sway version information")]
    async fn sway_get_version(&self) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg("", SwayMessageType::GetVersion).await
    }

    #[tool(description = "List input devices")]
    async fn sway_get_inputs(&self) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg("", SwayMessageType::GetInputs).await
    }

    #[tool(description = "Get the current Sway configuration")]
    async fn sway_get_config(&self) -> Result<CallToolResult, McpError> {
        self.execute_swaymsg("", SwayMessageType::GetConfig).await
    }
}

impl Default for SwayMcp {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for SwayMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info({
                let mut info = Implementation::default();
                info.name = env!("CARGO_PKG_NAME").to_string();
                info.version = env!("CARGO_PKG_VERSION").to_string();
                info
            })
            .with_instructions(
                "MCP server for Sway window manager control. Call sway_help once per session for command documentation."
                    .to_string(),
            )
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DmsIpcArgs {
    /// The IPC target, e.g. 'notifications', 'audio', 'bar', 'spotlight'
    pub target: String,
    /// The function to call on the target, e.g. 'toggle', 'status', 'open'
    pub function: String,
    /// Optional positional arguments passed to the function
    #[serde(default)]
    pub args: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct DmsMcp {
    dms: PathBuf,
}

impl DmsMcp {
    pub async fn execute_dms(&self, args: &[String]) -> Result<CallToolResult, McpError> {
        run_capture(&self.dms, args, &[]).await
    }
}

#[tool_router]
impl DmsMcp {
    pub fn new() -> Self {
        Self {
            dms: PathBuf::from("dms"),
        }
    }

    /// Constructor with an explicit dms binary path. Used by tests.
    #[doc(hidden)]
    pub fn with_dms(dms: PathBuf) -> Self {
        Self { dms }
    }

    #[tool(
        description = "List all available DMS IPC targets and their functions. Use this once per session to discover what can be called with dms_ipc_call."
    )]
    pub async fn dms_help(&self) -> Result<CallToolResult, McpError> {
        let result = self
            .execute_dms(&["ipc".to_string(), "--help".to_string()])
            .await?;
        if result.is_error != Some(true)
            && result.content.iter().any(|content| {
                content.as_text().is_some_and(|text| {
                    text.text.contains("Could not retrieve")
                        || text.text.contains("No IPC targets available")
                })
            })
        {
            return err_result("DMS could not discover IPC targets; is the shell running?");
        }
        Ok(result)
    }

    #[tool(
        description = "Call a DMS IPC function, e.g. target='notifications' function='toggle', or target='audio' function='setvolume' args=['50']. Use dms_help to see all targets and functions."
    )]
    pub async fn dms_ipc_call(
        &self,
        Parameters(args): Parameters<DmsIpcArgs>,
    ) -> Result<CallToolResult, McpError> {
        let mut argv = vec![
            "ipc".to_string(),
            "call".to_string(),
            "--".to_string(),
            args.target,
            args.function,
        ];
        argv.extend(args.args.unwrap_or_default());
        self.execute_dms(&argv).await
    }
}

impl Default for DmsMcp {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for DmsMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info({
                let mut info = Implementation::default();
                info.name = "dms-mcp".to_string();
                info.version = env!("CARGO_PKG_VERSION").to_string();
                info
            })
            .with_instructions(
                "MCP server for DankMaterialShell IPC control. Call dms_help once per session to discover all targets and functions."
                    .to_string(),
            )
    }
}
