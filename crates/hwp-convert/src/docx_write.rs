//! Direct OOXML output for the supported conversion model.
use crate::model::{Block, Cell, Image, Paragraph, RichDocument, Run, Style, Table};
use crate::package::{encoded, write_package, Parts, XmlNode, MAX_COLUMNS, MAX_GRID_WORK};
use crate::style_index::indexed_value;
use hwp_core::errors::{require, Result, WorkbenchError, MAX_NODES};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

fn word(tag: &str) -> XmlNode {
    XmlNode::new(format!("w:{tag}"))
}
fn root(tag: &str) -> XmlNode {
    word(tag)
        .attr("xmlns:w", W)
        .attr("xmlns:r", R)
        .attr("xmlns:a", A)
        .attr("xmlns:wp", WP)
        .attr("xmlns:pic", PIC)
}
fn native_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}
fn required(style: &Style, key: &str) -> Result<String> {
    style
        .get(key)
        .map(native_value)
        .ok_or_else(|| WorkbenchError::new("BAD_XML", "Missing conversion style attribute."))
}
fn add_properties(parent: &mut XmlNode, name: &str, style: &Style, keys: &[(&str, &str)]) {
    let mut properties = word(name);
    for (key, native) in keys {
        if let Some(value) = style.get(*key) {
            properties.set(format!("w:{native}"), native_value(value));
        }
    }
    if !properties.attributes.is_empty() {
        parent.push(properties);
    }
}

struct Writer {
    parts: Parts,
    relationships: XmlNode,
    types: XmlNode,
    numbers: Vec<Style>,
    number_ids: HashMap<String, usize>,
    image_count: usize,
    paragraph_count: usize,
}
impl Writer {
    fn new() -> Self {
        Self {
            parts: Parts::new(),
            // OPC roots and their children must have an unprefixed default namespace.
            relationships: XmlNode::new("Relationships").attr("xmlns", REL),
            types: XmlNode::new("Types").attr("xmlns", CT),
            numbers: Vec::new(),
            number_ids: HashMap::new(),
            image_count: 0,
            paragraph_count: 0,
        }
    }

    fn run(&mut self, run: &Run) -> Result<XmlNode> {
        let mut native = word("r");
        let mut properties = word("rPr");
        if let Some(font) = run.style.get("font") {
            let font = native_value(font);
            properties.push(
                word("rFonts")
                    .attr("w:ascii", &font)
                    .attr("w:hAnsi", &font)
                    .attr("w:eastAsia", &font),
            );
        }
        for (key, tag) in [("bold", "b"), ("italic", "i"), ("strike", "strike")] {
            if let Some(value) = run.style.get(key) {
                properties.push(word(tag).attr("w:val", u8::from(truth(value))));
            }
        }
        if let Some(size) = run.style.get("size") {
            let size = size
                .as_f64()
                .ok_or_else(|| WorkbenchError::new("BAD_XML", "Invalid conversion font size."))?;
            let half_points = (size * 2.0).round_ties_even();
            require(
                half_points.is_finite(),
                "LIMIT",
                "Conversion font size exceeds limits.",
            )?;
            properties.push(word("sz").attr("w:val", format!("{half_points:.0}")));
        }
        for (key, tag) in [("colour", "color"), ("underline", "u")] {
            if let Some(value) = run.style.get(key) {
                properties.push(word(tag).attr("w:val", native_value(value)));
            }
        }
        native.push(properties);
        if let Some(image) = &run.image {
            native.push(self.image(image)?);
        }
        let text = run.text.replace("\r\n", "\n").replace('\r', "\n");
        for (line_index, chunk) in text.split('\n').enumerate() {
            if line_index > 0 {
                native.push(word("br"));
            }
            for (tab_index, part) in chunk.split('\t').enumerate() {
                if tab_index > 0 {
                    native.push(word("tab"));
                }
                if !part.is_empty() || (text.is_empty() && run.image.is_none()) {
                    native.push(word("t").attr("xml:space", "preserve").text(part));
                }
            }
        }
        Ok(native)
    }

    fn paragraph(&mut self, paragraph: &Paragraph) -> Result<XmlNode> {
        require(
            self.paragraph_count < MAX_NODES,
            "LIMIT",
            "Too many conversion paragraphs.",
        )?;
        self.paragraph_count += 1;
        let mut node = word("p");
        let mut properties = word("pPr");
        if paragraph.heading != 0 {
            properties.push(word("pStyle").attr("w:val", format!("Heading{}", paragraph.heading)));
        }
        if let Some(alignment) = paragraph.style.get("alignment") {
            properties.push(word("jc").attr("w:val", native_value(alignment)));
        }
        add_properties(
            &mut properties,
            "ind",
            &paragraph.style,
            &[
                ("left", "left"),
                ("right", "right"),
                ("first", "firstLine"),
                ("hanging", "hanging"),
            ],
        );
        add_properties(
            &mut properties,
            "spacing",
            &paragraph.style,
            &[
                ("before", "before"),
                ("after", "after"),
                ("line", "line"),
                ("lineRule", "lineRule"),
            ],
        );
        for key in ["keepNext", "keepLines", "widowControl"] {
            if let Some(value) = paragraph.style.get(key) {
                properties.push(word(key).attr("w:val", u8::from(truth(value))));
            }
        }
        if paragraph.page_break {
            properties.push(word("pageBreakBefore"));
        }
        if let Some(number) = paragraph
            .numbering
            .as_ref()
            .filter(|number| !number.is_empty())
        {
            let number_id =
                indexed_value(&mut self.numbers, &mut self.number_ids, number.clone())? + 1;
            properties.push(
                word("numPr")
                    .child(word("ilvl").attr("w:val", required(number, "level")?))
                    .child(word("numId").attr("w:val", number_id)),
            );
        }
        node.push(properties);
        for run in &paragraph.runs {
            node.push(self.run(run)?);
        }
        Ok(node)
    }

    fn cell(&mut self, cell: &Cell, continuation: bool, depth: usize) -> Result<XmlNode> {
        let mut native = word("tc");
        let mut properties = word("tcPr");
        if cell.colspan > 1 {
            properties.push(word("gridSpan").attr("w:val", cell.colspan));
        }
        if continuation || cell.rowspan > 1 {
            let merge = if continuation {
                word("vMerge")
            } else {
                word("vMerge").attr("w:val", "restart")
            };
            properties.push(merge);
        }
        if !cell.borders.is_empty() {
            let mut borders = word("tcBorders");
            for (side, border) in &cell.borders {
                let border = border.as_object().ok_or_else(|| {
                    WorkbenchError::new("BAD_XML", "Invalid conversion cell border.")
                })?;
                borders.push(
                    word(side)
                        .attr("w:val", required(border, "kind")?)
                        .attr("w:sz", required(border, "size")?)
                        .attr("w:color", required(border, "colour")?),
                );
            }
            properties.push(borders);
        }
        if let Some(shading) = cell.shading.as_ref().filter(|shading| !shading.is_empty()) {
            properties.push(word("shd").attr("w:val", "clear").attr("w:fill", shading));
        }
        native.push(properties);
        if continuation {
            native.push(word("p"));
        } else {
            for block in &cell.blocks {
                native.push(self.block(block, depth + 1)?);
            }
            if !matches!(cell.blocks.last(), Some(Block::Paragraph(_))) {
                // OOXML requires a final paragraph after a nested table.
                native.push(word("p"));
            }
        }
        Ok(native)
    }

    fn table(&mut self, table: &Table, depth: usize) -> Result<XmlNode> {
        require(
            table.widths.len() <= MAX_COLUMNS,
            "LIMIT",
            "Too many table columns.",
        )?;
        require(
            table.rows.len() <= MAX_NODES,
            "LIMIT",
            "Too many table rows.",
        )?;
        let width = table.widths.iter().try_fold(0i64, |sum, width| {
            sum.checked_add(*width)
                .ok_or_else(|| WorkbenchError::new("LIMIT", "Table width exceeds limits."))
        })?;
        let mut node = word("tbl").child(
            word("tblPr")
                .child(word("tblW").attr("w:w", width).attr("w:type", "dxa"))
                .child(word("tblLayout").attr("w:type", "fixed")),
        );
        let mut grid = word("tblGrid");
        for width in &table.widths {
            grid.push(word("gridCol").attr("w:w", width));
        }
        node.push(grid);
        let mut active: BTreeMap<usize, (&Cell, usize)> = BTreeMap::new();
        let mut work = 0usize;
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut positions = BTreeMap::new();
            for cell in row {
                require(
                    cell.colspan > 0
                        && cell.rowspan > 0
                        && cell.rowspan <= MAX_NODES
                        && cell
                            .column
                            .checked_add(cell.colspan)
                            .is_some_and(|end| end <= MAX_COLUMNS),
                    "LIMIT",
                    "Table cell exceeds column limits.",
                )?;
                require(
                    positions.insert(cell.column, (cell, false)).is_none(),
                    "BAD_XML",
                    "Overlapping conversion table cells.",
                )?;
            }
            active.retain(|_, (_, last_row)| row_index <= *last_row);
            for (column, (cell, _)) in &active {
                require(
                    positions.insert(*column, (*cell, true)).is_none(),
                    "BAD_XML",
                    "Overlapping conversion table cells.",
                )?;
            }
            let first_column = positions.keys().next().copied().unwrap_or(0);
            let mut row_properties = word("trPr");
            if first_column > 0 {
                row_properties.push(word("gridBefore").attr("w:val", first_column));
            }
            let mut native_row = word("tr");
            let mut cursor = first_column;
            for (column, (cell, continuation)) in positions {
                require(
                    column >= cursor,
                    "BAD_XML",
                    "Overlapping conversion table cells.",
                )?;
                while cursor < column {
                    self.grid_work(&mut work, 1)?;
                    let gap = Cell {
                        blocks: vec![Block::Paragraph(Paragraph::default())],
                        column: cursor,
                        ..Cell::default()
                    };
                    native_row.push(self.cell(&gap, false, depth)?);
                    cursor += 1; // Column positions are bounded by MAX_COLUMNS.
                }
                self.grid_work(&mut work, cell.colspan)?;
                native_row.push(self.cell(cell, continuation, depth)?);
                if cell.rowspan > 1 && !continuation {
                    let last_row = row_index.checked_add(cell.rowspan - 1).ok_or_else(|| {
                        WorkbenchError::new("LIMIT", "Table mapping exceeds work limit.")
                    })?;
                    active.insert(column, (cell, last_row));
                }
                cursor = column + cell.colspan; // Checked before this cell entered positions.
            }
            if cursor < table.widths.len() {
                row_properties.push(word("gridAfter").attr("w:val", table.widths.len() - cursor));
            }
            if !row_properties.children.is_empty() {
                native_row.children.insert(0, row_properties);
            }
            node.push(native_row);
        }
        Ok(node)
    }

    fn grid_work(&self, work: &mut usize, amount: usize) -> Result<()> {
        *work = work.saturating_add(amount);
        require(
            *work <= MAX_GRID_WORK,
            "LIMIT",
            "Table mapping exceeds work limit.",
        )
    }

    fn block(&mut self, block: &Block, depth: usize) -> Result<XmlNode> {
        require(
            depth <= hwp_core::xml_io::MAX_DEPTH,
            "LIMIT",
            "XML nesting exceeds limits.",
        )?;
        match block {
            Block::Paragraph(paragraph) => self.paragraph(paragraph),
            Block::Table(table) => self.table(table, depth),
        }
    }

    fn image(&mut self, image: &Image) -> Result<XmlNode> {
        require(
            self.image_count < hwp_core::package_io::MAX_MEMBERS,
            "LIMIT",
            "Too many output ZIP members.",
        )?;
        self.image_count += 1;
        let index = self.image_count;
        let extension = match image.media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/bmp" => "bmp",
            _ => {
                return Err(WorkbenchError::new(
                    "UNSUPPORTED_FORMAT",
                    "Unsupported conversion image format.",
                ))
            }
        };
        let name = format!("word/media/image{index}.{extension}");
        self.parts.insert(name.clone(), image.data.clone());
        self.relationships.push(
            XmlNode::new("Relationship")
                .attr("Id", format!("image{index}"))
                .attr("Type", format!("{R}/image"))
                .attr("Target", format!("media/image{index}.{extension}")),
        );
        self.types.push(
            XmlNode::new("Override")
                .attr("PartName", format!("/{name}"))
                .attr("ContentType", &image.media_type),
        );
        // One DOCX twip is 635 English Metric Units.
        let width = image
            .width
            .checked_mul(635)
            .ok_or_else(|| WorkbenchError::new("LIMIT", "Image dimensions exceed limits."))?;
        let height = image
            .height
            .checked_mul(635)
            .ok_or_else(|| WorkbenchError::new("LIMIT", "Image dimensions exceed limits."))?;
        let nonvisual = XmlNode::new("pic:nvPicPr")
            .child(
                XmlNode::new("pic:cNvPr")
                    .attr("id", index)
                    .attr("name", &image.name),
            )
            .child(XmlNode::new("pic:cNvPicPr"));
        let fill = XmlNode::new("pic:blipFill")
            .child(XmlNode::new("a:blip").attr("r:embed", format!("image{index}")))
            .child(XmlNode::new("a:stretch").child(XmlNode::new("a:fillRect")));
        let transform = XmlNode::new("a:xfrm")
            .child(XmlNode::new("a:off").attr("x", 0).attr("y", 0))
            .child(XmlNode::new("a:ext").attr("cx", width).attr("cy", height));
        let shape = XmlNode::new("pic:spPr").child(transform).child(
            XmlNode::new("a:prstGeom")
                .attr("prst", "rect")
                .child(XmlNode::new("a:avLst")),
        );
        let picture = XmlNode::new("pic:pic")
            .child(nonvisual)
            .child(fill)
            .child(shape);
        let graphic = XmlNode::new("a:graphic").child(
            XmlNode::new("a:graphicData")
                .attr("uri", PIC)
                .child(picture),
        );
        let inline = XmlNode::new("wp:inline")
            .attr("distT", 0)
            .attr("distB", 0)
            .attr("distL", 0)
            .attr("distR", 0)
            .child(
                XmlNode::new("wp:extent")
                    .attr("cx", width)
                    .attr("cy", height),
            )
            .child(
                XmlNode::new("wp:docPr")
                    .attr("id", index)
                    .attr("name", &image.name),
            )
            .child(graphic);
        Ok(word("drawing").child(inline))
    }

    fn page(&self, page: &Style) -> Result<XmlNode> {
        let mut section = word("sectPr");
        section.push(
            word("pgSz")
                .attr("w:w", required(page, "width")?)
                .attr("w:h", required(page, "height")?)
                .attr("w:orient", required(page, "orientation")?),
        );
        let mut margins = word("pgMar");
        for key in [
            "left", "right", "top", "bottom", "header", "footer", "gutter",
        ] {
            margins.set(format!("w:{key}"), required(page, key)?);
        }
        section.push(margins);
        Ok(section)
    }

    fn styles(&self) -> XmlNode {
        let mut styles = root("styles");
        styles.push(
            word("style")
                .attr("w:type", "paragraph")
                .attr("w:styleId", "Normal")
                .attr("w:default", 1)
                .child(word("name").attr("w:val", "Normal")),
        );
        for index in 1..10 {
            styles.push(
                word("style")
                    .attr("w:type", "paragraph")
                    .attr("w:styleId", format!("Heading{index}"))
                    .child(word("name").attr("w:val", format!("heading {index}")))
                    .child(word("basedOn").attr("w:val", "Normal"))
                    .child(word("pPr").child(word("outlineLvl").attr("w:val", index - 1))),
            );
        }
        styles
    }

    fn numbering(&self) -> Result<XmlNode> {
        let mut numbering = root("numbering");
        for (index, number) in self.numbers.iter().enumerate() {
            let index = index + 1;
            let level = word("lvl")
                .attr("w:ilvl", required(number, "level")?)
                .child(word("start").attr("w:val", required(number, "start")?))
                .child(word("numFmt").attr("w:val", required(number, "kind")?))
                .child(word("lvlText").attr("w:val", required(number, "text")?));
            numbering.push(
                word("abstractNum")
                    .attr("w:abstractNumId", index)
                    .child(level),
            );
            numbering.push(
                word("num")
                    .attr("w:numId", index)
                    .child(word("abstractNumId").attr("w:val", index)),
            );
        }
        Ok(numbering)
    }

    fn write(mut self, document: &RichDocument) -> Result<Vec<u8>> {
        let mut body = word("body");
        for (index, section) in document.sections.iter().enumerate() {
            for block in &section.blocks {
                body.push(self.block(block, 0)?);
            }
            let page = self.page(&section.page)?;
            if index + 1 < document.sections.len() {
                if body.children.last().is_none_or(|last| last.name != "w:p") {
                    body.push(word("p"));
                }
                let last = body.children.last_mut().ok_or_else(|| {
                    WorkbenchError::new("BAD_XML", "Missing conversion section paragraph.")
                })?;
                let property_index =
                    match last.children.iter().position(|child| child.name == "w:pPr") {
                        Some(index) => index,
                        None => {
                            last.children.insert(0, word("pPr"));
                            0
                        }
                    };
                let properties = last.children.get_mut(property_index).ok_or_else(|| {
                    WorkbenchError::new("BAD_XML", "Missing conversion paragraph properties.")
                })?;
                properties.push(page);
            } else {
                body.push(page);
            }
        }
        self.parts.insert(
            "word/document.xml".into(),
            encoded(&root("document").child(body))?,
        );
        self.parts
            .insert("word/styles.xml".into(), encoded(&self.styles())?);
        self.parts
            .insert("word/numbering.xml".into(), encoded(&self.numbering()?)?);
        for name in ["styles", "numbering"] {
            self.relationships.push(
                XmlNode::new("Relationship")
                    .attr("Id", name)
                    .attr("Type", format!("{R}/{name}"))
                    .attr("Target", format!("{name}.xml")),
            );
        }
        self.parts.insert(
            "word/_rels/document.xml.rels".into(),
            encoded(&self.relationships)?,
        );
        let relationships = XmlNode::new("Relationships").attr("xmlns", REL).child(
            XmlNode::new("Relationship")
                .attr("Id", "document")
                .attr("Type", format!("{R}/officeDocument"))
                .attr("Target", "word/document.xml"),
        );
        self.parts
            .insert("_rels/.rels".into(), encoded(&relationships)?);
        self.types.push(
            XmlNode::new("Default")
                .attr("Extension", "xml")
                .attr("ContentType", "application/xml"),
        );
        self.types
            .push(XmlNode::new("Default").attr("Extension", "rels").attr(
                "ContentType",
                "application/vnd.openxmlformats-package.relationships+xml",
            ));
        for (name, kind) in [
            ("document", "document.main"),
            ("styles", "styles"),
            ("numbering", "numbering"),
        ] {
            self.types.push(XmlNode::new("Override").attr("PartName", format!("/word/{name}.xml")).attr("ContentType", format!("application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml")));
        }
        self.parts
            .insert("[Content_Types].xml".into(), encoded(&self.types)?);
        write_package(&self.parts)
    }
}

/// Write a new DOCX package. This operation never reads or overwrites a path.
pub fn write_docx(document: &RichDocument) -> Result<Vec<u8>> {
    crate::model::check_output_controls(document, crate::model::MAX_OUTPUT_CONTROLS)?;
    Writer::new().write(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Section;

    #[test]
    fn writer_rejects_overlapping_cells_instead_of_losing_content() {
        let content = || {
            vec![Block::Paragraph(Paragraph {
                runs: vec![Run {
                    text: "Keep".into(),
                    ..Run::default()
                }],
                ..Paragraph::default()
            })]
        };
        for rows in [
            vec![vec![
                Cell {
                    blocks: content(),
                    ..Cell::default()
                },
                Cell {
                    blocks: content(),
                    ..Cell::default()
                },
            ]],
            vec![
                vec![Cell {
                    blocks: content(),
                    rowspan: 2,
                    ..Cell::default()
                }],
                vec![Cell {
                    blocks: content(),
                    ..Cell::default()
                }],
            ],
            vec![vec![
                Cell {
                    blocks: content(),
                    colspan: 2,
                    ..Cell::default()
                },
                Cell {
                    column: 1,
                    blocks: content(),
                    ..Cell::default()
                },
            ]],
        ] {
            let document = RichDocument {
                sections: vec![Section {
                    blocks: vec![Block::Table(Table {
                        rows,
                        widths: vec![1000, 1000],
                    })],
                    ..Section::default()
                }],
                ..RichDocument::default()
            };
            assert_eq!(write_docx(&document).unwrap_err().code, "BAD_XML");
        }
    }

    #[test]
    fn writer_emits_an_empty_section_package() {
        let document = RichDocument {
            sections: vec![Section::default()],
            ..RichDocument::default()
        };
        assert!(write_docx(&document).is_ok());
    }
}
