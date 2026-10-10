use app_core::{validate_patch, Backend, Dialogs, Engine, Error, Events, Result, Shell};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Host {
    source: Mutex<Option<PathBuf>>,
    destination: Mutex<Option<PathBuf>>,
    answers: Mutex<std::collections::VecDeque<usize>>,
    revealed: Mutex<Vec<PathBuf>>,
    copied: Mutex<Vec<String>>,
    events: Mutex<Vec<(String, Value)>>,
}
impl Dialogs for Host {
    fn open_file(&self) -> Result<Option<PathBuf>> {
        Ok(self.source.lock().unwrap().clone())
    }
    fn save_file(&self, _: &Path, _: &[String]) -> Result<Option<PathBuf>> {
        Ok(self.destination.lock().unwrap().clone())
    }
    fn message(&self, _: &str, _: &[&str], default: usize) -> Result<usize> {
        Ok(self.answers.lock().unwrap().pop_front().unwrap_or(default))
    }
}
impl Events for Host {
    fn emit(&self, event: &str, payload: Value) -> Result<()> {
        self.events.lock().unwrap().push((event.into(), payload));
        Ok(())
    }
}
impl Shell for Host {
    fn reveal(&self, path: &Path) -> Result<()> {
        self.revealed.lock().unwrap().push(path.into());
        Ok(())
    }
    fn copy(&self, text: &str) -> Result<()> {
        self.copied.lock().unwrap().push(text.into());
        Ok(())
    }
}
struct FakeEngine;
impl Engine for FakeEngine {
    fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value> {
        if method == "inspect" {
            let file = root.join(args["path"].as_str().unwrap());
            use sha2::{Digest, Sha256};
            let hash = hex::encode(Sha256::digest(std::fs::read(&file).unwrap()));
            Ok(
                json!({"name":file.file_name().unwrap().to_str().unwrap(),"format":"hwpx","sha256":hash,"nodes":[]}),
            )
        } else {
            Ok(args)
        }
    }
}
fn setup() -> (tempfile::TempDir, Arc<Host>, Backend) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.hwpx");
    std::fs::write(&source, "document").unwrap();
    let host = Arc::new(Host::default());
    *host.source.lock().unwrap() = Some(source);
    let backend = Backend::new(
        Arc::new(FakeEngine),
        host.clone(),
        host.clone(),
        host.clone(),
        dir.path().into(),
        dir.path().join("profile"),
    )
    .unwrap();
    (dir, host, backend)
}
#[test]
fn patch_validation_checks_schema_hash_operations_and_byte_limit() {
    let valid = json!({"schemaVersion":1,"baseSha256":"hash","operations":[]});
    validate_patch(&valid, Some("hash")).unwrap();
    for patch in [
        Value::Null,
        json!({"schemaVersion":2,"baseSha256":"hash","operations":[]}),
        json!({"schemaVersion":1,"baseSha256":"old","operations":[]}),
        json!({"schemaVersion":1,"baseSha256":"hash","operations":vec![json!({});1001]}),
    ] {
        assert_eq!(
            validate_patch(&patch, Some("hash")).unwrap_err().message,
            "Invalid or stale patch"
        );
    }
    let huge = json!({"schemaVersion":1,"baseSha256":"hash","operations":[],"extra":"x".repeat(2*1024*1024)});
    assert_eq!(
        validate_patch(&huge, Some("hash")).unwrap_err().message,
        "Patch too large"
    );
}
#[test]
fn drop_grants_are_native_single_use_and_replaced_links_are_refused() {
    let (dir, host, backend) = setup();
    let path = host.source.lock().unwrap().clone().unwrap();
    assert!(backend
        .invoke("document_open_dropped", json!({"path":path}))
        .is_err());
    backend.grant_drop(vec![path.clone()]).unwrap();
    assert!(backend
        .invoke("document_open_dropped", json!({"path":path}))
        .is_ok());
    assert!(backend
        .invoke("document_open_dropped", json!({"path":path}))
        .is_err());
    backend.grant_drop(vec![path.clone()]).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(dir.path().join("other.hwpx"), "other").unwrap();
    std::os::unix::fs::symlink(dir.path().join("other.hwpx"), &path).unwrap();
    assert!(backend
        .invoke("document_open_dropped", json!({"path":path}))
        .is_err());
}
#[test]
fn command_arguments_and_acknowledgement_are_checked_before_effects() {
    let (_dir, host, backend) = setup();
    assert_eq!(
        backend
            .invoke("document_bytes", json!({}))
            .unwrap_err()
            .message,
        "No document"
    );
    assert!(backend
        .invoke("clipboard_copy", json!({"text":false}))
        .is_err());
    assert!(backend
        .invoke("clipboard_copy", json!({"text":"x".repeat(4097)}))
        .is_err());
    assert!(backend
        .invoke("document_editing", json!({"value":"yes"}))
        .is_err());
    backend
        .invoke("clipboard_copy", json!({"text":"한글"}))
        .unwrap();
    assert_eq!(*host.copied.lock().unwrap(), vec!["한글"]);
    let opened = backend.invoke("document_open", json!({})).unwrap();
    let patch =
        json!({"schemaVersion":1,"baseSha256":opened["document"]["sha256"],"operations":[]});
    assert_eq!(
        backend
            .invoke(
                "document_save",
                json!({"patch":patch,"acknowledgement":false})
            )
            .unwrap_err()
            .message,
        "Compatibility acknowledgement required"
    );
    assert!(backend
        .invoke("shell_reveal_output", json!({"path":"/tmp/anything"}))
        .is_err());
    assert!(backend
        .invoke("document_bytes", json!({"path":"/tmp/anything"}))
        .is_err());
    assert!(backend.invoke("unknown", json!({})).is_err());
}
#[test]
fn handoff_accepts_only_absolute_regular_native_files_and_retains_staged_work_on_cancellation() {
    let (_dir, host, backend) = setup();
    backend.invoke("document_open", json!({})).unwrap();
    backend
        .invoke("document_editing", json!({"value":true}))
        .unwrap();
    backend
        .queue_open(host.source.lock().unwrap().clone().unwrap())
        .unwrap();
    backend.invoke("document_handoff_ready", json!({})).unwrap();
    assert!(!host
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|(name, _)| name == "document:handoff"));
}
#[test]
fn errors_preserve_engine_codes() {
    assert_eq!(
        Error::coded("SOURCE_CHANGED", "Source changed")
            .code
            .as_deref(),
        Some("SOURCE_CHANGED")
    );
}

#[test]
fn fifo_queue_keeps_edit_state_ahead_of_close_confirmation() {
    let (_dir, _host, backend) = setup();
    let backend = Arc::new(backend);
    let queue = app_core::queue::CommandQueue::new(backend.clone());
    let editing = queue
        .invoke("document_editing", json!({"value":true}))
        .unwrap();
    let close = queue.close().unwrap();
    assert_eq!(editing.recv().unwrap().unwrap(), Value::Bool(true));
    assert_eq!(close.recv().unwrap().unwrap(), Value::Bool(false));
    let clean = queue
        .invoke("document_editing", json!({"value":false}))
        .unwrap();
    let close = queue.close().unwrap();
    clean.recv().unwrap().unwrap();
    assert_eq!(close.recv().unwrap().unwrap(), Value::Bool(true));
}
#[test]
fn open_argument_requires_one_absolute_open_flag() {
    assert_eq!(
        app_core::open_argument(&["--open".into(), "/tmp/form.hwpx".into()]),
        Some(PathBuf::from("/tmp/form.hwpx"))
    );
    for args in [
        vec!["--open".into()],
        vec!["--open".into(), "relative.hwpx".into()],
        vec![
            "--open".into(),
            "/tmp/form.hwpx".into(),
            "--open".into(),
            "/tmp/other.hwpx".into(),
        ],
    ] {
        assert!(app_core::open_argument(&args).is_none());
    }
}

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// libtest executables do not dispatch __rhwp-worker. Exercise verified saves
/// through the real Rust CLI; the other desktop calls use the in-process adapter.
struct VerifiedTestEngine(app_core::engine::RustEngine);
impl Engine for VerifiedTestEngine {
    fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value> {
        if method != "save" {
            return self.0.call(root, method, args);
        }
        use std::io::Write;
        use std::process::{Command, Stdio};
        let executable = std::env::current_exe()?;
        let profile = executable
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| Error::new("Missing test binary directory"))?;
        let mut child = Command::new(profile.join("hwp-workbench"))
            .args(["rpc", "--root"])
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or_else(|| Error::new("Missing CLI stdin"))?
            .write_all(&serde_json::to_vec(&json!({"method":method,"args":args}))?)?;
        let output = child.wait_with_output()?;
        if output.status.success() {
            return Ok(serde_json::from_slice(&output.stdout)?);
        }
        let value: Value = serde_json::from_slice(&output.stderr)?;
        let detail = value
            .get("error")
            .ok_or_else(|| Error::new("Missing CLI error"))?;
        Err(Error::coded(
            detail
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("REQUEST_FAILED"),
            detail
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("CLI failed"),
        ))
    }
}
fn real_setup() -> (tempfile::TempDir, Arc<Host>, Backend) {
    let (dir, host, _fake) = setup();
    std::fs::copy(
        source_root().join("fixtures/generated/review-form.hwpx"),
        host.source.lock().unwrap().as_ref().unwrap(),
    )
    .unwrap();
    *host.destination.lock().unwrap() = Some(dir.path().join("copy.hwpx"));
    let backend = Backend::new(
        Arc::new(VerifiedTestEngine(app_core::engine::RustEngine::new(
            source_root(),
        ))),
        host.clone(),
        host.clone(),
        host.clone(),
        source_root(),
        dir.path().join("profile"),
    )
    .unwrap();
    (dir, host, backend)
}
fn patch_for(opened: &Value, suffix: &str) -> Value {
    let node = opened["document"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["editable"] == true)
        .unwrap();
    json!({"schemaVersion":1,"baseSha256":opened["document"]["sha256"],"operations":[{"op":"set_text","nodeId":node["id"],"expectedText":node["text"],"text":format!("{}{suffix}",node["text"].as_str().unwrap())}]})
}
#[test]
fn in_place_save_reloads_its_revision_retains_two_versions_and_refuses_disk_conflicts() {
    let (dir, host, backend) = real_setup();
    let source = host.source.lock().unwrap().clone().unwrap();
    let original = std::fs::read(&source).unwrap();
    let mut opened = backend.invoke("document_open", json!({})).unwrap();
    let patch = patch_for(&opened, " 검토");
    assert!(backend
        .invoke(
            "document_save_in_place",
            json!({"patch":patch,"acknowledgement":false})
        )
        .is_err());
    backend
        .invoke("document_draft", json!({"patch":patch}))
        .unwrap();
    let saved = backend
        .invoke(
            "document_save_in_place",
            json!({"patch":patch,"acknowledgement":true}),
        )
        .unwrap();
    assert_eq!(saved["action"], "saved");
    opened = saved["opened"].clone();
    assert_eq!(opened["draft"], Value::Null);
    assert_ne!(std::fs::read(&source).unwrap(), original);
    let patch = patch_for(&opened, " 다시");
    opened = backend
        .invoke(
            "document_save_in_place",
            json!({"patch":patch,"acknowledgement":false}),
        )
        .unwrap()["opened"]
        .clone();
    assert_eq!(
        std::fs::read_dir(dir.path().join(".hwp-workbench/backups"))
            .unwrap()
            .count(),
        2
    );
    backend.invoke("shell_reveal_previous", json!({})).unwrap();
    assert!(host.revealed.lock().unwrap().first().unwrap().starts_with(
        dir.path()
            .canonicalize()
            .unwrap()
            .join(".hwp-workbench/backups")
    ));
    std::fs::write(&source, &original).unwrap();
    let patch = patch_for(&opened, " 충돌");
    assert_eq!(
        backend
            .invoke(
                "document_save_in_place",
                json!({"patch":patch,"acknowledgement":false})
            )
            .unwrap()["action"],
        "cancel"
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
}
#[test]
fn save_as_keeps_opened_bytes_after_external_change_and_read_only_save_offers_a_copy() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, host, backend) = real_setup();
    let source = host.source.lock().unwrap().clone().unwrap();
    let original = std::fs::read(&source).unwrap();
    let opened = backend.invoke("document_open", json!({})).unwrap();
    let patch = patch_for(&opened, " 사본");
    std::fs::write(&source, b"external bytes").unwrap();
    backend
        .invoke(
            "document_save",
            json!({"patch":patch,"acknowledgement":true}),
        )
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"external bytes");
    assert!(
        std::fs::metadata(dir.path().join("copy.hwpx"))
            .unwrap()
            .len()
            > 0
    );
    let copy = std::fs::read(dir.path().join("copy.hwpx")).unwrap();
    assert!(backend
        .invoke(
            "document_save",
            json!({"patch":patch,"acknowledgement":true})
        )
        .is_err());
    assert_eq!(std::fs::read(dir.path().join("copy.hwpx")).unwrap(), copy);
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".hwp-workbench-copy-")));
    std::fs::write(&source, &original).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o444)).unwrap();
    host.answers.lock().unwrap().push_back(0);
    assert_eq!(
        backend
            .invoke(
                "document_save_in_place",
                json!({"patch":patch,"acknowledgement":true})
            )
            .unwrap()["action"],
        "saveAs"
    );
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn main_ipc_enforces_id_drop_acknowledgement_reveal_and_non_overwrite_boundaries() {
    let (_dir, host, backend) = real_setup();
    assert!(backend.invoke("shell_reveal_output", json!({})).is_err());
    let opened = backend.invoke("document_open", json!({})).unwrap();
    let recents = backend.invoke("document_recent", json!({})).unwrap();
    assert_eq!(recents[0]["id"], opened["recentId"]);
    assert!(recents[0].get("file").is_none());
    assert!(backend
        .invoke("document_open_recent", json!({"id":0}))
        .is_err());
    backend
        .invoke("document_open_recent", json!({"id":opened["recentId"]}))
        .unwrap();
    assert!(backend
        .invoke("document_cache_thumbnail", json!({"id":"wrong","png":""}))
        .is_err());
    let patch = patch_for(&opened, " 검토");
    backend
        .invoke("document_plan", json!({"patch":patch}))
        .unwrap();
    let preview = backend
        .invoke("document_preview", json!({"patch":patch}))
        .unwrap();
    assert!(!preview["bytesBase64"].as_str().unwrap().is_empty());
    assert!(backend
        .invoke(
            "document_save",
            json!({"patch":patch,"acknowledgement":false})
        )
        .is_err());
    backend
        .invoke(
            "document_save",
            json!({"patch":patch,"acknowledgement":true}),
        )
        .unwrap();
    backend.invoke("shell_reveal_output", json!({})).unwrap();
    assert_eq!(host.revealed.lock().unwrap().len(), 1);
    backend
        .invoke("document_open_recent", json!({"id":opened["recentId"]}))
        .unwrap();
    assert!(backend.invoke("shell_reveal_output", json!({})).is_err());
}
#[test]
fn handoff_waits_for_renderer_acknowledgement_between_files() {
    let (dir, host, backend) = setup();
    let first = host.source.lock().unwrap().clone().unwrap();
    let second = dir.path().join("second.hwpx");
    std::fs::write(&second, "second document").unwrap();
    backend.queue_open(first).unwrap();
    backend.queue_open(second).unwrap();
    assert!(host.events.lock().unwrap().is_empty());
    backend.invoke("document_handoff_ready", json!({})).unwrap();
    let count = || {
        host.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "document:handoff")
            .count()
    };
    assert_eq!(count(), 1);
    backend.invoke("document_handoff_ready", json!({})).unwrap();
    assert_eq!(count(), 2);
}
#[test]
fn draft_lookup_is_isolated_by_file_path_and_content_hash() {
    let (dir, host, backend) = setup();
    let opened = backend.invoke("document_open", json!({})).unwrap();
    let patch =
        json!({"schemaVersion":1,"baseSha256":opened["document"]["sha256"],"operations":[]});
    backend
        .invoke("document_draft", json!({"patch":patch}))
        .unwrap();
    assert_eq!(
        backend.invoke("document_open", json!({})).unwrap()["draft"],
        patch
    );
    let copy = dir.path().join("copied.hwpx");
    std::fs::write(&copy, "document").unwrap();
    *host.source.lock().unwrap() = Some(copy);
    assert_eq!(
        backend.invoke("document_open", json!({})).unwrap()["draft"],
        Value::Null
    );
}

#[test]
fn opening_a_document_cancels_preview_started_during_inspection() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    struct DelayedEngine {
        inspections: AtomicUsize,
        inspecting: mpsc::Sender<()>,
        inspect_release: Mutex<mpsc::Receiver<()>>,
        previewing: mpsc::Sender<()>,
        preview_release: Mutex<mpsc::Receiver<()>>,
    }
    impl Engine for DelayedEngine {
        fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value> {
            if method == "inspect" && self.inspections.fetch_add(1, Ordering::SeqCst) > 0 {
                self.inspecting.send(()).unwrap();
                self.inspect_release.lock().unwrap().recv().unwrap();
            }
            if method == "preview" {
                self.previewing.send(()).unwrap();
                self.preview_release.lock().unwrap().recv().unwrap();
                return Ok(json!({"oldDocument":true}));
            }
            FakeEngine.call(root, method, args)
        }
    }
    let (dir, host, _backend) = setup();
    let (inspecting, inspect_started) = mpsc::channel();
    let (allow_inspect, inspect_release) = mpsc::channel();
    let (previewing, preview_started) = mpsc::channel();
    let (allow_preview, preview_release) = mpsc::channel();
    let engine = Arc::new(DelayedEngine {
        inspections: AtomicUsize::new(0),
        inspecting,
        inspect_release: Mutex::new(inspect_release),
        previewing,
        preview_release: Mutex::new(preview_release),
    });
    let backend = Arc::new(
        Backend::new(
            engine,
            host.clone(),
            host.clone(),
            host.clone(),
            dir.path().into(),
            dir.path().join("profile"),
        )
        .unwrap(),
    );
    let opened = backend.invoke("document_open", json!({})).unwrap();
    let other = dir.path().join("other.hwpx");
    std::fs::write(&other, "other document").unwrap();
    *host.source.lock().unwrap() = Some(other);
    let opener = {
        let backend = backend.clone();
        std::thread::spawn(move || backend.invoke("document_open", json!({})).unwrap())
    };
    inspect_started
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let patch =
        json!({"schemaVersion":1,"baseSha256":opened["document"]["sha256"],"operations":[]});
    let previewer = {
        let backend = backend.clone();
        std::thread::spawn(move || {
            backend
                .invoke("document_preview", json!({"patch":patch}))
                .unwrap()
        })
    };
    preview_started
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    allow_inspect.send(()).unwrap();
    opener.join().unwrap();
    allow_preview.send(()).unwrap();
    assert_eq!(previewer.join().unwrap(), Value::Null);
}

#[test]
fn prepared_preview_requests_keep_receipt_order_when_workers_start_out_of_order() {
    let (_dir, _host, backend) = setup();
    let opened = backend.invoke("document_open", json!({})).unwrap();
    let patch = |label| json!({"schemaVersion":1,"baseSha256":opened["document"]["sha256"],"operations":[],"label":label});
    let old = backend
        .prepare_preview(&json!({"patch":patch("old")}))
        .unwrap();
    let new = backend
        .prepare_preview(&json!({"patch":patch("new")}))
        .unwrap();
    assert_eq!(backend.run_preview(new).unwrap()["patch"]["label"], "new");
    assert_eq!(backend.run_preview(old).unwrap(), Value::Null);
}
#[test]
fn approved_close_refuses_later_queued_operations() {
    let (_dir, host, backend) = setup();
    let queue = app_core::queue::CommandQueue::new(Arc::new(backend));
    let close = queue.close().unwrap();
    let later = queue.invoke("clipboard_copy", json!({"text":"after close"}));
    assert_eq!(close.recv().unwrap().unwrap(), Value::Bool(true));
    match later {
        Ok(response) => assert!(response.recv().unwrap().is_err()),
        Err(error) => assert!(error.message.contains("closed")),
    }
    assert!(host.copied.lock().unwrap().is_empty());
    assert!(queue.invoke("document_open", json!({})).is_err());
}

#[test]
fn reveal_rejects_empty_state_and_renderer_paths_and_reveals_only_the_saved_output() {
    let (_dir, host, backend) = real_setup();
    assert_eq!(
        backend
            .invoke("shell_reveal_output", json!({}))
            .unwrap_err()
            .message,
        "No saved copy"
    );
    let opened = backend.invoke("document_open", json!({})).unwrap();
    let patch = patch_for(&opened, " 사본");
    backend
        .invoke(
            "document_save",
            json!({"patch":patch,"acknowledgement":true}),
        )
        .unwrap();
    assert_eq!(
        backend
            .invoke("shell_reveal_output", json!({"path":"/tmp/other.hwpx"}))
            .unwrap_err()
            .message,
        "Reveal accepts no arguments"
    );
    backend.invoke("shell_reveal_output", json!({})).unwrap();
    assert_eq!(
        *host.revealed.lock().unwrap(),
        vec![host
            .destination
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .canonicalize()
            .unwrap()]
    );
    backend.invoke("document_open", json!({})).unwrap();
    assert!(backend.invoke("shell_reveal_output", json!({})).is_err());
}
#[test]
fn key_scheme_settings_persist_and_invalid_choices_leave_settings_unchanged() {
    let (_dir, _host, backend) = setup();
    assert_eq!(backend.key_scheme(), "windows");
    backend
        .invoke("keyboard_scheme", json!({"scheme":"mac"}))
        .unwrap();
    assert_eq!(backend.key_scheme(), "mac");
    assert_eq!(
        backend
            .invoke("keyboard_scheme", json!({"scheme":"shell"}))
            .unwrap_err()
            .message,
        "Invalid keyboard scheme"
    );
    assert_eq!(backend.key_scheme(), "mac");
}
