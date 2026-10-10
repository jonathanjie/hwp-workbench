//! Shared agent and native-save checks; no Python exporter or GUI runtime.
use hwp_core::document::Document;
use hwp_core::package_io::ZipPackage;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn setup(inner: &str) -> (TempDir, Document) {
    let base = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/data/fixtures-make_hwpx-f1a1032946d6a513.hwpx"),
    )
    .unwrap();
    let source = Document::new(&base, "form.hwpx").unwrap();
    let xml = std::str::from_utf8(source.part("Contents/section0.xml").unwrap()).unwrap();
    let start = xml.find("<hp:t>").unwrap() + 6;
    let end = start + xml[start..].find("</hp:t>").unwrap();
    let changed = [&xml[..start], inner, &xml[end..]].concat();
    let bytes = ZipPackage::new(&base)
        .unwrap()
        .rewrite(&BTreeMap::from([(
            "Contents/section0.xml".to_owned(),
            changed.into_bytes(),
        )]))
        .unwrap();
    let folder = TempDir::new().unwrap();
    std::fs::write(folder.path().join("form.hwpx"), &bytes).unwrap();
    (folder, Document::new(bytes, "form.hwpx").unwrap())
}

fn run(folder: &TempDir, command: &str, input: &Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hwp-workbench"))
        .args([command, "--root"])
        .arg(folder.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    if command == "mcp" {
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":0,"method":"initialize"})
        )
        .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
    }
    writeln!(stdin, "{input}").unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    serde_json::from_str(if command == "mcp" {
        stdout.lines().last().unwrap()
    } else {
        &stdout
    })
    .unwrap()
}

fn rpc(folder: &TempDir, op: &str, args: Value) -> Value {
    run(folder, "rpc", &json!({"method":op,"args":args}))
}

#[test]
fn cli_and_mcp_read_search_replace_keep_placeholder_contract() {
    let (folder, source) = setup("□<hp:fwSpace/>동의함");
    let text = "□\u{fdd0}동의함";
    assert_eq!(source.nodes[0].text, text);
    let read = rpc(&folder, "read", json!({"path":"form.hwpx"}));
    assert!(read["text"].as_str().unwrap().contains(text));
    let search = rpc(
        &folder,
        "search",
        json!({"path":"form.hwpx","query":"\u{fdd0}"}),
    );
    assert_eq!(search["matches"][0]["context"], text);
    let preview = rpc(
        &folder,
        "replace",
        json!({"path":"form.hwpx","find":"□","replaceWith":"■"}),
    );
    assert_eq!(preview["patch"]["operations"][0]["expectedText"], text);
    assert_eq!(preview["patch"]["operations"][0]["text"], "■\u{fdd0}동의함");
    let mcp = run(
        &folder,
        "mcp",
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"hwp_read","arguments":{"path":"form.hwpx"}}}),
    );
    let result: Value =
        serde_json::from_str(mcp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["text"], read["text"]);
}

#[test]
fn save_with_native_worker_keeps_marker_bytes_and_backup() {
    let inner = "□<hp:fwSpace/><hp:nbSpace/><hp:tab width='0' type='LEFT' leader='0'/><hp:lineBreak/><hp:hyphen/>동의함";
    let (folder, source) = setup(inner);
    let text = source.nodes[0].text.replace('□', "■");
    let patch = json!({"schemaVersion":1,"baseSha256":source.hash,"operations":[{"op":"set_text","nodeId":source.nodes[0].id,"expectedText":source.nodes[0].text,"text":text}]});
    let result = rpc(
        &folder,
        "save",
        json!({"path":"form.hwpx","patch":patch,"acknowledgeRisks":true}),
    );
    assert_eq!(result["saved"], true);
    assert_eq!(result["independentCheck"]["opened"], true);
    assert_eq!(result["independentCheck"]["editedTextsFound"], true);
    assert_eq!(
        std::fs::read(folder.path().join(result["backup"].as_str().unwrap())).unwrap(),
        source.original
    );
    let saved = Document::new(
        std::fs::read(folder.path().join("form.hwpx")).unwrap(),
        "saved.hwpx",
    )
    .unwrap();
    let original_xml = std::str::from_utf8(source.part("Contents/section0.xml").unwrap()).unwrap();
    assert_eq!(
        saved.part("Contents/section0.xml").unwrap(),
        original_xml
            .replace(inner, &inner.replace('□', "■"))
            .as_bytes()
    );
}

#[test]
fn native_save_after_marker_deletion_checks_reader_text() {
    let (folder, source) = setup("앞<hp:fwSpace a='first'/>중<hp:fwSpace a='second'/>뒤");
    let patch = json!({"schemaVersion":1,"baseSha256":source.hash,"operations":[{"op":"set_text","nodeId":source.nodes[0].id,"expectedText":source.nodes[0].text,"text":"앞중\u{fdd1}뒤"}]});
    let result = rpc(
        &folder,
        "save",
        json!({"path":"form.hwpx","patch":patch,"acknowledgeRisks":true}),
    );
    assert_eq!(result["independentCheck"]["editedTextsFound"], true);
    let saved = Document::new(
        std::fs::read(folder.path().join("form.hwpx")).unwrap(),
        "saved.hwpx",
    )
    .unwrap();
    assert_eq!(saved.nodes[0].text, "앞중\u{fdd0}뒤");
}
