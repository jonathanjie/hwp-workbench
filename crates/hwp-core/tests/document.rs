mod common;
use common::*;
use flate2::Decompress;
use hwp_core::document::Document;
use serde_json::json;
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, ZipWriter};

#[test]
fn test_hwpx_extract_korean() {
    let document = document(&hwpx());
    assert_eq!(document.format, "hwpx");
    assert!(document.nodes.iter().any(|node| node.text == "마리나체인"));
}
#[test]
fn test_hwpx_noop_byte_identical() {
    let bytes = hwpx();
    assert_eq!(document(&bytes).serialize(false).unwrap(), bytes);
}
#[test]
fn test_hwpx_variable_length_save_reopen() {
    let mut document = document(&hwpx());
    edit_first(&mut document, "확정 검토서 — Final");
    assert_eq!(reopened(&document).nodes[0].text, "확정 검토서 — Final");
}
#[test]
fn test_opaque_parts_preserved() {
    let mut document = document(&hwpx());
    edit_first(&mut document, "edited");
    assert_parts_unchanged(&document, &reopened(&document), "Contents/section0.xml");
}
#[test]
fn test_xml_surgical_change() {
    let mut document = document(&hwpx());
    let old = std::str::from_utf8(document.part("Contents/section0.xml").unwrap()).unwrap();
    let expected = old.replace(&document.nodes[0].text, "제목 수정");
    edit_first(&mut document, "제목 수정");
    assert_eq!(
        reopened(&document).part("Contents/section0.xml").unwrap(),
        expected.as_bytes()
    );
}
#[test]
fn test_xml_escapes_and_emoji() {
    let mut document = document(&hwpx());
    let replacement = "<safe> & \"quoted\" 😀";
    edit_first(&mut document, replacement);
    assert_eq!(reopened(&document).nodes[0].text, replacement);
}
#[test]
fn test_empty_selfclosing_node() {
    let mut document = document(&hwpx());
    let node = document
        .nodes
        .iter()
        .find(|node| node.text.is_empty())
        .unwrap()
        .clone();
    document.patch(&patch(&document, &node, "채움")).unwrap();
    assert_eq!(
        reopened(&document)
            .nodes
            .iter()
            .find(|item| item.id == node.id)
            .unwrap()
            .text,
        "채움"
    );
}
#[test]
fn test_clear_text_node() {
    let mut document = document(&hwpx());
    edit_first(&mut document, "");
    assert_eq!(reopened(&document).nodes[0].text, "");
}
#[test]
fn test_multiple_text_nodes() {
    let mut document = document(&hwpx());
    let mut edit = patch(&document, &document.nodes[0], "first edited");
    let second = patch(&document, &document.nodes[2], "둘째");
    edit["operations"]
        .as_array_mut()
        .unwrap()
        .push(second["operations"][0].clone());
    document.patch(&edit).unwrap();
    let result = reopened(&document);
    assert_eq!(result.nodes[0].text, "first edited");
    assert_eq!(result.nodes[2].text, "둘째");
}
#[test]
fn test_table_cell_target() {
    let mut document = document(&hwpx());
    let node = document
        .nodes
        .iter()
        .find(|node| node.role == "table-cell" && node.text == "마리나체인")
        .unwrap()
        .clone();
    document
        .patch(&patch(&document, &node, "MarinaChain"))
        .unwrap();
    assert_eq!(
        reopened(&document)
            .nodes
            .iter()
            .find(|item| item.id == node.id)
            .unwrap()
            .text,
        "MarinaChain"
    );
}
#[test]
fn test_revert_is_noop() {
    let bytes = hwpx();
    let mut document = document(&bytes);
    let mut edit = patch(&document, &document.nodes[0], "edited");
    document.patch(&edit).unwrap();
    edit["operations"][0]["expectedText"] = json!("edited");
    edit["operations"][0]["text"] = json!(document.nodes[0].text);
    document.patch(&edit).unwrap();
    assert_eq!(document.serialize(false).unwrap(), bytes);
}
#[test]
fn test_risk_ack_required() {
    let mut document = document(&hwpx());
    edit_first(&mut document, "edited");
    assert_eq!(
        document.serialize(false).unwrap_err().code,
        "RISK_ACK_REQUIRED"
    );
}
#[test]
fn test_stale_hash() {
    let mut document = document(&hwpx());
    let mut edit = patch(&document, &document.nodes[0], "edited");
    edit["baseSha256"] = json!("wrong");
    assert_eq!(document.patch(&edit).unwrap_err().code, "STALE_DOCUMENT");
}
#[test]
fn test_stale_expected_text() {
    let mut document = document(&hwpx());
    let mut edit = patch(&document, &document.nodes[0], "edited");
    edit["operations"][0]["expectedText"] = json!("wrong");
    assert_eq!(document.patch(&edit).unwrap_err().code, "STALE_TEXT");
}
#[test]
fn test_patch_atomicity() {
    let mut document = document(&hwpx());
    let mut edit = patch(&document, &document.nodes[0], "edited");
    let mut invalid = edit["operations"][0].clone();
    invalid["nodeId"] = json!("bad");
    edit["operations"].as_array_mut().unwrap().push(invalid);
    assert_eq!(document.patch(&edit).unwrap_err().code, "NODE_NOT_FOUND");
    assert_eq!(document.inspect()["pendingChanges"], 0);
}
#[test]
fn test_duplicate_patch_target() {
    let mut document = document(&hwpx());
    let mut edit = patch(&document, &document.nodes[0], "edited");
    let duplicate = edit["operations"][0].clone();
    edit["operations"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(document.patch(&edit).unwrap_err().code, "BAD_PATCH");
}
#[test]
fn test_control_characters_blocked() {
    // A lone surrogate is not a Rust string; U+FFFE tests the same scalar policy.
    for replacement in ["one\ntwo", "a\tb", "\0", "\r", "\u{fffe}"] {
        let mut document = document(&hwpx());
        let edit = patch(&document, &document.nodes[0], replacement);
        assert_eq!(document.patch(&edit).unwrap_err().code, "CONTROL_TEXT");
    }
}
#[test]
fn test_foreign_zip_rejected() {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    archive
        .start_file("test.txt", SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"hi").unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    assert_eq!(
        Document::new(&bytes, "document").err().unwrap().code,
        "NOT_HWPX"
    );
}
#[test]
fn test_xml_inline_line_break_editable() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_xml_nested_element_locked",
        "fixtures.make_hwpx",
    )
    .remove(0);
    let mut document = document(&bytes);
    assert!(document.nodes[0].editable);
    assert!(document.nodes[0].text.contains('\u{fdd0}'));
    let edit = patch(&document, &document.nodes[0], "edited");
    document.patch(&edit).unwrap();
    assert_eq!(reopened(&document).nodes[0].text, "edited");
}
#[test]
fn test_alternate_namespace_prefix() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_alternate_namespace_prefix",
        "fixtures.make_hwpx",
    )
    .remove(0);
    let mut document = document(&bytes);
    edit_first(&mut document, "수정");
    let result = reopened(&document);
    assert_eq!(result.nodes[0].text, "수정");
    assert!(result
        .part("Contents/section0.xml")
        .unwrap()
        .windows(5)
        .any(|bytes| bytes == b"<q:t>"));
}
#[test]
fn test_signature_blocks_editing() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_signature_blocks_editing",
        "fixtures.make_hwpx",
    )
    .remove(0);
    let mut document = document(&bytes);
    assert_eq!(document.inspect()["capabilities"]["saveAs"], false);
    let edit = patch(&document, &document.nodes[0], "edited");
    assert_eq!(document.patch(&edit).unwrap_err().code, "SAVE_BLOCKED");
}
#[test]
fn test_unknown_binary_is_retained() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_unknown_binary_is_retained",
        "fixtures.make_hwpx",
    )
    .remove(0);
    let mut document = document(&bytes);
    edit_first(&mut document, "edited");
    assert_eq!(
        document.part("custom/alien.bin"),
        reopened(&document).part("custom/alien.bin")
    );
}
#[test]
fn test_hwpx_preview_warning() {
    let codes = warning_codes(&document(&hwpx()));
    for code in [
        "LAYOUT_UNVERIFIED",
        "PREVIEW_CACHE_RETAINED",
        "TABLE_LAYOUT",
    ] {
        assert!(codes.iter().any(|item| item == code));
    }
}
#[test]
fn test_hwp_read_and_control_skip() {
    assert_eq!(document(&hwp(true)).nodes[0].text, "마리나체인 검토 중");
}
#[test]
fn test_hwp_sections_in_numeric_order() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_hwp_sections_in_numeric_order",
        "fixtures.cfb_from_streams",
    )
    .remove(0);
    let document = document(&bytes);
    assert_eq!(
        document
            .nodes
            .iter()
            .map(|node| node.part.as_str())
            .collect::<Vec<_>>(),
        [
            "BodyText/Section0",
            "BodyText/Section2",
            "BodyText/Section10"
        ]
    );
}
#[test]
fn test_hwp_noop_byte_identical() {
    for compressed in [true, false] {
        let bytes = hwp(compressed);
        assert_eq!(document(&bytes).serialize(false).unwrap(), bytes);
    }
}
#[test]
fn test_hwp_equal_length_edit_reopen() {
    for compressed in [false, true] {
        let mut document = document(&hwp(compressed));
        edit_first(&mut document, "마리나체인 검토 완");
        let result = reopened(&document);
        assert_eq!(result.nodes[0].text, "마리나체인 검토 완");
        assert_parts_unchanged(&document, &result, "BodyText/Section0");
    }
}
#[test]
fn test_hwp_valid_compression_trailer_opens() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_hwp_valid_compression_trailer_opens",
        "fixtures.cfb_from_streams",
    )
    .remove(0);
    let document = document(&bytes);
    assert_eq!(document.nodes[0].text, "마리나체인 검토 중");
    assert_eq!(document.serialize(false).unwrap(), bytes);
}
#[test]
fn test_hwp_bad_compression_trailer_rejected() {
    for bytes in purpose_fixtures(
        "test_engine.EngineTests.test_hwp_bad_compression_trailer_rejected",
        "fixtures.cfb_from_streams",
    ) {
        assert_eq!(
            Document::new(bytes, "document").err().unwrap().code,
            "LIMIT"
        );
    }
}
#[test]
fn test_hwp_compression_trailer_refreshed_after_edit() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_hwp_compression_trailer_refreshed_after_edit",
        "fixtures.cfb_from_streams",
    )
    .remove(0);
    let mut document = document(&bytes);
    edit_first(&mut document, "마리나체인 검토 완");
    let result = reopened(&document);
    let stream = result.part("BodyText/Section0").unwrap();
    let mut decoder = Decompress::new(false);
    let mut payload = Vec::with_capacity(100_000);
    decoder
        .decompress_vec(stream, &mut payload, flate2::FlushDecompress::Finish)
        .unwrap();
    let trailer = &stream[decoder.total_in() as usize..];
    let mut crc = flate2::Crc::new();
    crc.update(&payload);
    let expected: Vec<u8> = crc
        .sum()
        .to_le_bytes()
        .into_iter()
        .chain((payload.len() as u32).to_le_bytes())
        .collect();
    assert_eq!(trailer, expected);
    assert_eq!(result.nodes[0].text, "마리나체인 검토 완");
    assert_eq!(result.part("DocInfo"), document.part("DocInfo"));
}
#[test]
fn test_hwp_length_change_reopens() {
    let mut document = document(&hwp(true));
    edit_first(&mut document, "짧음");
    assert_eq!(reopened(&document).nodes[0].text, "짧음");
}
#[test]
fn test_hwp_encrypted_blocked() {
    let bytes = fixture(
        "fixtures.make_hwp",
        json!({"compressed": true, "flags": 2, "major": 3, "long": false}),
    );
    assert_eq!(
        Document::new(bytes, "document").err().unwrap().code,
        "PROTECTED_HWP"
    );
}
#[test]
fn test_hwp_distribution_blocked() {
    let bytes = fixture(
        "fixtures.make_hwp",
        json!({"compressed": true, "flags": 4, "major": 3, "long": false}),
    );
    assert_eq!(
        Document::new(bytes, "document").err().unwrap().code,
        "PROTECTED_HWP"
    );
}
#[test]
fn test_hwp_default_scripts_editable_and_preserved() {
    for bytes in purpose_fixtures(
        "test_engine.EngineTests.test_hwp_default_scripts_editable_and_preserved",
        "fixtures.cfb_from_streams",
    ) {
        let mut document = document(&bytes);
        assert_eq!(document.inspect()["capabilities"]["saveAs"], true);
        let inspection = document.inspect();
        let warning = inspection["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|warning| warning["code"] == "DEFAULT_SCRIPT_STUB")
            .unwrap();
        assert_eq!(warning["severity"], "info");
        edit_first(&mut document, "마리나체인 검토 완");
        let result = reopened(&document);
        for name in ["Scripts/DefaultJScript", "Scripts/JScriptVersion"] {
            assert_eq!(document.part(name), result.part(name));
        }
    }
}
#[test]
fn test_hwp_nondefault_scripts_block_editing() {
    for bytes in purpose_fixtures(
        "test_engine.EngineTests.test_hwp_nondefault_scripts_block_editing",
        "fixtures.cfb_from_streams",
    ) {
        let mut document = document(&bytes);
        assert_eq!(document.inspect()["capabilities"]["saveAs"], false);
        assert!(!document.nodes[0].editable);
        let edit = patch(&document, &document.nodes[0], "edited");
        assert_eq!(document.patch(&edit).unwrap_err().code, "SAVE_BLOCKED");
        assert!(!warning_codes(&document)
            .iter()
            .any(|code| code == "DEFAULT_SCRIPT_STUB"));
    }
}
#[test]
fn test_hwpx_default_scripts_editable_and_preserved() {
    let fixtures = purpose_fixtures(
        "test_engine.EngineTests.test_hwpx_default_scripts_editable_and_preserved",
        "fixtures.make_hwpx",
    );
    assert!(!fixtures.is_empty());
    for bytes in fixtures {
        let mut document = document(&bytes);
        assert!(document.nodes[0].editable);
        assert_eq!(document.inspect()["capabilities"]["saveAs"], true);
        let codes = warning_codes(&document);
        assert!(codes.iter().any(|code| code == "DEFAULT_SCRIPT_STUB"));
        assert!(!codes
            .iter()
            .any(|code| code == "PROTECTED_OR_ACTIVE_CONTENT"));
        edit_first(&mut document, "마리나체인 검토 완");
        let result = reopened(&document);
        assert_eq!(result.nodes[0].text, "마리나체인 검토 완");
        for name in ["Scripts/headerScripts.js", "Scripts/sourceScripts.js"] {
            assert_eq!(document.part(name), result.part(name));
        }
    }
}
#[test]
fn test_hwpx_nondefault_scripts_block_editing() {
    let fixtures = purpose_fixtures(
        "test_engine.EngineTests.test_hwpx_nondefault_scripts_block_editing",
        "fixtures.make_hwpx",
    );
    assert_eq!(fixtures.len(), 3);
    for bytes in fixtures {
        let mut document = document(&bytes);
        assert_eq!(document.inspect()["capabilities"]["saveAs"], false);
        assert!(!document.nodes[0].editable);
        let edit = patch(&document, &document.nodes[0], "edited");
        assert_eq!(document.patch(&edit).unwrap_err().code, "SAVE_BLOCKED");
        assert!(!warning_codes(&document)
            .iter()
            .any(|code| code == "DEFAULT_SCRIPT_STUB"));
    }
}
#[test]
fn test_hwpx_default_scripts_do_not_hide_signatures() {
    let fixtures = purpose_fixtures(
        "test_engine.EngineTests.test_hwpx_default_scripts_do_not_hide_signatures",
        "fixtures.make_hwpx",
    );
    assert!(!fixtures.is_empty());
    for bytes in fixtures {
        let mut document = document(&bytes);
        assert_eq!(document.inspect()["capabilities"]["saveAs"], false);
        let edit = patch(&document, &document.nodes[0], "edited");
        assert_eq!(document.patch(&edit).unwrap_err().code, "SAVE_BLOCKED");
    }
}
#[test]
fn test_hwp_signature_blocked() {
    let bytes = fixture(
        "fixtures.make_hwp",
        json!({"compressed": true, "flags": 64, "major": 3, "long": false}),
    );
    assert_eq!(document(&bytes).inspect()["capabilities"]["saveAs"], false);
}
#[test]
fn test_cfb_v4() {
    let bytes = fixture(
        "fixtures.make_hwp",
        json!({"compressed": true, "flags": 0, "major": 4, "long": false}),
    );
    let mut document = document(&bytes);
    edit_first(&mut document, "마리나체인 검토 완");
    assert_eq!(reopened(&document).nodes[0].text, "마리나체인 검토 완");
}
#[test]
fn test_cfb_long_extended_record() {
    let bytes = fixture(
        "fixtures.make_hwp",
        json!({"compressed": false, "flags": 0, "major": 3, "long": true}),
    );
    let mut document = document(&bytes);
    let replacement = document.nodes[0].text.replace("테스트", "검토서");
    edit_first(&mut document, &replacement);
    assert!(reopened(&document).nodes[0].text.starts_with("검토서"));
}
#[test]
fn test_foreign_bytes_rejected() {
    assert_eq!(
        Document::new(b"not a document", "document")
            .err()
            .unwrap()
            .code,
        "UNSUPPORTED_FORMAT"
    );
}

#[test]
fn test_zip_traversal_rejected() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_zip_traversal_rejected",
        "fixtures.make_hwpx",
    )
    .remove(0);
    assert_eq!(
        Document::new(bytes, "document").err().unwrap().code,
        "UNSAFE_ZIP"
    );
}
#[test]
fn test_dtd_rejected() {
    let bytes = purpose_fixtures(
        "test_engine.EngineTests.test_dtd_rejected",
        "fixtures.make_hwpx",
    )
    .remove(0);
    assert_eq!(
        Document::new(bytes, "document").err().unwrap().code,
        "UNSAFE_XML"
    );
}
#[test]
fn test_zip_duplicate_rejected() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for name in ["mimetype", "fakefake"] {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"application/hwp+zip").unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    let locations: Vec<usize> = bytes
        .windows(8)
        .enumerate()
        .filter_map(|(offset, text)| (text == b"fakefake").then_some(offset))
        .collect();
    assert_eq!(locations.len(), 2);
    for offset in locations {
        bytes[offset..offset + 8].copy_from_slice(b"mimetype");
    }
    assert_eq!(
        Document::new(bytes, "document").err().unwrap().code,
        "UNSAFE_ZIP"
    );
}
#[test]
fn test_utf16_entities_are_rejected_in_all_package_parts() {
    let fixtures = purpose_fixtures("test_package_safety.XmlEncodingTests.test_utf16_entities_are_rejected_in_all_package_parts", "fixtures.make_hwpx");
    assert_eq!(
        fixtures.len(),
        20,
        "Both byte orders, with and without BOM, across five XML parts"
    );
    for bytes in fixtures {
        assert_eq!(
            Document::new(bytes, "document").err().unwrap().code,
            "XML_ENCODING"
        );
    }
}
#[test]
fn short_reference_duplicate_and_complete_patch_checks_are_atomic() {
    let mut document = document(&hwpx());
    assert_eq!(
        document.resolve_node_id("s0:t0").unwrap(),
        document.nodes[0].id
    );
    let mut edit = patch(&document, &document.nodes[0], "edited");
    let mut alias = edit["operations"][0].clone();
    alias["nodeId"] = json!("s0:t0");
    edit["operations"].as_array_mut().unwrap().push(alias);
    assert_eq!(document.patch(&edit).unwrap_err().code, "BAD_PATCH");
    assert_eq!(document.inspect()["pendingChanges"], 0);
    let mut edit = patch(&document, &document.nodes[0], "edited");
    edit["operations"][0]["extra"] = json!(true);
    assert_eq!(document.patch(&edit).unwrap_err().code, "BAD_PATCH");
    assert_eq!(document.inspect()["pendingChanges"], 0);
}

#[test]
fn inspect_and_diff_shapes_follow_contract_field_order() {
    for bytes in [hwpx(), hwp(true)] {
        let document = document(&bytes);
        let inspection = document.inspect();
        assert_eq!(
            inspection
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "schemaVersion",
                "format",
                "name",
                "sha256",
                "nodes",
                "warnings",
                "view",
                "fonts",
                "capabilities",
                "parts",
                "pendingChanges",
            ]
        );
        assert_eq!(
            inspection["nodes"][0]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "id",
                "text",
                "part",
                "role",
                "editable",
                "paragraph",
                "style",
                "anchor",
                "paragraphRecordIndex",
                "textStart",
                "textEnd",
            ]
        );
        assert_eq!(
            inspection["nodes"][0]["anchor"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["section", "paragraph", "charOffset", "inCell"]
        );
        assert_eq!(
            document
                .diff()
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["baseSha256", "changes", "warnings", "layoutCertified"]
        );
        assert_eq!(
            inspection["capabilities"],
            json!({"textEdit": true, "lengthChangingText": true, "structuralEdit": false, "formattingEdit": false, "certifiedLayout": false, "saveAs": true})
        );
    }
}
