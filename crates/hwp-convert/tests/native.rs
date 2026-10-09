use hwp_convert::hwpx_write::{HC, HP};
use hwp_convert::package::{read_package, XmlExt};
use hwp_convert::{read_docx, write_hwpx};
use hwp_verify::worker::{run_worker, WorkerStatus};
use serde_json::{json, Value};
use std::io::Cursor;

const SOURCE: &[u8] =
    include_bytes!("../../../tests/data/convert_fixtures-docx-3bf7c69f1761e6c0.docx");

#[test]
fn test_hwpx_writer_reopens_with_native_text() {
    let source = read_docx(SOURCE).unwrap();
    let payload = write_hwpx(&source).unwrap();
    let (parts, trees) = read_package(&payload).unwrap();
    let section = &trees["Contents/section0.xml"];
    let texts: Vec<String> = section
        .descendants(HP, "p")
        .into_iter()
        .filter_map(|paragraph| {
            let runs = paragraph.children.iter().filter(|item| {
                item.name.namespace.as_deref() == Some(HP) && item.name.local == "run"
            });
            let mut text = String::new();
            let mut has_table = false;
            for run in runs {
                has_table |= run.child(HP, "tbl").is_some();
                for node in run.children.iter().filter(|node| {
                    node.name.namespace.as_deref() == Some(HP) && node.name.local == "t"
                }) {
                    text.push_str(node.text.as_deref().unwrap_or(""));
                    for child in &node.children {
                        if child.name.local == "lineBreak" {
                            text.push('\n');
                        }
                        if child.name.local == "tab" {
                            text.push('\t');
                        }
                        text.push_str(child.tail.as_deref().unwrap_or(""));
                    }
                }
            }
            if has_table && text.is_empty() {
                None
            } else {
                Some(text)
            }
        })
        .collect();
    assert_eq!(texts, source.paragraph_texts());
    let image = source.sections[0]
        .blocks
        .iter()
        .flat_map(|block| hwp_convert::model::paragraphs(std::slice::from_ref(block)))
        .flat_map(|paragraph| &paragraph.runs)
        .find_map(|run| run.image.as_ref())
        .unwrap();
    assert!(parts.values().any(|part| part == &image.data));
    assert!(!section.descendants(HC, "img").is_empty());
}

fn worker_request(payload: &[u8], request: Value) -> Value {
    let header = serde_json::to_vec(&request).unwrap();
    let mut frame = (header.len() as u32).to_le_bytes().to_vec();
    frame.extend(header);
    frame.extend(payload);
    let mut output = Vec::new();
    assert_eq!(
        run_worker(Cursor::new(frame), &mut output).unwrap(),
        WorkerStatus::Success
    );
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn test_hwpx_opens_and_renders_in_rhwp() {
    let payload = write_hwpx(&read_docx(SOURCE).unwrap()).unwrap();
    let observed = worker_request(&payload, json!({"op":"probe"}));
    assert!(observed["pageCount"].as_u64().unwrap() >= 1);
    for text in [
        "Heading 한글",
        "Across columns",
        "Across rows",
        "Image caption",
    ] {
        assert!(observed["fullText"].as_str().unwrap().contains(text));
    }
    let rendered = worker_request(&payload, json!({"op":"render","page":0}));
    let svg = rendered["svg"].as_str().unwrap();
    assert!(svg.contains("<svg"));
    let tree = hwp_core::xml_io::xml_tree(svg.as_bytes()).unwrap();
    assert!(tree
        .descendants("http://www.w3.org/2000/svg", "text")
        .iter()
        .filter_map(|node| node.text.as_deref())
        .collect::<String>()
        .contains("Heading"));
    assert!(svg.contains("data:image/png;base64,"));
}
