use super::*;
use crate::process::{invoke_with_limits, Limits};

fn python(script: &str) -> WorkerCommand {
    WorkerCommand::new("python3", vec!["-c".into(), script.into()])
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
        &python("import sys; sys.stdout.buffer.write(b'x'*65536); sys.stdout.flush()"),
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
        &python("import time; time.sleep(10)"),
        settings,
    )
    .unwrap_err();
    assert_eq!(error.code, "INDEPENDENT_TIMEOUT");
    assert_eq!(error.message, "rhwp exceeded the 30-second timeout.");
}

#[test]
fn drains_output_while_sending_large_input() {
    let script = "import sys; sys.stdout.write(' '*262144); sys.stdout.flush(); sys.stderr.write(' '*262144); sys.stderr.flush(); data=sys.stdin.buffer.read(); print('{\"bytes\":'+str(len(data))+'}')";
    let settings = Limits {
        timeout: std::time::Duration::from_secs(5),
        max_output: 1024 * 1024,
    };
    let payload = vec![0xff; 1024 * 1024];
    let result =
        invoke_with_limits(&payload, "probe", None, None, &python(script), settings).unwrap();
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
        &python("import sys; sys.stderr.write('x'*65536); sys.stderr.flush()"),
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
        &python("import os,time; os.close(0); os.close(1); os.close(2); time.sleep(10)"),
        settings,
    )
    .unwrap_err();
    assert_eq!(error.code, "INDEPENDENT_TIMEOUT");
}

#[test]
fn validates_response_and_exit_status() {
    for (script, code, message) in [
        (
            "print('not JSON')",
            "INDEPENDENT_PROTOCOL",
            "rhwp returned invalid JSON.",
        ),
        (
            "print('[]')",
            "INDEPENDENT_PROTOCOL",
            "rhwp returned a non-object response.",
        ),
        (
            "print('{\"error\":3}')",
            "INDEPENDENT_PROTOCOL",
            "Invalid rhwp error response.",
        ),
        (
            "import sys; print('{}'); sys.exit(2)",
            "INDEPENDENT_FAILED",
            "rhwp exited without a valid result.",
        ),
        ("print('{\"error\":{}}')", "RHWP_FAILED", "rhwp failed."),
        (
            "import sys; sys.stdout.buffer.write(b'\\xff')",
            "INDEPENDENT_PROTOCOL",
            "rhwp returned invalid JSON.",
        ),
    ] {
        let error =
            invoke_with_limits(b"x", "probe", None, None, &python(script), limits()).unwrap_err();
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
