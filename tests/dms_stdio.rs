mod common;

use common::{McpClient, SCRIPT_LOCK, path_with, tool_body, write_script};
use serde_json::json;

const BIN: &str = env!("CARGO_BIN_EXE_dms-mcp");

/// A fake dms that echoes all its arguments back as JSON, and exits
/// non-zero with an error payload on stdout when first arg is `__fail__`.
const FAKE_DMS: &str = r#"#!/bin/sh
for a in "$@"; do
    if [ "$a" = "__fail__" ]; then
        printf 'something went wrong\n'
        exit 2
    fi
done
printf '{"argv":"%s"}\n' "$*"
"#;

fn spawn(env: &[(&str, &str)]) -> McpClient {
    McpClient::spawn(BIN, env)
}

#[test]
fn initialize_reports_server_info() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let client = spawn(&[("PATH", &path_with(dir.path()))]);

    let result = &client.init_response["result"];
    assert_eq!(result["serverInfo"]["name"], json!("dms-mcp"));
    assert_eq!(result["serverInfo"]["version"], json!("0.0.2"));
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["instructions"].is_string());
}

#[test]
fn tools_list_returns_dms_tools() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.request("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"dms_help"));
    assert!(names.contains(&"dms_ipc_call"));
    assert_eq!(tools.len(), 2);

    let call_tool = tools.iter().find(|t| t["name"] == "dms_ipc_call").unwrap();
    let required = call_tool["inputSchema"]["required"].as_array().unwrap();
    assert!(required.contains(&json!("target")));
    assert!(required.contains(&json!("function")));
}

#[test]
fn dms_ipc_call_passes_target_and_function() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool(
        "dms_ipc_call",
        json!({"target": "notifications", "function": "toggle"}),
    );
    assert_eq!(resp["result"]["isError"], json!(false));
    let body = tool_body(&resp);
    assert_eq!(body["success"], json!(true));
    assert_eq!(
        body["data"]["argv"],
        json!("ipc call -- notifications toggle")
    );
}

#[test]
fn dms_ipc_call_forwards_args() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool(
        "dms_ipc_call",
        json!({"target": "audio", "function": "setvolume", "args": ["50"]}),
    );
    let body = tool_body(&resp);
    assert_eq!(
        body["data"]["argv"],
        json!("ipc call -- audio setvolume 50")
    );
}

#[test]
fn dms_ipc_call_requires_target_and_function() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("dms_ipc_call", json!({"target": "audio"}));
    let failed = resp["result"]["isError"] == json!(true) || resp.get("error").is_some();
    assert!(failed, "expected error, got: {resp}");
}

#[test]
fn dms_help_runs_dms_ipc() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("dms_help", json!({}));
    let body = tool_body(&resp);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["data"]["argv"], json!("ipc --help"));
}

#[test]
fn dms_failure_is_error_result() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool(
        "dms_ipc_call",
        json!({"target": "__fail__", "function": "toggle"}),
    );
    assert_eq!(resp["result"]["isError"], json!(true));
    let body = tool_body(&resp);
    assert_eq!(body["success"], json!(false));
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("something went wrong")
    );
}

#[test]
#[ignore = "requires a running DMS session matching the CLI; read-only"]
fn live_dms_queries() {
    let runtime =
        std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR must select the test session");
    let mut client = spawn(&[("XDG_RUNTIME_DIR", &runtime)]);
    let resp = client.call_tool("dms_help", json!({}));
    assert_eq!(resp["result"]["isError"], false, "{resp}");
    assert!(
        tool_body(&resp)["data"]
            .as_str()
            .unwrap()
            .contains("Targets:")
    );
    for (target, function) in [
        ("audio", "status"),
        ("notifications", "getDoNotDisturb"),
        ("theme", "getMode"),
    ] {
        let resp = client.call_tool(
            "dms_ipc_call",
            json!({"target": target, "function": function}),
        );
        assert_eq!(resp["result"]["isError"], false, "{target}: {resp}");
        assert_eq!(tool_body(&resp)["success"], true);
    }
}

#[test]
fn unknown_tool_returns_error() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "dms", FAKE_DMS);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("dms_does_not_exist", json!({}));
    let failed = resp["result"]["isError"] == json!(true) || resp.get("error").is_some();
    assert!(failed, "expected error, got: {resp}");
}
