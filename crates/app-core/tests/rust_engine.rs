use app_core::{
    engine::{desktop_engine, ToolAction},
    Engine,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

const NAME: &str = "source.hwpx.20261009T120000Z-abcdef.json";
fn proposal() -> Value {
    let hash = "a".repeat(64);
    json!({"schemaVersion":1,"kind":"hwp-workbench-proposal","document":{"name":"source.hwpx","sha256":hash},"author":"Claude","title":"Review","createdAt":"2026-10-09T12:00:00Z","patch":{"schemaVersion":1,"baseSha256":hash,"operations":[{"op":"set_text","nodeId":"native-a","expectedText":"old","text":"new"},{"op":"set_text","nodeId":"native-b","expectedText":"old","text":"new"}]}})
}
fn engine() -> Arc<dyn Engine> {
    desktop_engine(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}
fn read(engine: &dyn Engine, root: &Path, hash: &str) -> Value {
    engine
        .agent_link(root, "source.hwpx", "read", json!({"sha256":hash}))
        .unwrap()
}
fn outcome(
    engine: &dyn Engine,
    root: &Path,
    file: &str,
    accepted: Value,
    declined: Value,
) -> app_core::Result<Value> {
    engine.agent_link(
        root,
        "source.hwpx",
        "outcome",
        json!({"file":file,"acceptedNodeIds":accepted,"declinedNodeIds":declined}),
    )
}
#[test]
fn bounded_reader_distinguishes_stale_and_unreadable_proposals_outcomes_are_exclusive_and_complete()
{
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let engine = engine();
    let folder = root.join(".hwp-workbench/proposals");
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join(NAME);
    fs::write(&file, serde_json::to_vec(&proposal()).unwrap()).unwrap();
    assert_eq!(
        read(engine.as_ref(), root, &"b".repeat(64))[0]["status"],
        "stale"
    );
    let rows = read(engine.as_ref(), root, &"a".repeat(64));
    assert_eq!(rows[0]["status"], "pending");
    assert_eq!(rows[0]["patch"], proposal()["patch"]);
    assert!(
        outcome(engine.as_ref(), root, NAME, json!(["native-a"]), json!([]))
            .unwrap_err()
            .message
            .contains("complete")
    );
    let result = outcome(
        engine.as_ref(),
        root,
        NAME,
        json!(["native-a"]),
        json!(["native-b"]),
    )
    .unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["acceptedNodeIds"], json!(["native-a"]));
    assert_eq!(result["declinedNodeIds"], json!(["native-b"]));
    assert_eq!(
        read(engine.as_ref(), root, &"a".repeat(64))[0]["status"],
        "partial"
    );
    assert!(outcome(
        engine.as_ref(),
        root,
        NAME,
        json!(["native-a", "native-b"]),
        json!([])
    )
    .is_err());
    fs::write(&file, vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        read(engine.as_ref(), root, &"a".repeat(64))[0]["status"],
        "unreadable"
    );
    fs::remove_file(&file).unwrap();
    let target = root.join("target.json");
    fs::write(&target, serde_json::to_vec(&proposal()).unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &file).unwrap();
    assert_eq!(
        read(engine.as_ref(), root, &"a".repeat(64))[0]["status"],
        "unreadable"
    );
    fs::remove_file(&file).unwrap();
    let mut excessive = proposal();
    excessive["patch"]["operations"] = json!((0..1001).map(|i| json!({"op":"set_text","nodeId":format!("node-{i}"),"expectedText":"old","text":"new"})).collect::<Vec<_>>());
    fs::write(&file, serde_json::to_vec(&excessive).unwrap()).unwrap();
    assert_eq!(
        read(engine.as_ref(), root, &"a".repeat(64))[0]["status"],
        "unreadable"
    );
    let second = NAME.replace("abcdef", "123456");
    fs::write(
        folder.join(&second),
        serde_json::to_vec(&proposal()).unwrap(),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        &target,
        folder.join(second.replace(".json", ".outcome.json")),
    )
    .unwrap();
    let rows = read(engine.as_ref(), root, &"a".repeat(64));
    assert_eq!(
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row["file"] == second)
            .unwrap()["status"],
        "needsReview"
    );
    assert!(outcome(
        engine.as_ref(),
        root,
        &second,
        json!(["native-a", "native-b"]),
        json!([])
    )
    .is_err());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(target).unwrap()).unwrap(),
        proposal()
    );
    fs::remove_dir_all(root.join(".hwp-workbench")).unwrap();
    std::os::unix::fs::symlink(root, root.join(".hwp-workbench")).unwrap();
    assert!(engine
        .agent_link(
            root,
            "source.hwpx",
            "read",
            json!({"sha256":"a".repeat(64)})
        )
        .unwrap_err()
        .message
        .contains("symlink"));
}
#[test]
fn atomic_session_writer_refuses_symlink_destinations_and_proposal_reader_caps_at_200() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let engine = engine();
    let session = json!({"schemaVersion":1,"app":"hwp-workbench","appVersion":"0.1.0-alpha.1","document":{"name":"source.hwpx","format":"hwpx","sha256":"a".repeat(64)},"selection":null,"staged":null,"updatedAt":"2026-10-09T12:00:00Z"});
    assert_eq!(
        engine
            .agent_link(root, "source.hwpx", "session", session.clone())
            .unwrap(),
        true
    );
    let mut selected = session.clone();
    selected["selection"] = json!({"nodeId":"native-a","text":"current"});
    engine
        .agent_link(root, "source.hwpx", "session", selected)
        .unwrap();
    let destination = root.join(".hwp-workbench/source.hwpx.session.json");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&destination).unwrap()).unwrap()["selection"]
            ["text"],
        "current"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_file(&destination).unwrap();
    let target = root.join("target");
    fs::write(&target, b"untouched").unwrap();
    std::os::unix::fs::symlink(&target, &destination).unwrap();
    assert!(engine
        .agent_link(root, "source.hwpx", "session", session)
        .unwrap_err()
        .message
        .contains("symlink"));
    assert_eq!(fs::read(target).unwrap(), b"untouched");
    let folder = root.join(".hwp-workbench/proposals");
    for i in 0..201 {
        fs::write(
            folder.join(format!("source.hwpx.20261009T120000Z-{i:06x}.json")),
            serde_json::to_vec(&proposal()).unwrap(),
        )
        .unwrap();
    }
    fs::write(
        folder.join("source.hwpx.other.20261009T120000Z-abcdef.json"),
        serde_json::to_vec(&proposal()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        read(engine.as_ref(), root, &"a".repeat(64))
            .as_array()
            .unwrap()
            .len(),
        200
    );
}
#[test]
fn request_limits_names_actions_and_complete_outcome_are_validated() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let engine = engine();
    assert_eq!(
        engine
            .agent_link(root, "source.hwpx", "unknown", json!({}))
            .unwrap_err()
            .message,
        "Unknown link action."
    );
    for name in [
        "../source.hwpx",
        "source.docx",
        "/source.hwp",
        "source.hwp/",
    ] {
        assert_eq!(
            engine
                .agent_link(root, name, "read", json!({"sha256":"x"}))
                .unwrap_err()
                .message,
            "Invalid document name."
        );
    }
    assert_eq!(
        engine
            .agent_link(
                root,
                "source.hwpx",
                "session",
                json!({"large":"x".repeat(2*1024*1024)})
            )
            .unwrap_err()
            .message,
        "Link request too large"
    );
    assert_eq!(
        engine
            .agent_link(root, "source.hwpx", "session", json!({}))
            .unwrap_err()
            .message,
        "Invalid session."
    );
    assert_eq!(
        engine
            .tool(ToolAction::Install {
                target: "all".into()
            })
            .unwrap_err()
            .message,
        "Invalid skill target"
    );
    assert_eq!(
        engine
            .tool(ToolAction::McpConfig {
                client: "json".into(),
                root: root.into()
            })
            .unwrap_err()
            .message,
        "Invalid MCP client"
    );
    let folder = root.join(".hwp-workbench/proposals");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(NAME), serde_json::to_vec(&proposal()).unwrap()).unwrap();
    let error = engine.agent_link(root,"source.hwpx","outcome",json!({"file":NAME,"expectedPatch":{},"acceptedNodeIds":["native-a","native-b"],"declinedNodeIds":[]})).unwrap_err();
    assert_eq!(error.message, "Proposal changed before the outcome write.");
    for (accepted, declined) in [
        (json!(["native-a", "native-a"]), json!(["native-b"])),
        (json!(["unknown"]), json!(["native-a", "native-b"])),
        (json!([1]), json!([])),
    ] {
        assert!(outcome(engine.as_ref(), root, NAME, accepted, declined).is_err());
    }
    let excessive = json!((0..1001)
        .map(|index| format!("node-{index}"))
        .collect::<Vec<_>>());
    assert_eq!(
        outcome(engine.as_ref(), root, NAME, excessive, json!([]))
            .unwrap_err()
            .message,
        "Outcome must contain the complete proposal once."
    );
    outcome(
        engine.as_ref(),
        root,
        NAME,
        json!(["native-a", "native-b"]),
        json!([]),
    )
    .unwrap();
    let file = folder.join(NAME);
    let mut bytes = fs::read(&file).unwrap();
    bytes.push(b' ');
    fs::write(file, bytes).unwrap();
    assert_eq!(
        read(engine.as_ref(), root, &"a".repeat(64))[0]["status"],
        "needsReview"
    );
}
#[test]
fn cancelled_rust_request_does_not_start_work() {
    let error = engine()
        .call_cancelled(
            Path::new("/missing"),
            "preview",
            json!({}),
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap_err();
    assert_eq!(error.message, "Preview dropped");
}

#[test]
fn factory_calls_native_service_without_an_interpreter_or_source_checkout() {
    let scratch = tempfile::tempdir().unwrap();
    let engine = desktop_engine(scratch.path().join("no-python-source"));
    let result = engine
        .call(scratch.path(), "capabilities", json!({}))
        .unwrap();
    assert_eq!(result, hwp_workbench::service::capabilities());
    let error = engine
        .call(scratch.path(), "inspect", json!({"path":"missing.hwpx"}))
        .unwrap_err();
    assert_eq!(error.code.as_deref(), Some("READ_FAILED"));
    assert_eq!(
        engine
            .call(
                scratch.path(),
                "inspect",
                json!({"large":"x".repeat(4*1024*1024)})
            )
            .unwrap_err()
            .message,
        "Request too large"
    );
}
