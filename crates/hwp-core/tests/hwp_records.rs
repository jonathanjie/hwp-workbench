use hwp_core::hwp_records::{parse_section, rewrite_section, TextPosition};

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn record(tag: u32, level: u32, data: &[u8]) -> Vec<u8> {
    let size = data.len();
    let mut output = (tag | (level << 10) | ((size.min(4095) as u32) << 20))
        .to_le_bytes()
        .to_vec();
    if size >= 4095 {
        output.extend_from_slice(&(size as u32).to_le_bytes());
    }
    output.extend_from_slice(data);
    output
}

fn paragraph(text: &str, shapes: &[(u32, u32)]) -> Vec<u8> {
    let payload = utf16(&format!("{text}\r"));
    let mut header = Vec::new();
    header.extend_from_slice(&((payload.len() as u32 / 2) | 0x80000000).to_le_bytes());
    header.extend_from_slice(&0x12345678_u32.to_le_bytes());
    header.extend_from_slice(&7_u16.to_le_bytes());
    header.extend_from_slice(&[8, 9]);
    header.extend_from_slice(&(shapes.len() as u16).to_le_bytes());
    header.extend_from_slice(&1_u16.to_le_bytes());
    header.extend_from_slice(&3_u16.to_le_bytes());
    header.extend_from_slice(&0xabcdef01_u32.to_le_bytes());
    header.extend_from_slice(b"EXTRA");
    let units = text.encode_utf16().count() as u32;
    let lines: Vec<u8> = [0, 3.min(units), 6.min(units)]
        .into_iter()
        .flat_map(|start| {
            let mut line = start.to_le_bytes().to_vec();
            line.extend_from_slice(&[start as u8 + 1; 32]);
            line
        })
        .collect();
    let ranges: Vec<u8> = [2.min(units), 6.min(units + 1), 0xfeedbeef]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    [
        record(66, 0, &header),
        record(67, 1, &payload),
        record(
            68,
            1,
            &shapes
                .iter()
                .flat_map(|(start, shape)| [*start, *shape].into_iter().flat_map(u32::to_le_bytes))
                .collect::<Vec<_>>(),
        ),
        record(69, 1, &lines),
        record(70, 1, &ranges),
        record(99, 1, b"opaque child"),
    ]
    .concat()
}

fn position(raw: &[u8], start: usize, end: usize) -> TextPosition {
    let section = parse_section(raw).unwrap();
    let payload_start = section.records[1].payload_start;
    TextPosition {
        paragraph_record_index: Some(0),
        record: 1,
        start: payload_start + start * 2,
        end: payload_start + end * 2,
        text_start: start,
        text_end: end,
    }
}

fn words(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

#[test]
fn test_grow_updates_metadata_and_preserves_other_fields() {
    let mut document = document_with_paragraph("abcdef", &[(0, 10), (2, 20), (4, 30), (6, 40)]);
    assert_eq!(
        document.inspect()["capabilities"]["lengthChangingText"],
        true
    );
    set_text(&mut document, 0, "ABCDEFGHIJ");
    let saved = saved_document(&document);
    assert_eq!(saved.nodes[0].text, "ABCDEFGHIJ");
    let inspection = saved.inspect();
    let warnings = inspection["warnings"].as_array().unwrap();
    assert!(warnings
        .iter()
        .any(|warning| warning["code"] == "HWP_REFLOW_UNVERIFIED"));
    assert!(!warnings
        .iter()
        .any(|warning| warning["code"] == "HWP_EQUAL_LENGTH_ONLY"));
    let raw = paragraph("abcdef", &[(0, 10), (2, 20), (4, 30), (6, 40)]);
    let old = parse_section(&raw).unwrap();
    let output = rewrite_section(&old, &[(position(&raw, 0, 6), utf16("ABCDEFGHIJ"))]).unwrap();
    let new = parse_section(&output).unwrap();
    assert_eq!(words(&new.records[0].data[..4]), vec![0x8000000b]);
    assert_eq!(&new.records[0].data[4..], &old.records[0].data[4..]);
    assert_eq!(
        words(&new.records[2].data),
        vec![0, 10, 2, 20, 4, 30, 10, 40]
    );
    assert_eq!(words(&new.records[4].data), vec![2, 10, 0xfeedbeef]);
    for (offset, expected) in [(0, 0), (36, 3), (72, 10)] {
        assert_eq!(
            words(&new.records[3].data[offset..offset + 4]),
            vec![expected]
        );
        assert_eq!(
            &new.records[3].data[offset + 4..offset + 36],
            &old.records[3].data[offset + 4..offset + 36]
        );
    }
    assert_eq!(new.records[5].original, old.records[5].original);
}

#[test]
fn test_deletion_keeps_surviving_control_and_suffix_shapes() {
    let cases = [
        ("ab\ncd", vec![(0, 10), (2, 20)], 0, 2, vec![0, 20]),
        (
            "xy\nab\ncd",
            vec![(0, 10), (3, 20), (5, 30)],
            3,
            5,
            vec![0, 10, 3, 30],
        ),
        (
            "xy\nab",
            vec![(0, 10), (3, 20), (5, 30)],
            3,
            5,
            vec![0, 10, 3, 30],
        ),
        (
            "ab\ncd",
            vec![(0, 10), (2, 20), (2, 30)],
            0,
            2,
            vec![0, 20, 0, 30],
        ),
        (
            "ab\ncd",
            vec![(0, 10), (0, 11), (2, 20)],
            0,
            2,
            vec![0, 10, 0, 11, 0, 20],
        ),
    ];
    for (text, shapes, start, end, expected) in cases {
        let raw = paragraph(text, &shapes);
        let section = parse_section(&raw).unwrap();
        let output = rewrite_section(&section, &[(position(&raw, start, end), vec![])]).unwrap();
        let saved = parse_section(&output).unwrap();
        assert_eq!(words(&saved.records[2].data), expected);
        assert_eq!(
            saved.records[1].data,
            utf16(&format!("{}{}\r", &text[..start], &text[end..]))
        );
    }
}

#[test]
fn test_shrink_keeps_shape_of_surviving_terminator() {
    let raw = paragraph("abcdef", &[(0, 10), (2, 20), (4, 30), (6, 40)]);
    let section = parse_section(&raw).unwrap();
    let output = rewrite_section(&section, &[(position(&raw, 0, 6), utf16("XY"))]).unwrap();
    let saved = parse_section(&output).unwrap();
    assert_eq!(words(&saved.records[2].data), vec![0, 10, 2, 40]);
    assert_eq!(
        u16::from_le_bytes(saved.records[0].data[12..14].try_into().unwrap()),
        2
    );
    assert_eq!(words(&saved.records[4].data), vec![2, 2, 0xfeedbeef]);
    assert_eq!(
        &saved.records[0].data[4..12],
        &section.records[0].data[4..12]
    );
    assert_eq!(&saved.records[0].data[14..], &section.records[0].data[14..]);
}

#[test]
fn test_two_edits_compose_across_runs_and_shape_boundaries() {
    let raw = paragraph("ab\ncd", &[(0, 10), (1, 20), (2, 30), (3, 40), (5, 50)]);
    let section = parse_section(&raw).unwrap();
    let output = rewrite_section(
        &section,
        &[
            (position(&raw, 0, 2), utf16("ABCDE")),
            (position(&raw, 3, 5), utf16("Z")),
        ],
    )
    .unwrap();
    let saved = parse_section(&output).unwrap();
    assert_eq!(saved.records[1].data, utf16("ABCDE\nZ\r"));
    assert_eq!(
        words(&saved.records[2].data),
        vec![0, 10, 1, 20, 5, 30, 6, 40, 7, 50]
    );
    assert_eq!(words(&saved.records[4].data), vec![5, 8, 0xfeedbeef]);
}

#[test]
fn test_existing_duplicate_shape_starts_survive_when_not_collapsed() {
    let raw = paragraph("abcdef", &[(0, 10), (0, 20), (4, 30), (6, 40)]);
    let output = rewrite_section(
        &parse_section(&raw).unwrap(),
        &[(position(&raw, 0, 6), utf16("ABCDEFGHIJ"))],
    )
    .unwrap();
    assert_eq!(
        words(&parse_section(&output).unwrap().records[2].data),
        vec![0, 10, 0, 20, 4, 30, 10, 40]
    );
}

#[test]
fn test_edit_limit_is_checked_before_large_record_allocation() {
    let raw = paragraph("abcdef", &[(0, 10)]);
    let section = parse_section(&raw).unwrap();
    let error = hwp_core::hwp_records::rewrite_section_with_limit(
        &section,
        &[(position(&raw, 0, 6), utf16(&"X".repeat(1000)))],
        100,
    )
    .unwrap_err();
    assert_eq!(error.code, "LIMIT");
    assert_eq!(
        error.message,
        "Edited HWP record or section exceeds 32 MiB."
    );
}

#[test]
fn malformed_record_headers_fail_with_exact_errors() {
    for (raw, message) in [
        (vec![0; 3], "Truncated record header."),
        (
            (4095_u32 << 20).to_le_bytes().to_vec(),
            "Truncated extended record header.",
        ),
        (
            (1_u32 << 20).to_le_bytes().to_vec(),
            "Record payload exceeds stream bounds.",
        ),
    ] {
        let error = parse_section(&raw).unwrap_err();
        assert_eq!(error.code, "BAD_HWP_RECORD");
        assert_eq!(error.message, message);
    }
}

#[test]
fn duplicate_shape_groups_across_records_survive_collapse() {
    let mut raw = paragraph("ab\ncd", &[(0, 10), (0, 11)]);
    let mut section = parse_section(&raw).unwrap();
    // Split the duplicate group across native character shape records.
    section.records[0].data[12..14].copy_from_slice(&3_u16.to_le_bytes());
    raw = [
        record(66, 0, &section.records[0].data),
        section.records[1].original.clone(),
        record(
            68,
            1,
            &[0_u32, 10]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        ),
        record(
            68,
            1,
            &[0_u32, 11, 2, 20]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        ),
        section.records[3].original.clone(),
        section.records[4].original.clone(),
        section.records[5].original.clone(),
    ]
    .concat();
    let output = rewrite_section(
        &parse_section(&raw).unwrap(),
        &[(position(&raw, 0, 2), vec![])],
    )
    .unwrap();
    let saved = parse_section(&output).unwrap();
    assert_eq!(words(&saved.records[2].data), vec![0, 10]);
    assert_eq!(words(&saved.records[3].data), vec![0, 11, 0, 20]);
}

fn fixture(compressed: bool) -> Vec<u8> {
    let file = if compressed {
        "fixtures-make_hwp-a1462a4723a91114.hwp"
    } else {
        "fixtures-make_hwp-6652ceb672d9e7b4.hwp"
    };
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/data")
            .join(file),
    )
    .unwrap()
}

fn document_with_paragraph(text: &str, shapes: &[(u32, u32)]) -> hwp_core::document::Document {
    let cfb = hwp_core::cfb::CompoundFile::new(&fixture(false)).unwrap();
    let changes = std::collections::BTreeMap::from([(
        "BodyText/Section0".to_owned(),
        paragraph(text, shapes),
    )]);
    hwp_core::document::Document::new(cfb.rewrite(&changes).unwrap(), "test.hwp").unwrap()
}

fn set_text(document: &mut hwp_core::document::Document, index: usize, text: &str) {
    let node = &document.nodes[index];
    let patch = serde_json::json!({"schemaVersion":1,"baseSha256":document.hash,"operations":[{"op":"set_text","nodeId":node.id,"expectedText":node.text,"text":text}]});
    document.patch(&patch).unwrap();
}

fn saved_document(document: &hwp_core::document::Document) -> hwp_core::document::Document {
    hwp_core::document::Document::new(document.serialize(true).unwrap(), "saved.hwp").unwrap()
}

#[test]
fn test_nodes_retain_record_owner_and_code_unit_bounds() {
    let document = hwp_core::document::Document::new(fixture(false), "test.hwp").unwrap();
    let node = &document.nodes[0];
    assert_eq!(node.id, "BodyText/Section0#record:1:text:0");
    assert_eq!(node.paragraph_record_index, Some(0));
    assert_eq!(node.text_start, Some(16));
    assert_eq!(node.text_end, Some(16 + node.text.encode_utf16().count()));
    let section = &document.hwp_sections[&node.part];
    assert_eq!(section.paragraphs[&0].children, vec![1, 2, 3, 4, 6]);
    assert_eq!(
        section
            .records
            .iter()
            .flat_map(|item| item.original.clone())
            .collect::<Vec<_>>(),
        document.sections[&node.part]
    );
}

#[test]
fn test_nested_paragraph_has_its_own_owner() {
    let cfb = hwp_core::cfb::CompoundFile::new(&fixture(false)).unwrap();
    let body = [
        record(66, 0, &[0; 22]),
        record(67, 1, &utf16("A😀\r")),
        record(71, 1, &[b" lbt".as_slice(), &[0; 20]].concat()),
        record(66, 2, &[0; 22]),
        record(67, 3, &utf16("cell\r")),
    ]
    .concat();
    let changes = std::collections::BTreeMap::from([("BodyText/Section0".to_owned(), body)]);
    let document =
        hwp_core::document::Document::new(cfb.rewrite(&changes).unwrap(), "test.hwp").unwrap();
    assert_eq!(
        document
            .nodes
            .iter()
            .map(|node| node.paragraph_record_index)
            .collect::<Vec<_>>(),
        vec![Some(0), Some(3)]
    );
    assert_eq!(
        document
            .nodes
            .iter()
            .map(|node| (node.text_start, node.text_end))
            .collect::<Vec<_>>(),
        vec![(Some(0), Some(3)), (Some(0), Some(4))]
    );
}

#[test]
fn test_generated_fixture_accepts_grow_and_shrink() {
    for replacement in ["longer text 😀 with more words", "짧음"] {
        let mut document = hwp_core::document::Document::new(fixture(true), "test.hwp").unwrap();
        set_text(&mut document, 0, replacement);
        assert_eq!(saved_document(&document).nodes[0].text, replacement);
    }
}

#[test]
fn test_empty_text_node_accepts_insertion() {
    let mut document = document_with_paragraph("", &[(0, 10)]);
    assert_eq!(document.nodes.len(), 1);
    assert_eq!(document.nodes[0].text, "");
    assert_eq!(
        (document.nodes[0].text_start, document.nodes[0].text_end),
        (Some(0), Some(0))
    );
    set_text(&mut document, 0, "new 😀 text");
    assert_eq!(saved_document(&document).nodes[0].text, "new 😀 text");
}

#[test]
fn test_text_to_empty_retains_terminator_and_all_streams() {
    let source = fixture(true);
    let old = hwp_core::cfb::CompoundFile::new(&source).unwrap();
    let mut document = hwp_core::document::Document::new(source, "test.hwp").unwrap();
    set_text(&mut document, 0, "");
    let bytes = document.serialize(true).unwrap();
    let saved = hwp_core::document::Document::new(&bytes, "saved.hwp").unwrap();
    assert_eq!(saved.nodes[0].text, "");
    assert!(saved.hwp_sections["BodyText/Section0"].records[1]
        .data
        .ends_with(b"\r\0"));
    for (part, data) in old.streams {
        if part != "BodyText/Section0" {
            assert_eq!(
                hwp_core::cfb::CompoundFile::new(&bytes).unwrap().streams[&part],
                data
            );
        }
    }
}

#[test]
fn test_deleting_one_of_two_runs_does_not_fail_old_node_id_verification() {
    let mut document = document_with_paragraph("ab\ncd", &[(0, 10), (3, 20)]);
    set_text(&mut document, 0, "");
    set_text(&mut document, 1, "longer");
    let saved = saved_document(&document);
    assert_eq!(
        saved.hwp_sections["BodyText/Section0"].records[1].data,
        utf16("\nlonger\r")
    );
    assert_eq!(
        saved
            .nodes
            .iter()
            .map(|node| node.text.as_str())
            .collect::<Vec<_>>(),
        vec!["longer"]
    );
}

#[test]
fn test_record_grows_and_shrinks_across_extended_size_boundary() {
    let mut document = hwp_core::document::Document::new(fixture(false), "test.hwp").unwrap();
    set_text(&mut document, 0, &"X".repeat(2032));
    let mut saved = saved_document(&document);
    let item = &saved.hwp_sections["BodyText/Section0"].records[1];
    assert!(item.data.len() >= 4095);
    assert_eq!(words(&item.original[..4])[0] >> 20, 4095);
    assert_eq!(words(&item.original[4..8])[0] as usize, item.data.len());
    set_text(&mut saved, 0, "short");
    let shrunk = saved_document(&saved);
    let item = &shrunk.hwp_sections["BodyText/Section0"].records[1];
    assert_eq!(
        words(&item.original[..4])[0] as usize >> 20,
        item.data.len()
    );
    assert_eq!(shrunk.nodes[0].text, "short");
}

#[test]
fn test_control_code_insertions_are_refused_with_hint() {
    let mut document = hwp_core::document::Document::new(fixture(true), "test.hwp").unwrap();
    for text in ["before\tother", "before\nother", "\r", "\u{2}"] {
        let node = &document.nodes[0];
        let patch = serde_json::json!({"schemaVersion":1,"baseSha256":document.hash,"operations":[{"op":"set_text","nodeId":node.id,"expectedText":node.text,"text":text}]});
        let error = document.patch(&patch).unwrap_err();
        assert_eq!(error.code, "UNSUPPORTED_ELEMENT");
        assert!(error.message.contains("control"));
        assert_eq!(document.serialize(false).unwrap(), fixture(true));
    }
}

#[test]
fn test_edit_must_not_touch_adjacent_or_extended_control_units() {
    let document = hwp_core::document::Document::new(fixture(false), "test.hwp").unwrap();
    let section = &document.hwp_sections["BodyText/Section0"];
    let node = &document.nodes[0];
    let text_start = node.text_start.unwrap();
    let text_end = node.text_end.unwrap();
    let payload = section.records[1].payload_start;
    for (start_change, end_change) in [(0, 2), (-2, 0), (-14, 0)] {
        let start = (payload as isize + (text_start * 2) as isize + start_change) as usize;
        let end = (payload as isize + (text_end * 2) as isize + end_change) as usize;
        let position = TextPosition {
            paragraph_record_index: Some(0),
            record: 1,
            start,
            end,
            text_start: (text_start as isize + start_change / 2) as usize,
            text_end: (text_end as isize + end_change / 2) as usize,
        };
        let error = hwp_core::hwp_records::validate_text_edit(section, &position).unwrap_err();
        assert_eq!(error.code, "UNSUPPORTED_ELEMENT");
        assert!(error.message.contains("control"));
    }
}

#[test]
fn test_noop_patch_and_revert_are_byte_identical() {
    for compressed in [false, true] {
        let payload = fixture(compressed);
        let mut document = hwp_core::document::Document::new(&payload, "test.hwp").unwrap();
        let original = document.nodes[0].text.clone();
        set_text(&mut document, 0, &original);
        assert_eq!(document.serialize(false).unwrap(), payload);
        set_text(&mut document, 0, "changed length");
        // The expected text is the current staged text, on the original node ID.
        let patch = serde_json::json!({"schemaVersion":1,"baseSha256":document.hash,"operations":[{"op":"set_text","nodeId":document.nodes[0].id,"expectedText":"changed length","text":original}]});
        document.patch(&patch).unwrap();
        assert_eq!(document.serialize(false).unwrap(), payload);
    }
}

#[test]
fn test_bad_paragraph_metadata_rejects_atomic_patch() {
    for (offset, length, value) in [(0, 4, 1_u32), (12, 2, 99), (14, 2, 99), (16, 2, 99)] {
        let cfb = hwp_core::cfb::CompoundFile::new(&fixture(false)).unwrap();
        let mut raw = cfb.streams["BodyText/Section0"].clone();
        let section = parse_section(&raw).unwrap();
        let at = section.records[0].payload_start + offset;
        raw[at..at + length].copy_from_slice(&value.to_le_bytes()[..length]);
        let changes = std::collections::BTreeMap::from([("BodyText/Section0".to_owned(), raw)]);
        let original = cfb.rewrite(&changes).unwrap();
        let mut document = hwp_core::document::Document::new(&original, "test.hwp").unwrap();
        let node = &document.nodes[0];
        let patch = serde_json::json!({"schemaVersion":1,"baseSha256":document.hash,"operations":[{"op":"set_text","nodeId":node.id,"expectedText":node.text,"text":"changed"}]});
        assert_eq!(
            document.patch(&patch).unwrap_err().code,
            "UNSUPPORTED_ELEMENT"
        );
        assert_eq!(document.serialize(false).unwrap(), original);
    }
}

#[test]
fn test_untouched_noncanonical_extended_header_is_preserved() {
    let cfb = hwp_core::cfb::CompoundFile::new(&fixture(false)).unwrap();
    let opaque = [
        ((99_u32) | (1 << 10) | (4095 << 20))
            .to_le_bytes()
            .as_slice(),
        &3_u32.to_le_bytes(),
        b"raw",
    ]
    .concat();
    let raw = [
        cfb.streams["BodyText/Section0"].as_slice(),
        opaque.as_slice(),
    ]
    .concat();
    let changes = std::collections::BTreeMap::from([("BodyText/Section0".to_owned(), raw)]);
    let mut document =
        hwp_core::document::Document::new(cfb.rewrite(&changes).unwrap(), "test.hwp").unwrap();
    set_text(&mut document, 0, "longer text 😀");
    assert_eq!(
        saved_document(&document).hwp_sections["BodyText/Section0"]
            .records
            .last()
            .unwrap()
            .original,
        opaque
    );
}

fn inflate_stream(bytes: &[u8]) -> (Vec<u8>, &[u8]) {
    use std::io::Read;
    let mut decoder = flate2::read::DeflateDecoder::new(bytes);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).unwrap();
    let consumed = decoder.total_in() as usize;
    (raw, &bytes[consumed..])
}

fn compression_trailer(raw: &[u8]) -> Vec<u8> {
    let mut crc = 0xffff_ffff_u32;
    for byte in raw {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    [(!crc).to_le_bytes(), (raw.len() as u32).to_le_bytes()].concat()
}

fn add_trailer(payload: &[u8], trailer: bool) -> Vec<u8> {
    if !trailer {
        return payload.to_vec();
    }
    let cfb = hwp_core::cfb::CompoundFile::new(payload).unwrap();
    let compressed = &cfb.streams["BodyText/Section0"];
    let (raw, _) = inflate_stream(compressed);
    let stream = [compressed.as_slice(), compression_trailer(&raw).as_slice()].concat();
    cfb.rewrite(&std::collections::BTreeMap::from([(
        "BodyText/Section0".to_owned(),
        stream,
    )]))
    .unwrap()
}

#[test]
fn test_length_changes_refresh_optional_compression_trailer() {
    for trailer in [false, true] {
        for replacement in ["longer 😀 plain text replacement", "x", ""] {
            let source = add_trailer(&fixture(true), trailer);
            let old = hwp_core::cfb::CompoundFile::new(&source).unwrap();
            let mut document = hwp_core::document::Document::new(source, "test.hwp").unwrap();
            set_text(&mut document, 0, replacement);
            let bytes = document.serialize(true).unwrap();
            let saved = hwp_core::document::Document::new(&bytes, "saved.hwp").unwrap();
            assert_eq!(saved.nodes[0].text, replacement);
            let cfb = hwp_core::cfb::CompoundFile::new(&bytes).unwrap();
            let (raw, remaining) = inflate_stream(&cfb.streams["BodyText/Section0"]);
            assert_eq!(
                remaining,
                if trailer {
                    compression_trailer(&raw)
                } else {
                    vec![]
                }
            );
            for (part, data) in old.streams {
                if part != "BodyText/Section0" {
                    assert_eq!(cfb.streams[&part], data);
                }
            }
        }
    }
}

fn assert_independent_paragraph(payload: &[u8], text: &str) {
    let document = rhwp::parse_document(payload).unwrap();
    assert_eq!(document.sections[0].paragraphs[0].text, text);
    let native = rhwp::DocumentCore::from_bytes(payload).unwrap();
    assert!(native.page_count() >= 1);
}

#[test]
fn test_generated_hwp_variants_reopen_with_exact_paragraph_text() {
    let generated = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/generated/legacy-review.hwp"),
    )
    .unwrap();
    for (compressed, trailer) in [(false, false), (true, false), (true, true)] {
        let source = if compressed {
            generated.clone()
        } else {
            fixture(false)
        };
        let payload = add_trailer(&source, trailer);
        for replacement in [
            "longer replacement 😀 with many words".to_owned(),
            "x".to_owned(),
            String::new(),
            "X".repeat(2032),
        ] {
            let mut document = hwp_core::document::Document::new(&payload, "test.hwp").unwrap();
            set_text(&mut document, 0, &replacement);
            let bytes = document.serialize(true).unwrap();
            assert_independent_paragraph(&bytes, &replacement);
            if replacement.is_empty() || replacement.len() > 2000 {
                let mut reopened = hwp_core::document::Document::new(bytes, "saved.hwp").unwrap();
                set_text(&mut reopened, 0, "inserted 😀");
                assert_independent_paragraph(&reopened.serialize(true).unwrap(), "inserted 😀");
            }
        }
    }
}

#[test]
fn test_shape_boundary_and_two_run_edits_reopen_exactly() {
    for replacement in ["ABCDEFGHIJ", "XY", ""] {
        let mut document = document_with_paragraph("abcdef", &[(0, 10), (2, 20), (4, 30), (6, 40)]);
        set_text(&mut document, 0, replacement);
        assert_independent_paragraph(&document.serialize(true).unwrap(), replacement);
    }
    for (first, second) in [("ABCDE", "Z"), ("", "longer"), ("longer", "")] {
        let mut document = document_with_paragraph("ab\ncd", &[(0, 10), (3, 20)]);
        set_text(&mut document, 0, first);
        set_text(&mut document, 1, second);
        assert_independent_paragraph(
            &document.serialize(true).unwrap(),
            &format!("{first}\n{second}"),
        );
    }
}

#[test]
fn record_count_is_bounded() {
    let raw = record(99, 0, b"").repeat(hwp_core::errors::MAX_NODES + 1);
    let error = parse_section(&raw).unwrap_err();
    assert_eq!(error.code, "LIMIT");
    assert_eq!(error.message, "Too many HWP records.");
}

#[test]
fn malformed_owned_text_and_truncated_controls_fail_closed() {
    let misaligned = [record(66, 0, &[0; 22]), record(67, 1, &[65])].concat();
    let error = parse_section(&misaligned).unwrap_err();
    assert_eq!(error.code, "BAD_HWP_TEXT");
    assert_eq!(error.message, "Misaligned UTF-16 text record.");
    let mut raw = paragraph("", &[(0, 10)]);
    let mut section = parse_section(&raw).unwrap();
    section.records[1].data = vec![2, 0];
    raw = section
        .records
        .iter()
        .map(|item| record(item.tag, item.level, &item.data))
        .collect::<Vec<_>>()
        .concat();
    let section = parse_section(&raw).unwrap();
    let error =
        hwp_core::hwp_records::validate_text_edit(&section, &position(&raw, 0, 0)).unwrap_err();
    assert_eq!(error.code, "BAD_HWP_TEXT");
    assert_eq!(error.message, "Truncated HWP control sequence.");
}

#[test]
fn custom_rewrite_budget_cannot_relax_native_section_limit() {
    let raw = record(99, 0, &vec![0; hwp_core::errors::MAX_PART - 7]);
    let section = parse_section(&raw).unwrap();
    let result = hwp_core::hwp_records::rewrite_section_with_limit(
        &section,
        &[],
        hwp_core::errors::MAX_PART + 1,
    );
    assert!(
        result.is_err(),
        "A custom budget must not relax the native section limit"
    );
    let error = result.unwrap_err();
    assert_eq!(error.code, "LIMIT");
    assert_eq!(error.message, "Edited HWP section exceeds 32 MiB.");
}
