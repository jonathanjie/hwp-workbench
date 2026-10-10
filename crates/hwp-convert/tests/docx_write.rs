use hwp_convert::model::{Block, Cell, Image, Paragraph, RichDocument, Run, Section, Style, Table};
use hwp_convert::package::read_package;
use hwp_convert::{read_docx, read_hwpx, write_docx, write_hwpx};
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixture() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data");
    let manifest: Vec<Value> =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let entry = manifest
        .iter()
        .find(|entry| {
            entry["builder"] == "convert_fixtures.docx"
                && entry["parameters"] == json!({"extra_body": "", "extra_parts": null})
        })
        .unwrap();
    std::fs::read(root.join(entry["name"].as_str().unwrap())).unwrap()
}

fn paragraph(block: &Block) -> &Paragraph {
    match block {
        Block::Paragraph(paragraph) => paragraph,
        _ => panic!("Expected paragraph"),
    }
}
fn table(block: &Block) -> &Table {
    match block {
        Block::Table(table) => table,
        _ => panic!("Expected table"),
    }
}
fn style(value: Value) -> Style {
    value.as_object().unwrap().clone()
}

#[test]
fn test_docx_package_parts_use_unprefixed_namespaces() {
    let output = write_docx(&read_docx(&fixture()).unwrap()).unwrap();
    let (parts, _) = read_package(&output).unwrap();
    for (name, start) in [
        ("[Content_Types].xml", "<Types xmlns=\""),
        ("_rels/.rels", "<Relationships xmlns=\""),
        ("word/_rels/document.xml.rels", "<Relationships xmlns=\""),
    ] {
        let text = std::str::from_utf8(parts.get(name).unwrap()).unwrap();
        assert!(text.contains(start), "{name}: {text}");
        assert!(!text.contains("ns0:"), "{name}");
    }
}

#[test]
fn test_rich_hwpx_to_docx_preserves_content_and_geometry() {
    let source = read_docx(&fixture()).unwrap();
    let pivot = read_hwpx(&write_hwpx(&source).unwrap()).unwrap();
    let reopened = read_docx(&write_docx(&pivot).unwrap()).unwrap();
    assert_eq!(reopened.paragraph_texts(), source.paragraph_texts());
    assert_eq!(pivot.paragraph_texts(), source.paragraph_texts());
    for rich in [&pivot, &reopened] {
        let section = &rich.sections[0];
        assert_eq!(section.page, source.sections[0].page);
        assert_eq!(paragraph(&section.blocks[0]).heading, 1);
        assert_eq!(paragraph(&section.blocks[1]).runs[0].style["strike"], true);
        assert_eq!(paragraph(&section.blocks[1]).style["before"], 120);
        let table = table(&section.blocks[4]);
        assert_eq!(table.widths, [1800, 2200]);
        assert_eq!(table.rows[0][0].colspan, 2);
        assert_eq!(table.rows[1][0].rowspan, 2);
        assert_eq!(table.rows[0][0].shading.as_deref(), Some("DDEEFF"));
        assert_eq!(table.rows[0][0].borders["top"]["size"], 8);
        assert_eq!(table.rows[1][0].borders["bottom"]["kind"], "double");
        assert_eq!(
            paragraph(&section.blocks[5])
                .runs
                .last()
                .unwrap()
                .image
                .as_ref()
                .unwrap()
                .data,
            paragraph(&source.sections[0].blocks[5])
                .runs
                .last()
                .unwrap()
                .image
                .as_ref()
                .unwrap()
                .data
        );
        assert_eq!(
            paragraph(&section.blocks[2]).numbering.as_ref().unwrap()["kind"],
            "bullet"
        );
        assert_eq!(
            paragraph(&section.blocks[3]).numbering.as_ref().unwrap()["start"],
            3
        );
        assert!(paragraph(&section.blocks[6]).page_break);
        assert!(rich.unsupported.is_empty(), "{:?}", rich.unsupported);
    }
}

#[test]
fn test_docx_distinct_numbering_converts_in_seconds() {
    let blocks = (0..24000).map(|index| Block::Paragraph(Paragraph {
        runs: vec![Run { text: "x".to_owned(), ..Run::default() }],
        numbering: Some(style(json!({"kind":"decimal", "level":0, "start":index + 1, "text":"%1.", "style":{"left":index}}))),
        ..Paragraph::default()
    })).collect();
    let document = RichDocument {
        sections: vec![Section {
            blocks,
            ..Section::default()
        }],
        ..RichDocument::default()
    };
    let started = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    let output = write_docx(&document).unwrap();
    let finished = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    // Match the CPU-time budget; host scheduling is not CPU work.
    let elapsed = (finished.tv_sec - started.tv_sec) as f64
        + (finished.tv_nsec - started.tv_nsec) as f64 / 1_000_000_000.0;
    assert!(
        elapsed < 8.0,
        "24000 numbering styles used {elapsed:.3} CPU seconds"
    );
    let reopened = read_docx(&output).unwrap();
    assert_eq!(reopened.sections[0].blocks.len(), 24000);
    assert_eq!(
        paragraph(reopened.sections[0].blocks.last().unwrap())
            .numbering
            .as_ref()
            .unwrap()["start"],
        24000
    );
}

#[test]
fn writer_preserves_sparse_tables_nested_tables_and_multiple_sections() {
    let child = Block::Table(Table {
        rows: vec![vec![Cell {
            blocks: vec![Block::Paragraph(Paragraph {
                runs: vec![Run {
                    text: "Nested".into(),
                    ..Run::default()
                }],
                ..Paragraph::default()
            })],
            ..Cell::default()
        }]],
        widths: vec![1000],
    });
    let document = RichDocument {
        sections: vec![
            Section {
                blocks: vec![Block::Table(Table {
                    widths: vec![1000, 1000, 1000, 1000, 1000],
                    rows: vec![
                        vec![
                            Cell {
                                column: 1,
                                blocks: vec![child],
                                ..Cell::default()
                            },
                            Cell {
                                column: 3,
                                rowspan: 2,
                                blocks: vec![Block::Paragraph(Paragraph {
                                    runs: vec![Run {
                                        text: "Merged".into(),
                                        ..Run::default()
                                    }],
                                    ..Paragraph::default()
                                })],
                                ..Cell::default()
                            },
                        ],
                        vec![],
                    ],
                })],
                ..Section::default()
            },
            Section {
                blocks: vec![Block::Paragraph(Paragraph {
                    runs: vec![Run {
                        text: "Tail\r\nline\ttab\rfinal".into(),
                        style: style(
                            json!({"bold":false, "italic":false, "strike":false, "size":12.25, "font":"Arial", "colour":"ABCDEF", "underline":"none"}),
                        ),
                        ..Run::default()
                    }],
                    ..Paragraph::default()
                })],
                ..Section::default()
            },
        ],
        ..RichDocument::default()
    };
    let first = write_docx(&document).unwrap();
    assert_eq!(first, write_docx(&document).unwrap());
    let (parts, _) = read_package(&first).unwrap();
    let xml = std::str::from_utf8(parts.get("word/document.xml").unwrap()).unwrap();
    assert!(xml.contains("w:gridBefore"));
    assert!(xml.contains("w:gridAfter"));
    assert!(xml.contains("w:vMerge w:val=\"restart\""));
    assert_eq!(xml.matches("<w:sectPr").count(), 2);
    assert!(xml.contains("w:sz w:val=\"24\""));
    let reopened = read_docx(&first).unwrap();
    assert_eq!(reopened.sections.len(), 2);
    let sparse = table(&reopened.sections[0].blocks[0]);
    assert_eq!(sparse.rows[0][0].column, 1);
    assert_eq!(sparse.rows[0][1].column, 2);
    assert_eq!(sparse.rows[0][2].column, 3);
    assert_eq!(sparse.rows[0][2].rowspan, 2);
    let tail = paragraph(&reopened.sections[1].blocks[0]);
    for property in ["bold", "italic", "strike"] {
        assert_eq!(tail.runs[0].style[property], false);
    }
    assert_eq!(tail.runs[0].style["size"], 12.0);
    assert_eq!(tail.runs[0].style["font"], "Arial");
    assert_eq!(tail.runs[0].style["colour"], "ABCDEF");
    assert_eq!(tail.runs[0].style["underline"], "none");
    assert_eq!(
        reopened.paragraph_texts().last().unwrap(),
        "Tail\nline\ttab\nfinal"
    );
}

#[test]
fn writer_rejects_unbounded_sparse_cell_positions() {
    let document = RichDocument {
        sections: vec![Section {
            blocks: vec![Block::Table(Table {
                widths: vec![1000],
                rows: vec![vec![Cell {
                    column: usize::MAX,
                    ..Cell::default()
                }]],
            })],
            ..Section::default()
        }],
        ..RichDocument::default()
    };
    assert_eq!(write_docx(&document).unwrap_err().code, "LIMIT");
}

#[test]
fn writer_rejects_image_dimension_overflow() {
    let document = RichDocument {
        sections: vec![Section {
            blocks: vec![Block::Paragraph(Paragraph {
                runs: vec![Run {
                    image: Some(Image {
                        data: vec![0],
                        media_type: "image/png".into(),
                        width: i64::MAX,
                        ..Image::default()
                    }),
                    ..Run::default()
                }],
                ..Paragraph::default()
            })],
            ..Section::default()
        }],
        ..RichDocument::default()
    };
    assert_eq!(write_docx(&document).unwrap_err().code, "LIMIT");
}
