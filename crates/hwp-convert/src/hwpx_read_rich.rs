//! Read supported rich HWPX content and count each unsupported feature.
use crate::hwpx_write::{HC, HH, HP, HWP_PER_TWIP, MM_PER_BORDER_UNIT, OPF};
use crate::model::{
    style_payload_bytes, Block, Cell, Image, Paragraph, RichDocument, Run, Section, Style, Table,
};
use crate::package::{integer, read_package, Parts, Trees, XmlExt, MAX_COLUMNS, MAX_GRID_WORK};
use hwp_core::errors::{require, Result, WorkbenchError, MAX_EXPANDED, MAX_NODES};
use hwp_core::xml_io::Element;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn numeric(value: Option<&str>, default: i64) -> Result<i64> {
    integer(value, default, -10_000_000, 10_000_000)
}
fn twips(value: Option<&str>, default: i64) -> Result<i64> {
    Ok(
        (numeric(value, default * HWP_PER_TWIP)? as f64 / HWP_PER_TWIP as f64).round_ties_even()
            as i64,
    )
}
fn colour(value: Option<&str>) -> Option<String> {
    let text = value?.strip_prefix('#').unwrap_or(value?);
    (text.len() == 6 && text.bytes().all(|c| c.is_ascii_hexdigit()))
        .then(|| text.to_ascii_uppercase())
}
fn matches(node: &Element, namespace: &str, name: &str) -> bool {
    node.name.namespace.as_deref() == Some(namespace) && node.name.local == name
}
fn key(value: Option<&str>) -> Option<String> {
    value.map(str::to_owned)
}
fn first<'a>(node: &'a Element, namespace: &str, name: &str) -> Option<&'a Element> {
    node.iter()
        .skip(1)
        .find(|item| matches(item, namespace, name))
}
fn section_number(name: &str) -> Option<&str> {
    let number = name
        .strip_prefix("Contents/section")?
        .strip_suffix(".xml")?;
    (!number.is_empty() && number.bytes().all(|c| c.is_ascii_digit())).then_some(number)
}
fn feature(name: &str) -> Option<&'static str> {
    Some(match name {
        "fieldBegin" => "fields",
        "equation" => "equations",
        "footNote" => "footnotes",
        "endNote" => "endnotes",
        "header" => "headers",
        "footer" => "footers",
        "rect" | "ellipse" | "line" | "polygon" | "curve" | "container" => "shapes",
        "drawText" | "textBox" => "textBoxes",
        "memo" => "comments",
        "chart" => "charts",
        "ole" => "embeddedObjects",
        "trackChange" | "trackChangeBegin" => "trackedChanges",
        "bookmark" => "bookmarks",
        "pageNum" => "pageNumbering",
        "video" => "video",
        "compose" => "composedText",
        _ => return None,
    })
}

type ReferenceMap<T> = HashMap<Option<String>, T>;
struct Reader<'a> {
    parts: &'a Parts,
    trees: &'a Trees,
    document: RichDocument,
    count: usize,
    image_bytes: usize,
    image_budget: usize,
    style_budget: usize,
    retained_style_bytes: usize,
    cached_style_bytes: usize,
    chars: ReferenceMap<Style>,
    paras: ReferenceMap<&'a Element>,
    styles: ReferenceMap<i64>,
    fills: ReferenceMap<(Style, Option<String>)>,
    numbers: ReferenceMap<HashMap<i64, Style>>,
    bullets: ReferenceMap<Style>,
    media: ReferenceMap<(String, String)>,
}
impl<'a> Reader<'a> {
    fn retain_style(&mut self, style: &Style) -> Result<()> {
        self.retained_style_bytes = self
            .retained_style_bytes
            .saturating_add(style_payload_bytes(style));
        require(
            self.retained_style_bytes <= self.style_budget,
            "LIMIT",
            "Expanded conversion styles exceed limits.",
        )
    }
    fn cache_style(&mut self, style: &Style) -> Result<()> {
        self.cached_style_bytes = self
            .cached_style_bytes
            .saturating_add(style_payload_bytes(style));
        require(
            self.cached_style_bytes <= self.style_budget,
            "LIMIT",
            "Expanded conversion styles exceed limits.",
        )
    }
    fn retain_paragraph(&mut self, paragraph: &Paragraph) -> Result<()> {
        self.retain_style(&paragraph.style)?;
        if let Some(numbering) = &paragraph.numbering {
            self.retain_style(numbering)?;
        }
        Ok(())
    }

    fn new(parts: &'a Parts, trees: &'a Trees) -> Result<Self> {
        let mut reader = Self {
            parts,
            trees,
            document: RichDocument::default(),
            count: 0,
            image_bytes: 0,
            image_budget: MAX_EXPANDED,
            style_budget: MAX_EXPANDED,
            retained_style_bytes: 0,
            cached_style_bytes: 0,
            chars: HashMap::new(),
            paras: HashMap::new(),
            styles: HashMap::new(),
            fills: HashMap::new(),
            numbers: HashMap::new(),
            bullets: HashMap::new(),
            media: HashMap::new(),
        };
        reader.references()?;
        for (name, tree) in trees {
            for item in tree.iter() {
                if item.name.namespace.as_deref() == Some(HP) {
                    if let Some(feature) = feature(&item.name.local) {
                        reader.document.loss(feature, 1);
                    }
                }
            }
            if name.starts_with("Contents/masterpage") {
                reader.document.loss("masterPages", 1);
            }
        }
        for name in parts.keys() {
            let lower = name.to_lowercase();
            if ["signature", "scripts/", "vbascript"]
                .iter()
                .any(|word| lower.contains(word))
            {
                reader.document.loss("activeOrSignedContent", 1);
            }
            if name.starts_with("BinData/") && !reader.media.values().any(|(item, _)| item == name)
            {
                reader.document.loss("unreferencedBinaryParts", 1);
            }
        }
        Ok(reader)
    }
    fn references(&mut self) -> Result<()> {
        let header = self.trees.get("Contents/header.xml").ok_or_else(|| {
            WorkbenchError::new("NOT_HWPX", "Missing HWPX header or content manifest.")
        })?;
        let mut fonts = ReferenceMap::new();
        for face in header.iter().filter(|node| matches(node, HH, "fontface")) {
            if matches!(face.get("lang"), Some("LATIN" | "HANGUL")) {
                for font in &face.children {
                    fonts.insert(
                        key(font.get("id")),
                        font.get("face").unwrap_or("Arial").to_owned(),
                    );
                }
            }
        }
        for char in header.iter().filter(|node| matches(node, HH, "charPr")) {
            let font = char.child(HH, "fontRef").and_then(|font| {
                font.get("latin")
                    .filter(|value| !value.is_empty())
                    .or_else(|| font.get("hangul"))
            });
            let underline = char.child(HH, "underline");
            let underline_style = if underline.is_some_and(|node| node.get("type") != Some("NONE"))
            {
                match underline.and_then(|node| node.get("shape")) {
                    Some("DOUBLE") => "double",
                    Some("DOT") => "dotted",
                    Some("DASH") => "dash",
                    Some("WAVE") => "wave",
                    _ => "single",
                }
            } else {
                "none"
            };
            let style = json!({"size":integer(char.get("height"),1100,1,10_000_000)? as f64/100.0,"colour":colour(char.get("textColor")).unwrap_or_else(|| "000000".into()),"font": fonts.get(&key(font)).map(String::as_str).unwrap_or("Arial"), "bold": char.child(HH,"bold").is_some(),"italic":char.child(HH,"italic").is_some(),"underline":underline_style,"strike":char.child(HH,"strikeout").is_some_and(|node| !matches!(node.get("shape"), None | Some("NONE")))});
            if let Value::Object(style) = style {
                self.cache_style(&style)?;
                self.chars.insert(key(char.get("id")), style);
            }
            for item in &char.children {
                let tag = item.name.local.as_str();
                let lost = match tag {
                    "fontRef" | "bold" | "italic" | "underline" | "strikeout" => false,
                    "ratio" | "spacing" | "relSz" | "offset" => {
                        let expected = if matches!(tag, "ratio" | "relSz") {
                            "100"
                        } else {
                            "0"
                        };
                        item.attributes.iter().any(|attr| attr.value != expected)
                    }
                    "outline" | "shadow" => item.get("type").unwrap_or("NONE") != "NONE",
                    _ => true,
                };
                if lost {
                    self.document.loss(&format!("characterProperty:{tag}"), 1);
                }
            }
        }
        for para in header.iter().filter(|node| matches(node, HH, "paraPr")) {
            self.paras.insert(key(para.get("id")), para);
        }
        for style in header.iter().filter(|node| matches(node, HH, "style")) {
            let name = style
                .get("engName")
                .filter(|name| !name.is_empty())
                .or_else(|| style.get("name"))
                .unwrap_or("")
                .to_lowercase();
            let heading = name
                .strip_prefix("heading")
                .and_then(|number| {
                    let number = number.trim_start();
                    (number.len() == 1)
                        .then(|| number.parse::<i64>().ok())
                        .flatten()
                        .filter(|number| (1..=9).contains(number))
                })
                .unwrap_or(0);
            self.styles.insert(key(style.get("id")), heading);
        }
        for fill in header.iter().filter(|node| matches(node, HH, "borderFill")) {
            let mut borders = Style::new();
            for side in ["left", "right", "top", "bottom"] {
                let Some(border) = fill.child(HH, &format!("{side}Border")) else {
                    continue;
                };
                if border.get("type") == Some("NONE") {
                    continue;
                }
                let width = border
                    .get("width")
                    .unwrap_or("0.1 mm")
                    .strip_suffix(" mm")
                    .unwrap_or(border.get("width").unwrap_or("0.1 mm"));
                let mut pieces = width.split('.');
                let valid_part =
                    |part: &str| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit());
                let valid = pieces.next().is_some_and(valid_part)
                    && pieces.next().is_none_or(valid_part)
                    && pieces.next().is_none();
                require(
                    valid && width.len() <= 20,
                    "BAD_XML",
                    "Invalid HWPX border width.",
                )?;
                let width = width
                    .parse::<f64>()
                    .map_err(|_| WorkbenchError::new("BAD_XML", "Invalid HWPX border width."))?;
                let border_size = (width / MM_PER_BORDER_UNIT).round_ties_even();
                require(
                    border_size.is_finite() && border_size >= 0.0 && border_size < i64::MAX as f64,
                    "LIMIT",
                    "Document numeric attribute exceeds limits.",
                )?;
                let (kind, known) = match border.get("type") {
                    Some("SOLID") => ("single", true),
                    Some("DOUBLE_SLIM") => ("double", true),
                    Some("DOT") => ("dotted", true),
                    Some("DASH") => ("dashed", true),
                    _ => ("single", false),
                };
                borders.insert(side.into(), json!({"kind":kind,"size":border_size as i64,"colour":colour(border.get("color")).unwrap_or_else(|| "000000".into())}));
                if !known {
                    self.document.loss("borderStyles", 1);
                }
            }
            let shading =
                first(fill, HC, "winBrush").and_then(|brush| colour(brush.get("faceColor")));
            self.cache_style(&borders)?;
            self.fills.insert(key(fill.get("id")), (borders, shading));
            if fill
                .iter()
                .any(|item| matches!(item.name.local.as_str(), "gradBrush" | "imgBrush"))
            {
                self.document.loss("complexCellShading", 1);
            }
        }
        for numbering in header.iter().filter(|node| matches(node, HH, "numbering")) {
            let mut levels = HashMap::new();
            for head in &numbering.children {
                let level = integer(head.get("level"), 1, 1, 10)? - 1;
                if head.get("numFormat") != Some("DIGIT") {
                    continue;
                }
                let number = json!({"kind":"decimal","level":level.min(8),"start": numeric(head.get("start"), numeric(numbering.get("start"),1)?)?,"text":head.text.as_deref().filter(|text| !text.is_empty()).unwrap_or("^1.").replace('^',"%"),"id":numbering.get("id")});
                if let Value::Object(number) = number {
                    self.cache_style(&number)?;
                    levels.insert(level, number);
                }
            }
            self.numbers.insert(key(numbering.get("id")), levels);
        }
        for bullet in header.iter().filter(|node| matches(node, HH, "bullet")) {
            let number = json!({"kind":"bullet","level":(integer(bullet.child(HH,"paraHead").and_then(|head| head.get("level")),1,0,9)?-1).max(0),"start":1,"text":bullet.get("char").unwrap_or("•"),"id":bullet.get("id")});
            if let Value::Object(number) = number {
                self.cache_style(&number)?;
                self.bullets.insert(key(bullet.get("id")), number);
            }
        }
        let manifest = self.trees.get("Contents/content.hpf").ok_or_else(|| {
            WorkbenchError::new("NOT_HWPX", "Missing HWPX header or content manifest.")
        })?;
        for item in manifest.iter().filter(|node| matches(node, OPF, "item")) {
            let name = item.get("href").unwrap_or("");
            let media_type = item.get("media-type").unwrap_or("");
            if self.parts.contains_key(name)
                && (name.starts_with("BinData/") || media_type.starts_with("image/"))
            {
                self.media.insert(
                    key(item.get("id")),
                    (name.to_owned(), media_type.to_owned()),
                );
            }
        }
        Ok(())
    }
    fn para_style(&mut self, node: Option<&Element>) -> Result<(Style, Option<Style>)> {
        let mut style = Style::new();
        let Some(node) = node else {
            return Ok((style, None));
        };
        if let Some(alignment) = node.child(HH, "align") {
            let align = alignment.get("horizontal").unwrap_or("LEFT");
            style.insert(
                "alignment".into(),
                json!(if align == "JUSTIFY" {
                    "both".into()
                } else {
                    align.to_lowercase()
                }),
            );
        }
        let scope = first(node, HP, "default").unwrap_or(node);
        if let Some(margin) = first(scope, HH, "margin") {
            for item in &margin.children {
                let name = match item.name.local.as_str() {
                    "intent" => "first",
                    "left" => "left",
                    "right" => "right",
                    "prev" => "before",
                    "next" => "after",
                    _ => continue,
                };
                style.insert(name.into(), json!(twips(item.get("value"), 0)?));
            }
            if let Some(first) = style
                .get("first")
                .and_then(Value::as_i64)
                .filter(|value| *value < 0)
            {
                style.remove("first");
                style.insert("hanging".into(), json!(-first));
            }
        }
        if let Some(spacing) = first(scope, HH, "lineSpacing") {
            let rule = match spacing.get("type") {
                Some("FIXED") => "exact",
                Some("AT_LEAST") => "atLeast",
                _ => "auto",
            };
            style.insert("lineRule".into(), json!(rule));
            let line = if rule == "auto" {
                (numeric(spacing.get("value"), 100)? as f64 * 240.0 / 100.0).round_ties_even()
                    as i64
            } else {
                twips(spacing.get("value"), 0)?
            };
            style.insert("line".into(), json!(line));
        }
        if let Some(breaking) = node.child(HH, "breakSetting") {
            for (source, target) in [
                ("keepWithNext", "keepNext"),
                ("keepLines", "keepLines"),
                ("widowOrphan", "widowControl"),
                ("pageBreakBefore", "pageBreakBefore"),
            ] {
                if breaking.get(source) == Some("1") {
                    style.insert(target.into(), json!(true));
                }
            }
        }
        let mut number = None;
        if let Some(heading) = node
            .child(HH, "heading")
            .filter(|heading| matches!(heading.get("type"), Some("NUMBER" | "BULLET")))
        {
            number = if heading.get("type") == Some("BULLET") {
                self.bullets.get(&key(heading.get("idRef"))).cloned()
            } else {
                self.numbers
                    .get(&key(heading.get("idRef")))
                    .and_then(|levels| levels.get(&integer(heading.get("level"), 0, 0, 9).ok()?))
                    .cloned()
            };
            // Parse a numeric level even when the referenced numbering does not exist.
            if heading.get("type") != Some("BULLET") {
                integer(heading.get("level"), 0, 0, 9)?;
            }
            if number.as_ref().is_none_or(Style::is_empty) {
                self.document.loss("complexNumbering", 1);
            }
        }
        Ok((style, number))
    }
    fn image(&mut self, picture: &Element) -> Result<Option<Image>> {
        let image = first(picture, HC, "img");
        let Some((name, media_type)) = self
            .media
            .get(&key(image.and_then(|node| node.get("binaryItemIDRef"))))
            .cloned()
        else {
            self.document.loss("unavailableImages", 1);
            return Ok(None);
        };
        if !matches!(
            media_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/bmp"
        ) {
            self.document.loss("unsupportedImageTypes", 1);
            return Ok(None);
        }
        let size = picture
            .child(HP, "sz")
            .or_else(|| picture.child(HP, "curSz"));
        let width = twips(size.and_then(|node| node.get("width")), 1440)?;
        let height = twips(size.and_then(|node| node.get("height")), 1440)?;
        if picture
            .child(HP, "pos")
            .is_some_and(|node| node.get("treatAsChar") != Some("1"))
        {
            self.document.loss("floatingImagePlacement", 1);
        }
        if picture
            .child(HP, "rotationInfo")
            .is_some_and(|node| node.get("angle").unwrap_or("0") != "0")
        {
            self.document.loss("imageRotation", 1);
        }
        if picture.child(HP, "flip").is_some_and(|node| {
            ["horizontal", "vertical"]
                .iter()
                .any(|key| !matches!(node.get(key).unwrap_or("0"), "0" | "false"))
        }) {
            self.document.loss("imageTransforms", 1);
        }
        if let (Some(clip), Some(original)) =
            (picture.child(HP, "imgClip"), picture.child(HP, "orgSz"))
        {
            let bounds = [
                numeric(clip.get("left"), 0)?,
                numeric(clip.get("top"), 0)?,
                numeric(clip.get("right"), 0)?,
                numeric(clip.get("bottom"), 0)?,
            ];
            let full = [
                0,
                0,
                numeric(original.get("width"), 0)?,
                numeric(original.get("height"), 0)?,
            ];
            if bounds.iter().any(|value| *value != 0) && bounds != full {
                self.document.loss("imageCropping", 1);
            }
        }
        if image.is_some_and(|node| {
            ["bright", "contrast", "alpha"]
                .iter()
                .any(|key| node.get(key).unwrap_or("0") != "0")
                || node.get("effect").unwrap_or("REAL_PIC") != "REAL_PIC"
        }) {
            self.document.loss("imageEffects", 1);
        }
        let source = self
            .parts
            .get(&name)
            .ok_or_else(|| WorkbenchError::new("NOT_HWPX", "Missing HWPX image part."))?;
        // The rich model owns a Vec per image,
        // so repeated references need a combined allocation budget.
        self.image_bytes = self.image_bytes.saturating_add(source.len());
        require(
            self.image_bytes <= self.image_budget,
            "LIMIT",
            "Expanded conversion images exceed limits.",
        )?;
        let data = source.clone();
        let name = name.rsplit('/').next().unwrap_or(&name).to_owned();
        Ok(Some(Image {
            data,
            media_type,
            width: width.max(1),
            height: height.max(1),
            name,
        }))
    }
    fn text(&mut self, node: &Element) -> String {
        let mut text = node.text.clone().unwrap_or_default();
        for item in &node.children {
            let tag = item.name.local.as_str();
            match tag {
                "lineBreak" => text.push('\n'),
                "tab" => text.push('\t'),
                "nbSpace" => text.push('\u{a0}'),
                "fwSpace" => text.push('\u{2007}'),
                "hyphen" => text.push('\u{ad}'),
                "columnBreak" => {
                    text.push('\n');
                    self.document.loss("columnBreaks", 1);
                }
                _ => {
                    self.document.loss(&format!("textControl:{tag}"), 1);
                    text.push_str(&self.text(item));
                }
            }
            text.push_str(item.tail.as_deref().unwrap_or(""));
        }
        text
    }
    fn paragraph(&mut self, node: &Element) -> Result<Vec<Block>> {
        self.count += 1;
        require(
            self.count <= MAX_NODES,
            "LIMIT",
            "Too many conversion paragraphs.",
        )?;
        let (mut style, numbering) =
            self.para_style(self.paras.get(&key(node.get("paraPrIDRef"))).copied())?;
        // Keep pageBreakBefore in the style when pageBreak is explicit.
        let page_break = if node.get("pageBreak") == Some("1") {
            true
        } else {
            style
                .remove("pageBreakBefore")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        };
        let mut paragraph = Paragraph {
            style: style.clone(),
            numbering,
            heading: self
                .styles
                .get(&key(node.get("styleIDRef")))
                .copied()
                .unwrap_or(0),
            page_break,
            ..Default::default()
        };
        let mut blocks = vec![];
        let mut has_content = false;
        let has_table = node
            .children
            .iter()
            .filter(|run| matches(run, HP, "run"))
            .any(|run| run.children.iter().any(|item| matches(item, HP, "tbl")));
        for run in node.children.iter().filter(|run| matches(run, HP, "run")) {
            let char_style = self
                .chars
                .get(&key(run.get("charPrIDRef")))
                .cloned()
                .unwrap_or_default();
            for item in &run.children {
                let tag = item.name.local.as_str();
                match tag {
                    "t" => {
                        let text = self.text(item);
                        if has_table && text.is_empty() {
                            continue;
                        }
                        self.retain_style(&char_style)?;
                        paragraph.runs.push(Run {
                            text,
                            style: char_style.clone(),
                            image: None,
                        });
                        has_content = true;
                    }
                    "pic" => {
                        if let Some(image) = self.image(item)? {
                            self.retain_style(&char_style)?;
                            paragraph.runs.push(Run {
                                style: char_style.clone(),
                                image: Some(image),
                                ..Default::default()
                            });
                            has_content = true;
                        }
                    }
                    "tbl" => {
                        if has_content {
                            self.retain_paragraph(&paragraph)?;
                            blocks.push(Block::Paragraph(paragraph));
                            paragraph = Paragraph {
                                style: style.clone(),
                                ..Default::default()
                            };
                            has_content = false;
                        }
                        blocks.push(Block::Table(self.table(item)?));
                    }
                    "secPr" | "ctrl" => {
                        for control in &item.children {
                            let name = control.name.local.as_str();
                            if !matches!(
                                name,
                                "colPr"
                                    | "pagePr"
                                    | "margin"
                                    | "grid"
                                    | "startNum"
                                    | "visibility"
                                    | "lineNumberShape"
                                    | "footNotePr"
                                    | "endNotePr"
                                    | "pageBorderFill"
                                    | "header"
                                    | "footer"
                                    | "footNote"
                                    | "endNote"
                                    | "fieldBegin"
                                    | "fieldEnd"
                            ) {
                                self.document.loss(&format!("control:{name}"), 1);
                            }
                            if name == "colPr" && control.get("colCount").unwrap_or("1") != "1" {
                                self.document.loss("multipleColumns", 1);
                            }
                        }
                    }
                    "equation" | "ole" | "chart" | "rect" | "ellipse" | "line" | "polygon"
                    | "curve" | "container" | "fieldBegin" | "fieldEnd" | "footNote"
                    | "endNote" | "header" | "footer" => {}
                    _ => self.document.loss(&format!("runElement:{tag}"), 1),
                }
            }
        }
        if has_content || blocks.is_empty() {
            if paragraph.runs.is_empty() {
                paragraph.runs.push(Run::default());
            }
            self.retain_paragraph(&paragraph)?;
            blocks.push(Block::Paragraph(paragraph));
        }
        Ok(blocks)
    }
    fn table(&mut self, node: &Element) -> Result<Table> {
        let count = integer(node.get("colCnt"), 1, 1, MAX_COLUMNS as i64)? as usize;
        let row_count = integer(node.get("rowCnt"), 1, 1, MAX_NODES as i64)? as usize;
        require(
            count * row_count <= MAX_GRID_WORK,
            "LIMIT",
            "Table grid exceeds work limit.",
        )?;
        let mut table = Table {
            rows: vec![vec![]; row_count],
            widths: vec![0; count],
        };
        let mut occupancy = HashSet::new();
        let mut constraints = vec![];
        for (row_index, row) in node
            .children
            .iter()
            .filter(|row| matches(row, HP, "tr"))
            .enumerate()
        {
            let mut column = 0;
            for tc in row.children.iter().filter(|tc| matches(tc, HP, "tc")) {
                let address = tc.child(HP, "cellAddr");
                let span = tc.child(HP, "cellSpan");
                let row_at = integer(
                    address.and_then(|item| item.get("rowAddr")),
                    row_index as i64,
                    0,
                    row_count as i64 - 1,
                )? as usize;
                let col_at = integer(
                    address.and_then(|item| item.get("colAddr")),
                    column as i64,
                    0,
                    count as i64 - 1,
                )? as usize;
                let colspan = integer(
                    span.and_then(|item| item.get("colSpan")),
                    1,
                    1,
                    count as i64,
                )? as usize;
                let rowspan = integer(
                    span.and_then(|item| item.get("rowSpan")),
                    1,
                    1,
                    row_count as i64,
                )? as usize;
                require(
                    col_at + colspan <= count && row_at + rowspan <= row_count,
                    "BAD_XML",
                    "Table span leaves grid.",
                )?;
                for r in row_at..row_at + rowspan {
                    for c in col_at..col_at + colspan {
                        require(
                            occupancy.insert((r, c)),
                            "BAD_XML",
                            "Overlapping native table cells.",
                        )?;
                    }
                }
                let (borders, shading) = self
                    .fills
                    .get(&key(tc.get("borderFillIDRef")))
                    .cloned()
                    .unwrap_or_default();
                self.retain_style(&borders)?;
                let mut cell = Cell {
                    column: col_at,
                    colspan,
                    rowspan,
                    borders,
                    shading,
                    ..Default::default()
                };
                if let Some(sublist) = tc.child(HP, "subList") {
                    for paragraph in sublist
                        .children
                        .iter()
                        .filter(|paragraph| matches(paragraph, HP, "p"))
                    {
                        cell.blocks.extend(self.paragraph(paragraph)?);
                    }
                }
                table
                    .rows
                    .get_mut(row_at)
                    .ok_or_else(|| WorkbenchError::new("BAD_XML", "Table span leaves grid."))?
                    .push(cell);
                if let Some(size) = tc.child(HP, "cellSz") {
                    let width = twips(size.get("width"), 1440)?;
                    constraints.push((col_at, colspan, width));
                    if colspan == 1 {
                        if let Some(target) = table.widths.get_mut(col_at) {
                            *target = width;
                        }
                    }
                }
                column = col_at + colspan;
            }
        }
        for (column, span, width) in constraints {
            let widths = table
                .widths
                .get_mut(column..column + span)
                .ok_or_else(|| WorkbenchError::new("BAD_XML", "Table span leaves grid."))?;
            let unknown = widths.iter().filter(|width| **width == 0).count();
            if unknown > 0 {
                let remaining = width - widths.iter().sum::<i64>();
                let share = (remaining as f64 / unknown as f64).round_ties_even() as i64;
                for width in widths.iter_mut().filter(|width| **width == 0) {
                    *width = share.max(1);
                }
            }
        }
        for width in &mut table.widths {
            if *width == 0 {
                *width = 1440;
            }
        }
        for row in &mut table.rows {
            row.sort_by_key(|cell| cell.column);
        }
        Ok(table)
    }
    fn page(root: &Element) -> Result<Style> {
        let mut page = Section::default().page;
        let Some(properties) = first(root, HP, "pagePr") else {
            return Ok(page);
        };
        for name in ["width", "height"] {
            page.insert(
                name.into(),
                json!(twips(
                    properties.get(name),
                    page.get(name).and_then(Value::as_i64).unwrap_or(0)
                )?),
            );
        }
        let landscape = properties.get("landscape") == Some("NARROWLY");
        page.insert(
            "orientation".into(),
            json!(if landscape { "landscape" } else { "portrait" }),
        );
        if landscape
            && page.get("width").and_then(Value::as_i64)
                < page.get("height").and_then(Value::as_i64)
        {
            let width = page.get("width").cloned().unwrap_or(Value::Null);
            let height = page.get("height").cloned().unwrap_or(Value::Null);
            page.insert("width".into(), height);
            page.insert("height".into(), width);
        }
        if let Some(margin) = properties.child(HP, "margin") {
            for name in [
                "left", "right", "top", "bottom", "header", "footer", "gutter",
            ] {
                page.insert(
                    name.into(),
                    json!(twips(
                        margin.get(name),
                        page.get(name).and_then(Value::as_i64).unwrap_or(0)
                    )?),
                );
            }
        }
        Ok(page)
    }
    fn read(mut self) -> Result<RichDocument> {
        let mut names = vec![];
        for name in self.trees.keys() {
            if let Some(number) = section_number(name) {
                names.push((numeric(Some(number), 0)?, name));
            }
        }
        names.sort_by_key(|(index, _)| *index);
        for (_, name) in names {
            let root = self
                .trees
                .get(name)
                .ok_or_else(|| WorkbenchError::new("NOT_HWPX", "No body sections found."))?;
            let mut section = Section {
                page: Self::page(root)?,
                ..Default::default()
            };
            for item in &root.children {
                if matches(item, HP, "p") {
                    section.blocks.extend(self.paragraph(item)?);
                } else {
                    self.document
                        .loss(&format!("sectionElement:{}", item.name.local), 1);
                }
            }
            self.document.sections.push(section);
        }
        Ok(self.document)
    }
}
/// Parse HWPX rich content with the same package and body requirements as native open.
pub fn read_hwpx(payload: &[u8]) -> Result<RichDocument> {
    let (parts, trees) = read_package(payload)?;
    require(
        parts
            .get("mimetype")
            .is_some_and(|value| value.as_slice().trim_ascii() == b"application/hwp+zip"),
        "NOT_HWPX",
        "Missing or invalid HWPX mimetype.",
    )?;
    require(
        parts.contains_key("Contents/header.xml") && parts.contains_key("Contents/content.hpf"),
        "NOT_HWPX",
        "Missing HWPX header or content manifest.",
    )?;
    require(
        parts.keys().any(|name| section_number(name).is_some()),
        "NOT_HWPX",
        "No body sections found.",
    )?;
    let text_nodes = trees
        .iter()
        .filter(|(name, _)| section_number(name).is_some())
        .map(|(_, tree)| tree.iter().filter(|node| matches(node, HP, "t")).count())
        .sum::<usize>();
    require(text_nodes <= MAX_NODES, "LIMIT", "Too many text nodes.")?;
    Reader::new(&parts, &trees)?.read()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hwpx_write::write_hwpx;

    #[test]
    fn repeated_image_payloads_obey_combined_budget() {
        let image = Image {
            data: vec![0; 8],
            media_type: "image/png".into(),
            ..Default::default()
        };
        let source = RichDocument {
            sections: vec![Section {
                blocks: vec![Block::Paragraph(Paragraph {
                    runs: vec![
                        Run {
                            image: Some(image.clone()),
                            ..Default::default()
                        },
                        Run {
                            image: Some(image),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                })],
                ..Default::default()
            }],
            ..Default::default()
        };
        let payload = write_hwpx(&source).unwrap();
        let (parts, trees) = read_package(&payload).unwrap();
        let mut reader = Reader::new(&parts, &trees).unwrap();
        reader.image_budget = 12;
        let error = reader.read().unwrap_err();
        assert_eq!(error.code, "LIMIT");
        assert_eq!(error.message, "Expanded conversion images exceed limits.");
    }
}

#[cfg(test)]
mod style_budget_tests {
    use super::*;
    use crate::hwpx_write::write_hwpx;

    #[test]
    fn repeated_run_styles_obey_combined_budget() {
        let source = RichDocument {
            sections: vec![Section {
                blocks: vec![Block::Paragraph(Paragraph {
                    runs: vec![
                        Run {
                            text: "first".into(),
                            ..Default::default()
                        },
                        Run {
                            text: "second".into(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                })],
                ..Default::default()
            }],
            ..Default::default()
        };
        let payload = write_hwpx(&source).unwrap();
        let (parts, trees) = read_package(&payload).unwrap();
        let mut reader = Reader::new(&parts, &trees).unwrap();
        reader.style_budget = 1;
        let error = reader.read().unwrap_err();
        assert_eq!(error.code, "LIMIT");
        assert_eq!(error.message, "Expanded conversion styles exceed limits.");
    }
}

#[cfg(test)]
mod cache_budget_tests {
    use super::*;

    #[test]
    fn cached_character_styles_obey_combined_budget() {
        let source =
            include_bytes!("../../../tests/data/convert_fixtures-package-fcdbb758946b471e.hwpx");
        let (parts, trees) = read_package(source).unwrap();
        let mut reader = Reader::new(&parts, &trees).unwrap();
        reader.style_budget = 1;
        reader.cached_style_bytes = 0;
        let error = reader.references().unwrap_err();
        assert_eq!(error.code, "LIMIT");
        assert_eq!(error.message, "Expanded conversion styles exceed limits.");
    }
}
