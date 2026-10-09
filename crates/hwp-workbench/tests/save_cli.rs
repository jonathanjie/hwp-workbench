//! Native CLI save checks run the real hidden worker, never a GUI runtime.
use hwp_core::document::{sha, Document};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn setup(format: &str) -> (TempDir, Vec<u8>, Value) {
    let folder = TempDir::new().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/generated")
        .join(if format == "hwp" {
            "legacy-review.hwp"
        } else {
            "review-form.hwpx"
        });
    let bytes = fs::read(source).unwrap();
    fs::write(folder.path().join(format!("form.{format}")), &bytes).unwrap();
    let document = Document::new(&bytes, "form").unwrap();
    let node = document.nodes.first().unwrap();
    let patch = json!({"schemaVersion":1,"baseSha256":document.hash,"operations":[{"op":"set_text","nodeId":node.id,"expectedText":node.text,"text":"Saved Korean text 한글"}]});
    fs::write(
        folder.path().join("preview.json"),
        serde_json::to_vec(&json!({"patch":patch,"changes":[]})).unwrap(),
    )
    .unwrap();
    (folder, bytes, patch)
}
fn cli(folder: &TempDir, format: &str, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hwp-workbench"))
        .arg("save")
        .arg(format!("form.{format}"))
        .args(["--patch", "preview.json", "--root"])
        .arg(folder.path())
        .args(arguments)
        .output()
        .unwrap()
}
macro_rules! save_cli_tests {
    ($name:ident,$format:literal) => {
        mod $name {
            use super::*;
            #[test]
            fn test_cli_save_accepts_preview_envelope_and_backup_directory() {
                let (folder, original, _) = setup($format);
                let output = cli(
                    &folder,
                    $format,
                    &["--acknowledge-risks", "--backup-dir", "versions"],
                );
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let result: Value = serde_json::from_slice(&output.stdout).unwrap();
                let bytes = fs::read(folder.path().join(concat!("form.", $format))).unwrap();
                assert_eq!(
                    Document::new(&bytes, "saved").unwrap().nodes[0].text,
                    "Saved Korean text 한글"
                );
                assert_eq!(result["sha256"], sha(&bytes));
                assert_eq!(
                    Path::new(result["backup"].as_str().unwrap()).parent(),
                    Some(Path::new("versions"))
                );
                assert_eq!(
                    fs::read(folder.path().join(result["backup"].as_str().unwrap())).unwrap(),
                    original
                );
                assert_eq!(result["independentCheck"]["opened"], true);
                let stale = cli(&folder, $format, &["--acknowledge-risks"]);
                assert_eq!(stale.status.code(), Some(2));
                assert_eq!(
                    serde_json::from_slice::<Value>(&stale.stderr).unwrap()["error"]["code"],
                    "SOURCE_CHANGED"
                );
            }
            #[test]
            fn test_cli_save_requires_risk_acknowledgement() {
                let (folder, original, _) = setup($format);
                let output = cli(&folder, $format, &[]);
                assert_eq!(output.status.code(), Some(2));
                assert_eq!(
                    serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
                    "RISK_ACK_REQUIRED"
                );
                assert_eq!(
                    fs::read(folder.path().join(concat!("form.", $format))).unwrap(),
                    original
                );
            }
            #[test]
            fn test_real_independent_reader_reopens_saved_output() {
                let (folder, _, _) = setup($format);
                let output = cli(&folder, $format, &["--acknowledge-risks"]);
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let result: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(result["independentCheck"]["opened"], true);
                assert_eq!(result["independentCheck"]["editedTextsFound"], true);
                assert!(result["independentCheck"]["missing"]
                    .as_array()
                    .unwrap()
                    .is_empty());
            }
        }
    };
}
save_cli_tests!(hwpx, "hwpx");
save_cli_tests!(hwp, "hwp");

#[test]
fn test_table_cell_edit_is_found_in_full_text() {
    let (folder, original, _) = setup("hwpx");
    let document = Document::new(&original, "form.hwpx").unwrap();
    let node = document
        .nodes
        .iter()
        .find(|node| node.role == "table-cell")
        .unwrap();
    let text = "Changed cell 😀 — longer text";
    let patch = json!({"schemaVersion":1,"baseSha256":document.hash,"operations":[{"op":"set_text","nodeId":node.id,"expectedText":node.text,"text":text}]});
    fs::write(
        folder.path().join("preview.json"),
        serde_json::to_vec(&patch).unwrap(),
    )
    .unwrap();
    let output = cli(&folder, "hwpx", &["--acknowledge-risks"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["independentCheck"]["editMethods"][&node.id],
        "fullText"
    );
    assert_eq!(result["independentCheck"]["editedTextsFound"], true);
}
