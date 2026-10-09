//! Emit supported rich content as HWPX; this is an explicit lossy conversion.
use crate::model::{Block, Image, Paragraph, RichDocument, Style, Table};
use crate::package::{encoded, write_package, Parts, XmlNode};
use crate::style_index::{immutable_value, indexed_value};
use hwp_core::errors::{require, Result, WorkbenchError};
use serde_json::Value;
use std::collections::HashMap;

pub const HP: &str = "http://www.hancom.co.kr/hwpml/2011/paragraph";
pub const HC: &str = "http://www.hancom.co.kr/hwpml/2011/core";
pub const HH: &str = "http://www.hancom.co.kr/hwpml/2011/head";
pub const HS: &str = "http://www.hancom.co.kr/hwpml/2011/section";
pub const OPF: &str = "http://www.idpf.org/2007/opf/";
/// HWPUNIT is 1/7200 inch, five times the model's twip unit.
pub const HWP_PER_TWIP: i64 = 5;
/// DOCX border widths are eighths of a point; one point is 25.4/72 mm.
pub const MM_PER_BORDER_UNIT: f64 = 25.4 / 72.0 / 8.0;
const FONT_KEYS: [&str; 7] = [
    "hangul", "latin", "hanja", "japanese", "other", "symbol", "user",
];
const LANGUAGES: [&str; 7] = [
    "HANGUL", "LATIN", "HANJA", "JAPANESE", "OTHER", "SYMBOL", "USER",
];
fn text<'a>(style: &'a Style, name: &str, default: &'a str) -> &'a str {
    style.get(name).and_then(Value::as_str).unwrap_or(default)
}
fn integer_style(style: &Style, name: &str, default: i64) -> i64 {
    style.get(name).and_then(Value::as_i64).unwrap_or(default)
}
fn float(style: &Style, name: &str, default: f64) -> f64 {
    style.get(name).and_then(Value::as_f64).unwrap_or(default)
}
fn flag(style: &Style, name: &str) -> bool {
    style.get(name).and_then(Value::as_bool).unwrap_or(false)
}
fn scaled(value: i64) -> Result<i64> {
    value
        .checked_mul(HWP_PER_TWIP)
        .ok_or_else(|| WorkbenchError::new("LIMIT", "Document numeric attribute exceeds limits."))
}
fn node(name: &str, attributes: &[(&str, &str)]) -> XmlNode {
    let mut node = XmlNode::new(name);
    for (key, value) in attributes {
        node.set(*key, *value);
    }
    node
}
fn namespaces(node: XmlNode) -> XmlNode {
    node.attr("xmlns:hp", HP)
        .attr("xmlns:hc", HC)
        .attr("xmlns:hh", HH)
        .attr("xmlns:hs", HS)
        .attr("xmlns:opf", OPF)
}
fn border_type(kind: &str) -> &str {
    match kind {
        "single" => "SOLID",
        "double" => "DOUBLE_SLIM",
        "dotted" => "DOT",
        "dashed" => "DASH",
        _ => "NONE",
    }
}
fn underline_type(kind: &str) -> &str {
    match kind {
        "double" => "DOUBLE",
        "dotted" => "DOT",
        "dash" => "DASH",
        "wave" => "WAVE",
        _ => "SOLID",
    }
}

struct Writer<'a> {
    document: &'a RichDocument,
    char_styles: Vec<Style>,
    para_styles: Vec<Style>,
    fonts: Vec<String>,
    fills: Vec<(Style, Option<String>)>,
    numbers: Vec<Style>,
    char_ids: HashMap<String, usize>,
    para_ids: HashMap<String, usize>,
    font_ids: HashMap<String, usize>,
    fill_ids: HashMap<String, usize>,
    number_ids: HashMap<String, usize>,
    decimal_ids: HashMap<String, usize>,
    bullet_ids: HashMap<String, usize>,
    parts: Parts,
    object_id: usize,
    paragraph_id: usize,
    table_depth: usize,
}
impl<'a> Writer<'a> {
    fn new(document: &'a RichDocument) -> Result<Self> {
        let mut writer = Self {
            document,
            char_styles: vec![],
            para_styles: vec![],
            fonts: vec![],
            fills: vec![],
            numbers: vec![],
            char_ids: HashMap::new(),
            para_ids: HashMap::new(),
            font_ids: HashMap::new(),
            fill_ids: HashMap::new(),
            number_ids: HashMap::new(),
            decimal_ids: HashMap::new(),
            bullet_ids: HashMap::new(),
            parts: Parts::new(),
            object_id: 1,
            paragraph_id: 0,
            table_depth: 0,
        };
        indexed_value(&mut writer.char_styles, &mut writer.char_ids, Style::new())?;
        indexed_value(&mut writer.para_styles, &mut writer.para_ids, Style::new())?;
        indexed_value(&mut writer.fonts, &mut writer.font_ids, "Arial".to_owned())?;
        indexed_value(
            &mut writer.fills,
            &mut writer.fill_ids,
            (Style::new(), None),
        )?;
        writer
            .parts
            .insert("mimetype".into(), b"application/hwp+zip".to_vec());
        Ok(writer)
    }
    fn paragraph(
        &mut self,
        paragraph: &Paragraph,
        prefix: Option<XmlNode>,
        container_only: bool,
    ) -> Result<XmlNode> {
        let mut style = paragraph.style.clone();
        if let Some(numbering) = paragraph
            .numbering
            .as_ref()
            .filter(|number| !number.is_empty())
        {
            style.insert("numbering".into(), Value::Object(numbering.clone()));
            indexed_value(&mut self.numbers, &mut self.number_ids, numbering.clone())?;
        }
        let para_id = indexed_value(&mut self.para_styles, &mut self.para_ids, style)?;
        let mut root = XmlNode::new("hp:p")
            .attr("id", self.paragraph_id)
            .attr("paraPrIDRef", para_id)
            .attr("styleIDRef", paragraph.heading)
            .attr("pageBreak", u8::from(paragraph.page_break))
            .attr("columnBreak", 0)
            .attr("merged", 0);
        self.paragraph_id = self
            .paragraph_id
            .checked_add(1)
            .ok_or_else(|| WorkbenchError::new("LIMIT", "Too many conversion paragraphs."))?;
        if let Some(prefix) = prefix {
            root.push(prefix);
        }
        for run in &paragraph.runs {
            let char_id =
                indexed_value(&mut self.char_styles, &mut self.char_ids, run.style.clone())?;
            indexed_value(
                &mut self.fonts,
                &mut self.font_ids,
                text(&run.style, "font", "Arial").to_owned(),
            )?;
            let mut native_run = XmlNode::new("hp:run").attr("charPrIDRef", char_id);
            if let Some(image) = &run.image {
                native_run.push(self.image(image)?);
            }
            if !run.text.is_empty() || run.image.is_none() {
                let mut native_text = XmlNode::new("hp:t");
                let normalized = run.text.replace("\r\n", "\n").replace('\r', "\n");
                for (line_index, line) in normalized.split('\n').enumerate() {
                    if line_index > 0 {
                        native_text.push(XmlNode::new("hp:lineBreak"));
                    }
                    for (tab_index, content) in line.split('\t').enumerate() {
                        if tab_index > 0 {
                            native_text.push(
                                XmlNode::new("hp:tab")
                                    .attr("width", 4000)
                                    .attr("leader", 0)
                                    .attr("type", 1),
                            );
                        }
                        if let Some(tail) = native_text.children.last_mut() {
                            tail.tail.push_str(content);
                        } else {
                            native_text.text.push_str(content);
                        }
                    }
                }
                native_run.push(native_text);
            }
            root.push(native_run);
        }
        if paragraph.runs.is_empty() && !container_only {
            root.push(
                XmlNode::new("hp:run")
                    .attr("charPrIDRef", 0)
                    .child(XmlNode::new("hp:t")),
            );
        }
        Ok(root)
    }
    fn object_common(root: &mut XmlNode, width: i64, height: i64) {
        root.push(
            node(
                "hp:sz",
                &[
                    ("widthRelTo", "ABSOLUTE"),
                    ("heightRelTo", "ABSOLUTE"),
                    ("protect", "0"),
                ],
            )
            .attr("width", width)
            .attr("height", height),
        );
        root.push(node(
            "hp:pos",
            &[
                ("treatAsChar", "1"),
                ("affectLSpacing", "0"),
                ("flowWithText", "1"),
                ("allowOverlap", "0"),
                ("holdAnchorAndSO", "0"),
                ("vertRelTo", "PARA"),
                ("horzRelTo", "COLUMN"),
                ("vertAlign", "TOP"),
                ("horzAlign", "LEFT"),
                ("vertOffset", "0"),
                ("horzOffset", "0"),
            ],
        ));
        root.push(node(
            "hp:outMargin",
            &[("left", "0"), ("right", "0"), ("top", "0"), ("bottom", "0")],
        ));
    }
    fn next_object(&mut self) -> Result<usize> {
        let id = self.object_id;
        self.object_id = self.object_id.checked_add(1).ok_or_else(|| {
            WorkbenchError::new("LIMIT", "Document numeric attribute exceeds limits.")
        })?;
        Ok(id)
    }
    fn image(&mut self, image: &Image) -> Result<XmlNode> {
        let index = self.next_object()?;
        let extension = match image.media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/bmp" => "bmp",
            _ => {
                return Err(WorkbenchError::new(
                    "BAD_ARGUMENTS",
                    "Unsupported image media type.",
                ))
            }
        };
        self.parts.insert(
            format!("BinData/image{index}.{extension}"),
            image.data.clone(),
        );
        let width = scaled(image.width)?;
        let height = scaled(image.height)?;
        let mut picture = node(
            "hp:pic",
            &[
                ("zOrder", "0"),
                ("numberingType", "PICTURE"),
                ("textWrap", "TOP_AND_BOTTOM"),
                ("textFlow", "BOTH_SIDES"),
                ("reverse", "0"),
            ],
        )
        .attr("id", index);
        picture.push(node("hp:offset", &[("x", "0"), ("y", "0")]));
        for tag in ["hp:orgSz", "hp:curSz"] {
            picture.push(
                XmlNode::new(tag)
                    .attr("width", width)
                    .attr("height", height),
            );
        }
        picture.push(node("hp:flip", &[("horizontal", "0"), ("vertical", "0")]));
        picture.push(
            XmlNode::new("hp:rotationInfo")
                .attr("angle", 0)
                .attr("centerX", width.div_euclid(2))
                .attr("centerY", height.div_euclid(2))
                .attr("rotateimage", 1),
        );
        let mut rendering = XmlNode::new("hp:renderingInfo");
        for tag in ["hc:transMatrix", "hc:scaMatrix", "hc:rotMatrix"] {
            rendering.push(node(
                tag,
                &[
                    ("e1", "1"),
                    ("e2", "0"),
                    ("e3", "0"),
                    ("e4", "0"),
                    ("e5", "1"),
                    ("e6", "0"),
                ],
            ));
        }
        picture.push(rendering);
        picture.push(node(
            "hp:lineShape",
            &[
                ("color", "#000000"),
                ("width", "0"),
                ("style", "NONE"),
                ("endCap", "FLAT"),
                ("headStyle", "NORMAL"),
                ("tailStyle", "NORMAL"),
            ],
        ));
        let mut rectangle = XmlNode::new("hp:imgRect");
        for (tag, x, y) in [
            ("hc:pt0", 0, 0),
            ("hc:pt1", width, 0),
            ("hc:pt2", width, height),
            ("hc:pt3", 0, height),
        ] {
            rectangle.push(XmlNode::new(tag).attr("x", x).attr("y", y));
        }
        picture.push(rectangle);
        picture.push(
            XmlNode::new("hp:imgClip")
                .attr("left", 0)
                .attr("right", width)
                .attr("top", 0)
                .attr("bottom", height),
        );
        picture.push(node(
            "hp:inMargin",
            &[("left", "0"), ("right", "0"), ("top", "0"), ("bottom", "0")],
        ));
        picture.push(
            node(
                "hc:img",
                &[
                    ("bright", "0"),
                    ("contrast", "0"),
                    ("effect", "REAL_PIC"),
                    ("alpha", "0"),
                ],
            )
            .attr("binaryItemIDRef", format!("image{index}")),
        );
        Self::object_common(&mut picture, width, height);
        Ok(picture)
    }
    fn table(&mut self, table: &Table, prefix: Option<XmlNode>) -> Result<XmlNode> {
        self.table_depth += 1;
        require(
            self.table_depth <= hwp_core::xml_io::MAX_DEPTH,
            "LIMIT",
            "XML nesting exceeds limits.",
        )?;
        let mut carrier = self.paragraph(&Paragraph::default(), prefix, true)?;
        let table_id = self.next_object()?;
        let mut native = node(
            "hp:tbl",
            &[
                ("cellSpacing", "0"),
                ("borderFillIDRef", "1"),
                ("pageBreak", "CELL"),
                ("repeatHeader", "0"),
            ],
        )
        .attr("id", table_id)
        .attr("rowCnt", table.rows.len())
        .attr("colCnt", table.widths.len());
        let widths = table
            .widths
            .iter()
            .map(|width| scaled(*width))
            .collect::<Result<Vec<_>>>()?;
        let total_width = widths.iter().try_fold(0i64, |sum, width| {
            sum.checked_add(*width).ok_or_else(|| {
                WorkbenchError::new("LIMIT", "Document numeric attribute exceeds limits.")
            })
        })?;
        let height = i64::try_from(table.rows.len().max(1))
            .ok()
            .and_then(|rows| rows.checked_mul(2400))
            .ok_or_else(|| {
                WorkbenchError::new("LIMIT", "Document numeric attribute exceeds limits.")
            })?;
        Self::object_common(&mut native, total_width, height);
        native.push(node(
            "hp:inMargin",
            &[("left", "0"), ("right", "0"), ("top", "0"), ("bottom", "0")],
        ));
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut tr = XmlNode::new("hp:tr");
            for cell in row {
                let mut borders = cell.borders.clone();
                for (side, inside) in [
                    ("top", "insideH"),
                    ("bottom", "insideH"),
                    ("left", "insideV"),
                    ("right", "insideV"),
                ] {
                    if !borders.contains_key(side) {
                        if let Some(border) = borders.get(inside).cloned() {
                            borders.insert(side.into(), border);
                        }
                    }
                }
                let fill_id = indexed_value(
                    &mut self.fills,
                    &mut self.fill_ids,
                    (borders, cell.shading.clone()),
                )? + 1;
                let mut tc = node(
                    "hp:tc",
                    &[
                        ("name", ""),
                        ("header", "0"),
                        ("hasMargin", "1"),
                        ("protect", "0"),
                    ],
                )
                .attr("borderFillIDRef", fill_id);
                let mut sublist = node(
                    "hp:subList",
                    &[
                        ("id", ""),
                        ("textDirection", "HORIZONTAL"),
                        ("lineWrap", "BREAK"),
                        ("vertAlign", "TOP"),
                        ("linkListIDRef", "0"),
                        ("linkListNextIDRef", "0"),
                        ("textWidth", "0"),
                        ("textHeight", "0"),
                        ("hasTextRef", "0"),
                        ("hasNumRef", "0"),
                    ],
                );
                if cell.blocks.is_empty() {
                    sublist.push(self.paragraph(&Paragraph::default(), None, false)?);
                }
                for block in &cell.blocks {
                    sublist.push(self.block(block, None)?);
                }
                tc.push(sublist);
                tc.push(
                    XmlNode::new("hp:cellAddr")
                        .attr("colAddr", cell.column)
                        .attr("rowAddr", row_index),
                );
                tc.push(
                    XmlNode::new("hp:cellSpan")
                        .attr("colSpan", cell.colspan)
                        .attr("rowSpan", cell.rowspan),
                );
                // Python's slice tolerates a span beyond the widths array. Keep that
                // writer behavior; reopening the malformed grid fails closed.
                let start = cell.column.min(widths.len());
                let end = cell.column.saturating_add(cell.colspan).min(widths.len());
                let cell_width = widths.get(start..end).unwrap_or_default().iter().try_fold(
                    0i64,
                    |sum, width| {
                        sum.checked_add(*width).ok_or_else(|| {
                            WorkbenchError::new(
                                "LIMIT",
                                "Document numeric attribute exceeds limits.",
                            )
                        })
                    },
                )?;
                let cell_height = i64::try_from(cell.rowspan)
                    .ok()
                    .and_then(|span| span.checked_mul(2400))
                    .ok_or_else(|| {
                        WorkbenchError::new("LIMIT", "Document numeric attribute exceeds limits.")
                    })?;
                tc.push(
                    XmlNode::new("hp:cellSz")
                        .attr("width", cell_width)
                        .attr("height", cell_height),
                );
                tc.push(node(
                    "hp:cellMargin",
                    &[("left", "0"), ("right", "0"), ("top", "0"), ("bottom", "0")],
                ));
                tr.push(tc);
            }
            native.push(tr);
        }
        carrier.push(XmlNode::new("hp:run").attr("charPrIDRef", 0).child(native));
        self.table_depth -= 1;
        Ok(carrier)
    }
    fn block(&mut self, block: &Block, prefix: Option<XmlNode>) -> Result<XmlNode> {
        match block {
            Block::Paragraph(paragraph) => self.paragraph(paragraph, prefix, false),
            Block::Table(table) => self.table(table, prefix),
        }
    }
    fn section_prefix(page: &Style) -> Result<XmlNode> {
        let landscape = text(page, "orientation", "portrait") == "landscape";
        let mut width = integer_style(page, "width", 11906);
        let mut height = integer_style(page, "height", 16838);
        if landscape && width > height {
            std::mem::swap(&mut width, &mut height);
        }
        let mut page_pr = XmlNode::new("hp:pagePr")
            .attr("landscape", if landscape { "NARROWLY" } else { "WIDELY" })
            .attr("width", scaled(width)?)
            .attr("height", scaled(height)?)
            .attr("gutterType", "LEFT_ONLY");
        let mut margin = XmlNode::new("hp:margin");
        for key in [
            "left", "right", "top", "bottom", "header", "footer", "gutter",
        ] {
            margin.set(key, scaled(integer_style(page, key, 0))?);
        }
        page_pr.push(margin);
        let section = node(
            "hp:secPr",
            &[
                ("id", ""),
                ("textDirection", "HORIZONTAL"),
                ("spaceColumns", "0"),
                ("tabStop", "4000"),
                ("masterPageCnt", "0"),
            ],
        )
        .child(page_pr);
        let columns = node(
            "hp:colPr",
            &[
                ("id", ""),
                ("type", "NEWSPAPER"),
                ("layout", "LEFT"),
                ("colCount", "1"),
                ("sameSz", "1"),
                ("sameGap", "0"),
            ],
        );
        Ok(XmlNode::new("hp:run")
            .attr("charPrIDRef", 0)
            .child(section)
            .child(XmlNode::new("hp:ctrl").child(columns)))
    }
    fn number_id(&mut self, number: &Style) -> Result<usize> {
        let indices = if text(number, "kind", "") == "bullet" {
            &mut self.bullet_ids
        } else {
            &mut self.decimal_ids
        };
        let key = immutable_value(number)?;
        let next = indices.len() + 1;
        Ok(*indices.entry(key).or_insert(next))
    }
    fn numbering(&mut self, refs: &mut XmlNode) -> Result<()> {
        let mut numberings = XmlNode::new("hh:numberings").attr(
            "itemCnt",
            self.numbers
                .iter()
                .filter(|number| text(number, "kind", "") != "bullet")
                .count(),
        );
        let mut bullets = XmlNode::new("hh:bullets").attr(
            "itemCnt",
            self.numbers
                .iter()
                .filter(|number| text(number, "kind", "") == "bullet")
                .count(),
        );
        for number in self.numbers.clone() {
            let index = self.number_id(&number)?;
            let head = node(
                "hh:paraHead",
                &[
                    ("align", "LEFT"),
                    ("useInstWidth", "1"),
                    ("autoIndent", "1"),
                    ("widthAdjust", "0"),
                    ("textOffsetType", "PERCENT"),
                    ("textOffset", "50"),
                    ("charPrIDRef", "0"),
                ],
            )
            .attr(
                "level",
                integer_style(&number, "level", 0).saturating_add(1),
            );
            if text(&number, "kind", "") == "bullet" {
                let bullet = text(&number, "text", "•")
                    .chars()
                    .next()
                    .map(|c| c.to_string())
                    .unwrap_or_default();
                bullets.push(
                    XmlNode::new("hh:bullet")
                        .attr("id", index)
                        .attr("char", bullet)
                        .attr("useImage", 0)
                        .child(head),
                );
            } else {
                let start = integer_style(&number, "start", 1);
                numberings.push(
                    XmlNode::new("hh:numbering")
                        .attr("id", index)
                        .attr("start", start)
                        .child(
                            head.attr("start", start)
                                .attr("numFormat", "DIGIT")
                                .text(text(&number, "text", "%1.").replace('%', "^")),
                        ),
                );
            }
        }
        refs.push(numberings);
        refs.push(bullets);
        Ok(())
    }
    fn header(&mut self) -> Result<XmlNode> {
        let mut root = namespaces(
            XmlNode::new("hh:head")
                .attr("version", "1.5")
                .attr("secCnt", self.document.sections.len()),
        );
        root.push(node(
            "hh:beginNum",
            &[
                ("page", "1"),
                ("footnote", "1"),
                ("endnote", "1"),
                ("pic", "1"),
                ("tbl", "1"),
                ("equation", "1"),
            ],
        ));
        let mut refs = XmlNode::new("hh:refList");
        let mut fontfaces = XmlNode::new("hh:fontfaces").attr("itemCnt", 7);
        for language in LANGUAGES {
            let mut face = XmlNode::new("hh:fontface")
                .attr("lang", language)
                .attr("fontCnt", self.fonts.len());
            for (index, font) in self.fonts.iter().enumerate() {
                face.push(
                    XmlNode::new("hh:font")
                        .attr("id", index)
                        .attr("face", font)
                        .attr("type", "TTF")
                        .attr("isEmbedded", 0),
                );
            }
            fontfaces.push(face);
        }
        refs.push(fontfaces);
        let mut fills = XmlNode::new("hh:borderFills").attr("itemCnt", self.fills.len());
        for (index, (borders, shading)) in self.fills.iter().enumerate() {
            let mut fill = node(
                "hh:borderFill",
                &[("threeD", "0"), ("shadow", "0"), ("centerLine", "NONE")],
            )
            .attr("id", index + 1);
            for side in ["left", "right", "top", "bottom"] {
                let empty = Style::new();
                let border = borders
                    .get(side)
                    .and_then(Value::as_object)
                    .unwrap_or(&empty);
                fill.push(
                    XmlNode::new(format!("hh:{side}Border"))
                        .attr("type", border_type(text(border, "kind", "")))
                        .attr(
                            "width",
                            format!("{:.4} mm", float(border, "size", 4.0) * MM_PER_BORDER_UNIT),
                        )
                        .attr("color", format!("#{}", text(border, "colour", "000000"))),
                );
            }
            if let Some(shading) = shading.as_ref().filter(|value| !value.is_empty()) {
                fill.push(
                    XmlNode::new("hc:fillBrush").child(
                        XmlNode::new("hc:winBrush")
                            .attr("faceColor", format!("#{shading}"))
                            .attr("hatchColor", "#000000")
                            .attr("alpha", 0),
                    ),
                );
            }
            fills.push(fill);
        }
        refs.push(fills);
        let mut characters =
            XmlNode::new("hh:charProperties").attr("itemCnt", self.char_styles.len());
        for (index, style) in self.char_styles.iter().enumerate() {
            let colour = format!("#{}", text(style, "colour", "000000"));
            let mut char = node(
                "hh:charPr",
                &[
                    ("shadeColor", "none"),
                    ("useFontSpace", "0"),
                    ("useKerning", "0"),
                ],
            )
            .attr("id", index)
            .attr(
                "height",
                (float(style, "size", 11.0) * 100.0).round_ties_even() as i64,
            )
            .attr("textColor", &colour);
            let font_id = self
                .font_ids
                .get(&immutable_value(&text(style, "font", "Arial"))?)
                .copied()
                .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Missing indexed font."))?;
            for (tag, value) in [
                ("hh:fontRef", font_id as i64),
                ("hh:ratio", 100),
                ("hh:spacing", 0),
                ("hh:relSz", 100),
                ("hh:offset", 0),
            ] {
                let mut property = XmlNode::new(tag);
                for key in FONT_KEYS {
                    property.set(key, value);
                }
                char.push(property);
            }
            if flag(style, "bold") {
                char.push(XmlNode::new("hh:bold"));
            }
            if flag(style, "italic") {
                char.push(XmlNode::new("hh:italic"));
            }
            let underline = text(style, "underline", "none");
            char.push(
                XmlNode::new("hh:underline")
                    .attr(
                        "type",
                        if underline == "none" {
                            "NONE"
                        } else {
                            "BOTTOM"
                        },
                    )
                    .attr("shape", underline_type(underline))
                    .attr("color", &colour),
            );
            char.push(
                XmlNode::new("hh:strikeout")
                    .attr(
                        "shape",
                        if flag(style, "strike") {
                            "SOLID"
                        } else {
                            "NONE"
                        },
                    )
                    .attr("color", colour),
            );
            characters.push(char);
        }
        refs.push(characters);
        refs.push(
            XmlNode::new("hh:tabProperties").attr("itemCnt", 1).child(
                XmlNode::new("hh:tabPr")
                    .attr("id", 0)
                    .attr("autoTabLeft", 0)
                    .attr("autoTabRight", 0),
            ),
        );
        self.numbering(&mut refs)?;
        let mut properties =
            XmlNode::new("hh:paraProperties").attr("itemCnt", self.para_styles.len());
        for (index, style) in self.para_styles.clone().iter().enumerate() {
            let mut para = node(
                "hh:paraPr",
                &[
                    ("tabPrIDRef", "0"),
                    ("condense", "0"),
                    ("fontLineHeight", "0"),
                    ("snapToGrid", "0"),
                ],
            )
            .attr("id", index);
            let alignment = match text(style, "alignment", "left") {
                "both" | "justify" => "JUSTIFY".to_owned(),
                "start" => "LEFT".into(),
                "end" => "RIGHT".into(),
                other => other.to_uppercase(),
            };
            para.push(
                XmlNode::new("hh:align")
                    .attr("horizontal", alignment)
                    .attr("vertical", "BASELINE"),
            );
            let numbering = style
                .get("numbering")
                .and_then(Value::as_object)
                .filter(|number| !number.is_empty());
            let (kind, id, level) = if let Some(numbering) = numbering {
                (
                    if text(numbering, "kind", "") == "bullet" {
                        "BULLET"
                    } else {
                        "NUMBER"
                    },
                    self.number_id(numbering)?,
                    integer_style(numbering, "level", 0),
                )
            } else {
                ("NONE", 0, 0)
            };
            para.push(
                XmlNode::new("hh:heading")
                    .attr("type", kind)
                    .attr("idRef", id)
                    .attr("level", level),
            );
            para.push(
                node(
                    "hh:breakSetting",
                    &[
                        ("breakLatinWord", "KEEP_WORD"),
                        ("breakNonLatinWord", "KEEP_WORD"),
                        ("pageBreakBefore", "0"),
                        ("lineWrap", "BREAK"),
                    ],
                )
                .attr("widowOrphan", u8::from(flag(style, "widowControl")))
                .attr("keepWithNext", u8::from(flag(style, "keepNext")))
                .attr("keepLines", u8::from(flag(style, "keepLines"))),
            );
            let mut margin = XmlNode::new("hh:margin");
            let hanging = integer_style(style, "hanging", 0)
                .checked_neg()
                .ok_or_else(|| {
                    WorkbenchError::new("LIMIT", "Document numeric attribute exceeds limits.")
                })?;
            for (tag, value) in [
                ("hc:intent", integer_style(style, "first", hanging)),
                ("hc:left", integer_style(style, "left", 0)),
                ("hc:right", integer_style(style, "right", 0)),
                ("hc:prev", integer_style(style, "before", 0)),
                ("hc:next", integer_style(style, "after", 0)),
            ] {
                margin.push(
                    XmlNode::new(tag)
                        .attr("value", scaled(value)?)
                        .attr("unit", "HWPUNIT"),
                );
            }
            para.push(margin);
            let rule = text(style, "lineRule", "auto");
            let line = integer_style(style, "line", 240);
            let spacing_type = match rule {
                "exact" => "FIXED",
                "atLeast" => "AT_LEAST",
                _ => "PERCENT",
            };
            let spacing = if rule == "auto" {
                (line as f64 * 100.0 / 240.0).round_ties_even() as i64
            } else {
                scaled(line)?
            };
            para.push(
                XmlNode::new("hh:lineSpacing")
                    .attr("type", spacing_type)
                    .attr("value", spacing)
                    .attr("unit", "HWPUNIT"),
            );
            properties.push(para);
        }
        refs.push(properties);
        let mut styles = XmlNode::new("hh:styles").attr("itemCnt", 10);
        for index in 0..10 {
            let name = if index == 0 {
                "Normal".into()
            } else {
                format!("Heading {index}")
            };
            styles.push(
                node(
                    "hh:style",
                    &[
                        ("type", "PARA"),
                        ("paraPrIDRef", "0"),
                        ("charPrIDRef", "0"),
                        ("nextStyleIDRef", "0"),
                        ("langID", "1042"),
                        ("lockForm", "0"),
                    ],
                )
                .attr("id", index)
                .attr("name", &name)
                .attr("engName", name),
            );
        }
        refs.push(styles);
        root.push(refs);
        Ok(root)
    }
    fn write(mut self) -> Result<Vec<u8>> {
        let mut manifest = XmlNode::new("opf:package")
            .attr("xmlns:opf", OPF)
            .attr("version", "3.0");
        let mut items = XmlNode::new("opf:manifest");
        let mut spine = XmlNode::new("opf:spine");
        items.push(
            XmlNode::new("opf:item")
                .attr("id", "header")
                .attr("href", "Contents/header.xml")
                .attr("media-type", "application/xml"),
        );
        for (index, section) in self.document.sections.iter().enumerate() {
            let mut root = namespaces(XmlNode::new("hs:sec"));
            let mut prefix = Some(Self::section_prefix(&section.page)?);
            if section.blocks.is_empty() {
                root.push(self.paragraph(&Paragraph::default(), prefix.take(), false)?);
            }
            for block in &section.blocks {
                root.push(self.block(block, prefix.take())?);
            }
            let name = format!("Contents/section{index}.xml");
            self.parts.insert(name.clone(), encoded(&root)?);
            items.push(
                XmlNode::new("opf:item")
                    .attr("id", format!("section{index}"))
                    .attr("href", name)
                    .attr("media-type", "application/xml"),
            );
            spine.push(XmlNode::new("opf:itemref").attr("idref", format!("section{index}")));
        }
        for name in self
            .parts
            .keys()
            .filter(|name| name.starts_with("BinData/"))
        {
            let binary = name.rsplit('/').next().unwrap_or(name);
            let (id, extension) = binary
                .rsplit_once('.')
                .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Missing image extension."))?;
            let media_type = match extension {
                "png" => "image/png",
                "jpg" => "image/jpeg",
                "gif" => "image/gif",
                "bmp" => "image/bmp",
                _ => {
                    return Err(WorkbenchError::new(
                        "BAD_ARGUMENTS",
                        "Unsupported image media type.",
                    ))
                }
            };
            items.push(
                XmlNode::new("opf:item")
                    .attr("id", id)
                    .attr("href", name)
                    .attr("media-type", media_type)
                    .attr("isEmbeded", 1),
            );
        }
        manifest.push(items);
        manifest.push(spine);
        let header = self.header()?;
        self.parts
            .insert("Contents/header.xml".into(), encoded(&header)?);
        self.parts
            .insert("Contents/content.hpf".into(), encoded(&manifest)?);
        self.parts.insert(
            "version.xml".into(),
            b"<HCFVersion major=\"5\" minor=\"0\" micro=\"0\" buildNumber=\"0\"/>".to_vec(),
        );
        self.parts.insert("META-INF/container.xml".into(),b"<container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"Contents/content.hpf\" media-type=\"application/hwp+zip\"/></rootfiles></container>".to_vec());
        write_package(&self.parts)
    }
}
/// Write supported rich content to a new deterministic HWPX package.
pub fn write_hwpx(document: &RichDocument) -> Result<Vec<u8>> {
    crate::model::check_output_controls(document, crate::model::MAX_OUTPUT_CONTROLS)?;
    Writer::new(document)?.write()
}
