use hwp_core::document::Document;
use hwp_core::format_readout::{number, read_format};
use hwp_core::package_io::ZipPackage;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

fn fixture(name: &str) -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read(root.join("fixtures/generated").join(name)).expect("generated fixture exists")
}

#[test]
fn test_read_only_format_and_unreadable_fields() {
    let original = fixture("review-form.hwpx");
    let document = Document::new(&original, "form.hwpx").expect("fixture opens");
    let first = document.nodes.first().expect("fixture has text");
    let result = read_format(&document, Some(&first.id)).expect("format reads");
    assert_eq!(result["character"]["size"], 22.0);
    assert_eq!(result["character"]["font"], "Apple SD Gothic Neo");
    assert_eq!(result["character"]["widthRatio"], 100.0);
    assert_eq!(
        document.serialize(true).expect("unchanged serialization"),
        original
    );
    let error = read_format(&document, Some("missing")).expect_err("unknown node fails");
    assert_eq!(error.code, "BAD_ARGUMENTS");
    assert_eq!(error.message, "Unknown text node.");
    let legacy = Document::new(fixture("legacy-review.hwp"), "legacy.hwp").expect("legacy opens");
    let unknown = read_format(&legacy, None).expect("legacy format reads");
    assert_eq!(unknown["character"], json!({}));
    assert_eq!(
        unknown
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["character", "paragraph", "page", "style", "fonts", "source"]
    );
}

#[test]
fn native_readout_matches_all_fixture_fields() {
    let document = Document::new(fixture("review-form.hwpx"), "form.hwpx").expect("fixture opens");
    assert_eq!(
        read_format(&document, None).expect("format reads"),
        json!({
            "character": {"font": "Apple SD Gothic Neo", "size": 22.0,
                "widthRatio": 100.0, "letterSpacing": 0.0, "bold": true,
                "italic": false, "underline": false, "colour": "000000"},
            "paragraph": {"alignment": "LEFT", "lineSpacing": "160%"},
            "page": {"width": 210.00155555555554, "height": 296.9965555555556,
                "orientation": "portrait", "left": 30.00022222222222,
                "right": 30.00022222222222, "top": 19.99897222222222,
                "bottom": 15.00011111111111, "header": 15.00011111111111,
                "footer": 15.00011111111111, "gutter": 0.0},
            "style": {"name": "Normal"}, "fonts": ["Apple SD Gothic Neo"],
            "source": "native-header"
        })
    );
}

#[test]
fn number_retains_finite_values_and_ignores_unreadable_native_fields() {
    for value in [
        None,
        Some("missing"),
        Some("NaN"),
        Some("inf"),
        Some("1e9999"),
    ] {
        assert_eq!(number(value, 1.0), None);
    }
    assert_eq!(number(Some(" 2200 "), 100.0), Some(22.0));
    assert_eq!(number(Some("1_200"), 100.0), Some(12.0));
    assert_eq!(number(Some("12__00"), 100.0), None);
    assert_eq!(number(Some("١_٢٠٠"), 100.0), Some(12.0));
}

#[test]
fn line_spacing_uses_native_units_and_python_precision() {
    let original = fixture("review-form.hwpx");
    let document = Document::new(&original, "form.hwpx").expect("fixture opens");
    let header = std::str::from_utf8(document.part("Contents/header.xml").expect("header exists"))
        .expect("header is UTF-8");
    let package = ZipPackage::new(&original).expect("fixture ZIP opens");
    for (kind, value, expected) in [
        ("FIXED", "1250", Some("12.5 pt")),
        ("AT_LEAST", "1234.56789", Some("12.3457 pt")),
        ("PERCENT", "0.00001", Some("1e-05%")),
        ("PERCENT", "999999.5", Some("1e+06%")),
        ("PERCENT", "100000.5", Some("100000%")),
        ("PERCENT", "NaN", None),
        ("UNKNOWN", "160", None),
    ] {
        let replacement = format!("type=\"{kind}\" value=\"{value}\"");
        let changed = header.replace("type=\"PERCENT\" value=\"160\"", &replacement);
        let bytes = package
            .rewrite(&BTreeMap::from([(
                "Contents/header.xml".to_owned(),
                changed.into_bytes(),
            )]))
            .expect("fixture header changes");
        let document = Document::new(bytes, "spacing.hwpx").expect("variant opens");
        let result = read_format(&document, None).expect("variant format reads");
        assert_eq!(
            result["paragraph"]
                .get("lineSpacing")
                .and_then(|item| item.as_str()),
            expected
        );
    }
}

#[test]
fn native_character_height_underscores_keep_preview_and_readout_in_agreement() {
    let original = fixture("review-form.hwpx");
    let document = Document::new(&original, "form.hwpx").expect("fixture opens");
    let header = std::str::from_utf8(document.part("Contents/header.xml").expect("header exists"))
        .expect("header is UTF-8")
        .replace("height=\"2200\"", "height=\"1_200\"");
    let bytes = ZipPackage::new(&original)
        .expect("fixture ZIP opens")
        .rewrite(&BTreeMap::from([(
            "Contents/header.xml".to_owned(),
            header.into_bytes(),
        )]))
        .expect("fixture header changes");
    let document = Document::new(bytes, "native-height.hwpx").expect("variant opens");
    assert_eq!(
        document.nodes.first().expect("text exists").style["size"],
        12.0
    );
    assert_eq!(
        read_format(&document, None).expect("format reads")["character"]["size"],
        12.0
    );
}
