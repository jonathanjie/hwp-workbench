//! Supported conversion semantics. Geometry uses DOCX twips (1/1440 inch).
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub type Style = Map<String, Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub data: Vec<u8>,
    pub media_type: String,
    pub width: i64,
    pub height: i64,
    pub name: String,
}
impl Default for Image {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            media_type: String::new(),
            width: 1440,
            height: 1440,
            name: "Image".into(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub style: Style,
    pub image: Option<Image>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub style: Style,
    pub heading: i64,
    pub numbering: Option<Style>,
    pub page_break: bool,
}
impl Paragraph {
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub column: usize,
    pub colspan: usize,
    pub rowspan: usize,
    pub borders: Style,
    pub shading: Option<String>,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            column: 0,
            colspan: 1,
            rowspan: 1,
            borders: Style::new(),
            shading: None,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
    pub widths: Vec<i64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

pub fn default_page() -> Style {
    // A4 paper with one-inch margins; header/footer distances are half an inch.
    let values = json!({"width":11906,"height":16838,"orientation":"portrait","left":1440,"right":1440,"top":1440,"bottom":1440,"header":720,"footer":720,"gutter":0});
    values.as_object().cloned().unwrap_or_default()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub blocks: Vec<Block>,
    pub page: Style,
}
impl Default for Section {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            page: default_page(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichDocument {
    pub sections: Vec<Section>,
    pub unsupported: Map<String, Value>,
}
impl RichDocument {
    pub fn loss(&mut self, feature: &str, count: usize) {
        if count > 0 {
            let old = self
                .unsupported
                .get(feature)
                .and_then(Value::as_u64)
                .unwrap_or(0);
            self.unsupported.insert(
                feature.into(),
                Value::from(old.saturating_add(count as u64)),
            );
        }
    }
    pub fn paragraph_texts(&self) -> Vec<String> {
        self.sections
            .iter()
            .flat_map(|section| paragraphs(&section.blocks))
            .map(Paragraph::text)
            .collect()
    }
}
/// Traverse nested tables in the same row/cell order as the contract model.
pub fn paragraphs(blocks: &[Block]) -> impl Iterator<Item = &Paragraph> {
    let mut pending: Vec<&Block> = blocks.iter().rev().collect();
    std::iter::from_fn(move || {
        while let Some(block) = pending.pop() {
            match block {
                Block::Paragraph(paragraph) => return Some(paragraph),
                Block::Table(table) => {
                    for row in table.rows.iter().rev() {
                        for cell in row.iter().rev() {
                            pending.extend(cell.blocks.iter().rev());
                        }
                    }
                }
            }
        }
        None
    })
}
/// Native breaks start paragraphs; images belong only to the first text piece.
pub fn split_page_breaks(blocks: Vec<Block>) -> Vec<Block> {
    let mut result = Vec::new();
    for block in blocks {
        match block {
            Block::Table(mut table) => {
                for row in &mut table.rows {
                    for cell in row {
                        cell.blocks = split_page_breaks(std::mem::take(&mut cell.blocks));
                    }
                }
                result.push(Block::Table(table));
            }
            Block::Paragraph(mut paragraph) => {
                let runs = std::mem::take(&mut paragraph.runs);
                // Clone paragraph properties without copying the complete run
                // payload again for every inline page break.
                let mut current = paragraph.clone();
                for run in &runs {
                    for (index, piece) in run.text.split('\u{c}').enumerate() {
                        if index > 0 {
                            result.push(Block::Paragraph(current));
                            current = Paragraph {
                                runs: Vec::new(),
                                page_break: true,
                                numbering: None,
                                heading: 0,
                                style: paragraph.style.clone(),
                            };
                        }
                        current.runs.push(Run {
                            text: piece.into(),
                            image: if index == 0 { run.image.clone() } else { None },
                            style: run.style.clone(),
                        });
                    }
                }
                result.push(Block::Paragraph(current));
            }
        }
    }
    result
}

/// Count copied property strings before repeated style references can amplify
/// a bounded XML part into an unbounded conversion model allocation.
pub fn style_payload_bytes(style: &Style) -> usize {
    fn value_bytes(value: &Value) -> usize {
        match value {
            Value::String(text) => text.len(),
            Value::Array(items) => items
                .iter()
                .fold(0usize, |size, item| size.saturating_add(value_bytes(item))),
            Value::Object(object) => style_payload_bytes(object),
            _ => 0,
        }
    }
    style.iter().fold(0usize, |size, (key, value)| {
        size.saturating_add(key.len())
            .saturating_add(value_bytes(value))
    })
}

/// Keep generated tab/break objects within the shared XML element budget.
pub(crate) const MAX_OUTPUT_CONTROLS: usize = hwp_core::errors::MAX_NODES * 10;
pub(crate) fn check_output_controls(
    document: &RichDocument,
    limit: usize,
) -> hwp_core::errors::Result<()> {
    let mut controls = 0usize;
    for section in &document.sections {
        for paragraph in paragraphs(&section.blocks) {
            for run in &paragraph.runs {
                let mut characters = run.text.chars().peekable();
                while let Some(character) = characters.next() {
                    if character == '\r' && characters.peek() == Some(&'\n') {
                        characters.next(); // CRLF becomes one native line break.
                    }
                    if matches!(character, '\t' | '\r' | '\n') {
                        controls = controls.saturating_add(1);
                        hwp_core::errors::require(
                            controls <= limit,
                            "LIMIT",
                            "Conversion XML complexity exceeds limits.",
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_controls_are_bounded_before_xml_objects_are_allocated() {
        let document = RichDocument {
            sections: vec![Section {
                blocks: vec![Block::Paragraph(Paragraph {
                    runs: vec![Run {
                        text: "\t\n\r\n".repeat(4),
                        ..Run::default()
                    }],
                    ..Paragraph::default()
                })],
                ..Section::default()
            }],
            ..RichDocument::default()
        };
        assert_eq!(
            check_output_controls(&document, 11).unwrap_err().code,
            "LIMIT"
        );
        assert!(check_output_controls(&document, 12).is_ok());
    }
}
