use std::collections::BTreeMap;
use std::io::Cursor;

use base64::prelude::*;
use hwp_verify::{
    check_output, export, invoke, probe, render_page_svg, worker::run_worker, WorkerCommand,
};
use serde_json::{json, Value};

fn worker() -> WorkerCommand {
    WorkerCommand::new(env!("CARGO_BIN_EXE_hwp-verify-worker"), Vec::new())
}

fn fixture() -> &'static [u8] {
    include_bytes!("../../../fixtures/generated/review-form.hwpx")
}

fn frame(request: Value, payload: &[u8]) -> Vec<u8> {
    let encoded = serde_json::to_vec(&request).unwrap();
    let mut framed = (encoded.len() as u32).to_le_bytes().to_vec();
    framed.extend(encoded);
    framed.extend(payload);
    framed
}

fn direct(frame: &[u8]) -> Value {
    let mut output = Vec::new();
    run_worker(Cursor::new(frame), &mut output).unwrap();
    serde_json::from_slice(&output).unwrap()
}

fn fake(result: Value) -> WorkerCommand {
    let encoded = serde_json::to_string(&result).unwrap();
    WorkerCommand::new(
        "node",
        vec![
            "-e".into(),
            format!(
                "process.stdin.resume(); process.stdin.on('end', () => console.log({encoded:?}))"
            )
            .into(),
        ],
    )
}

#[test]
fn test_truncated_frame_is_json_error() {
    assert_eq!(
        direct(b"\x04\x00"),
        json!({"error": {"code": "BAD_REQUEST", "message": "Missing JSON length header."}})
    );
}

#[test]
fn test_path_argument_is_rejected() {
    assert_eq!(
        direct(&frame(
            json!({"op": "probe", "path": "/etc/passwd"}),
            fixture()
        ))["error"]["code"],
        "BAD_REQUEST"
    );
}

#[test]
fn test_frame_retains_binary_document_and_unicode() {
    let observed = probe(fixture(), &worker()).unwrap();
    assert_eq!(observed["version"], "0.8.7");
    assert_eq!(
        observed["sections"][0]["paragraphs"][0]["text"],
        "내부 업무 검토서"
    );
}

#[test]
fn test_bridge_errors_are_workbench_errors() {
    assert_eq!(
        probe(b"not a document", &worker()).unwrap_err().code,
        "RHWP_FAILED"
    );
}

#[test]
fn test_apply_reports_confirmed_text() {
    let observed = json!({"version": "0.8.7", "pageCount": 1, "sections": [{"paragraphs": [{"text": "Changed 😀"}]}]});
    let edits = BTreeMap::from([("node".into(), "Changed 😀".into())]);
    assert_eq!(
        check_output(fixture(), &edits, &fake(observed)),
        json!({"engine": "rhwp 0.8.7", "opened": true, "pageCount": 1, "editedTextsFound": true, "missing": [], "editMethods": {"node": "paragraphs"}})
    );
}

#[test]
fn test_empty_replacement_cannot_be_confirmed_by_substring() {
    let observed = json!({"version": "0.8.7", "pageCount": 1, "sections": [{"paragraphs": [{"text": "Original text was not deleted"}]}], "fullText": "Original text was not deleted"});
    let edits = BTreeMap::from([("node".into(), "".into())]);
    assert_eq!(
        check_output(fixture(), &edits, &fake(observed)),
        json!({"engine": "rhwp 0.8.7", "opened": true, "pageCount": 1, "editedTextsFound": false, "missing": [], "unverified": ["node"], "editMethods": {"node": "unverified-empty"}})
    );
}

#[test]
fn test_apply_reports_missing_text() {
    let observed = json!({"version": "0.8.7", "pageCount": 1, "sections": []});
    let edits = BTreeMap::from([("node".into(), "Changed 😀".into())]);
    let result = check_output(fixture(), &edits, &fake(observed));
    assert_eq!(result["editedTextsFound"], false);
    assert_eq!(result["missing"], json!(["node"]));
}

#[test]
fn test_apply_reports_unavailable_parser() {
    let observed = json!({"available": false, "reason": "Verification worker is unavailable."});
    assert_eq!(
        check_output(fixture(), &BTreeMap::new(), &fake(observed.clone())),
        observed
    );
}

#[test]
fn test_independent_failure_does_not_block_verified_save() {
    let observed = json!({"error": {"code": "RHWP_FAILED", "message": "Cannot open."}});
    assert_eq!(
        check_output(fixture(), &BTreeMap::new(), &fake(observed)),
        json!({"engine": "rhwp 0.8.7", "opened": false, "error": {"code": "RHWP_FAILED", "message": "Cannot open.", "hint": "Check the request and compatibility limits; report the code if the problem continues.", "retryable": false}})
    );
}

#[test]
fn test_render_returns_requested_svg() {
    let result = render_page_svg(fixture(), 0, &worker()).unwrap();
    assert_eq!(result["page"], 0);
    assert!(result["svg"].as_str().unwrap().contains("<svg"));
}

#[test]
fn test_out_of_range_page_fails() {
    assert_eq!(
        render_page_svg(fixture(), 99999, &worker())
            .unwrap_err()
            .code,
        "BAD_REQUEST"
    );
}

#[test]
fn test_exports_have_loss_report_and_native_bytes() {
    for format in ["hwp", "hwpx"] {
        let result = export(fixture(), format, &worker()).unwrap();
        assert_eq!(result["format"], format);
        assert!(result["lossReport"].is_object());
        let bytes = BASE64_STANDARD
            .decode(result["bytesBase64"].as_str().unwrap())
            .unwrap();
        assert!(bytes.starts_with(if format == "hwp" {
            b"\xd0\xcf\x11\xe0"
        } else {
            b"PK\x03\x04"
        }));
        assert!(
            probe(&bytes, &worker()).unwrap()["pageCount"]
                .as_u64()
                .unwrap()
                >= 1
        );
    }
}

#[test]
fn test_table_cell_edit_is_found_in_full_text() {
    let mut document = hwp_core::document::Document::new(fixture(), "form.hwpx").unwrap();
    let node = document
        .nodes
        .iter()
        .find(|node| node.role == "table-cell")
        .unwrap()
        .clone();
    let replacement = "Changed cell 😀 — longer text";
    document.patch(&json!({"schemaVersion": 1, "baseSha256": document.hash, "operations": [
        {"op": "set_text", "nodeId": node.id, "expectedText": node.text, "text": replacement}
    ]})).unwrap();
    let output = document.serialize(true).unwrap();
    let observed = probe(&output, &worker()).unwrap();
    for section in observed["sections"].as_array().unwrap() {
        for paragraph in section["paragraphs"].as_array().unwrap() {
            assert!(!paragraph["text"].as_str().unwrap().contains(replacement));
        }
    }
    assert!(observed["fullText"].as_str().unwrap().contains(replacement));
    let edits = BTreeMap::from([("node".into(), replacement.into())]);
    let check = check_output(&output, &edits, &worker());
    assert_eq!(check["editedTextsFound"], true);
    assert_eq!(check["editMethods"], json!({"node": "fullText"}));
}

#[test]
fn test_declared_json_length_is_bounded() {
    assert_eq!(
        direct(&4097_u32.to_le_bytes())["error"]["code"],
        "BAD_REQUEST"
    );
}

#[test]
fn rejects_empty_input_and_wrong_operation_fields() {
    assert_eq!(probe(b"", &worker()).unwrap_err().code, "LIMIT");
    assert_eq!(
        invoke(fixture(), "probe", Some(0), None, &worker())
            .unwrap_err()
            .code,
        "BAD_REQUEST"
    );
    assert_eq!(
        export(fixture(), "docx", &worker()).unwrap_err().code,
        "BAD_REQUEST"
    );
}

#[test]
fn malformed_probe_is_an_unopened_report() {
    let result = check_output(fixture(), &BTreeMap::new(), &fake(json!({})));
    assert_eq!(result["opened"], false);
    assert_eq!(result["error"]["code"], "INDEPENDENT_PROTOCOL");
}

#[test]
fn input_failure_has_the_oracles_recovery_fields() {
    let result = check_output(b"", &BTreeMap::new(), &worker());
    assert_eq!(
        result["error"],
        json!({"code": "LIMIT", "message": "Independent parser input must be between 1 byte and 64 MiB.", "hint": "Reduce the file, request, pattern or result size to the documented limit.", "retryable": true})
    );
}

#[test]
fn report_retains_the_callers_edit_order() {
    let edits = [("z".into(), "".into()), ("a".into(), "".into())];
    let observed = json!({"version": "0.8.7", "pageCount": 1, "sections": []});
    let result = check_output(
        fixture(),
        edits.iter().map(|(id, text)| (id, text)),
        &fake(observed),
    );
    assert_eq!(result["unverified"], json!(["z", "a"]));
}
