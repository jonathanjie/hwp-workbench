use hwp_convert::model::{Block, Paragraph, RichDocument, Run, Section};
use hwp_convert::package::{read_package, write_package};
use hwp_convert::{read_hwpx, write_hwpx};
use serde_json::json;

#[test]
fn test_native_image_flip_crop_and_effects_are_reported() {
    let source =
        include_bytes!("../../../tests/data/convert_fixtures-package-fcdbb758946b471e.hwpx");
    let rich = read_hwpx(source).unwrap();
    assert_eq!(rich.unsupported.get("imageTransforms"), Some(&json!(1)));
    assert_eq!(rich.unsupported.get("imageCropping"), Some(&json!(1)));
    assert_eq!(rich.unsupported.get("imageEffects"), Some(&json!(1)));
}

#[test]
fn test_hwpx_features_without_docx_origin() {
    let paragraph = Paragraph {
        runs: vec![Run {
            text: "한글\tText\nNext".into(),
            style: json!({"size":18,"bold":true,"font":"Arial","colour":"2468AC"})
                .as_object()
                .unwrap()
                .clone(),
            ..Default::default()
        }],
        heading: 2,
        numbering: Some(
            json!({"kind":"decimal","level":1,"start":3,"text":"%1."})
                .as_object()
                .unwrap()
                .clone(),
        ),
        style: json!({"hanging":360,"lineRule":"exact","line":300,"before":120,"keepNext":true})
            .as_object()
            .unwrap()
            .clone(),
        page_break: true,
    };
    let source = RichDocument {
        sections: vec![Section {
            blocks: vec![Block::Paragraph(paragraph)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let payload = write_hwpx(&source).unwrap();
    assert_eq!(payload, write_hwpx(&source).unwrap());
    let reopened = read_hwpx(&payload).unwrap();
    assert_eq!(source.paragraph_texts(), reopened.paragraph_texts());
    assert_eq!(source.sections[0].page, reopened.sections[0].page);
    assert!(reopened.unsupported.is_empty());
    let Block::Paragraph(paragraph) = &reopened.sections[0].blocks[0] else {
        panic!("paragraph expected")
    };
    assert_eq!(paragraph.heading, 2);
    assert!(paragraph.page_break);
    assert_eq!(paragraph.style.get("hanging"), Some(&json!(360)));
    assert_eq!(paragraph.style.get("line"), Some(&json!(300)));
    assert_eq!(paragraph.runs[0].style.get("size"), Some(&json!(18.0)));
    assert_eq!(
        paragraph.numbering.as_ref().unwrap().get("start"),
        Some(&json!(3))
    );
}

#[test]
fn table_column_span_exceeds_column_budget() {
    let payload =
        include_bytes!("../../../tests/data/convert_fixtures-package-fcdbb758946b471e.hwpx");
    let (mut parts, _) = read_package(payload).unwrap();
    let xml = String::from_utf8(parts.get("Contents/section0.xml").unwrap().clone()).unwrap();
    parts.insert(
        "Contents/section0.xml".into(),
        xml.replace("colSpan=\"2\"", "colSpan=\"3\"").into_bytes(),
    );
    assert_eq!(
        read_hwpx(&write_package(&parts).unwrap()).unwrap_err().code,
        "LIMIT"
    );
}

#[test]
fn native_text_node_budget_includes_unsupported_content() {
    let source =
        include_bytes!("../../../tests/data/convert_fixtures-package-fcdbb758946b471e.hwpx");
    let (mut parts, _) = read_package(source).unwrap();
    let controls = "<hp:t/>".repeat(hwp_core::errors::MAX_NODES + 1);
    let xml = format!(
        "<hs:sec xmlns:hs=\"{}\" xmlns:hp=\"{}\"><hp:rect>{controls}</hp:rect></hs:sec>",
        hwp_convert::hwpx_write::HS,
        hwp_convert::hwpx_write::HP
    );
    parts.insert("Contents/section0.xml".into(), xml.into_bytes());
    let error = read_hwpx(&write_package(&parts).unwrap()).unwrap_err();
    assert_eq!(error.code, "LIMIT");
    assert_eq!(error.message, "Too many text nodes.");
}

#[test]
fn table_cells_overlap_is_rejected() {
    let source =
        include_bytes!("../../../tests/data/convert_fixtures-package-fcdbb758946b471e.hwpx");
    let (mut parts, _) = read_package(source).unwrap();
    let xml = String::from_utf8(parts.get("Contents/section0.xml").unwrap().clone()).unwrap();
    parts.insert(
        "Contents/section0.xml".into(),
        xml.replace("colAddr=\"1\"", "colAddr=\"0\"").into_bytes(),
    );
    let error = read_hwpx(&write_package(&parts).unwrap()).unwrap_err();
    assert_eq!(error.code, "BAD_XML");
    assert_eq!(error.message, "Overlapping native table cells.");
}

#[test]
fn oversized_border_width_is_rejected_instead_of_saturated() {
    let mut parts = hwp_convert::package::read_package(
        &hwp_convert::write_hwpx(
            &hwp_convert::read_docx(include_bytes!(
                "../../../tests/data/convert_fixtures-docx-3bf7c69f1761e6c0.docx"
            ))
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
    .0;
    let header = String::from_utf8(parts["Contents/header.xml"].clone()).unwrap();
    let header = header.replace("width=\"0.3528 mm\"", "width=\"99999999999999999999 mm\"");
    assert!(header.contains("99999999999999999999"));
    parts.insert("Contents/header.xml".into(), header.into_bytes());
    let error =
        hwp_convert::read_hwpx(&hwp_convert::package::write_package(&parts).unwrap()).unwrap_err();
    assert_eq!(error.code, "LIMIT");
}
