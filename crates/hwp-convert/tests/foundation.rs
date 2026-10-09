use hwp_convert::{model::*, package::*, style_index::indexed_value};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn split_page_breaks_retains_images_and_disables_repeated_numbering() {
    let paragraph = Paragraph {
        runs: vec![Run {
            text: "first\u{c}last".into(),
            image: Some(Image {
                data: vec![1, 2, 3],
                ..Image::default()
            }),
            ..Run::default()
        }],
        heading: 1,
        numbering: Some(json!({"kind":"bullet"}).as_object().unwrap().clone()),
        ..Paragraph::default()
    };
    let blocks = split_page_breaks(vec![Block::Paragraph(paragraph)]);
    assert_eq!(blocks.len(), 2);
    let Block::Paragraph(first) = &blocks[0] else {
        panic!("paragraph expected")
    };
    assert_eq!(first.runs[0].image.as_ref().unwrap().data, vec![1, 2, 3]);
    let Block::Paragraph(second) = &blocks[1] else {
        panic!("paragraph expected")
    };
    assert!(second.page_break);
    assert!(second.numbering.is_none());
    assert_eq!(second.heading, 0);
    assert_eq!(second.text(), "last");
    assert!(second.runs[0].image.is_none());
}

#[test]
fn package_output_is_deterministic_and_bounded() {
    let parts = Parts::from([
        ("mimetype".into(), b"application/hwp+zip".to_vec()),
        ("test.xml".into(), b"<test/>".to_vec()),
    ]);
    let first = write_package(&parts).unwrap();
    assert_eq!(first, write_package(&parts).unwrap());
    assert_eq!(read_package(&first).unwrap().0, parts);
}

#[test]
fn style_keys_are_value_based_and_keep_first_use_order() {
    let mut values = Vec::new();
    let mut indices = HashMap::new();
    assert_eq!(
        indexed_value(&mut values, &mut indices, json!({"a":1,"b":2})).unwrap(),
        0
    );
    assert_eq!(
        indexed_value(&mut values, &mut indices, json!({"b":2,"a":1})).unwrap(),
        0
    );
    assert_eq!(
        indexed_value(&mut values, &mut indices, json!({"a":2})).unwrap(),
        1
    );
    assert_eq!(values.len(), 2);
}

#[test]
fn member_targets_allow_internal_parent_links_only() {
    assert_eq!(
        member_target("word/document.xml", "../media/p.png").unwrap(),
        "media/p.png"
    );
    assert_eq!(
        member_target("word/document.xml", "../../outside")
            .unwrap_err()
            .code,
        "UNSAFE_ZIP"
    );
    assert_eq!(
        member_target("word/document.xml", "https://host")
            .unwrap_err()
            .code,
        "UNSAFE_ZIP"
    );
}

#[test]
fn test_text_checks_report_duplicate_deletion_and_reordering() {
    let report = hwp_convert::text_check(
        &["Repeated".into(), "Repeated".into(), "Last".into()],
        &["Repeated".into(), "Last".into()],
        "source document",
    );
    assert_eq!(report["missingParagraphCount"], 1);
    assert_eq!(report["matches"], false);
    assert_eq!(
        hwp_convert::text_check(
            &["First".into(), "Second".into()],
            &["Second".into(), "First".into()],
            "source document"
        )["matches"],
        false
    );
    assert_eq!(
        hwp_convert::text_check(
            &["한글\n  text".into()],
            &["한글 text".into()],
            "source document"
        )["matches"],
        true
    );
}

#[test]
fn text_reports_match_python_alignment_and_bounded_fallback() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/text-reports.json")).unwrap();
    for case in cases {
        let before: Vec<String> = serde_json::from_value(case["before"].clone()).unwrap();
        let after: Vec<String> = serde_json::from_value(case["after"].clone()).unwrap();
        assert_eq!(
            hwp_convert::text_check(&before, &after, "source document"),
            case["report"]
        );
    }
}

#[test]
fn numeric_attributes_keep_python_integer_syntax_and_error_codes() {
    assert_eq!(integer(Some(" +１_۲ "), 0, -100, 100).unwrap(), 12);
    assert_eq!(
        integer(Some("9999999999999999999999999999"), 0, -100, 100)
            .unwrap_err()
            .code,
        "LIMIT"
    );
    for bad in ["", "1__2", "_1", "1_", "1.0", "\u{1c}1"] {
        assert_eq!(
            integer(Some(bad), 0, -100, 100).unwrap_err().code,
            "BAD_XML",
            "{bad:?}"
        );
    }
    assert_eq!(
        integer(Some(&"1".repeat(4301)), 0, -100, 100)
            .unwrap_err()
            .code,
        "BAD_XML"
    );
}

#[test]
fn both_writers_reject_control_amplification_before_creating_xml() {
    let document = RichDocument {
        sections: vec![Section {
            blocks: vec![Block::Paragraph(Paragraph {
                runs: vec![Run {
                    text: "\t".repeat(hwp_core::errors::MAX_NODES * 10 + 1),
                    ..Run::default()
                }],
                ..Paragraph::default()
            })],
            ..Section::default()
        }],
        ..RichDocument::default()
    };
    for write in [hwp_convert::write_docx, hwp_convert::write_hwpx] {
        let error = write(&document).unwrap_err();
        assert_eq!(error.code, "LIMIT");
        assert_eq!(error.message, "Conversion XML complexity exceeds limits.");
    }
}
