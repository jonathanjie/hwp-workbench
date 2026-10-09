use super::*;
use serde_json::json;
use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};

struct FakeHost;
impl Dialogs for FakeHost {
    fn open_file(&self) -> app_core::Result<Option<PathBuf>> {
        Ok(None)
    }
    fn save_file(&self, _: &Path, _: &[String]) -> app_core::Result<Option<PathBuf>> {
        Ok(None)
    }
    fn message(&self, _: &str, _: &[&str], default: usize) -> app_core::Result<usize> {
        Ok(default)
    }
}
impl Events for FakeHost {
    fn emit(&self, _: &str, _: Value) -> app_core::Result<()> {
        Ok(())
    }
}
impl Shell for FakeHost {
    fn reveal(&self, _: &Path) -> app_core::Result<()> {
        Ok(())
    }
    fn copy(&self, _: &str) -> app_core::Result<()> {
        Ok(())
    }
}
struct FakeEngine;
impl app_core::Engine for FakeEngine {
    fn call(&self, _: &Path, _: &str, _: Value) -> app_core::Result<Value> {
        Ok(Value::Null)
    }
}
#[test]
fn mock_runtime_registers_all_commands_and_checks_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost);
    let backend = Arc::new(
        Backend::new(
            Arc::new(FakeEngine),
            host.clone(),
            host.clone(),
            host,
            temp.path().into(),
            temp.path().join("profile"),
        )
        .unwrap(),
    );
    let app = commands(mock_builder())
        .manage(CommandQueue::new(backend.clone()))
        .manage(backend)
        // Use the production capability manifest, with the mock runtime only.
        .build(tauri::generate_context!())
        .unwrap();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    for command in command_names::COMMAND_NAMES {
        let response = get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: (*command).into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost/index.html".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(json!({"untrustedPath":"/tmp/anything"})),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.into(),
            },
        );
        let error = response.unwrap_err();
        assert!(
            !error.to_string().contains("not found"),
            "{command}: {error}"
        );
        assert!(
            error.to_string().contains("arguments") || error.to_string().contains("No document"),
            "{command}: {error}"
        );
    }
    let response = get_ipc_response(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: "clipboard_copy".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost/index.html".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({"text":false})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    );
    assert!(response
        .unwrap_err()
        .to_string()
        .contains("Invalid clipboard text"));
    for (command, args, expected) in [
        ("clipboard_copy", json!({"text":"한글"}), json!(true)),
        ("document_recent", json!({}), json!([])),
        ("document_editing", json!({"value":true}), json!(true)),
    ] {
        let response = get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: command.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.into(),
            },
        );
        assert_eq!(response.unwrap().deserialize::<Value>().unwrap(), expected);
    }
    for command in [
        "plugin:fs|read_file",
        "plugin:shell|execute",
        "plugin:http|fetch",
        "plugin:opener|open_path",
        "plugin:dialog|open",
        "plugin:clipboard-manager|write_text",
        "plugin:event|emit",
    ] {
        let response = get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: command.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(json!({})),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.into(),
            },
        );
        let error = response.unwrap_err();
        assert!(
            error.to_string().contains("not allowed"),
            "{command}: {error}"
        );
    }
    // Even an otherwise allowed command cannot come from a remote origin.
    let response = get_ipc_response(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: "clipboard_copy".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "https://example.com/".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({"text":"remote"})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    );
    assert!(response.unwrap_err().to_string().contains("not allowed"));
    // Check the shell guard as well as the request-origin capability gate.
    webview
        .navigate("https://example.com/".parse().unwrap())
        .unwrap();
    let response = get_ipc_response(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: "clipboard_copy".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({"text":"remote"})),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    );
    assert_eq!(response.unwrap_err(), json!("Unauthorized IPC sender"));
}
#[test]
fn command_and_capability_lists_match_backend_and_expose_no_plugins() {
    let expected: Vec<_> = app_core::COMMANDS.iter().map(|(name, _)| *name).collect();
    assert_eq!(command_names::COMMAND_NAMES, expected);
    let cap: Value = serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
    let permissions = cap["permissions"].as_array().unwrap();
    assert_eq!(permissions.len(), expected.len() + 2);
    for command in expected {
        assert!(permissions.contains(&json!(format!("allow-{}", command.replace('_', "-")))));
    }
    assert!(permissions
        .iter()
        .all(|item| item.as_str().unwrap().starts_with("allow-")
            || ["core:event:allow-listen", "core:event:allow-unlisten"]
                .contains(&item.as_str().unwrap())));
}
#[test]
fn navigation_is_confined_to_the_bundled_index() {
    for url in [
        "tauri://localhost",
        "tauri://localhost/index.html",
        "tauri://localhost/",
        "http://tauri.localhost/index.html",
    ] {
        assert!(allowed_navigation(&url.parse().unwrap()));
    }
    for url in [
        "https://example.com/",
        "tauri://localhost/other.html",
        "tauri://elsewhere/index.html",
        "http://localhost/index.html",
        "data:text/html,hello",
    ] {
        assert!(!allowed_navigation(&url.parse().unwrap()));
    }
}
#[test]
fn menu_accelerators_use_renderer_key_table() {
    assert_eq!(
        accelerator("open", "windows").unwrap(),
        Some("Alt+O".into())
    );
    assert_eq!(
        accelerator("saveCopy", "windows").unwrap(),
        Some("Alt+V".into())
    );
    assert_eq!(
        accelerator("saveCopy", "mac").unwrap(),
        Some("Command+Shift+S".into())
    );
    assert_eq!(
        accelerator("replace", "windows").unwrap(),
        Some("Control+H".into())
    );
}
#[test]
fn native_message_mapping_keeps_cancel_as_default() {
    let buttons = [
        "Reload / 불러오기 다시",
        "Save As / 다른 이름으로 저장",
        "Cancel / 취소",
    ];
    let order = dialog_order(&buttons, 2).unwrap();
    assert_eq!(order, vec![2, 0, 1]);
    assert_eq!(
        dialog_result(
            &buttons,
            &order,
            MessageDialogResult::Custom(buttons[1].into()),
            2
        ),
        1
    );
    assert_eq!(
        dialog_result(&buttons, &order, MessageDialogResult::Cancel, 2),
        2
    );
}

#[test]
fn unbundled_binary_uses_the_source_checkout_for_the_engine() {
    let checkout = engine_source_root(None).unwrap();
    assert!(checkout.join("engine/hwp_workbench").is_dir());
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        engine_source_root(Some(empty.path().to_path_buf())).unwrap(),
        checkout
    );
    let bundle = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(bundle.path().join("app/engine/hwp_workbench")).unwrap();
    assert_eq!(
        engine_source_root(Some(bundle.path().to_path_buf())).unwrap(),
        bundle.path().join("app")
    );
}
