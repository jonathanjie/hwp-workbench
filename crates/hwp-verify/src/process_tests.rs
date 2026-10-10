use super::*;
use crate::process::{invoke_with_limits, Limits};

fn node(script: &str) -> WorkerCommand {
    WorkerCommand::new("node", vec!["-e".into(), script.into()])
}

fn limits() -> Limits {
    Limits {
        timeout: std::time::Duration::from_secs(5),
        max_output: 1024,
    }
}

#[test]
fn test_output_cap_stops_child() {
    let error = invoke_with_limits(
        b"x",
        "probe",
        None,
        None,
        &node("process.stdout.write('x'.repeat(65536))"),
        limits(),
    )
    .unwrap_err();
    assert_eq!(error.code, "INDEPENDENT_LIMIT");
    assert_eq!(error.message, "rhwp output exceeds 64 MiB.");
}

#[test]
fn test_timeout_stops_child() {
    let mut settings = limits();
    settings.timeout = std::time::Duration::from_millis(50);
    let error = invoke_with_limits(
        b"x",
        "probe",
        None,
        None,
        &node("setTimeout(() => {}, 10000)"),
        settings,
    )
    .unwrap_err();
    assert_eq!(error.code, "INDEPENDENT_TIMEOUT");
    assert_eq!(error.message, "rhwp exceeded the 30-second timeout.");
}

#[test]
fn drains_output_while_sending_large_input() {
    let script = "process.stdout.write(' '.repeat(262144)); process.stderr.write(' '.repeat(262144)); let bytes=0; process.stdin.on('data', data => bytes+=data.length); process.stdin.on('end', () => console.log(JSON.stringify({bytes})))";
    let settings = Limits {
        timeout: std::time::Duration::from_secs(5),
        max_output: 1024 * 1024,
    };
    let payload = vec![0xff; 1024 * 1024];
    let result =
        invoke_with_limits(&payload, "probe", None, None, &node(script), settings).unwrap();
    assert_eq!(
        result["bytes"],
        payload.len() + 4 + b"{\"op\":\"probe\"}".len()
    );
}

#[test]
fn stderr_counts_towards_output_bound() {
    let error = invoke_with_limits(
        b"x",
        "probe",
        None,
        None,
        &node("process.stderr.write('x'.repeat(65536))"),
        limits(),
    )
    .unwrap_err();
    assert_eq!(error.code, "INDEPENDENT_LIMIT");
}

#[test]
fn timeout_covers_child_after_pipes_close() {
    let mut settings = limits();
    settings.timeout = std::time::Duration::from_millis(100);
    let error = invoke_with_limits(
        b"x",
        "probe",
        None,
        None,
        &node("const fs=require('node:fs'); [0,1,2].forEach(fd => fs.closeSync(fd)); setTimeout(() => {}, 10000)"),
        settings,
    )
    .unwrap_err();
    assert_eq!(error.code, "INDEPENDENT_TIMEOUT");
}

#[test]
fn validates_response_and_exit_status() {
    for (script, code, message) in [
        (
            "console.log('not JSON')",
            "INDEPENDENT_PROTOCOL",
            "rhwp returned invalid JSON.",
        ),
        (
            "console.log('[]')",
            "INDEPENDENT_PROTOCOL",
            "rhwp returned a non-object response.",
        ),
        (
            "console.log('{\"error\":3}')",
            "INDEPENDENT_PROTOCOL",
            "Invalid rhwp error response.",
        ),
        (
            "console.log('{}'); process.exitCode=2",
            "INDEPENDENT_FAILED",
            "rhwp exited without a valid result.",
        ),
        (
            "console.log('{\"error\":{}}')",
            "RHWP_FAILED",
            "rhwp failed.",
        ),
        (
            "process.stdout.write(Buffer.from([255]))",
            "INDEPENDENT_PROTOCOL",
            "rhwp returned invalid JSON.",
        ),
    ] {
        let error =
            invoke_with_limits(b"x", "probe", None, None, &node(script), limits()).unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.message, message);
    }
}

#[test]
fn missing_worker_is_failed_without_faking_unavailable_node() {
    let command = WorkerCommand::new("/absent/hwp-verify-worker", Vec::new());
    assert_eq!(
        invoke(b"x", "probe", None, None, &command)
            .unwrap_err()
            .code,
        "INDEPENDENT_FAILED"
    );
}

#[test]
fn input_cap_precedes_worker_launch() {
    let command = WorkerCommand::new("/absent/hwp-verify-worker", Vec::new());
    let payload = vec![0; MAX_DOCUMENT + 1];
    assert_eq!(probe(&payload, &command).unwrap_err().code, "LIMIT");
}
