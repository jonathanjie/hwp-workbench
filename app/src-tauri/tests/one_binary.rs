//! These process checks can only enter CLI mode or the hidden worker. No GUI runs.
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn run(mut command: Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if started.elapsed() > Duration::from_secs(30) {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "Headless process timed out: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn app() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_hwp-workbench-app"))
}
#[test]
fn app_binary_via_temporary_cli_symlink_answers_version_read_and_rpc() {
    let scratch = tempfile::tempdir().unwrap();
    let launcher = scratch.path().join("hwp-workbench");
    std::os::unix::fs::symlink(app(), &launcher).unwrap();
    fs::write(
        scratch.path().join("fixture.hwpx"),
        include_bytes!("../../../fixtures/generated/review-form.hwpx"),
    )
    .unwrap();
    let mut command = Command::new(&launcher);
    command.arg("--version").env("PATH", "");
    let output = run(command, b"");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        hwp_workbench::service::VERSION
    );
    let mut command = Command::new(&launcher);
    command
        .args(["read", "fixture.hwpx", "--format", "text", "--root"])
        .arg(scratch.path())
        .env("PATH", "");
    let output = run(command, b"");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("검토"));
    assert!(text.contains("sha256="));
    let mut command = Command::new(&launcher);
    command
        .args(["rpc", "--root"])
        .arg(scratch.path())
        .env("PATH", "");
    let output = run(command, br#"{"method":"capabilities","args":{}}"#);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        hwp_workbench::service::capabilities()
    );
    let mut command = Command::new(&launcher);
    command.arg("not-a-command");
    let output = run(command, b"");
    assert_eq!(output.status.code(), Some(2));
}
#[test]
fn built_app_binary_answers_one_worker_request_before_tauri_starts() {
    let mut command = Command::new(app());
    command.arg("__rhwp-worker").env("PATH", "");
    let request = serde_json::to_vec(&json!({"op":"probe"})).unwrap();
    let mut input = (request.len() as u32).to_le_bytes().to_vec();
    input.extend_from_slice(&request);
    input.extend_from_slice(include_bytes!(
        "../../../fixtures/generated/review-form.hwpx"
    ));
    let output = run(command, &input);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["version"], "0.8.7");
    assert!(reply["pageCount"].as_u64().unwrap() >= 1);
    assert!(!reply["fullText"].as_str().unwrap().is_empty());
}
