use super::*;
use serde_json::json;

#[test]
fn every_request_requires_the_token_and_safe_relative_route() {
    assert!(route("/secret/index.html", "secret").is_ok());
    for url in [
        "/index.html",
        "/other/index.html",
        "/secret/../package.json",
        "/secret/%2e%2e/package.json",
        "/secret//index.html",
    ] {
        assert!(route(url, "secret").is_err(), "{url}");
    }
}

#[test]
fn static_assets_refuse_unknown_types_and_symlinks() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("index.html"), b"renderer").unwrap();
    std::os::unix::fs::symlink("index.html", directory.path().join("escape.js")).unwrap();
    assert_eq!(
        asset(directory.path(), "index.html").unwrap().0,
        b"renderer"
    );
    assert!(asset(directory.path(), "escape.js").is_err());
    assert!(asset(directory.path(), "Cargo.toml").is_err());
}

#[test]
fn scripted_dialogs_consume_answers_and_cancel_when_empty() {
    let platform = Platform::default();
    lock(&platform.state)
        .unwrap()
        .opens
        .push_back(Some(PathBuf::from("/one.hwpx")));
    lock(&platform.state).unwrap().saves.push_back(None);
    lock(&platform.state).unwrap().answers.push_back(1);
    assert_eq!(
        platform.open_file().unwrap(),
        Some(PathBuf::from("/one.hwpx"))
    );
    assert_eq!(platform.open_file().unwrap(), None);
    assert_eq!(
        platform.save_file(Path::new("/default"), &[]).unwrap(),
        None
    );
    assert_eq!(
        platform
            .message("question", &["cancel", "open"], 0)
            .unwrap(),
        1
    );
    assert_eq!(
        platform
            .message("question", &["cancel", "open"], 0)
            .unwrap(),
        0
    );
    assert_eq!(
        platform.snapshot().unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn event_stream_preserves_names_and_json_payloads() {
    let platform = Platform::default();
    let receiver = platform.subscribe().unwrap();
    platform
        .emit("agent:inbox", json!({"rows": ["한글"]}))
        .unwrap();
    assert_eq!(
        receiver.recv().unwrap(),
        "event: agent:inbox\ndata: {\"rows\":[\"한글\"]}\n\n"
    );
}

struct NoDocumentEngine;
impl app_core::Engine for NoDocumentEngine {
    fn call(&self, _root: &Path, _method: &str, _args: Value) -> Result<Value> {
        Err(Error::new("Engine must not run for transport-only checks"))
    }
}

fn test_host(directory: &Path) -> Host {
    let platform = Arc::new(Platform::default());
    let backend = Arc::new(
        Backend::new(
            Arc::new(NoDocumentEngine),
            platform.clone(),
            platform.clone(),
            platform.clone(),
            directory.to_owned(),
            directory.join("profile"),
        )
        .unwrap(),
    );
    Host {
        queue: CommandQueue::new(backend.clone()),
        backend,
        platform,
        assets: directory.to_owned(),
        token: "secret".into(),
    }
}

fn http_request(address: &str, path: &str, body: Option<&str>) -> String {
    use std::net::TcpStream;
    let mut connection = TcpStream::connect(address).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let body = body.unwrap_or_default();
    let method = if body.is_empty() { "GET" } else { "POST" };
    write!(connection, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    response
}

#[test]
#[ignore = "requires loopback socket permission; run with --include-ignored outside the worker sandbox"]
fn http_endpoints_authenticate_and_use_the_shared_command_validation() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("index.html"), "renderer").unwrap();
    let host = test_host(directory.path());
    let server = Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_string();
    let worker = std::thread::spawn(move || {
        for _ in 0..7 {
            let request = server
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap();
            host.request(request).unwrap();
        }
    });
    assert!(http_request(&address, "/index.html", None).starts_with("HTTP/1.1 403"));
    assert!(http_request(&address, "/secret/index.html", None).contains("renderer"));
    assert!(
        http_request(&address, "/secret/invoke/shell_exec", Some("{}"))
            .contains("Unknown desktop command")
    );
    assert!(http_request(
        &address,
        "/secret/invoke/shell_reveal_output",
        Some(r#"{"path":"/other"}"#)
    )
    .contains("Reveal accepts no arguments"));
    assert!(http_request(
        &address,
        "/secret/invoke/clipboard_copy",
        Some(r#"{"text":"한글"}"#)
    )
    .starts_with("HTTP/1.1 200"));
    assert!(http_request(&address, "/secret/test/state", Some("{}")).contains("한글"));
    assert!(http_request(&address, "/secret/invoke/document_open", Some("{}")).ends_with("null"));
    worker.join().unwrap();
}

#[test]
#[ignore = "requires loopback socket permission; run with --include-ignored outside the worker sandbox"]
fn live_sse_flushes_registration_and_named_events_without_buffering() {
    use std::io::{BufRead, BufReader};
    use std::net::TcpStream;
    let directory = tempfile::tempdir().unwrap();
    let host = test_host(directory.path());
    let platform = host.platform.clone();
    let server = Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_string();
    let worker = std::thread::spawn(move || {
        host.request(
            server
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    });
    let mut connection = TcpStream::connect(&address).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        connection,
        "GET /secret/events HTTP/1.1\r\nHost: {address}\r\n\r\n"
    )
    .unwrap();
    let mut reader = BufReader::new(connection);
    let mut line = String::new();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == ": ready\n" {
            break;
        }
    }
    platform
        .emit("document:handoff", json!({"result":null,"error":"한글"}))
        .unwrap();
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "event: document:handoff\n" {
            break;
        }
    }
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "data: {\"result\":null,\"error\":\"한글\"}\n");
    drop(reader);
    worker.join().unwrap();
}

#[test]
fn test_controls_queue_paths_record_shell_effects_and_consume_drop_grants() {
    let directory = tempfile::tempdir().unwrap();
    let host = test_host(directory.path());
    let file = directory.path().join("drop.hwpx");
    std::fs::write(&file, b"synthetic transport fixture").unwrap();
    host.test("dialog/open", json!({"path":file})).unwrap();
    assert_eq!(host.platform.open_file().unwrap(), Some(file.clone()));
    host.test("dialog/save", json!({"path":null})).unwrap();
    assert_eq!(host.platform.save_file(&file, &[]).unwrap(), None);
    host.test("dialog/message", json!({"answer":1})).unwrap();
    assert_eq!(
        host.platform
            .message("question", &["cancel", "open"], 0)
            .unwrap(),
        1
    );
    host.platform.copy("한글").unwrap();
    host.platform.reveal(&file).unwrap();
    assert_eq!(
        host.test("state", json!({})).unwrap()["copied"],
        json!(["한글"])
    );
    assert!(host.test("menu", json!({"command":"shell"})).is_err());
    assert!(host
        .test("dialog/open", json!({"path":"relative.hwpx"}))
        .is_err());
    let receiver = host.platform.subscribe().unwrap();
    host.test("menu", json!({"command":"undo"})).unwrap();
    assert!(receiver
        .recv()
        .unwrap()
        .contains("event: app:command\ndata: \"undo\""));
    host.test("drop", json!({"paths":[file]})).unwrap();
    assert!(receiver
        .recv()
        .unwrap()
        .contains("event: tauri://drag-drop"));
    let first = host
        .backend
        .invoke("document_open_dropped", json!({"path":file}))
        .unwrap_err();
    assert_eq!(
        first.message,
        "Engine must not run for transport-only checks"
    );
    let second = host
        .backend
        .invoke("document_open_dropped", json!({"path":file}))
        .unwrap_err();
    assert_eq!(second.message, "Unsupported document");
}
