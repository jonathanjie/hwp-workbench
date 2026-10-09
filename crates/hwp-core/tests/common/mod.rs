#![allow(dead_code)]

use hwp_core::document::{Document, TextNode};
use serde_json::{json, Value};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn manifest() -> Vec<Value> {
    serde_json::from_slice(&std::fs::read(root().join("tests/data/manifest.json")).unwrap())
        .unwrap()
}

pub fn fixture(builder: &str, parameters: Value) -> Vec<u8> {
    let entries = manifest();
    let entry = entries
        .iter()
        .find(|entry| entry["builder"] == builder && entry["parameters"] == parameters)
        .unwrap_or_else(|| panic!("Missing fixture for {builder}: {parameters}"));
    named_fixture(entry["name"].as_str().unwrap())
}

pub fn named_fixture(name: &str) -> Vec<u8> {
    std::fs::read(root().join("tests/data").join(name)).unwrap()
}

pub fn purpose_fixtures(test: &str, builder: &str) -> Vec<Vec<u8>> {
    let paths: Vec<_> = manifest()
        .iter()
        .filter(|entry| {
            // The exporter also records make_hwp's intermediate CFB builds.
            // Those unchanged base containers are not the final custom fixture.
            let name = entry["name"].as_str().unwrap();
            let intermediate_hwp = builder == "fixtures.cfb_from_streams"
                && matches!(
                    name,
                    "fixtures-cfb_from_streams-01b249ab7f677a59.hwp"
                        | "fixtures-cfb_from_streams-6563504cc4fd99b3.hwp"
                );
            entry["builder"] == builder
                && entry["purpose"].as_str().unwrap().contains(test)
                && !intermediate_hwp
        })
        .map(|entry| entry["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(!paths.is_empty(), "Missing fixtures for {test}");
    paths.iter().map(|name| named_fixture(name)).collect()
}

pub fn hwpx() -> Vec<u8> {
    fixture(
        "fixtures.make_hwpx",
        json!({"extra": null, "section": null}),
    )
}

pub fn hwp(compressed: bool) -> Vec<u8> {
    fixture(
        "fixtures.make_hwp",
        json!({"compressed": compressed, "flags": 0, "major": 3, "long": false}),
    )
}

pub fn document(bytes: &[u8]) -> Document {
    Document::new(bytes, "document").unwrap()
}

pub fn patch(document: &Document, node: &TextNode, text: &str) -> Value {
    json!({"schemaVersion": 1, "baseSha256": document.hash, "operations": [
        {"op": "set_text", "nodeId": node.id, "expectedText": node.text, "text": text}
    ]})
}

pub fn edit_first(document: &mut Document, text: &str) {
    let edit = patch(document, &document.nodes[0], text);
    document.patch(&edit).unwrap();
}

pub fn reopened(document: &Document) -> Document {
    self::document(&document.serialize(true).unwrap())
}

pub fn warning_codes(document: &Document) -> Vec<String> {
    document.inspect()["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap().to_owned())
        .collect()
}

pub fn assert_parts_unchanged(before: &Document, after: &Document, edited_part: &str) {
    let inspection = before.inspect();
    let names = inspection["parts"].as_array().unwrap();
    assert_eq!(
        names.len(),
        after.inspect()["parts"].as_array().unwrap().len()
    );
    for part in names {
        let name = part["name"].as_str().unwrap();
        if name != edited_part {
            assert_eq!(
                before.part(name),
                after.part(name),
                "Opaque part changed: {name}"
            );
        }
    }
}

pub fn assert_unchanged_zip_payloads(before: &[u8], after: &[u8], edited_part: &str) {
    let mut original = zip::ZipArchive::new(Cursor::new(before)).unwrap();
    let mut saved = zip::ZipArchive::new(Cursor::new(after)).unwrap();
    assert_eq!(original.len(), saved.len());
    assert_eq!(original.comment(), saved.comment());
    for index in 0..original.len() {
        let mut old = original.by_index_raw(index).unwrap();
        let mut new = saved.by_index_raw(index).unwrap();
        assert_eq!(old.name(), new.name());
        assert_eq!(old.compression(), new.compression());
        assert_eq!(old.last_modified(), new.last_modified());
        assert_eq!(old.comment(), new.comment());
        assert_eq!(old.extra_data(), new.extra_data());
        if old.name() != edited_part {
            let mut old_payload = Vec::new();
            let mut new_payload = Vec::new();
            old.read_to_end(&mut old_payload).unwrap();
            new.read_to_end(&mut new_payload).unwrap();
            assert_eq!(
                old_payload,
                new_payload,
                "Unchanged ZIP entry was recompressed: {}",
                old.name()
            );
        }
    }
}

pub fn assert_body_ranges(bytes: &[u8]) {
    let native = rhwp::document_core::DocumentCore::from_bytes(bytes).unwrap();
    let document = document(bytes);
    for node in &document.nodes {
        if let Some(offset) = node.anchor["charOffset"].as_u64() {
            let actual = native
                .get_text_range_native(
                    node.anchor["section"].as_u64().unwrap() as usize,
                    node.anchor["paragraph"].as_u64().unwrap() as usize,
                    offset as usize,
                    node.text.chars().count(),
                )
                .unwrap();
            assert_eq!(actual, node.text, "Anchor range differs: {}", node.id);
        }
    }
}
