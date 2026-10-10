mod common;
use common::*;
use hwp_core::document::Document;
use serde_json::{json, Value};

#[test]
fn test_empty_runs_do_not_count_as_body_anchor_evidence() {
    let bytes = named_fixture("fixtures-cfb_from_streams-9014551505d498d0.hwp");
    let document = document(&bytes);
    assert_eq!(document.nodes.len(), 1);
    assert!(document.nodes[0].text.is_empty());
    let nonempty_body = document
        .nodes
        .iter()
        .filter(|node| !node.text.is_empty() && !node.anchor["charOffset"].is_null())
        .count();
    assert_eq!(
        nonempty_body, 0,
        "Empty ranges must not supply independent text evidence"
    );
}
#[test]
fn test_empty_body_edit_is_skipped_and_corpus_is_partial() {
    let bytes = named_fixture("fixtures-cfb_from_streams-86e99c060643cf4d.hwp");
    let document = document(&bytes);
    assert!(document.nodes.is_empty());
    assert_eq!(document.serialize(false).unwrap(), bytes);
    eprintln!("SKIPPED edit: No printable body text. No-op identity alone is partial evidence.");
}
#[test]
fn test_length_changing_hwp_checks_save_reopen_and_render() {
    let mut document = document(&hwp(true));
    let original = document.nodes[0].text.clone();
    let replacement = original.clone() + " edited";
    edit_first(&mut document, &replacement);
    let bytes = document.serialize(true).unwrap();
    let saved = common::document(&bytes);
    assert_eq!(saved.nodes[0].text, replacement);
    assert!(replacement.encode_utf16().count() > original.encode_utf16().count());
    assert_parts_unchanged(&document, &saved, "BodyText/Section0");
    assert_rhwp_renders(&bytes, &replacement);
}

fn assert_rhwp_renders(bytes: &[u8], expected: &str) {
    let native = rhwp::document_core::DocumentCore::from_bytes(bytes).unwrap();
    assert!(native.page_count() >= 1);
    assert!(native.render_page_svg_native(0).unwrap().contains("<svg"));
    let text = native.get_text_range_native(0, 0, 0, 200_000).unwrap();
    assert!(text.contains(expected), "Independent native text differs");
}

fn assert_fixture_and_example(fixture_name: &str, example_name: &str, authored: Vec<u8>) {
    let bytes = std::fs::read(root().join("fixtures/generated").join(fixture_name)).unwrap();
    assert_eq!(
        bytes, authored,
        "Regenerate the checked-in authored fixture"
    );
    let example: Value =
        serde_json::from_slice(&std::fs::read(root().join("examples").join(example_name)).unwrap())
            .unwrap();
    let mut document = document(&bytes);
    document.patch(&example).unwrap();
    let saved = document.serialize(true).unwrap();
    let operation = &example["operations"][0];
    let edited = common::document(&saved);
    let node = edited
        .nodes
        .iter()
        .find(|node| node.id == operation["nodeId"].as_str().unwrap())
        .unwrap();
    assert_eq!(node.text, operation["text"].as_str().unwrap());
    assert_rhwp_renders(&bytes, operation["expectedText"].as_str().unwrap());
    assert_rhwp_renders(&saved, operation["text"].as_str().unwrap());
}
#[test]
fn test_authored_hwp_opens_and_renders_in_rhwp() {
    assert_fixture_and_example("legacy-review.hwp", "legacy.patch.json", hwp(true));
}
#[test]
fn test_authored_hwpx_opens_and_renders_in_rhwp() {
    assert_fixture_and_example("review-form.hwpx", "review.patch.json", hwpx());
}

// Frozen contract oracle classifications, recorded before the Rust reader existed.
// Rejected and unsupported fixtures are separately reported, never counted as edits.
fn oracle_classification() -> Value {
    serde_json::from_str(ORACLE_CLASSIFICATION).unwrap()
}

const ORACLE_CLASSIFICATION: &str = r#"{
  "errors": {
    "tests/data/convert_fixtures-docx-8de239670435ea1e.docx": "UNSAFE_ZIP",
    "tests/data/convert_fixtures-package-283c4fcceb08ce42.docx": "LIMIT",
    "tests/data/convert_fixtures-package-3c7cf6fcf255c8ab.docx": "LIMIT",
    "tests/data/convert_fixtures-package-ac4ed760d443b185.docx": "UNSAFE_ZIP",
    "tests/data/fixtures-cfb_from_streams-177d1cb5d262fc88.hwp": "PROTECTED_HWP",
    "tests/data/fixtures-cfb_from_streams-30afb41d9fdf4026.hwp": "LIMIT",
    "tests/data/fixtures-cfb_from_streams-35842c149daf08ae.hwp": "LIMIT",
    "tests/data/fixtures-cfb_from_streams-4ff326636825970b.hwp": "NOT_HWP",
    "tests/data/fixtures-cfb_from_streams-a50bee1d17b6844c.hwp": "LIMIT",
    "tests/data/fixtures-cfb_from_streams-b43aa59398652e00.hwp": "NOT_HWP",
    "tests/data/fixtures-cfb_from_streams-c78914b2f02160e8.hwp": "LIMIT",
    "tests/data/fixtures-cfb_from_streams-cd152fdf7c6367c3.hwp": "PROTECTED_HWP",
    "tests/data/fixtures-make_hwp-0d58fd4194b90732.hwp": "PROTECTED_HWP",
    "tests/data/fixtures-make_hwp-139777636d3de7d6.hwp": "PROTECTED_HWP",
    "tests/data/fixtures-make_hwpx-09e4cc42b43c2311.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-27abad450402c7b2.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-3079efd7bfebbe6e.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-33074ea345a328f0.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-37a57cfa9dedae74.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-3b01ffad21742ad9.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-4a049a343ca5f1e8.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-5384ff9ac4652618.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-5a46d7ca763154da.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-67361cec9731b3de.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-9062a2fc0161723a.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-95c3f89bd73ed204.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-995581b2a07793cd.hwpx": "UNSAFE_XML",
    "tests/data/fixtures-make_hwpx-9f323b57db1a351d.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-9fee0bb0fa88d0f2.hwpx": "LIMIT",
    "tests/data/fixtures-make_hwpx-a3c21eafb11982f8.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-a9b586ba3ad6d896.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-a9ece02b88700d0a.hwpx": "UNSAFE_ZIP",
    "tests/data/fixtures-make_hwpx-a9f1dee4606cb1b7.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-b327059328d3a5e9.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-d8aff7a61c3ab954.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-e831d848cfb45abd.hwpx": "XML_ENCODING",
    "tests/data/fixtures-make_hwpx-fe10e90758e9f9e8.hwpx": "XML_ENCODING"
  },
  "blocked": [
    "tests/data/fixtures-cfb_from_streams-0a8419cf19d4610b.hwp",
    "tests/data/fixtures-cfb_from_streams-20ee4c5b88bb3477.hwp",
    "tests/data/fixtures-cfb_from_streams-479fe38c7b25980e.hwp",
    "tests/data/fixtures-cfb_from_streams-93775cdb8ed6afd3.hwp",
    "tests/data/fixtures-cfb_from_streams-ce585a6a9a4b67dd.hwp",
    "tests/data/fixtures-make_hwp-16cbeb21ba91e881.hwp",
    "tests/data/fixtures-make_hwpx-08d7bef8f0c02364.hwpx",
    "tests/data/fixtures-make_hwpx-98c9e7fdc495ecbc.hwpx",
    "tests/data/fixtures-make_hwpx-b3ad48f9a1565dd4.hwpx",
    "tests/data/fixtures-make_hwpx-b3fd27938f0e79ef.hwpx",
    "tests/data/fixtures-make_hwpx-fdb124f5b91fea95.hwpx"
  ],
  "empty": [
    "tests/data/fixtures-cfb_from_streams-86e99c060643cf4d.hwp",
    "tests/data/fixtures-make_hwpx-f92a949cb17d61c8.hwpx"
  ],
  "editErrors": {
    "tests/data/fixtures-cfb_from_streams-1e6ce8ba92414de3.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-3f41d36e3a2d4d94.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-4b0c2740ad7b6f1c.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-4ba6521afe101bd9.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-584d17d74c9c670f.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-9014551505d498d0.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-9c8bbd6d55c59063.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-a8eceb273b5a9b71.hwp": "UNSUPPORTED_ELEMENT",
    "tests/data/fixtures-cfb_from_streams-b6681fbf5cf9a065.hwp": "UNSUPPORTED_ELEMENT"
  },
  "publicHashes": {
    "fixtures/public/pyhwp-aligns.hwp": "b101b4faff3c65f88ac9f8f583219cf5825d0747be85dbe45174dcd84207a3d1",
    "fixtures/public/pyhwp-borderfill.hwp": "49dee926351d25dcfa3ec448c8955426623bbc68d4a2ee7f1575691e8fd352a6",
    "fixtures/public/hwp-cli-report-tables.hwpx": "b304a203d977e3d32a33b2fea5ce7c5b01d0af74916fb30c60d96f355034de24"
  }
}"#;

#[test]
fn every_document_fixture_preserves_untouched_text_and_payloads() {
    let oracle = oracle_classification();
    let mut paths = Vec::new();
    for directory in ["tests/data", "fixtures/generated", "fixtures/public"] {
        let folder = root().join(directory);
        if directory == "fixtures/public" && !folder.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("hwp" | "hwpx" | "docx")
            ) {
                paths.push(path);
            }
        }
    }
    let public: Value = serde_json::from_slice(
        &std::fs::read(root().join("fixtures/public-manifest.json")).unwrap(),
    )
    .unwrap();
    for item in public["fixtures"].as_array().unwrap() {
        let name = item["name"].as_str().unwrap();
        if !root().join("fixtures/public").join(name).is_file() {
            eprintln!("SKIPPED: Public fixture not downloaded: {name}");
        }
    }
    paths.sort();
    assert!(!paths.is_empty());
    let mut edited_count = 0;
    let mut rejected_count = 0;
    let mut unsupported_count = 0;
    for path in paths {
        let relative = path.strip_prefix(root()).unwrap().to_str().unwrap();
        let source = std::fs::read(&path).unwrap();
        if let Some(expected_hash) = oracle["publicHashes"][relative].as_str() {
            // These SHA-256 values were captured only after verifying the manifest's Git blob SHA-1 and size.
            assert_eq!(
                hwp_core::document::sha(&source),
                expected_hash,
                "Pinned public bytes changed: {relative}"
            );
        }
        let expected_error = oracle["errors"][relative].as_str().or_else(|| {
            (path.extension().and_then(|extension| extension.to_str()) == Some("docx"))
                .then_some("NOT_HWPX")
        });
        let parsed = Document::new(&source, path.file_name().unwrap().to_str().unwrap());
        if let Some(expected) = expected_error {
            let error = parsed.err().unwrap_or_else(|| {
                panic!("Previously rejected fixture unexpectedly opened: {relative}")
            });
            assert_eq!(error.code, expected, "Rejection changed: {relative}");
            rejected_count += 1;
            eprintln!("REJECTED: {relative}: {}: {}", error.code, error.message);
            continue;
        }
        let mut document = parsed.unwrap_or_else(|error| {
            panic!("Supported fixture failed to open: {relative}: {error}")
        });
        let inspection = document.inspect();
        assert_eq!(inspection["format"], document.format);
        assert_eq!(
            document.serialize(false).unwrap(),
            source,
            "No-op identity: {relative}"
        );
        let Some(node) = document.nodes.iter().find(|node| node.editable).cloned() else {
            assert!(
                oracle["blocked"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(relative))
                    || oracle["empty"]
                        .as_array()
                        .unwrap()
                        .contains(&json!(relative)),
                "Unexpectedly no editable nodes: {relative}"
            );
            unsupported_count += 1;
            eprintln!("SKIPPED EDIT: {relative}: No editable text node");
            continue;
        };
        let first = node.text.chars().next();
        // Preserve UTF-16 width so this sweep compares one-character edits.
        let replacement_char = match first {
            Some('가') => '나',
            Some('😀') => '😁',
            Some(character) if character.len_utf16() == 2 => '😀',
            _ => '가',
        };
        let replacement =
            replacement_char.to_string() + &node.text.chars().skip(1).collect::<String>();
        let edit = patch(&document, &node, &replacement);
        let result = document.patch(&edit);
        if let Some(expected) = oracle["editErrors"][relative].as_str() {
            assert_eq!(
                result.unwrap_err().code,
                expected,
                "Edit rejection changed: {relative}"
            );
            unsupported_count += 1;
            eprintln!("UNSUPPORTED EDIT: {relative}: {expected}");
            continue;
        }
        result.unwrap_or_else(|error| panic!("Supported edit failed: {relative}: {error}"));
        let output = document
            .serialize(true)
            .unwrap_or_else(|error| panic!("Save failed: {relative}: {error}"));
        let saved = Document::new(&output, "saved").unwrap();
        assert_eq!(
            document.nodes.len(),
            saved.nodes.len(),
            "Node inventory changed: {relative}"
        );
        for (before, after) in document.nodes.iter().zip(&saved.nodes) {
            assert_eq!(before.id, after.id, "Node address changed: {relative}");
            assert_eq!(before.part, after.part);
            assert_eq!(before.role, after.role);
            let expected = if before.id == node.id {
                replacement.as_str()
            } else {
                before.text.as_str()
            };
            assert_eq!(
                after.text, expected,
                "Unexpected text change: {relative}: {}",
                before.id
            );
        }
        assert_parts_unchanged(&document, &saved, &node.part);
        if document.format == "hwpx" {
            assert_unchanged_zip_payloads(&source, &output, &node.part);
        } else {
            let before = &document.hwp_sections[&node.part];
            let after = &saved.hwp_sections[&node.part];
            assert_eq!(before.records.len(), after.records.len());
            let record_index = node
                .id
                .split_once("#record:")
                .unwrap()
                .1
                .split_once(":text:")
                .unwrap()
                .0
                .parse::<usize>()
                .unwrap();
            let length_changed =
                replacement.encode_utf16().count() != node.text.encode_utf16().count();
            for (old, new) in before.records.iter().zip(&after.records) {
                assert_eq!(old.tag, new.tag);
                assert_eq!(old.level, new.level);
                if old.index == record_index {
                    let base = before.text_offsets[&record_index];
                    let start = (node.text_start.unwrap() - base) * 2;
                    let end = (node.text_end.unwrap() - base) * 2;
                    let utf16: Vec<u8> = replacement
                        .encode_utf16()
                        .flat_map(u16::to_le_bytes)
                        .collect();
                    let mut expected = old.data.clone();
                    drop(expected.splice(start..end, utf16));
                    assert_eq!(
                        new.data, expected,
                        "Native controls or surrounding text changed: {relative}"
                    );
                }
                let same_paragraph = Some(old.index) == node.paragraph_record_index
                    || before.owners.get(&old.index).copied() == node.paragraph_record_index;
                let dependent_metadata =
                    length_changed && same_paragraph && matches!(old.tag, 66 | 68..=70);
                if old.index != record_index && !dependent_metadata {
                    assert_eq!(
                        old.original, new.original,
                        "Opaque record changed: {relative}: {}",
                        old.index
                    );
                }
            }
        }
        edited_count += 1;
        eprintln!("EDIT VERIFIED: {relative}");
    }
    assert!(edited_count > 0, "No successful edits were verified");
    eprintln!("Corpus sweep: {edited_count} edits verified; {rejected_count} rejected documents; {unsupported_count} unsupported edits. Hancom layout remains unverified.");
}
