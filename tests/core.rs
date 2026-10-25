use std::ffi::OsString;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use sway_mcp::{
    DmsIpcArgs, DmsMcp, SwayCommandArgs, SwayMcp, SwayMessageType, discover_sway_socket_inner, help,
};
use tokio::sync::Mutex;

/// Serializes tests that write-then-exec a script: while one thread holds a
/// script open for writing, another thread's fork() would inherit the write
/// fd and trigger ETXTBSY when the first script is exec'd.
static SWAYMSG_LOCK: Mutex<()> = Mutex::const_new(());

/// Write an executable shell script into `dir` and return its path.
fn fake_swaymsg(dir: &Path, script: &str) -> PathBuf {
    let path = dir.join("swaymsg");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    let mut perms = f.metadata().unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

/// Serialize the result to its wire JSON and parse the JSON body embedded
/// in the first text content block.
fn result_json(result: &CallToolResult) -> serde_json::Value {
    let wire = serde_json::to_value(result).unwrap();
    let text = wire["content"][0]["text"]
        .as_str()
        .expect("missing text content");
    serde_json::from_str(text).expect("result text is not JSON")
}

fn is_error(result: &CallToolResult) -> bool {
    serde_json::to_value(result).unwrap()["isError"] == serde_json::json!(true)
}

const ECHO_SCRIPT: &str = r#"#!/bin/sh
# echo the parsed -t flag, remaining args, and SWAYSOCK back as JSON
msg_type=""
while [ $# -gt 0 ]; do
    case "$1" in
        -t) msg_type="$2"; shift 2 ;;
        --) shift; break ;;
        *) break ;;
    esac
done
printf '{"type":"%s","command":"%s","swaysock":"%s"}\n' "$msg_type" "$*" "$SWAYSOCK"
"#;

#[test]
fn message_type_as_str_covers_all_variants() {
    let cases = [
        (SwayMessageType::Command, "command"),
        (SwayMessageType::GetTree, "get_tree"),
        (SwayMessageType::GetWorkspaces, "get_workspaces"),
        (SwayMessageType::GetOutputs, "get_outputs"),
        (SwayMessageType::GetMarks, "get_marks"),
        (SwayMessageType::GetVersion, "get_version"),
        (SwayMessageType::GetInputs, "get_inputs"),
        (SwayMessageType::GetConfig, "get_config"),
        (SwayMessageType::SendTick, "send_tick"),
        (SwayMessageType::Subscribe, "subscribe"),
    ];
    for (variant, expected) in cases {
        assert_eq!(variant.as_str(), expected);
    }
}

#[test]
fn message_type_deserializes_from_snake_case() {
    let args: SwayCommandArgs =
        serde_json::from_str(r#"{"command":"workspace 2","message_type":"get_tree"}"#).unwrap();
    assert_eq!(args.command, "workspace 2");
    assert_eq!(args.message_type, Some(SwayMessageType::GetTree));
}

#[test]
fn message_type_defaults_to_none() {
    let args: SwayCommandArgs = serde_json::from_str(r#"{"command":"kill"}"#).unwrap();
    assert_eq!(args.message_type, None);
}

#[test]
fn message_type_rejects_unknown_values() {
    let result: Result<SwayCommandArgs, _> =
        serde_json::from_str(r#"{"command":"kill","message_type":"bogus"}"#);
    assert!(result.is_err());
}

#[test]
fn command_args_require_command() {
    let result: Result<SwayCommandArgs, _> = serde_json::from_str(r#"{"message_type":"get_tree"}"#);
    assert!(result.is_err());
}

#[test]
fn socket_discovery_prefers_swaysock_env() {
    let result = discover_sway_socket_inner(
        Some(OsString::from("/tmp/custom.sock")),
        Some(PathBuf::from("/nonexistent")),
    );
    assert_eq!(result, Some(PathBuf::from("/tmp/custom.sock")));
}

#[test]
fn socket_discovery_finds_ipc_socket_in_runtime_dir() {
    let dir = tempfile::tempdir().unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(dir.path()).unwrap());
    let path = dir.path().join(format!("sway-ipc.{uid}.5678.sock"));
    let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

    let result = discover_sway_socket_inner(None, Some(dir.path().to_path_buf()));
    assert_eq!(result, Some(path));
}

#[test]
fn socket_discovery_ignores_non_matching_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("wayland-0"), "").unwrap();
    std::fs::write(dir.path().join("swaylock.sock"), "").unwrap();

    let result = discover_sway_socket_inner(None, Some(dir.path().to_path_buf()));
    assert_eq!(result, None);
}

#[test]
fn socket_discovery_ignores_regular_files_and_stale_sockets() {
    let dir = tempfile::tempdir().unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(dir.path()).unwrap());
    std::fs::write(dir.path().join(format!("sway-ipc.{uid}.1.sock")), "").unwrap();
    let path = dir.path().join(format!("sway-ipc.{uid}.2.sock"));
    drop(std::os::unix::net::UnixListener::bind(path).unwrap());
    assert_eq!(
        discover_sway_socket_inner(None, Some(dir.path().into())),
        None
    );
}

#[test]
fn socket_discovery_does_not_choose_between_live_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let uid = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(dir.path()).unwrap());
    let _first =
        std::os::unix::net::UnixListener::bind(dir.path().join(format!("sway-ipc.{uid}.1.sock")))
            .unwrap();
    let _second =
        std::os::unix::net::UnixListener::bind(dir.path().join(format!("sway-ipc.{uid}.2.sock")))
            .unwrap();
    assert_eq!(
        discover_sway_socket_inner(None, Some(dir.path().into())),
        None
    );
}

#[tokio::test]
async fn sway_payload_is_separated_from_options() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(
        dir.path(),
        "#!/bin/sh\n[ \"$1\" = '-t' ] && [ \"$2\" = 'send_tick' ] && [ \"$3\" = '--' ] && [ \"$4\" = '--help' ] || exit 9\nprintf 'true'\n",
    );
    let result = SwayMcp::with_swaymsg(bin, None)
        .execute_swaymsg("--help", SwayMessageType::SendTick)
        .await
        .unwrap();
    assert!(!is_error(&result), "{result:?}");
}

#[test]
fn socket_discovery_handles_missing_dir() {
    let result =
        discover_sway_socket_inner(None, Some(PathBuf::from("/definitely/not/a/real/dir")));
    assert_eq!(result, None);
}

#[test]
fn server_logging_respects_rust_log_off() {
    for bin in [
        env!("CARGO_BIN_EXE_sway-mcp"),
        env!("CARGO_BIN_EXE_dms-mcp"),
    ] {
        let mut child = std::process::Command::new(bin)
            .env("RUST_LOG", "off")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        writeln!(
            stdin,
            "{}",
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                    "clientInfo": {"name": "log-test", "version": "0"}}
            })
        )
        .unwrap();
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            output.stderr.is_empty(),
            "{bin}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn help_has_expected_structure() {
    let h = help();
    assert!(h["overview"].is_string());
    for category in [
        "workspaces",
        "windows",
        "layout",
        "fullscreen",
        "gaps",
        "marks",
        "outputs",
        "exec",
        "other",
    ] {
        assert!(
            h["categories"][category]["commands"].is_array(),
            "missing category {category}"
        );
    }
    assert_eq!(h["queries"]["available"].as_array().unwrap().len(), 7);
    assert!(h["examples"].is_object());
}

#[tokio::test]
async fn execute_swaymsg_passes_type_and_command() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), ECHO_SCRIPT);
    let server = SwayMcp::with_swaymsg(bin, None);

    let result = server
        .execute_swaymsg("workspace 2", SwayMessageType::Command)
        .await
        .unwrap();
    let body = result_json(&result);
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["type"], "command");
    assert_eq!(body["data"]["command"], "workspace 2");
    assert!(!is_error(&result));
}

#[tokio::test]
async fn execute_swaymsg_omits_empty_command() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), ECHO_SCRIPT);
    let server = SwayMcp::with_swaymsg(bin, None);

    let result = server
        .execute_swaymsg("", SwayMessageType::GetVersion)
        .await
        .unwrap();
    let body = result_json(&result);
    assert_eq!(body["data"]["type"], "get_version");
    assert_eq!(body["data"]["command"], "");
}

#[tokio::test]
async fn execute_swaymsg_passes_swaysock_env() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), ECHO_SCRIPT);
    let server = SwayMcp::with_swaymsg(
        bin,
        Some(PathBuf::from("/run/user/1000/sway-ipc.test.sock")),
    );

    let result = server
        .execute_swaymsg("", SwayMessageType::GetTree)
        .await
        .unwrap();
    let body = result_json(&result);
    assert_eq!(
        body["data"]["swaysock"],
        "/run/user/1000/sway-ipc.test.sock"
    );
}

#[tokio::test]
async fn execute_swaymsg_reports_nonzero_exit_with_stdout_payload() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(
        dir.path(),
        "#!/bin/sh\nprintf '[{\"success\":false,\"error\":\"bad command\"}]\\n'\nexit 2\n",
    );
    let server = SwayMcp::with_swaymsg(bin, None);

    let result = server
        .execute_swaymsg("bogus", SwayMessageType::Command)
        .await
        .unwrap();
    assert!(is_error(&result));
    let body = result_json(&result);
    assert_eq!(body["success"], false);
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("exit status: 2"), "got: {error}");
    assert!(error.contains("bad command"), "got: {error}");
}

#[tokio::test]
async fn execute_swaymsg_reports_stderr_when_present() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(
        dir.path(),
        "#!/bin/sh\necho 'something broke' >&2\nexit 1\n",
    );
    let server = SwayMcp::with_swaymsg(bin, None);

    let result = server
        .execute_swaymsg("x", SwayMessageType::Command)
        .await
        .unwrap();
    let body = result_json(&result);
    assert!(body["error"].as_str().unwrap().contains("something broke"));
}

#[tokio::test]
async fn execute_swaymsg_wraps_non_json_output_as_string() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), "#!/bin/sh\necho 'plain text output'\n");
    let server = SwayMcp::with_swaymsg(bin, None);

    let result = server
        .execute_swaymsg("", SwayMessageType::GetConfig)
        .await
        .unwrap();
    let body = result_json(&result);
    assert_eq!(body["success"], true);
    assert_eq!(body["data"], "plain text output");
}

#[tokio::test]
async fn execute_swaymsg_handles_missing_binary() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let server = SwayMcp::with_swaymsg(PathBuf::from("/nonexistent/swaymsg"), None);

    let result = server
        .execute_swaymsg("x", SwayMessageType::Command)
        .await
        .unwrap();
    assert!(is_error(&result));
    let body = result_json(&result);
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("failed to run"), "got: {error}");
    assert!(error.contains("nonexistent"), "got: {error}");
}

const ARGV_SCRIPT: &str = r#"#!/bin/sh
printf '{"argv":"%s"}\n' "$*"
"#;

#[test]
fn dms_ipc_args_deserialize() {
    let args: DmsIpcArgs =
        serde_json::from_str(r#"{"target":"audio","function":"setvolume","args":["50"]}"#).unwrap();
    assert_eq!(args.target, "audio");
    assert_eq!(args.function, "setvolume");
    assert_eq!(args.args, Some(vec!["50".to_string()]));
}

#[test]
fn dms_ipc_args_args_optional() {
    let args: DmsIpcArgs =
        serde_json::from_str(r#"{"target":"notifications","function":"toggle"}"#).unwrap();
    assert_eq!(args.args, None);
}

#[test]
fn dms_ipc_args_require_target_and_function() {
    assert!(serde_json::from_str::<DmsIpcArgs>(r#"{"target":"audio"}"#).is_err());
    assert!(serde_json::from_str::<DmsIpcArgs>(r#"{"function":"toggle"}"#).is_err());
}

#[tokio::test]
async fn dms_ipc_call_builds_argv() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), ARGV_SCRIPT);
    let server = DmsMcp::with_dms(bin);

    let result = server
        .dms_ipc_call(Parameters(DmsIpcArgs {
            target: "audio".to_string(),
            function: "setvolume".to_string(),
            args: Some(vec!["50".to_string()]),
        }))
        .await
        .unwrap();
    let body = result_json(&result);
    assert_eq!(body["data"]["argv"], "ipc call -- audio setvolume 50");
}

#[tokio::test]
async fn dms_ipc_call_omits_optional_args() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), ARGV_SCRIPT);
    let server = DmsMcp::with_dms(bin);

    let result = server
        .dms_ipc_call(Parameters(DmsIpcArgs {
            target: "notifications".to_string(),
            function: "toggle".to_string(),
            args: None,
        }))
        .await
        .unwrap();
    let body = result_json(&result);
    assert_eq!(body["data"]["argv"], "ipc call -- notifications toggle");
}

#[tokio::test]
async fn dms_help_runs_dms_ipc() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), ARGV_SCRIPT);
    let server = DmsMcp::with_dms(bin);

    let result = server.dms_help().await.unwrap();
    let body = result_json(&result);
    assert_eq!(body["data"]["argv"], "ipc --help");
}

#[tokio::test]
async fn execute_dms_wraps_non_json_output_as_string() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(dir.path(), "#!/bin/sh\necho 'Idle inhibit is disabled'\n");
    let server = DmsMcp::with_dms(bin);

    let result = server
        .execute_dms(&[
            "ipc".to_string(),
            "call".to_string(),
            "inhibit".to_string(),
            "status".to_string(),
        ])
        .await
        .unwrap();
    let body = result_json(&result);
    assert_eq!(body["success"], true);
    assert_eq!(body["data"], "Idle inhibit is disabled");
}

#[tokio::test]
async fn dms_help_reports_discovery_failure_even_on_zero_exit() {
    let _guard = SWAYMSG_LOCK.lock().await;
    for output in [
        "Could not retrieve available IPC targets (is DMS running?)",
        "Could not retrieve IPC targets.",
        "No IPC targets available",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_swaymsg(dir.path(), &format!("#!/bin/sh\nprintf '%s' '{output}'\n"));
        let result = DmsMcp::with_dms(bin).dms_help().await.unwrap();
        assert!(is_error(&result), "{result:?}");
    }
}

#[tokio::test]
async fn dms_preserves_argument_boundaries_and_option_like_values() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_swaymsg(
        dir.path(),
        "#!/bin/sh\n[ \"$#\" = 7 ] && [ \"$1\" = ipc ] && [ \"$2\" = call ] && [ \"$3\" = -- ] && [ \"$4\" = settings ] && [ \"$5\" = get ] && [ \"$6\" = 'a b; $(false)' ] && [ \"$7\" = --help ] || exit 9\nprintf 'true'\n",
    );
    let result = DmsMcp::with_dms(bin)
        .dms_ipc_call(Parameters(DmsIpcArgs {
            target: "settings".into(),
            function: "get".into(),
            args: Some(vec!["a b; $(false)".into(), "--help".into()]),
        }))
        .await
        .unwrap();
    assert!(!is_error(&result), "{result:?}");
}

#[tokio::test]
async fn subprocess_timeout_is_an_error_and_kills_the_child() {
    let _guard = SWAYMSG_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let bin = fake_swaymsg(
        dir.path(),
        &format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
            pid_file.display()
        ),
    );
    let result = DmsMcp::with_dms(bin).execute_dms(&[]).await.unwrap();
    assert!(is_error(&result));
    assert!(
        result_json(&result)["error"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    let pid = std::fs::read_to_string(pid_file).unwrap();
    for _ in 0..100 {
        if !Path::new(&format!("/proc/{}", pid.trim())).exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out child was not reaped");
}
