use super::*;
use std::sync::mpsc::{self, Receiver, Sender};
use tempfile::TempDir;

const PROPOSAL: &str = "source.hwpx.20261009T120000Z-abcdef.json";

#[derive(Default)]
struct FakeEngine {
    rows: Mutex<Value>,
    calls: Mutex<Vec<(String, Value)>>,
    plan_error: Mutex<Option<String>>,
    read_gate: Mutex<Option<(Sender<()>, Receiver<()>)>>,
    plan_gate: Mutex<Option<(Sender<()>, Receiver<()>)>>,
    session_gate: Mutex<Option<(Sender<()>, Receiver<()>)>>,
    session_error: Mutex<Option<String>>,
}

impl FakeEngine {
    fn block(gate: &Mutex<Option<(Sender<()>, Receiver<()>)>>) {
        let channels = gate.lock().unwrap().take();
        if let Some((started, resume)) = channels {
            started.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
}

impl Engine for FakeEngine {
    fn tool(&self, _action: crate::engine::ToolAction) -> Result<String, Error> {
        Err(Error::new("Not used"))
    }
    fn call(&self, _root: &Path, method: &str, args: Value) -> Result<Value, Error> {
        self.calls.lock().unwrap().push((method.into(), args));
        Self::block(&self.plan_gate);
        match self.plan_error.lock().unwrap().clone() {
            Some(message) => Err(Error::new(message)),
            None => Ok(json!({"ok": true})),
        }
    }

    fn agent_link(
        &self,
        _root: &Path,
        _name: &str,
        action: &str,
        args: Value,
    ) -> Result<Value, Error> {
        self.calls.lock().unwrap().push((action.into(), args));
        if action == "read" {
            let rows = self.rows.lock().unwrap().clone();
            Self::block(&self.read_gate);
            return Ok(rows);
        }
        if action == "outcome" {
            self.rows.lock().unwrap()[0]["status"] = json!("partial");
        }
        if action == "session" {
            Self::block(&self.session_gate);
            if let Some(message) = self.session_error.lock().unwrap().clone() {
                return Err(Error::new(message));
            }
        }
        Ok(json!(true))
    }
}

#[derive(Default)]
struct FakeEvents(Mutex<Vec<Value>>);

impl Events for FakeEvents {
    fn emit(&self, event: &str, payload: Value) -> Result<(), Error> {
        assert_eq!(event, "agent:inbox");
        self.0.lock().unwrap().push(payload);
        Ok(())
    }
}

fn patch(text: &str) -> Value {
    json!({"schemaVersion": 1, "baseSha256": "a".repeat(64), "operations": [
        {"op": "set_text", "nodeId": "native-a", "expectedText": "old", "text": text},
        {"op": "set_text", "nodeId": "native-b", "expectedText": "old", "text": "new"}
    ]})
}

fn rows(text: &str) -> Value {
    json!([{"file": PROPOSAL, "status": "pending", "author": "Claude", "patch": patch(text)}])
}

fn document(root: &Path, name: &str) -> Session {
    Session {
        file: root.join(name),
        root: root.into(),
        name: name.into(),
        format: "hwpx".into(),
        sha256: "a".repeat(64),
        recent_id: None,
        base_bytes: Vec::new(),
    }
}

struct Harness {
    directory: TempDir,
    engine: Arc<FakeEngine>,
    events: Arc<FakeEvents>,
    inbox: AgentInbox,
}

impl Harness {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let engine = Arc::new(FakeEngine::default());
        *engine.rows.lock().unwrap() = rows("new");
        let events = Arc::new(FakeEvents::default());
        let inbox = AgentInbox::new(
            engine.clone(),
            events.clone(),
            directory.path().join("settings.json"),
        );
        inbox.set_document(document(directory.path(), "source.hwpx"));
        Self {
            directory,
            engine,
            events,
            inbox,
        }
    }

    fn enable(&self) {
        self.inbox.toggle(json!(true)).unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.inbox.stop();
    }
}

fn gate(target: &Mutex<Option<(Sender<()>, Receiver<()>)>>) -> (Receiver<()>, Sender<()>) {
    let (started, wait_started) = mpsc::channel();
    let (resume, wait_resume) = mpsc::channel();
    *target.lock().unwrap() = Some((started, wait_resume));
    (wait_started, resume)
}

#[test]
fn engine_validated_inbox_and_complete_partial_decisions() {
    let harness = Harness::new();
    assert_eq!(
        harness.inbox.state(),
        json!({"enabled": false, "proposals": [], "error": null})
    );
    harness.enable();
    assert_eq!(
        harness.inbox.state()["proposals"][0]["decisions"],
        json!({})
    );
    assert!(harness
        .inbox
        .decide(json!(PROPOSAL), json!([]), json!(true))
        .is_err());
    let accepted = harness
        .inbox
        .decide(json!(PROPOSAL), json!(["native-a"]), json!(true))
        .unwrap();
    assert_eq!(
        accepted,
        json!({"author":"Claude", "operations":[patch("new")["operations"][0].clone()]})
    );
    assert!(harness
        .inbox
        .decide(json!(PROPOSAL), json!(["native-a"]), json!(true))
        .is_err());
    let declined = harness
        .inbox
        .decide(json!(PROPOSAL), json!(["native-b"]), json!(false))
        .unwrap();
    assert_eq!(declined, json!({"author":"Claude", "operations":[]}));
    let calls = harness.engine.calls.lock().unwrap();
    let outcome = calls
        .iter()
        .find(|(action, _)| action == "outcome")
        .unwrap();
    assert_eq!(
        outcome.1,
        json!({"file":PROPOSAL,"acceptedNodeIds":["native-a"],"declinedNodeIds":["native-b"],"expectedPatch":patch("new")})
    );
    drop(calls);
    assert!(harness
        .inbox
        .decide(json!(PROPOSAL), json!(["native-b"]), json!(true))
        .is_err());
    harness.inbox.toggle(json!(false)).unwrap();
    assert_eq!(harness.inbox.state()["proposals"], json!([]));
}

#[test]
fn scan_classifies_plan_failure_and_revalidates_changed_patch() {
    let harness = Harness::new();
    *harness.engine.plan_error.lock().unwrap() = Some("source changed".into());
    harness.enable();
    assert_eq!(harness.inbox.state()["proposals"][0]["status"], "stale");
    assert!(harness.inbox.state()["proposals"][0].get("patch").is_none());
    *harness.engine.plan_error.lock().unwrap() = Some("Expected text mismatch".into());
    harness.inbox.scan();
    assert_eq!(
        harness.inbox.state()["proposals"][0]["status"],
        "unreadable"
    );
    *harness.engine.plan_error.lock().unwrap() = None;
    harness.inbox.scan();
    harness
        .inbox
        .decide(json!(PROPOSAL), json!(["native-a"]), json!(true))
        .unwrap();
    *harness.engine.rows.lock().unwrap() = rows("changed");
    assert_eq!(
        harness
            .inbox
            .decide(json!(PROPOSAL), json!(["native-b"]), json!(true))
            .unwrap_err()
            .message,
        "Stale proposal · reopen"
    );
    harness.inbox.scan();
    assert_eq!(
        harness.inbox.state()["proposals"][0]["decisions"],
        json!({})
    );
}

#[test]
fn requested_scan_waits_for_running_scan_and_one_more_pass() {
    let harness = Harness::new();
    harness.enable();
    let (started, resume) = gate(&harness.engine.read_gate);
    let inbox = harness.inbox.clone();
    let first = thread::spawn(move || inbox.scan());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    *harness.engine.rows.lock().unwrap() = rows("latest");
    let inbox = harness.inbox.clone();
    let (finished, wait_finished) = mpsc::channel();
    let second = thread::spawn(move || {
        inbox.scan();
        finished.send(()).unwrap();
    });
    assert!(wait_finished
        .recv_timeout(Duration::from_millis(30))
        .is_err());
    resume.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();
    assert_eq!(
        harness.inbox.state()["proposals"][0]["patch"]["operations"][0]["text"],
        "latest"
    );
}

#[test]
fn old_scan_cannot_cache_validation_for_new_document() {
    let harness = Harness::new();
    harness.enable();
    *harness.engine.rows.lock().unwrap() = rows("different");
    let (started, resume) = gate(&harness.engine.plan_gate);
    let inbox = harness.inbox.clone();
    let scan = thread::spawn(move || inbox.scan());
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    harness
        .inbox
        .set_document(document(harness.directory.path(), "other.hwpx"));
    resume.send(()).unwrap();
    scan.join().unwrap();
    *harness.engine.plan_error.lock().unwrap() = Some("Invalid expected text".into());
    harness.enable();
    assert_eq!(
        harness.inbox.state()["proposals"][0]["status"],
        "unreadable"
    );
    assert_eq!(
        harness.inbox.state()["proposals"][0]["decisions"],
        json!({})
    );
}

#[test]
fn changed_document_rejects_in_flight_decision_without_outcome() {
    let harness = Harness::new();
    harness.enable();
    let (started, resume) = gate(&harness.engine.plan_gate);
    let inbox = harness.inbox.clone();
    let decision = thread::spawn(move || {
        inbox.decide(
            json!(PROPOSAL),
            json!(["native-a", "native-b"]),
            json!(true),
        )
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    harness
        .inbox
        .set_document(document(harness.directory.path(), "other.hwpx"));
    resume.send(()).unwrap();
    assert_eq!(
        decision.join().unwrap().unwrap_err().message,
        "Stale proposal · reopen"
    );
    assert!(!harness
        .engine
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|(action, _)| action == "outcome"));
}

#[test]
fn session_updates_are_debounced_and_cancelled_on_switch() {
    let harness = Harness::new();
    harness.enable();
    harness
        .inbox
        .update(json!({"nodeId":"native-a","text":"first"}), patch("first"));
    harness.inbox.update(
        json!({"nodeId":"native-a","text":"latest"}),
        patch("latest"),
    );
    thread::sleep(Duration::from_millis(380));
    let calls = harness.engine.calls.lock().unwrap();
    let sessions: Vec<_> = calls
        .iter()
        .filter(|(action, _)| action == "session")
        .collect();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions.last().unwrap().1["selection"]["text"], "latest");
    drop(calls);
    harness.inbox.update(Value::Null, json!({"operations":[]}));
    harness.inbox.stop();
    thread::sleep(Duration::from_millis(380));
    assert_eq!(
        harness
            .engine
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(action, _)| action == "session")
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn preferences_refuse_symlink_reads_and_writes() {
    use std::os::unix::fs::symlink;
    let harness = Harness::new();
    let target = harness.directory.path().join("target");
    fs::write(&target, "untouched").unwrap();
    symlink(&target, harness.directory.path().join("settings.json")).unwrap();
    harness
        .inbox
        .set_document(document(harness.directory.path(), "source.hwpx"));
    assert_eq!(harness.inbox.state()["enabled"], false);
    assert_eq!(
        harness.inbox.toggle(json!(true)).unwrap_err().message,
        "Unsafe Agent link settings"
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
    assert_eq!(fs::read_dir(harness.directory.path()).unwrap().count(), 2);
}

#[test]
fn preferences_restore_enabled_and_reject_invalid_switches() {
    let harness = Harness::new();
    assert_eq!(
        harness.inbox.toggle(json!("true")).unwrap_err().message,
        "Invalid Agent link switch"
    );
    harness.enable();
    let path = harness.directory.path().join("settings.json");
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved[harness
            .directory
            .path()
            .join("source.hwpx")
            .to_str()
            .unwrap()],
        true
    );
    harness
        .inbox
        .set_document(document(harness.directory.path(), "source.hwpx"));
    assert_eq!(harness.inbox.state()["enabled"], true);
    assert!(!harness.events.0.lock().unwrap().is_empty());
}

#[test]
fn polling_detects_proposals_without_watcher_hints() {
    let harness = Harness::new();
    harness.enable();
    *harness.engine.rows.lock().unwrap() = rows("polled");
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while harness.inbox.state()["proposals"][0]["patch"]["operations"][0]["text"] != "polled" {
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "Native notify events were unavailable in the managed macOS host; run explicitly on a qualified host"]
fn watcher_events_trigger_confined_engine_read() {
    let harness = Harness::new();
    let folder = harness.directory.path().join(".hwp-workbench/proposals");
    fs::create_dir_all(&folder).unwrap();
    harness.enable();
    *harness.engine.rows.lock().unwrap() = rows("watched");
    fs::write(folder.join(PROPOSAL), "watch hint only").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_millis(1800);
    while harness.inbox.state()["proposals"][0]["patch"]["operations"][0]["text"] != "watched" {
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
}

fn python_engine() -> crate::engine::PythonEngine {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    crate::engine::PythonEngine::new(source_root.to_path_buf())
}

fn proposal(hash: &str) -> Value {
    let mut patch = patch("new");
    patch["baseSha256"] = json!(hash);
    json!({"schemaVersion":1,"kind":"hwp-workbench-proposal","document":{"name":"source.hwpx","sha256":hash},
        "author":"Claude","title":"Review","createdAt":"2026-10-09T12:00:00Z","patch":patch})
}

#[cfg(unix)]
#[test]
fn bounded_reader_distinguishes_stale_and_unreadable_proposals_outcomes_are_exclusive_and_complete()
{
    use std::os::unix::fs::symlink;
    let engine = python_engine();
    let directory = TempDir::new().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let hash = "a".repeat(64);
    let folder = root.join(".hwp-workbench/proposals");
    fs::create_dir_all(&folder).unwrap();
    let proposal_file = folder.join(PROPOSAL);
    fs::write(
        &proposal_file,
        serde_json::to_vec(&proposal(&hash)).unwrap(),
    )
    .unwrap();
    let read = |hash: &str| engine.agent_link(&root, "source.hwpx", "read", json!({"sha256":hash}));
    let outcome = |file: &str, accepted: Value, declined: Value| {
        engine.agent_link(
            &root,
            "source.hwpx",
            "outcome",
            json!({"file":file,"acceptedNodeIds":accepted,"declinedNodeIds":declined}),
        )
    };
    assert_eq!(read(&"b".repeat(64)).unwrap()[0]["status"], "stale");
    assert_eq!(read(&hash).unwrap()[0]["status"], "pending");
    assert!(outcome(PROPOSAL, json!(["native-a"]), json!([]))
        .unwrap_err()
        .message
        .to_ascii_lowercase()
        .contains("complete"));
    outcome(PROPOSAL, json!(["native-a"]), json!(["native-b"])).unwrap();
    let record: Value = serde_json::from_slice(
        &fs::read(folder.join(PROPOSAL.replace(".json", ".outcome.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(record["status"], "partial");
    assert_eq!(record["acceptedNodeIds"], json!(["native-a"]));
    assert_eq!(record["declinedNodeIds"], json!(["native-b"]));
    assert!(
        outcome(PROPOSAL, json!(["native-a", "native-b"]), json!([]))
            .unwrap_err()
            .message
            .to_ascii_lowercase()
            .contains("exist")
    );
    fs::write(&proposal_file, vec![0; 2 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(read(&hash).unwrap()[0]["status"], "unreadable");
    fs::remove_file(&proposal_file).unwrap();
    let target = root.join("target.json");
    fs::write(&target, serde_json::to_vec(&proposal(&hash)).unwrap()).unwrap();
    symlink(&target, &proposal_file).unwrap();
    assert_eq!(read(&hash).unwrap()[0]["status"], "unreadable");
    fs::remove_file(&proposal_file).unwrap();
    let mut excessive = proposal(&hash);
    excessive["patch"]["operations"] = json!((0..1001).map(|index| json!({"op":"set_text","nodeId":format!("node-{index}"),"expectedText":"old","text":"new"})).collect::<Vec<_>>());
    fs::write(&proposal_file, serde_json::to_vec(&excessive).unwrap()).unwrap();
    assert_eq!(read(&hash).unwrap()[0]["status"], "unreadable");
    let second = PROPOSAL.replace("abcdef", "123456");
    fs::write(
        folder.join(&second),
        serde_json::to_vec(&proposal(&hash)).unwrap(),
    )
    .unwrap();
    symlink(
        &target,
        folder.join(second.replace(".json", ".outcome.json")),
    )
    .unwrap();
    let rows = read(&hash).unwrap();
    assert_eq!(
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row["file"] == second)
            .unwrap()["status"],
        "needsReview"
    );
    assert!(outcome(&second, json!(["native-a", "native-b"]), json!([])).is_err());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&target).unwrap()).unwrap(),
        proposal(&hash)
    );
    fs::remove_dir_all(root.join(".hwp-workbench")).unwrap();
    symlink(&root, root.join(".hwp-workbench")).unwrap();
    assert!(read(&hash)
        .unwrap_err()
        .message
        .to_ascii_lowercase()
        .contains("symlink"));
}

#[cfg(unix)]
#[test]
fn atomic_session_writer_refuses_symlink_destinations_and_proposal_reader_caps_at_200() {
    use std::os::unix::fs::symlink;
    let engine = python_engine();
    let directory = TempDir::new().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let hash = "a".repeat(64);
    let mut session = json!({"schemaVersion":1,"app":"hwp-workbench","appVersion":"0.1.0-alpha.1", "document":{"name":"source.hwpx","format":"hwpx","sha256":hash},"selection":null,"staged":null,"updatedAt":"2026-10-09T12:00:00Z"});
    engine
        .agent_link(&root, "source.hwpx", "session", session.clone())
        .unwrap();
    session["selection"] = json!({"nodeId":"native-a","text":"current"});
    engine
        .agent_link(&root, "source.hwpx", "session", session.clone())
        .unwrap();
    let destination = root.join(".hwp-workbench/source.hwpx.session.json");
    let saved: Value = serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
    assert_eq!(saved["selection"]["text"], "current");
    fs::remove_file(&destination).unwrap();
    let target = root.join("target");
    fs::write(&target, "untouched").unwrap();
    symlink(&target, &destination).unwrap();
    assert!(engine
        .agent_link(&root, "source.hwpx", "session", session)
        .unwrap_err()
        .message
        .to_ascii_lowercase()
        .contains("symlink"));
    assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
    let folder = root.join(".hwp-workbench/proposals");
    let bytes = serde_json::to_vec(&proposal(&hash)).unwrap();
    for index in 0..201 {
        fs::write(
            folder.join(format!("source.hwpx.20261009T120000Z-{index:06x}.json")),
            &bytes,
        )
        .unwrap();
    }
    fs::write(
        folder.join("source.hwpx.other.20261009T120000Z-abcdef.json"),
        &bytes,
    )
    .unwrap();
    let rows = engine
        .agent_link(&root, "source.hwpx", "read", json!({"sha256":hash}))
        .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 200);
    assert!(!rows
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["file"] == "source.hwpx.other.20261009T120000Z-abcdef.json"));
}

#[test]
fn outcome_rechecks_reviewed_patch_before_publication() {
    let engine = python_engine();
    let directory = TempDir::new().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let hash = "a".repeat(64);
    let folder = root.join(".hwp-workbench/proposals");
    fs::create_dir_all(&folder).unwrap();
    let mut replaced = proposal(&hash);
    replaced["patch"]["operations"][0]["text"] = json!("replacement");
    fs::write(
        folder.join(PROPOSAL),
        serde_json::to_vec(&replaced).unwrap(),
    )
    .unwrap();
    let error = engine.agent_link(&root, "source.hwpx", "outcome", json!({
        "file":PROPOSAL,"acceptedNodeIds":["native-a","native-b"],"declinedNodeIds":[],"expectedPatch":patch("new")
    })).unwrap_err();
    assert!(error
        .message
        .contains("Proposal changed before the outcome write."));
    assert!(!folder
        .join(PROPOSAL.replace(".json", ".outcome.json"))
        .exists());
    engine.agent_link(&root, "source.hwpx", "outcome", json!({
        "file":PROPOSAL,"acceptedNodeIds":["native-a","native-b"],"declinedNodeIds":[],"expectedPatch":replaced["patch"]
    })).unwrap();
    let bytes = fs::read(folder.join(PROPOSAL)).unwrap();
    let record: Value = serde_json::from_slice(
        &fs::read(folder.join(PROPOSAL.replace(".json", ".outcome.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(record["proposalSha256"], hex::encode(Sha256::digest(bytes)));
}

#[test]
fn changed_proposal_during_plan_cannot_transfer_old_decisions_to_new_patch() {
    let harness = Harness::new();
    harness.enable();
    let (started, resume) = gate(&harness.engine.plan_gate);
    let inbox = harness.inbox.clone();
    let decision =
        thread::spawn(move || inbox.decide(json!(PROPOSAL), json!(["native-a"]), json!(true)));
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    *harness.engine.rows.lock().unwrap() = rows("replacement");
    harness.inbox.scan();
    resume.send(()).unwrap();
    assert_eq!(
        decision.join().unwrap().unwrap_err().message,
        "Stale proposal · reopen"
    );
    assert_eq!(
        harness.inbox.state()["proposals"][0]["decisions"],
        json!({})
    );
    assert!(!harness
        .engine
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|(action, _)| action == "outcome"));
}

#[test]
fn failed_old_session_update_cannot_change_new_document_error() {
    let harness = Harness::new();
    harness.enable();
    let (started, resume) = gate(&harness.engine.session_gate);
    *harness.engine.session_error.lock().unwrap() = Some("Old session failed".into());
    harness
        .inbox
        .update(json!({"nodeId":"native-a","text":"changed"}), Value::Null);
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    harness
        .inbox
        .set_document(document(harness.directory.path(), "other.hwpx"));
    resume.send(()).unwrap();
    thread::sleep(Duration::from_millis(30));
    assert_eq!(harness.inbox.state()["error"], Value::Null);
    assert_eq!(harness.inbox.state()["enabled"], false);
}

#[test]
fn oversized_preferences_are_ignored_without_following_settings() {
    let harness = Harness::new();
    let file = harness
        .directory
        .path()
        .join("source.hwpx")
        .to_string_lossy()
        .into_owned();
    let settings = json!({file:true,"padding":"x".repeat(MAX_SETTINGS_BYTES as usize)});
    fs::write(
        harness.directory.path().join("settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    harness
        .inbox
        .set_document(document(harness.directory.path(), "source.hwpx"));
    assert_eq!(harness.inbox.state()["enabled"], false);
}

#[test]
fn watch_hints_are_coalesced_and_ignore_stale_generations() {
    let harness = Harness::new();
    harness.enable();
    let generation = lock(&harness.inbox.inner.state).generation;
    let (started, resume) = gate(&harness.engine.read_gate);
    watch_event(
        &harness.inbox.inner,
        generation,
        Ok(notify::Event::new(notify::EventKind::Any)),
    );
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    *harness.engine.rows.lock().unwrap() = rows("hint result");
    for _ in 0..20 {
        watch_event(
            &harness.inbox.inner,
            generation,
            Ok(notify::Event::new(notify::EventKind::Any)),
        );
    }
    resume.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while harness.inbox.state()["proposals"][0]["patch"]["operations"][0]["text"] != "hint result" {
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        harness
            .engine
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(action, _)| action == "read")
            .count(),
        3
    );
    harness.inbox.stop();
    *harness.engine.rows.lock().unwrap() = rows("stale hint");
    watch_event(
        &harness.inbox.inner,
        generation,
        Ok(notify::Event::new(notify::EventKind::Any)),
    );
    assert_eq!(
        harness
            .engine
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(action, _)| action == "read")
            .count(),
        3
    );
}
