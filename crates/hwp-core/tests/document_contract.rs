mod common;
use common::{document, hwpx, patch};
use hwp_core::document::{check_text, local, patch_input, sha, Document};
use hwp_core::package_io::ZipPackage;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

#[test]
fn text_validation_uses_unicode_scalars_and_exact_limits() {
    assert!(check_text(&json!("😀".repeat(200_000))).is_ok());
    let error = check_text(&json!("가".repeat(200_001))).unwrap_err();
    assert_eq!(error.code, "BAD_PATCH");
    assert_eq!(
        error.message,
        "Text must be a string of at most 200,000 characters."
    );
    for value in [Value::Null, json!(true), json!(7), json!([])] {
        assert_eq!(check_text(&value).unwrap_err().code, "BAD_PATCH");
    }
    for value in ["\t", "\n", "\r", "\0", "\u{fffe}", "\u{ffff}"] {
        assert_eq!(check_text(&json!(value)).unwrap_err().code, "CONTROL_TEXT");
    }
    for value in ["", " ", "\u{d7ff}", "\u{e000}", "\u{fffd}", "\u{10ffff}"] {
        assert!(check_text(&json!(value)).is_ok());
    }
}

#[test]
fn patch_envelopes_and_scalar_helpers_match_the_oracle() {
    let native = json!({"schemaVersion":1,"patch":{"ignored":true}});
    assert_eq!(patch_input(&native), &native);
    let envelope = json!({"patch":{"operations":[]},"warnings":[]});
    assert_eq!(patch_input(&envelope), &envelope["patch"]);
    assert_eq!(patch_input(&json!({"patch":null})), &Value::Null);
    assert_eq!(patch_input(&json!([1, 2])), &json!([1, 2]));
    assert_eq!(
        sha(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(local("{namespace}name"), "name");
    assert_eq!(local("plain"), "plain");
}

#[test]
fn schema_version_keeps_contract_numeric_equality() {
    for version in [json!(true), json!(1.0)] {
        let mut doc = document(&hwpx());
        let mut change = patch(&doc, &doc.nodes[0], "changed");
        change["schemaVersion"] = version;
        assert!(doc.patch(&change).is_ok());
    }
}

fn rewrite(changed: BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let bytes = hwpx();
    ZipPackage::new(&bytes).unwrap().rewrite(&changed).unwrap()
}

#[test]
fn mimetype_whitespace_includes_vertical_tab() {
    let bytes = rewrite(BTreeMap::from([(
        "mimetype".into(),
        b"\x0bapplication/hwp+zip\x0b".to_vec(),
    )]));
    assert_eq!(Document::new(&bytes, "document").unwrap().format, "hwpx");
}

#[test]
fn numerically_equal_section_names_keep_archive_order() {
    let bytes = hwpx();
    let mut package = ZipPackage::new(&bytes).unwrap();
    let names: Vec<String> = package.names().map(str::to_owned).collect();
    let source = document(&bytes);
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive
        .start_file("Contents/section00.xml", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(source.part("Contents/section0.xml").unwrap())
        .unwrap();
    let mut total = 0;
    for (index, name) in names.iter().enumerate() {
        let payload = package
            .read_member(index, total, 32 * 1024 * 1024, 128 * 1024 * 1024)
            .unwrap();
        total += payload.len();
        archive
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&payload).unwrap();
    }
    let output = archive.finish().unwrap().into_inner();
    let doc = Document::new(output, "document").unwrap();
    assert_eq!(doc.nodes.first().unwrap().part, "Contents/section00.xml");
    assert!(doc
        .resolve_node_id("s0:t0")
        .unwrap()
        .starts_with("Contents/section00.xml#"));
}
