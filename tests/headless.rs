#[allow(dead_code)]
mod common;

use std::fs::File;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::{McpClient, tool_body};
use serde_json::json;

struct Session(Child);

impl Drop for Session {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", "--", &format!("-{}", self.0.id())])
            .status();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires SWAY_TEST_BIN, DMS_TEST_BIN, dbus-run-session and a headless-capable Sway"]
fn isolated_desktop_stack() {
    let sway = std::env::var("SWAY_TEST_BIN").expect("set SWAY_TEST_BIN to the Sway binary");
    let dms = std::env::var("DMS_TEST_BIN").expect("set DMS_TEST_BIN to the DMS binary");
    let dir = tempfile::tempdir().unwrap();
    for name in ["home", "config", "cache", "data"] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    let mut env = vec![
        (
            "XDG_RUNTIME_DIR".to_string(),
            dir.path().display().to_string(),
        ),
        (
            "HOME".to_string(),
            dir.path().join("home").display().to_string(),
        ),
        (
            "XDG_CONFIG_HOME".to_string(),
            dir.path().join("config").display().to_string(),
        ),
        (
            "XDG_CACHE_HOME".to_string(),
            dir.path().join("cache").display().to_string(),
        ),
        (
            "XDG_DATA_HOME".to_string(),
            dir.path().join("data").display().to_string(),
        ),
        ("WLR_BACKENDS".to_string(), "headless".to_string()),
        ("WLR_RENDERER".to_string(), "pixman".to_string()),
        ("QT_QUICK_BACKEND".to_string(), "software".to_string()),
        ("DMS_DISABLE_HOT_RELOAD".to_string(), "1".to_string()),
        ("DBUS_SESSION_BUS_ADDRESS".to_string(), "".to_string()),
        ("DISPLAY".to_string(), "".to_string()),
        ("I3SOCK".to_string(), "".to_string()),
    ];
    let sway_log = dir.path().join("sway.log");
    let log = File::create(&sway_log).unwrap();
    let mut compositor = Session(
        Command::new(&sway)
            .args(["--unsupported-gpu", "-c", "/dev/null"])
            .envs(env.iter().map(|(k, v)| (k, v)))
            .env_remove("SWAYSOCK")
            .env_remove("I3SOCK")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let (socket, display) = loop {
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let socket = names
            .iter()
            .find(|n| n.starts_with("sway-ipc.") && n.ends_with(".sock"));
        let display = names
            .iter()
            .find(|n| n.starts_with("wayland-") && !n.ends_with(".lock"));
        if let (Some(socket), Some(display)) = (socket, display) {
            break (
                dir.path().join(socket).display().to_string(),
                display.clone(),
            );
        }
        assert!(
            compositor.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "Sway did not start: {}",
            std::fs::read_to_string(&sway_log).unwrap()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    env.push(("SWAYSOCK".to_string(), socket));
    env.push(("WAYLAND_DISPLAY".to_string(), display));
    let path = format!(
        "{}:{}:{}",
        std::path::Path::new(&sway).parent().unwrap().display(),
        std::path::Path::new(&dms).parent().unwrap().display(),
        std::env::var("PATH").unwrap()
    );
    env.push(("PATH".to_string(), path));
    let dms_log = dir.path().join("dms.log");
    let log = File::create(&dms_log).unwrap();
    let mut shell = Session(
        Command::new("dbus-run-session")
            .args(["--", &dms, "run"])
            .envs(env.iter().map(|(k, v)| (k, v)))
            .env_remove("DISPLAY")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .process_group(0)
            .spawn()
            .unwrap(),
    );
    let env: Vec<_> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut sway_client = McpClient::spawn(env!("CARGO_BIN_EXE_sway-mcp"), &env);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let response = sway_client.call_tool("sway_get_version", json!({}));
        if response["result"]["isError"] == false {
            break;
        }
        assert!(
            compositor.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "Sway IPC not ready: {response}\n{}",
            std::fs::read_to_string(&sway_log).unwrap()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    for query in [
        "get_tree",
        "get_workspaces",
        "get_outputs",
        "get_marks",
        "get_version",
        "get_inputs",
        "get_config",
    ] {
        let response = sway_client.call_tool(&format!("sway_{query}"), json!({}));
        assert_eq!(response["result"]["isError"], false, "{query}: {response}");
        assert!(
            tool_body(&response)["data"].is_object() || tool_body(&response)["data"].is_array()
        );
    }
    for command in [
        "workspace 2",
        "gaps inner current set 5",
        "gaps outer current set 10",
    ] {
        let response = sway_client.call_tool("sway_command", json!({"command": command}));
        assert_eq!(
            response["result"]["isError"], false,
            "{command}: {response}"
        );
    }
    let response = sway_client.call_tool(
        "sway_command",
        json!({"command": "--help", "message_type": "send_tick"}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let response = sway_client.call_tool("sway_command", json!({"command": "__invalid_command__"}));
    assert_eq!(response["result"]["isError"], true, "{response}");
    let mut dms_client = McpClient::spawn(env!("CARGO_BIN_EXE_dms-mcp"), &env);
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let response = dms_client.call_tool("dms_help", json!({}));
        if response["result"]["isError"] == false {
            assert!(
                tool_body(&response)["data"]
                    .as_str()
                    .unwrap()
                    .contains("Targets:")
            );
            break;
        }
        assert!(
            shell.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "DMS did not start: {response}\n{}",
            std::fs::read_to_string(&dms_log).unwrap()
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    for (target, function) in [("notifications", "getDoNotDisturb"), ("theme", "getMode")] {
        let response = dms_client.call_tool(
            "dms_ipc_call",
            json!({"target": target, "function": function}),
        );
        assert_eq!(response["result"]["isError"], false, "{target}: {response}");
        assert_eq!(tool_body(&response)["success"], true);
    }
}
