mod common;

use common::{McpClient, SCRIPT_LOCK, path_with, tool_body, write_script};
use serde_json::json;

const BIN: &str = env!("CARGO_BIN_EXE_sway-mcp");

/// A fake swaymsg that echoes its parsed arguments back as JSON, fails on
/// the `__fail__` command, and prints non-JSON for `__nonjson__`.
const FAKE_SWAYMSG: &str = r#"#!/bin/sh
msg_type=""
while [ $# -gt 0 ]; do
    case "$1" in
        -t) msg_type="$2"; shift 2 ;;
        --) shift; break ;;
        *) break ;;
    esac
done
cmd="$*"
case "$cmd" in
    __fail__)
        printf '[{"success":false,"error":"simulated failure"}]\n'
        exit 2
        ;;
    __nonjson__)
        printf 'not json at all\n'
        exit 0
        ;;
esac
printf '{"type":"%s","command":"%s","swaysock":"%s"}\n' "$msg_type" "$cmd" "$SWAYSOCK"
"#;

fn spawn(env: &[(&str, &str)]) -> McpClient {
    McpClient::spawn(BIN, env)
}

#[test]
fn initialize_reports_server_info() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let client = spawn(&[("PATH", &path_with(dir.path()))]);

    let result = &client.init_response["result"];
    assert_eq!(result["serverInfo"]["name"], json!("sway-mcp"));
    assert_eq!(result["serverInfo"]["version"], json!("0.0.2"));
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["instructions"].is_string());
}

#[test]
fn tools_list_returns_all_nine_tools() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.request("tools/list", json!({}));
    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "sway_help",
        "sway_command",
        "sway_get_tree",
        "sway_get_workspaces",
        "sway_get_outputs",
        "sway_get_marks",
        "sway_get_version",
        "sway_get_inputs",
        "sway_get_config",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    assert_eq!(tools.len(), 9);

    // sway_command must require `command` and expose the message_type enum.
    let cmd_tool = tools.iter().find(|t| t["name"] == "sway_command").unwrap();
    let schema = &cmd_tool["inputSchema"];
    assert_eq!(schema["required"], json!(["command"]));
    let msg_enum = &schema["$defs"]["SwayMessageType"]["enum"];
    assert!(msg_enum.as_array().unwrap().contains(&json!("get_tree")));
}

#[test]
fn tool_call_executes_swaymsg_and_returns_data() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("sway_get_version", json!({}));
    assert_eq!(resp["result"]["isError"], json!(false));
    let body = tool_body(&resp);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["data"]["type"], json!("get_version"));
}

#[test]
fn sway_command_passes_command_and_type() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool(
        "sway_command",
        json!({"command": "workspace 3", "message_type": "command"}),
    );
    let body = tool_body(&resp);
    assert_eq!(body["data"]["command"], json!("workspace 3"));
    assert_eq!(body["data"]["type"], json!("command"));
}

#[test]
fn sway_command_defaults_message_type_to_command() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("sway_command", json!({"command": "kill"}));
    let body = tool_body(&resp);
    assert_eq!(body["data"]["type"], json!("command"));
}

#[test]
fn sway_command_rejects_invalid_message_type() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool(
        "sway_command",
        json!({"command": "kill", "message_type": "not_a_type"}),
    );
    // Schema validation fails — the SDK responds with an error result or error response.
    let failed = resp["result"]["isError"] == json!(true) || resp.get("error").is_some();
    assert!(failed, "expected error, got: {resp}");
}

#[test]
fn tool_call_swaymsg_failure_is_error_result() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("sway_command", json!({"command": "__fail__"}));
    assert_eq!(resp["result"]["isError"], json!(true));
    let body = tool_body(&resp);
    assert_eq!(body["success"], json!(false));
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("simulated failure")
    );
}

#[test]
fn tool_call_non_json_output_returned_as_string() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("sway_command", json!({"command": "__nonjson__"}));
    let body = tool_body(&resp);
    assert_eq!(body["success"], json!(true));
    assert_eq!(body["data"], json!("not json at all"));
}

#[test]
fn swaysock_env_is_forwarded_to_swaymsg() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[
        ("PATH", &path_with(dir.path())),
        ("SWAYSOCK", "/tmp/test-ipc.sock"),
    ]);

    let resp = client.call_tool("sway_get_marks", json!({}));
    let body = tool_body(&resp);
    assert_eq!(body["data"]["swaysock"], json!("/tmp/test-ipc.sock"));
}

#[test]
fn unknown_tool_returns_error() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("sway_does_not_exist", json!({}));
    let failed = resp["result"]["isError"] == json!(true) || resp.get("error").is_some();
    assert!(failed, "expected error, got: {resp}");
}

#[test]
fn sway_help_returns_documentation() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);

    let resp = client.call_tool("sway_help", json!({}));
    let body = tool_body(&resp);
    assert_eq!(body["success"], json!(true));
    assert!(body["data"]["categories"]["workspaces"].is_object());
}

#[test]
fn all_query_tools_use_the_correct_message_type() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    write_script(dir.path(), "swaymsg", FAKE_SWAYMSG);
    let mut client = spawn(&[("PATH", &path_with(dir.path()))]);
    for query in [
        "get_tree",
        "get_workspaces",
        "get_outputs",
        "get_marks",
        "get_version",
        "get_inputs",
        "get_config",
    ] {
        let resp = client.call_tool(&format!("sway_{query}"), json!({}));
        assert_eq!(resp["result"]["isError"], false, "{resp}");
        let body = tool_body(&resp);
        assert_eq!(body["data"]["type"], query);
        assert_eq!(body["data"]["command"], "");
    }
}

#[test]
#[ignore = "requires a running Sway session; read-only"]
fn live_sway_queries() {
    let socket = std::env::var("SWAYSOCK").expect("SWAYSOCK must select the test session");
    let mut client = spawn(&[("SWAYSOCK", &socket)]);
    for (query, array) in [
        ("get_tree", false),
        ("get_workspaces", true),
        ("get_outputs", true),
        ("get_marks", true),
        ("get_version", false),
        ("get_inputs", true),
        ("get_config", false),
    ] {
        let resp = client.call_tool(&format!("sway_{query}"), json!({}));
        assert_eq!(resp["result"]["isError"], false, "{query}: {resp}");
        let data = &tool_body(&resp)["data"];
        assert!(
            if array {
                data.is_array()
            } else {
                data.is_object()
            },
            "{query}: unexpected response shape"
        );
    }
}

#[test]
fn command_requires_a_string_payload() {
    let _guard = SCRIPT_LOCK.lock().unwrap();
    let mut client = spawn(&[]);
    for args in [json!({}), json!({"command": 123})] {
        let resp = client.call_tool("sway_command", args);
        assert!(
            resp["result"]["isError"] == true || resp.get("error").is_some(),
            "{resp}"
        );
    }
}
