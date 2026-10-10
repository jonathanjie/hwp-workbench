use hwp_convert::model::{Block, Paragraph, Table};
use hwp_convert::{read_docx, read_hwpx, write_docx, write_hwpx};
use serde_json::json;

const STANDARD: &[u8] =
    include_bytes!("../../../tests/data/convert_fixtures-docx-3bf7c69f1761e6c0.docx");

fn paragraph(block: &Block) -> &Paragraph {
    match block {
        Block::Paragraph(paragraph) => paragraph,
        Block::Table(_) => panic!("Expected a paragraph in authored fixture"),
    }
}

fn table(block: &Block) -> &Table {
    match block {
        Block::Table(table) => table,
        Block::Paragraph(_) => panic!("Expected a table in authored fixture"),
    }
}

#[test]
fn test_docx_reader_preserves_rich_properties() {
    let rich = read_docx(STANDARD).unwrap();
    assert_eq!(
        rich.paragraph_texts(),
        [
            "Heading 한글",
            "Styled text \nsecond line",
            "Bullet item",
            "Number item",
            "Across columns",
            "Across rows",
            "Right one",
            "Right two",
            "Image caption",
            "New page",
            ""
        ]
    );
    let blocks = &rich.sections[0].blocks;
    assert_eq!(paragraph(&blocks[0]).heading, 1);
    assert_eq!(paragraph(&blocks[0]).runs[0].style["size"], json!(18.0));
    assert_eq!(paragraph(&blocks[1]).runs[0].style, json!({"bold":true,"italic":true,"underline":"single","strike":true,"size":14.0,"colour":"2468AC","font":"Arial"}).as_object().unwrap().clone());
    assert_eq!(paragraph(&blocks[1]).style["left"], 360);
    assert_eq!(paragraph(&blocks[1]).style["line"], 300);
    assert_eq!(
        paragraph(&blocks[2]).numbering.as_ref().unwrap()["kind"],
        "bullet"
    );
    assert_eq!(
        paragraph(&blocks[3]).numbering.as_ref().unwrap()["start"],
        3
    );
    let grid = table(&blocks[4]);
    assert_eq!(grid.widths, [1800, 2200]);
    assert_eq!(grid.rows[0][0].colspan, 2);
    assert_eq!(grid.rows[1][0].rowspan, 2);
    assert_eq!(grid.rows[0][0].shading.as_deref(), Some("DDEEFF"));
    assert_eq!(grid.rows[0][0].borders["top"]["colour"], "123456");
    let image = paragraph(&blocks[5])
        .runs
        .last()
        .unwrap()
        .image
        .as_ref()
        .unwrap();
    let (parts, _) = hwp_convert::package::read_package(STANDARD).unwrap();
    assert_eq!(image.data, parts["word/media/pixel.png"]);
    assert_eq!(rich.sections[0].page["width"], 16838);
    assert!(paragraph(&blocks[6]).page_break);
    assert!(rich.unsupported.is_empty());
}

#[test]
fn test_image_only_merge_continuation_is_kept_and_reported() {
    let source =
        include_bytes!("../../../tests/data/convert_fixtures-package-60bd5b8e40094da8.docx");
    let rich = read_docx(source).unwrap();
    let owner = &table(&rich.sections[0].blocks[4]).rows[1][0];
    let images: Vec<_> = owner
        .blocks
        .iter()
        .flat_map(|block| &paragraph(block).runs)
        .filter_map(|run| run.image.as_ref())
        .collect();
    let (parts, _) = hwp_convert::package::read_package(STANDARD).unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].data, parts["word/media/pixel.png"]);
    assert_eq!(rich.unsupported["nonemptyMergeContinuation"], 1);
    let (native, _) = hwp_convert::package::read_package(&write_hwpx(&rich).unwrap()).unwrap();
    assert!(native.values().any(|data| data == &images[0].data));
    let section = std::str::from_utf8(&native["Contents/section0.xml"]).unwrap();
    assert_eq!(section.matches("<hp:pic ").count(), 2);
}

#[test]
fn test_inline_page_break_keeps_text_order_and_break_location() {
    let source = include_bytes!("../../../tests/data/convert_fixtures-docx-dec5f22b42f814a4.docx");
    let rich = read_docx(source).unwrap();
    let blocks = &rich.sections[0].blocks;
    assert_eq!(
        &rich.paragraph_texts()[rich.paragraph_texts().len() - 2..],
        ["Before break", "After break"]
    );
    assert!(!paragraph(&blocks[blocks.len() - 2]).page_break);
    assert!(paragraph(blocks.last().unwrap()).page_break);
    let pivot = read_hwpx(&write_hwpx(&rich).unwrap()).unwrap();
    assert_eq!(pivot.paragraph_texts(), rich.paragraph_texts());
    assert!(paragraph(pivot.sections[0].blocks.last().unwrap()).page_break);
    let output = read_docx(&write_docx(&pivot).unwrap()).unwrap();
    assert!(paragraph(output.sections[0].blocks.last().unwrap()).page_break);
}

#[test]
fn test_character_style_does_not_reset_paragraph_style_size() {
    let source = include_bytes!("../../../tests/data/convert_fixtures-docx-ab3c2eff1d4e479a.docx");
    let rich = read_docx(source).unwrap();
    let run = &paragraph(rich.sections[0].blocks.last().unwrap()).runs[0];
    assert_eq!(run.style["size"], 18.0);
    assert_eq!(run.style["italic"], true);
}

#[test]
fn test_table_outer_and_inside_borders_are_resolved_per_cell() {
    let rich = read_docx(include_bytes!(
        "../../../tests/data/convert_fixtures-docx-0c119b074ab613c1.docx"
    ))
    .unwrap();
    let grid = table(rich.sections[0].blocks.last().unwrap());
    assert_eq!(grid.rows[0][0].borders["top"]["colour"], "FF0000");
    assert_eq!(grid.rows[0][0].borders["bottom"]["colour"], "00FF00");
    assert_eq!(grid.rows[1][0].borders["top"]["colour"], "00FF00");
}

#[test]
fn test_complex_field_counts_once_and_revisions_are_named() {
    let rich = read_docx(include_bytes!(
        "../../../tests/data/convert_fixtures-docx-6c7df8acf8cfe8f6.docx"
    ))
    .unwrap();
    assert_eq!(rich.unsupported["fields"], 1);
    assert_eq!(rich.unsupported["trackedChanges"], 1);
}

#[test]
fn test_image_crop_is_reported() {
    let rich = read_docx(include_bytes!(
        "../../../tests/data/convert_fixtures-package-5cf6556c0b4babee.docx"
    ))
    .unwrap();
    assert_eq!(rich.unsupported["imageCropping"], 1);
}

#[test]
fn test_default_paragraph_style_applies_without_explicit_reference() {
    let rich = read_docx(include_bytes!(
        "../../../tests/data/convert_fixtures-docx-d1c9cf326e994929.docx"
    ))
    .unwrap();
    let paragraph = paragraph(&rich.sections[0].blocks[5]);
    assert_eq!(paragraph.runs[0].style["bold"], true);
    assert_eq!(paragraph.runs[0].style["size"], 18.0);
    assert_eq!(paragraph.runs[0].style["font"], "Times New Roman");
    assert_eq!(paragraph.style["alignment"], "right");
}

#[test]
fn test_explicit_num_id_zero_disables_style_list() {
    let rich = read_docx(include_bytes!(
        "../../../tests/data/convert_fixtures-docx-30bda13f2179286f.docx"
    ))
    .unwrap();
    assert!(paragraph(rich.sections[0].blocks.last().unwrap())
        .numbering
        .is_none());
}

#[test]
fn test_oversized_spans_and_cyclic_styles_fail_closed() {
    assert_eq!(
        read_docx(include_bytes!(
            "../../../tests/data/convert_fixtures-docx-93c05a7b87d15136.docx"
        ))
        .unwrap_err()
        .code,
        "LIMIT"
    );
    assert_eq!(
        read_docx(include_bytes!(
            "../../../tests/data/convert_fixtures-docx-aa697a3d2d0bb147.docx"
        ))
        .unwrap_err()
        .code,
        "BAD_XML"
    );
}

#[test]
fn test_duplicate_archive_members_and_invalid_utf8() {
    let parts = [("same".into(), vec![b'a']), ("diff".into(), vec![b'b'])]
        .into_iter()
        .collect();
    let mut duplicate = hwp_convert::package::write_package(&parts).unwrap();
    // The ZIP writer rejects duplicate names, so change both member headers.
    for offset in 0..duplicate.len() - 3 {
        if &duplicate[offset..offset + 4] == b"diff" {
            duplicate[offset..offset + 4].copy_from_slice(b"same");
        }
    }
    assert_eq!(read_docx(&duplicate).unwrap_err().code, "UNSAFE_ZIP");
    assert_eq!(
        read_docx(include_bytes!(
            "../../../tests/data/convert_fixtures-docx-9498961910b746cc.docx"
        ))
        .unwrap_err()
        .code,
        "XML_ENCODING"
    );
}

#[test]
fn test_dtd_and_xml_depth_and_member_bounds() {
    for (source, code) in [
        (
            include_bytes!("../../../tests/data/convert_fixtures-docx-4e2eddb75cbf7380.docx")
                .as_slice(),
            "UNSAFE_XML",
        ),
        (
            include_bytes!("../../../tests/data/convert_fixtures-docx-e3986452df22bbfa.docx")
                .as_slice(),
            "LIMIT",
        ),
        (
            include_bytes!("../../../tests/data/convert_fixtures-docx-8de239670435ea1e.docx")
                .as_slice(),
            "UNSAFE_ZIP",
        ),
        (
            include_bytes!("../../../tests/data/convert_fixtures-package-3c7cf6fcf255c8ab.docx")
                .as_slice(),
            "LIMIT",
        ),
        (
            include_bytes!("../../../tests/data/convert_fixtures-package-dd7ccfd227e6aac2.docx")
                .as_slice(),
            "NOT_DOCX",
        ),
    ] {
        assert_eq!(read_docx(source).unwrap_err().code, code);
    }
}

#[test]
fn test_unsupported_objects_are_counted_in_both_directions_docx_mapper() {
    let rich = read_docx(include_bytes!(
        "../../../tests/data/convert_fixtures-docx-57a4ccf89fab7d9c.docx"
    ))
    .unwrap();
    for (feature, count) in [
        ("fields", 1),
        ("trackedChanges", 2),
        ("equations", 1),
        ("headers", 1),
        ("footers", 1),
        ("footnotes", 1),
        ("comments", 1),
        ("textBoxes", 1),
        ("shapes", 1),
    ] {
        assert_eq!(
            rich.unsupported.get(feature),
            Some(&json!(count)),
            "{feature}"
        );
    }
}

#[test]
fn test_24000_distinct_single_character_runs_convert_in_seconds_mapper() {
    let rich = read_docx(include_bytes!(
        "../../../tests/data/convert_fixtures-docx-240d3121c9ae56b8.docx"
    ))
    .unwrap();
    assert_eq!(rich.paragraph_texts(), ["x".repeat(24000)]);
    assert!(rich.unsupported.is_empty());
    let output = write_hwpx(&rich).unwrap();
    let reopened = read_hwpx(&output).unwrap();
    assert_eq!(reopened.paragraph_texts(), rich.paragraph_texts());
    assert!(reopened.unsupported.is_empty());
}
