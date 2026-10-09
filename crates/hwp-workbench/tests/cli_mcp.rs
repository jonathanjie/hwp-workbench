use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
fn run(args: &[&str], input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hwp-workbench"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn test_cli_rpc_and_parser_errors_include_recovery() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    for (args, input, code) in [
        (
            vec![
                "search",
                "missing.hwpx",
                "--root",
                root.path().to_str().unwrap(),
            ],
            "",
            "BAD_ARGUMENTS",
        ),
        (
            vec!["rpc", "--root", root.path().to_str().unwrap()],
            "{bad",
            "REQUEST_FAILED",
        ),
    ] {
        let output = run(&args, input);
        assert_eq!(output.status.code(), Some(2));
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], code);
        assert!(error["error"]["hint"].as_str().unwrap().len() > 1);
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn test_mcp_real_subprocess() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    let output = run(&["mcp", "--root", root.path().to_str().unwrap()], &input);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["result"]["tools"].as_array().unwrap().len(), 13);
}
fn workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/generated/review-form.hwpx");
    std::fs::copy(fixture, root.path().join("form.hwpx")).unwrap();
    root
}
fn messages(root: &std::path::Path, requests: Vec<Value>, initialize: bool) -> Vec<Value> {
    let mut sequence = Vec::new();
    if initialize {
        sequence.extend([json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),json!({"jsonrpc":"2.0","method":"notifications/initialized"})]);
    }
    sequence.extend(requests);
    let input = sequence
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut output = Vec::new();
    hwp_workbench::mcp::serve(root, std::io::Cursor::new(input), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn tool(root: &std::path::Path, name: &str, args: Value) -> Value {
    messages(root,vec![json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}})],true).pop().unwrap()
}
#[test]
fn test_initialize_tools() {
    let root = workspace();
    let replies = messages(
        root.path(),
        vec![json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})],
        true,
    );
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-11-25");
    let expected: Value = serde_json::from_str(include_str!("../src/mcp_tools.json")).unwrap();
    assert_eq!(replies[1]["result"]["tools"], expected);
}
#[test]
fn test_inspect() {
    let root = workspace();
    let reply = tool(root.path(), "hwp_inspect", json!({"path":"form.hwpx"}));
    assert_eq!(reply["result"]["structuredContent"]["format"], "hwpx");
}
#[test]
fn test_root_escape() {
    let root = workspace();
    let reply = tool(root.path(), "hwp_inspect", json!({"path":"/etc/passwd"}));
    assert_eq!(reply["result"]["isError"], true);
}
#[test]
fn test_apply() {
    let root = workspace();
    let document =
        hwp_workbench::service::invoke(root.path(), "inspect", &json!({"path":"form.hwpx"}))
            .unwrap();
    let node = &document["nodes"][0];
    let patch = json!({"schemaVersion":1,"baseSha256":document["sha256"],"operations":[{"op":"set_text","nodeId":node["id"],"expectedText":node["text"],"text":"Reviewed"}]});
    let reply = tool(
        root.path(),
        "hwp_apply",
        json!({"path":"form.hwpx","output":"new.hwpx","patch":patch,"acknowledgeRisks":true}),
    );
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert!(root.path().join("new.hwpx").is_file());
}
#[test]
fn test_requires_initialized() {
    let root = workspace();
    let reply = messages(
        root.path(),
        vec![json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})],
        false,
    );
    assert_eq!(reply[0]["error"]["code"], -32002);
}
#[test]
fn test_unknown_tool() {
    let root = workspace();
    let reply = tool(root.path(), "exec", json!({}));
    assert_eq!(reply["error"]["code"], -32602);
}
#[test]
fn test_wrong_argument_type() {
    let root = workspace();
    let reply = tool(root.path(), "hwp_inspect", json!([]));
    assert_eq!(reply["error"]["code"], -32602);
}
#[test]
fn test_unknown_method() {
    let root = workspace();
    let reply = messages(
        root.path(),
        vec![json!({"jsonrpc":"2.0","id":2,"method":"eval"})],
        true,
    );
    assert_eq!(reply[1]["error"]["code"], -32601);
}
#[test]
fn test_malformed_json() {
    let root = workspace();
    let mut output = Vec::new();
    hwp_workbench::mcp::serve(root.path(), std::io::Cursor::new("invalid\n"), &mut output).unwrap();
    let reply: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(reply["error"]["code"], -32700);
    assert_eq!(
        reply["error"]["message"],
        "Expecting value: line 1 column 1 (char 0)"
    );
}
#[test]
fn test_invalid_envelope() {
    let root = workspace();
    for invalid in [
        json!(["bad"]),
        json!({"jsonrpc":"2.0","method":"ping","id":true}),
        json!({"jsonrpc":"2.0","method":"ping","id":null}),
        json!({"jsonrpc":"2.0","method":"ping","id":1,"params":[]}),
    ] {
        let replies = messages(root.path(), vec![invalid], false);
        assert_eq!(replies[0]["error"]["code"], -32600);
    }
}
#[test]
fn test_ping() {
    let root = workspace();
    let reply = messages(
        root.path(),
        vec![json!({"jsonrpc":"2.0","id":8,"method":"ping"})],
        false,
    );
    assert_eq!(reply[0]["result"], json!({}));
}
#[test]
fn test_new_version_negotiates_supported_profile() {
    let root = workspace();
    let reply = messages(
        root.path(),
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"future"}}),
        ],
        false,
    );
    assert_eq!(reply[0]["result"]["protocolVersion"], "2025-11-25");
}
#[test]
fn test_notification_no_response() {
    let root = workspace();
    let reply = messages(
        root.path(),
        vec![json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":99}})],
        true,
    );
    assert_eq!(reply.len(), 1);
}
#[test]
fn test_cli_real_subprocess() {
    let root = workspace();
    let output = run(
        &[
            "inspect",
            "form.hwpx",
            "--root",
            root.path().to_str().unwrap(),
        ],
        "",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["format"], "hwpx");
    assert!(output.stderr.is_empty());
}
#[test]
fn test_new_tools_cli_mcp_service_parity() {
    let root = workspace();
    let inspect =
        hwp_workbench::service::invoke(root.path(), "inspect", &json!({"path":"form.hwpx"}))
            .unwrap();
    let id = inspect["nodes"][0]["id"].as_str().unwrap();
    for (method, args, flags) in [
        (
            "read",
            json!({"offset":0,"limit":1}),
            vec!["--offset", "0", "--limit", "1"],
        ),
        ("search", json!({"query":"검토"}), vec!["--query", "검토"]),
        (
            "replace",
            json!({"find":"검토","replaceWith":"확정"}),
            vec!["--find", "검토", "--replace-with", "확정"],
        ),
        (
            "fill",
            json!({"fields":{"기관명":"New organisation"}}),
            vec!["--field", "기관명=New organisation"],
        ),
        (
            "set",
            json!({"nodeId":id,"text":"Reviewed"}),
            vec!["--node-id", id, "--text", "Reviewed"],
        ),
        ("inspect", json!({"compact":true}), vec!["--compact"]),
    ] {
        let mut args = args;
        args.as_object_mut()
            .unwrap()
            .insert("path".to_owned(), json!("form.hwpx"));
        let expected = hwp_workbench::service::invoke(root.path(), method, &args).unwrap();
        let reply = tool(root.path(), &format!("hwp_{method}"), args);
        assert_eq!(reply["result"]["structuredContent"], expected);
        let mut arguments = vec![method, "form.hwpx", "--root", root.path().to_str().unwrap()];
        arguments.extend(flags);
        let output = run(&arguments, "");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            expected
        );
    }
}
#[test]
fn test_new_tool_schemas_reject_wrong_nested_types() {
    let root = workspace();
    for (name, args) in [
        ("hwp_read", json!({"limit":true})),
        ("hwp_search", json!({"query":"x","regex":"yes"})),
        (
            "hwp_replace",
            json!({"find":"x","replaceWith":"y","nodeIds":[4]}),
        ),
        ("hwp_fill", json!({"fields":{"기관명":4}})),
        ("hwp_set", json!({"nodeId":4,"text":"x"})),
    ] {
        let mut args = args;
        args.as_object_mut()
            .unwrap()
            .insert("path".to_owned(), json!("form.hwpx"));
        let reply = tool(root.path(), name, args);
        assert_eq!(reply["error"]["code"], -32602);
    }
}
#[test]
fn test_new_edit_tool_annotations_and_apply() {
    let root = workspace();
    let tools = hwp_workbench::mcp::tools();
    for tool in tools.as_array().unwrap().iter().filter(|tool| {
        matches!(
            tool["name"].as_str(),
            Some("hwp_replace" | "hwp_fill" | "hwp_set")
        )
    }) {
        assert_eq!(tool["annotations"]["readOnlyHint"], false);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
    }
    let reply = tool(
        root.path(),
        "hwp_replace",
        json!({"path":"form.hwpx","find":"검토","replaceWith":"확정","output":"replaced.hwpx","acknowledgeRisks":true}),
    );
    assert_eq!(
        reply["result"]["structuredContent"]["reopenedAndVerified"], true,
        "{reply}"
    );
}
#[test]
fn test_engine_and_protocol_errors_include_recovery() {
    let root = workspace();
    let reply = tool(root.path(), "hwp_read", json!({"path":"../outside.hwpx"}));
    let detail: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(detail["code"], "OUTSIDE_WORKSPACE");
    assert!(detail["hint"].is_string());
    assert!(detail["retryable"].is_boolean());
    assert_eq!(reply["result"]["structuredContent"]["error"], detail);
    let reply = tool(
        root.path(),
        "hwp_read",
        json!({"path":"form.hwpx","limit":"x"}),
    );
    assert_eq!(reply["error"]["code"], -32602);
    assert_eq!(reply["error"]["data"]["retryable"], true);
}
#[test]
fn message_limit_refuses_oversized_line() {
    let root = workspace();
    let mut output = Vec::new();
    hwp_workbench::mcp::serve(
        root.path(),
        std::io::Cursor::new(" ".repeat(hwp_workbench::mcp::MAX_MESSAGE + 1)),
        &mut output,
    )
    .unwrap();
    let reply: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(reply["error"]["code"], -32600);
}
#[test]
fn pipe_message_limit_does_not_wait_for_newline_or_eof() {
    for command in ["mcp", "rpc"] {
        let root = workspace();
        let mut child = Command::new(env!("CARGO_BIN_EXE_hwp-workbench"))
            .args([command, "--root", root.path().to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        input
            .write_all(&vec![b' '; hwp_workbench::mcp::MAX_MESSAGE + 1])
            .unwrap();
        let start = std::time::Instant::now();
        while child.try_wait().unwrap().is_none()
            && start.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let finished = child.try_wait().unwrap().is_some();
        if !finished {
            child.kill().unwrap();
        }
        let output = child.wait_with_output().unwrap();
        drop(input);
        assert!(
            finished,
            "{command} waited for EOF after the input exceeded the character limit"
        );
        let reply: Value = serde_json::from_slice(if command == "mcp" {
            &output.stdout
        } else {
            &output.stderr
        })
        .unwrap();
        if command == "mcp" {
            assert_eq!(reply["error"]["code"], -32600);
        } else {
            assert_eq!(reply["error"]["code"], "LIMIT");
        }
    }
}
#[test]
fn split_utf8_scalars_keep_the_character_limit() {
    let root = workspace();
    let request = json!({"jsonrpc":"2.0","id":"한😀","method":"ping"}).to_string() + "\n";
    let input = std::io::BufReader::with_capacity(1, std::io::Cursor::new(request));
    let mut output = Vec::new();
    hwp_workbench::mcp::serve(root.path(), input, &mut output).unwrap();
    let reply: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(reply["id"], "한😀");
    assert_eq!(reply["result"], json!({}));
    let oversized = "😀".repeat(hwp_workbench::mcp::MAX_MESSAGE + 1);
    let mut output = Vec::new();
    hwp_workbench::mcp::serve(root.path(), std::io::Cursor::new(oversized), &mut output).unwrap();
    let reply: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(reply["error"]["code"], -32600);
}
#[test]
fn internal_desktop_child_reports_validation_failure_without_gui() {
    let root = workspace();
    let output = run(
        &[
            "__desktop-source-open",
            "--root",
            root.path().to_str().unwrap(),
            "--path",
            "missing.hwpx",
        ],
        "",
    );
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["code"], "OPEN_FAILED");
    assert!(report["message"]
        .as_str()
        .unwrap()
        .starts_with("Could not launch HWP Workbench:"));
}
