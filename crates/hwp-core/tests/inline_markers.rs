mod common;
use common::*;
use hwp_core::document::Document;
use std::collections::BTreeMap;

fn with_text(inner: &str) -> Document {
    let source = document(&hwpx());
    let xml = std::str::from_utf8(source.part("Contents/section0.xml").unwrap()).unwrap();
    let start = xml.find("<hp:t>").unwrap() + "<hp:t>".len();
    let end = start + xml[start..].find("</hp:t>").unwrap();
    let changed = [&xml[..start], inner, &xml[end..]].concat();
    let bytes = hwp_core::package_io::ZipPackage::new(&source.original)
        .unwrap()
        .rewrite(&BTreeMap::from([(
            "Contents/section0.xml".to_owned(),
            changed.into_bytes(),
        )]))
        .unwrap();
    document(&bytes)
}

#[test]
fn edit_around_all_childless_markers_keeps_exact_native_bytes() {
    let inner = "□<hp:fwSpace data='x &amp; y' />가<hp:nbSpace/><hp:tab width='2000' type='LEFT' leader='0'/><hp:lineBreak/><hp:hyphen/>끝";
    let mut source = with_text(inner);
    let expected = "□\u{fdd0}가\u{fdd1}\u{fdd2}\u{fdd3}\u{fdd4}끝";
    assert!(source.nodes[0].editable);
    assert_eq!(source.nodes[0].text, expected);
    assert_eq!(
        source.inspect()["nodes"][0]["inlineMarkers"]["\u{fdd0}"],
        "\u{2007}"
    );
    edit_first(&mut source, &expected.replace('□', "■"));
    let result = reopened(&source);
    assert_eq!(result.nodes[0].text, expected.replace('□', "■"));
    assert_parts_unchanged(&source, &result, "Contents/section0.xml");
    let original = std::str::from_utf8(source.part("Contents/section0.xml").unwrap()).unwrap();
    assert_eq!(
        result.part("Contents/section0.xml").unwrap(),
        original.replace(inner, &inner.replace('□', "■")).as_bytes()
    );
}

#[test]
fn deleting_first_marker_keeps_second_markers_attributes() {
    let mut source = with_text("앞<hp:fwSpace kind='first'/>중<hp:fwSpace kind='second'/>뒤");
    assert_eq!(source.nodes[0].text, "앞\u{fdd0}중\u{fdd1}뒤");
    edit_first(&mut source, "앞중\u{fdd1}뒤");
    let result = reopened(&source);
    assert_eq!(result.nodes[0].text, "앞중\u{fdd0}뒤");
    let xml = std::str::from_utf8(result.part("Contents/section0.xml").unwrap()).unwrap();
    assert!(xml.contains("<hp:t>앞중<hp:fwSpace kind='second'/>뒤</hp:t>"));
    assert!(!xml.contains("kind='first'"));
}

#[test]
fn paired_empty_marker_bytes_are_retained() {
    let mut source = with_text("앞<hp:fwSpace ></hp:fwSpace>뒤");
    assert!(source.nodes[0].editable);
    assert_eq!(source.nodes[0].text, "앞\u{fdd0}뒤");
    edit_first(&mut source, "시작\u{fdd0}끝");
    assert!(
        std::str::from_utf8(reopened(&source).part("Contents/section0.xml").unwrap())
            .unwrap()
            .contains("시작<hp:fwSpace ></hp:fwSpace>끝")
    );
}

#[test]
fn invented_or_duplicated_placeholder_is_refused_atomically() {
    for (inner, value) in [
        ("plain", "new\u{fdd0}"),
        ("<hp:fwSpace/>", "\u{fdd1}"),
        ("<hp:fwSpace/>", "\u{fdd0}\u{fdd0}"),
    ] {
        let mut source = with_text(inner);
        let edit = patch(&source, &source.nodes[0], value);
        assert_eq!(source.patch(&edit).unwrap_err().code, "INLINE_MARKER");
        assert!(source.edits.is_empty());
    }
}

#[test]
fn unsafe_nested_text_and_placeholder_collisions_stay_locked() {
    for inner in [
        "앞<hp:fieldBegin/>뒤",
        "<hp:fwSpace>text</hp:fwSpace>",
        "<hp:fwSpace><hp:nbSpace/></hp:fwSpace>",
        "\u{fdd0}<hp:fwSpace/>",
        "&#xFDD0;<hp:fwSpace/>",
        "<other:fwSpace xmlns:other='urn:other'/>",
    ] {
        let mut source = with_text(inner);
        assert!(!source.nodes[0].editable, "{inner}");
        let edit = patch(&source, &source.nodes[0], "new");
        assert_eq!(source.patch(&edit).unwrap_err().code, "UNSUPPORTED_ELEMENT");
    }
}

#[test]
fn expected_text_requires_placeholders_and_markers_can_all_be_deleted() {
    let mut source = with_text("<hp:fwSpace/>");
    assert_eq!(source.nodes[0].text, "\u{fdd0}");
    let mut edit = patch(&source, &source.nodes[0], "");
    edit["operations"][0]["expectedText"] = serde_json::json!("\u{2007}");
    assert_eq!(source.patch(&edit).unwrap_err().code, "STALE_TEXT");
    edit_first(&mut source, "");
    assert_eq!(reopened(&source).nodes[0].text, "");
}
