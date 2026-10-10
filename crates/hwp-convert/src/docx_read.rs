//! Read supported OOXML content directly, without resolving external relationships.
use crate::model::{
    default_page, paragraphs, split_page_breaks, style_payload_bytes, Block, Cell, Image,
    Paragraph, RichDocument, Run, Section, Style, Table,
};
use crate::package::{
    integer, member_target, read_package, Parts, Trees, XmlExt, MAX_COLUMNS, MAX_GRID_WORK,
};
use hwp_core::errors::{require, Result, WorkbenchError, MAX_EXPANDED, MAX_NODES};
use hwp_core::xml_io::Element;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const MATH: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";

type StyleProperties = (Style, Style, i64, Option<Style>);

fn is(node: &Element, namespace: &str, local: &str) -> bool {
    node.name.namespace.as_deref() == Some(namespace) && node.name.local == local
}

fn attr<'a>(node: Option<&'a Element>, key: &str) -> Option<&'a str> {
    node.and_then(|node| node.attr(W, key))
}

fn value<'a>(node: Option<&'a Element>, tag: &str) -> Option<&'a str> {
    attr(node.and_then(|node| node.child(W, tag)), "val")
}

fn toggle(node: &Element) -> bool {
    !matches!(
        attr(Some(node), "val").unwrap_or("1"),
        "0" | "false" | "off"
    )
}

fn colour(value: Option<&str>) -> Option<String> {
    value
        .filter(|text| text.len() == 6 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_uppercase)
}

fn numeric(value: Option<&str>, default: i64) -> Result<i64> {
    integer(value, default, -10_000_000, 10_000_000)
}

fn heading_name(name: &str) -> Option<i64> {
    let lower = name.to_lowercase();
    let suffix = lower.strip_prefix("heading")?.trim_start();
    if suffix.len() == 1 {
        let digit = suffix.chars().next()?;
        if matches!(digit, '1'..='9') {
            return digit.to_digit(10).map(i64::from);
        }
    }
    None
}

struct Reader<'a> {
    parts: &'a Parts,
    trees: &'a Trees,
    document: RichDocument,
    count: usize,
    image_budget: usize,
    mapped_image_bytes: usize,
    style_budget: usize,
    retained_style_bytes: usize,
    styles: BTreeMap<Option<String>, &'a Element>,
    style_properties: BTreeMap<Option<String>, (Style, Style)>,
    default_style: Option<String>,
    default_run: Style,
    default_para: Style,
    numbers: BTreeMap<Option<String>, BTreeMap<i64, Style>>,
    rels: BTreeMap<Option<String>, &'a Element>,
    media_types: BTreeMap<String, String>,
    media_overrides: BTreeMap<String, String>,
}

impl<'a> Reader<'a> {
    fn retain_style_bytes(&mut self, bytes: usize) -> Result<()> {
        self.retained_style_bytes = self
            .retained_style_bytes
            .checked_add(bytes)
            .filter(|total| *total <= self.style_budget)
            .ok_or_else(|| {
                WorkbenchError::new("LIMIT", "Expanded conversion styles exceed limits.")
            })?;
        Ok(())
    }

    fn retain_run_style(&mut self, style: &Style, text: &str, paragraph: &Paragraph) -> Result<()> {
        let breaks = text
            .chars()
            .filter(|character| *character == '\u{c}')
            .count();
        // Reserve styles for the extra pieces before the page-break splitter clones them.
        let run_bytes = style_payload_bytes(style).saturating_mul(breaks.saturating_add(1));
        let paragraph_bytes = style_payload_bytes(&paragraph.style).saturating_mul(breaks);
        self.retain_style_bytes(run_bytes.saturating_add(paragraph_bytes))
    }

    fn new(parts: &'a Parts, trees: &'a Trees) -> Result<Self> {
        let mut reader = Self {
            parts,
            trees,
            document: RichDocument::default(),
            count: 0,
            image_budget: MAX_EXPANDED,
            mapped_image_bytes: 0,
            style_budget: MAX_EXPANDED,
            retained_style_bytes: 0,
            styles: BTreeMap::new(),
            style_properties: BTreeMap::new(),
            default_style: None,
            default_run: Style::new(),
            default_para: Style::new(),
            numbers: BTreeMap::new(),
            rels: BTreeMap::new(),
            media_types: BTreeMap::new(),
            media_overrides: BTreeMap::new(),
        };
        let types = trees
            .get("[Content_Types].xml")
            .filter(|node| is(node, CT, "Types"))
            .ok_or_else(|| WorkbenchError::new("NOT_DOCX", "Missing DOCX content types."))?;
        for item in &types.children {
            if is(item, CT, "Default") {
                reader.media_types.insert(
                    item.get("Extension").unwrap_or("").to_lowercase(),
                    item.get("ContentType").unwrap_or("").to_owned(),
                );
            } else if is(item, CT, "Override") {
                reader.media_overrides.insert(
                    item.get("PartName")
                        .unwrap_or("")
                        .trim_start_matches('/')
                        .to_owned(),
                    item.get("ContentType").unwrap_or("").to_owned(),
                );
            }
        }
        if let Some(root) = trees.get("word/_rels/document.xml.rels") {
            for item in root
                .children
                .iter()
                .filter(|node| is(node, REL, "Relationship"))
            {
                reader.rels.insert(item.get("Id").map(str::to_owned), item);
                if item.get("TargetMode") == Some("External") {
                    reader.document.loss("externalRelationships", 1);
                }
            }
        }
        reader.read_styles()?;
        reader.read_numbering()?;
        reader.unsupported_parts()?;
        Ok(reader)
    }

    fn run_style(&mut self, properties: Option<&Element>) -> Result<Style> {
        let mut result = Style::new();
        let Some(properties) = properties else {
            return Ok(result);
        };
        for item in &properties.children {
            let tag = item.name.local.as_str();
            match tag {
                "b" | "i" | "strike" => {
                    let key = match tag {
                        "b" => "bold",
                        "i" => "italic",
                        _ => "strike",
                    };
                    result.insert(key.into(), json!(toggle(item)));
                }
                "u" => {
                    let kind = attr(Some(item), "val").unwrap_or("single");
                    result.insert("underline".into(), json!(kind));
                    if !matches!(
                        kind,
                        "single" | "double" | "none" | "dotted" | "dash" | "wave"
                    ) {
                        self.document.loss("underlineStyles", 1);
                    }
                }
                "sz" => {
                    result.insert(
                        "size".into(),
                        json!(integer(attr(Some(item), "val"), 22, 1, 4000)? as f64 / 2.0),
                    );
                }
                "color" => {
                    result.insert(
                        "colour".into(),
                        json!(colour(attr(Some(item), "val")).unwrap_or_else(|| "000000".into())),
                    );
                    if attr(Some(item), "themeColor").is_some_and(|value| !value.is_empty()) {
                        self.document.loss("themeColours", 1);
                    }
                }
                "rFonts" => {
                    let face = ["eastAsia", "ascii", "hAnsi"]
                        .into_iter()
                        .filter_map(|key| attr(Some(item), key))
                        .find(|face| !face.is_empty());
                    if let Some(face) = face {
                        result.insert("font".into(), json!(face));
                    }
                    if item
                        .attributes
                        .iter()
                        .any(|attribute| attribute.name.expanded().contains("Theme"))
                    {
                        self.document.loss("themeFonts", 1);
                    }
                }
                "rStyle" | "rPrChange" => {}
                _ => self.document.loss(&format!("runProperty:{tag}"), 1),
            }
        }
        Ok(result)
    }

    fn para_style(&mut self, properties: Option<&Element>) -> Result<Style> {
        let mut result = Style::new();
        let Some(properties) = properties else {
            return Ok(result);
        };
        for item in &properties.children {
            let tag = item.name.local.as_str();
            match tag {
                "jc" => {
                    result.insert(
                        "alignment".into(),
                        json!(attr(Some(item), "val").unwrap_or("left")),
                    );
                }
                "ind" => {
                    for (source, target) in [
                        ("left", "left"),
                        ("right", "right"),
                        ("firstLine", "first"),
                        ("hanging", "hanging"),
                    ] {
                        if let Some(number) = attr(Some(item), source) {
                            result.insert(target.into(), json!(numeric(Some(number), 0)?));
                        }
                    }
                    if ["leftChars", "rightChars", "firstLineChars", "hangingChars"]
                        .into_iter()
                        .any(|key| attr(Some(item), key).is_some())
                    {
                        self.document.loss("characterIndents", 1);
                    }
                }
                "spacing" => {
                    for key in ["before", "after", "line"] {
                        if let Some(number) = attr(Some(item), key) {
                            result.insert(key.into(), json!(numeric(Some(number), 0)?));
                        }
                    }
                    result.insert(
                        "lineRule".into(),
                        json!(attr(Some(item), "lineRule").unwrap_or("auto")),
                    );
                }
                "pageBreakBefore" | "keepNext" | "keepLines" | "widowControl" => {
                    result.insert(tag.into(), json!(toggle(item)));
                }
                "pStyle" | "numPr" | "outlineLvl" | "sectPr" | "rPr" | "pPrChange" => {}
                _ => self.document.loss(&format!("paragraphProperty:{tag}"), 1),
            }
        }
        Ok(result)
    }

    fn read_styles(&mut self) -> Result<()> {
        let Some(root) = self.trees.get("word/styles.xml") else {
            return Ok(());
        };
        if let Some(defaults) = root.child(W, "docDefaults") {
            self.default_run = self.run_style(defaults.path(W, &["rPrDefault", "rPr"]))?;
            self.default_para = self.para_style(defaults.path(W, &["pPrDefault", "pPr"]))?;
            self.retain_style_bytes(
                style_payload_bytes(&self.default_run)
                    .saturating_add(style_payload_bytes(&self.default_para)),
            )?;
        }
        for style in root.children.iter().filter(|node| is(node, W, "style")) {
            let style_id = attr(Some(style), "styleId").map(str::to_owned);
            self.styles.insert(style_id.clone(), style);
            if attr(Some(style), "type").unwrap_or("paragraph") == "paragraph"
                && matches!(attr(Some(style), "default"), Some("1" | "true" | "on"))
            {
                self.default_style = style_id.clone();
            }
            let run = self.run_style(style.child(W, "rPr"))?;
            let para = self.para_style(style.child(W, "pPr"))?;
            self.retain_style_bytes(
                style_payload_bytes(&run).saturating_add(style_payload_bytes(&para)),
            )?;
            self.style_properties.insert(style_id, (run, para));
        }
        Ok(())
    }

    fn style(&mut self, style_id: Option<&str>, include_defaults: bool) -> Result<StyleProperties> {
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        let mut style_id = style_id.map(str::to_owned);
        while let Some(style) = self.styles.get(&style_id).copied() {
            require(
                seen.insert(style_id.clone()) && chain.len() < 128,
                "BAD_XML",
                "Cyclic or excessive DOCX style inheritance.",
            )?;
            chain.push(style);
            style_id = value(Some(style), "basedOn").map(str::to_owned);
        }
        let mut run = if include_defaults {
            self.default_run.clone()
        } else {
            Style::new()
        };
        let mut para = if include_defaults {
            self.default_para.clone()
        } else {
            Style::new()
        };
        let mut heading = 0;
        let mut numbering = None;
        for style in chain.into_iter().rev() {
            let properties = style.child(W, "pPr");
            if let Some((inherited_run, inherited_para)) = self
                .style_properties
                .get(&attr(Some(style), "styleId").map(str::to_owned))
            {
                run.extend(inherited_run.clone());
                para.extend(inherited_para.clone());
            }
            if let Some(outline) = value(properties, "outlineLvl") {
                heading = (integer(Some(outline), 0, 0, 9)? + 1).min(9);
            } else if let Some(level) = heading_name(value(Some(style), "name").unwrap_or("")) {
                heading = level;
            }
            if properties.and_then(|node| node.child(W, "numPr")).is_some() {
                numbering = self.number(properties)?;
            }
        }
        Ok((run, para, heading, numbering))
    }

    fn read_numbering(&mut self) -> Result<()> {
        let Some(root) = self.trees.get("word/numbering.xml") else {
            return Ok(());
        };
        let abstracts: BTreeMap<_, _> = root
            .children
            .iter()
            .filter(|node| is(node, W, "abstractNum"))
            .map(|node| (attr(Some(node), "abstractNumId"), node))
            .collect();
        for number in root.children.iter().filter(|node| is(node, W, "num")) {
            let Some(abstract_number) = abstracts.get(&value(Some(number), "abstractNumId")) else {
                continue;
            };
            let mut levels = BTreeMap::new();
            for level in abstract_number
                .children
                .iter()
                .filter(|node| is(node, W, "lvl"))
            {
                let index = integer(attr(Some(level), "ilvl"), 0, 0, 8)?;
                let kind = value(Some(level), "numFmt").unwrap_or("decimal");
                let text = value(Some(level), "lvlText").unwrap_or(if kind == "bullet" {
                    "•"
                } else {
                    "%1."
                });
                let style = self.para_style(level.child(W, "pPr"))?;
                let properties = json!({"kind":kind,"text":text,"level":index,"start":numeric(value(Some(level), "start"), 1)?,"id":attr(Some(number), "numId"),"style":style});
                if let Value::Object(properties) = properties {
                    // Many numbers can share one abstract level in the source.
                    self.retain_style_bytes(style_payload_bytes(&properties))?;
                    levels.insert(index, properties);
                }
            }
            for override_node in number
                .children
                .iter()
                .filter(|node| is(node, W, "lvlOverride"))
            {
                let index = integer(attr(Some(override_node), "ilvl"), 0, 0, 8)?;
                if let Some(level) = levels.get_mut(&index) {
                    if let Some(start) = value(Some(override_node), "startOverride") {
                        level.insert("start".into(), json!(numeric(Some(start), 1)?));
                    }
                }
                if override_node.child(W, "lvl").is_some() {
                    self.document.loss("numberingOverrides", 1);
                }
            }
            self.numbers
                .insert(attr(Some(number), "numId").map(str::to_owned), levels);
        }
        Ok(())
    }

    fn number(&mut self, properties: Option<&Element>) -> Result<Option<Style>> {
        let Some(number) = properties.and_then(|node| node.child(W, "numPr")) else {
            return Ok(None);
        };
        let num_id = value(Some(number), "numId").unwrap_or("0");
        if num_id == "0" {
            return Ok(None);
        }
        let level = integer(value(Some(number), "ilvl"), 0, 0, 8)?;
        let result = self
            .numbers
            .get(&Some(num_id.to_owned()))
            .and_then(|levels| levels.get(&level));
        match result {
            None => {
                self.document.loss("unresolvedNumbering", 1);
                Ok(None)
            }
            Some(result)
                if !matches!(
                    result.get("kind").and_then(Value::as_str),
                    Some("bullet" | "decimal")
                ) =>
            {
                self.document.loss("complexNumbering", 1);
                Ok(None)
            }
            Some(result) => Ok(Some(result.clone())),
        }
    }

    fn unsupported_parts(&mut self) -> Result<()> {
        for (name, tree) in self.trees {
            self.document.loss(
                "textBoxes",
                tree.iter()
                    .filter(|node| is(node, W, "txbxContent"))
                    .count(),
            );
            for item in tree.iter() {
                if item.name.namespace.as_deref() == Some(W)
                    && matches!(
                        item.name.local.as_str(),
                        "ins"
                            | "del"
                            | "moveFrom"
                            | "moveTo"
                            | "rPrChange"
                            | "pPrChange"
                            | "tblPrChange"
                            | "tcPrChange"
                            | "trPrChange"
                            | "sectPrChange"
                    )
                {
                    self.document.loss("trackedChanges", 1);
                }
                if is(item, W, "fldSimple")
                    || (is(item, W, "fldChar") && attr(Some(item), "fldCharType") == Some("begin"))
                {
                    self.document.loss("fields", 1);
                }
            }
            let stem = name
                .rsplit('/')
                .next()
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("");
            for (prefix, feature, tag) in [
                ("header", "headers", ""),
                ("footer", "footers", ""),
                ("footnotes", "footnotes", "footnote"),
                ("endnotes", "endnotes", "endnote"),
                ("comments", "comments", "comment"),
            ] {
                if stem.starts_with(prefix) {
                    if tag.is_empty() {
                        self.document.loss(feature, 1);
                    } else {
                        let mut count = 0;
                        for item in tree.children.iter().filter(|node| is(node, W, tag)) {
                            if numeric(attr(Some(item), "id"), 0)? >= 0 {
                                count += 1;
                            }
                        }
                        self.document.loss(feature, count);
                    }
                }
            }
            if name.contains("/charts/") {
                self.document.loss("charts", 1);
            }
        }
        for name in self.parts.keys() {
            if name.contains("vbaProject") || name.starts_with("_xmlsignatures/") {
                self.document.loss("activeOrSignedContent", 1);
            }
        }
        Ok(())
    }

    fn image(&mut self, drawing: &Element) -> Result<Option<Image>> {
        let Some(blip) = drawing.iter().skip(1).find(|node| is(node, A, "blip")) else {
            self.document.loss("shapes", 1);
            return Ok(None);
        };
        let relation = self
            .rels
            .get(&blip.attr(R, "embed").map(str::to_owned))
            .copied();
        let Some(relation) = relation.filter(|relation| {
            !relation.attributes.is_empty() && relation.get("TargetMode") != Some("External")
        }) else {
            self.document.loss("unavailableImages", 1);
            return Ok(None);
        };
        let name = member_target("word/document.xml", relation.get("Target").unwrap_or(""))?;
        let Some(data) = self.parts.get(&name) else {
            self.document.loss("unavailableImages", 1);
            return Ok(None);
        };
        let media_type = self
            .media_overrides
            .get(&name)
            .filter(|kind| !kind.is_empty())
            .or_else(|| {
                self.media_types
                    .get(&name.rsplit('.').next().unwrap_or("").to_lowercase())
            })
            .map(String::as_str)
            .unwrap_or("");
        if !matches!(
            media_type,
            "image/png" | "image/jpeg" | "image/gif" | "image/bmp"
        ) {
            self.document.loss("unsupportedImageTypes", 1);
            return Ok(None);
        }
        // Each mapped image owns media bytes and needs an allocation
        // budget when many drawings refer to one large package member.
        self.mapped_image_bytes = self
            .mapped_image_bytes
            .checked_add(data.len())
            .filter(|total| *total <= self.image_budget)
            .ok_or_else(|| WorkbenchError::new("LIMIT", "Mapped image payloads exceed limits."))?;
        if drawing.child(WP, "anchor").is_some() {
            self.document.loss("floatingImagePlacement", 1);
        }
        if drawing
            .iter()
            .skip(1)
            .find(|node| is(node, A, "srcRect"))
            .is_some_and(|crop| {
                crop.attributes
                    .iter()
                    .any(|attribute| attribute.value != "0")
            })
        {
            self.document.loss("imageCropping", 1);
        }
        if drawing
            .iter()
            .skip(1)
            .find(|node| is(node, A, "xfrm"))
            .is_some_and(|transform| {
                ["rot", "flipH", "flipV"]
                    .into_iter()
                    .any(|key| !matches!(transform.get(key).unwrap_or("0"), "0" | "false"))
            })
        {
            self.document.loss("imageTransforms", 1);
        }
        if drawing.iter().any(|node| {
            matches!(
                node.name.local.as_str(),
                "alphaModFix" | "duotone" | "grayscl" | "lum" | "biLevel"
            )
        }) {
            self.document.loss("imageEffects", 1);
        }
        let extent = drawing.iter().skip(1).find(|node| is(node, WP, "extent"));
        let width = integer(
            extent.and_then(|node| node.get("cx")),
            914400,
            1,
            1_000_000_000,
        )?;
        let height = integer(
            extent.and_then(|node| node.get("cy")),
            914400,
            1,
            1_000_000_000,
        )?;
        // OOXML extents use EMUs; 635 EMUs equal one twip. The odd divisor cannot have half ties.
        Ok(Some(Image {
            data: data.clone(),
            media_type: media_type.to_owned(),
            width: ((width + 317) / 635).max(1),
            height: ((height + 317) / 635).max(1),
            name: name.rsplit('/').next().unwrap_or("").to_owned(),
        }))
    }

    fn paragraph(&mut self, node: &Element) -> Result<Paragraph> {
        self.count += 1;
        require(
            self.count <= MAX_NODES,
            "LIMIT",
            "Too many conversion paragraphs.",
        )?;
        let properties = node.child(W, "pPr");
        let style_id = value(properties, "pStyle")
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| self.default_style.clone());
        let (run_style, mut para_style, mut heading, mut numbering) =
            self.style(style_id.as_deref(), true)?;
        para_style.extend(self.para_style(properties)?);
        if let Some(outline) = value(properties, "outlineLvl") {
            heading = (integer(Some(outline), 0, 0, 9)? + 1).min(9);
        }
        if properties.and_then(|node| node.child(W, "numPr")).is_some() {
            numbering = self.number(properties)?;
        }
        if let Some(style) = numbering
            .as_ref()
            .and_then(|number| number.get("style"))
            .and_then(Value::as_object)
        {
            let mut inherited = style.clone();
            inherited.extend(para_style);
            para_style = inherited;
        }
        let page_break = para_style
            .remove("pageBreakBefore")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut paragraph = Paragraph {
            runs: Vec::new(),
            style: para_style,
            heading,
            numbering,
            page_break,
        };
        self.retain_style_bytes(
            style_payload_bytes(&paragraph.style).saturating_add(
                paragraph
                    .numbering
                    .as_ref()
                    .map(style_payload_bytes)
                    .unwrap_or(0),
            ),
        )?;
        self.visit_paragraph(node, &run_style, &mut paragraph)?;
        if paragraph.runs.is_empty() {
            self.retain_style_bytes(style_payload_bytes(&run_style))?;
            paragraph.runs.push(Run {
                text: String::new(),
                style: run_style,
                image: None,
            });
        }
        Ok(paragraph)
    }

    fn visit_paragraph(
        &mut self,
        parent: &Element,
        run_style: &Style,
        paragraph: &mut Paragraph,
    ) -> Result<()> {
        for child in &parent.children {
            if is(child, W, "pPr") {
                continue;
            }
            if is(child, W, "r") {
                let props = child.child(W, "rPr");
                let mut style = run_style.clone();
                if let Some(character_style) =
                    value(props, "rStyle").filter(|value| !value.is_empty())
                {
                    style.extend(self.style(Some(character_style), false)?.0);
                }
                style.extend(self.run_style(props)?);
                for item in &child.children {
                    if item.name.namespace.as_deref() != Some(W) {
                        let feature = match item.name.local.as_str() {
                            "object" => "embeddedObjects".to_owned(),
                            "pict" => "shapes".to_owned(),
                            tag => format!("runElement:{tag}"),
                        };
                        self.document.loss(&feature, 1);
                        continue;
                    }
                    let text = match item.name.local.as_str() {
                        "t" | "delText" => Some(item.text.clone().unwrap_or_default()),
                        "br" => {
                            let kind = attr(Some(item), "type").unwrap_or("textWrapping");
                            if kind == "column" {
                                self.document.loss("columnBreaks", 1);
                            }
                            Some(if kind == "page" { "\u{c}" } else { "\n" }.into())
                        }
                        "cr" => Some("\n".into()),
                        "tab" => Some("\t".into()),
                        "drawing" => {
                            if let Some(image) = self.image(item)? {
                                self.retain_style_bytes(style_payload_bytes(&style))?;
                                paragraph.runs.push(Run {
                                    text: String::new(),
                                    style: style.clone(),
                                    image: Some(image),
                                });
                            }
                            None
                        }
                        "fldChar" | "instrText" | "footnoteReference" | "endnoteReference"
                        | "commentReference" | "rPr" => None,
                        "object" => {
                            self.document.loss("embeddedObjects", 1);
                            None
                        }
                        "pict" => {
                            self.document.loss("shapes", 1);
                            None
                        }
                        tag => {
                            self.document.loss(&format!("runElement:{tag}"), 1);
                            None
                        }
                    };
                    if let Some(text) = text {
                        self.retain_run_style(&style, &text, paragraph)?;
                        paragraph.runs.push(Run {
                            text,
                            style: style.clone(),
                            image: None,
                        });
                    }
                }
            } else {
                let tag = child.name.local.as_str();
                let feature = match tag {
                    "fldSimple" => Some("fields"),
                    "hyperlink" => Some("hyperlinks"),
                    "ins" | "del" | "moveFrom" | "moveTo" => Some("trackedChanges"),
                    "txbxContent" => Some("textBoxes"),
                    _ => None,
                };
                if child.name.namespace.as_deref() == Some(MATH) {
                    match tag {
                        "oMath" => self.document.loss("equations", 1),
                        "oMathPara" => self.document.loss(
                            "equations",
                            child
                                .children
                                .iter()
                                .filter(|node| is(node, MATH, "oMath"))
                                .count(),
                        ),
                        _ => {}
                    }
                    continue;
                }
                if !matches!(feature, Some("fields" | "trackedChanges" | "textBoxes")) {
                    self.document
                        .loss(feature.unwrap_or(&format!("paragraphElement:{tag}")), 1);
                }
                self.visit_paragraph(child, run_style, paragraph)?;
            }
        }
        Ok(())
    }

    fn borders(&mut self, node: Option<&Element>) -> Result<Style> {
        let mut result = Style::new();
        let Some(node) = node else {
            return Ok(result);
        };
        for item in &node.children {
            let side = item.name.local.as_str();
            if !matches!(
                side,
                "top" | "bottom" | "left" | "right" | "insideH" | "insideV"
            ) {
                self.document.loss("diagonalBorders", 1);
                continue;
            }
            let mut kind = attr(Some(item), "val").unwrap_or("single");
            if !matches!(
                kind,
                "single" | "double" | "dotted" | "dashed" | "nil" | "none"
            ) {
                self.document.loss("borderStyles", 1);
                kind = "single";
            }
            result.insert(side.into(), json!({"kind":kind,"size":integer(attr(Some(item), "sz"), 4, 0, 1024)?,"colour":colour(attr(Some(item), "color")).unwrap_or_else(|| "000000".into())}));
        }
        Ok(result)
    }

    fn table(&mut self, node: &Element) -> Result<Table> {
        let mut widths = Vec::new();
        if let Some(grid) = node.child(W, "tblGrid") {
            for item in &grid.children {
                widths.push(integer(attr(Some(item), "w"), 1440, 1, 10_000_000)?);
            }
        }
        require(
            widths.len() <= MAX_COLUMNS,
            "LIMIT",
            "Too many table columns.",
        )?;
        let mut table = Table {
            rows: Vec::new(),
            widths,
        };
        // Owners stay in their original rows; coordinates avoid mutable aliases across merge continuations.
        let mut active: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
        let properties = node.child(W, "tblPr");
        let inherited_borders =
            self.borders(properties.and_then(|node| node.child(W, "tblBorders")))?;
        let mut work = 0;
        for (row_index, row) in node
            .children
            .iter()
            .filter(|node| is(node, W, "tr"))
            .enumerate()
        {
            require(row_index < MAX_NODES, "LIMIT", "Too many table rows.")?;
            let row_props = row.child(W, "trPr");
            let mut column =
                integer(value(row_props, "gridBefore"), 0, 0, MAX_COLUMNS as i64)? as usize;
            let mut cells = Vec::new();
            let mut next_active = BTreeMap::new();
            for cell_node in row.children.iter().filter(|node| is(node, W, "tc")) {
                let cell_props = cell_node.child(W, "tcPr");
                let span =
                    integer(value(cell_props, "gridSpan"), 1, 1, MAX_COLUMNS as i64)? as usize;
                require(
                    column + span <= MAX_COLUMNS,
                    "LIMIT",
                    "Table cell exceeds column limits.",
                )?;
                let merge = cell_props.and_then(|node| node.child(W, "vMerge"));
                if merge.is_some() && attr(merge, "val").unwrap_or("continue") != "restart" {
                    let owner_coordinates = active.get(&column).copied();
                    let owner_span = owner_coordinates
                        .and_then(|(row, cell)| table.rows.get(row).and_then(|row| row.get(cell)))
                        .map(|cell| cell.colspan);
                    if owner_span == Some(span) {
                        let continuation = self.blocks(cell_node)?;
                        let has_content = continuation.iter().any(|block| match block {
                            Block::Table(_) => true,
                            Block::Paragraph(paragraph) => {
                                paragraph.page_break
                                    || paragraph
                                        .runs
                                        .iter()
                                        .any(|run| !run.text.is_empty() || run.image.is_some())
                            }
                        });
                        if let Some((owner_row, owner_cell)) = owner_coordinates {
                            let owner = table
                                .rows
                                .get_mut(owner_row)
                                .and_then(|row| row.get_mut(owner_cell))
                                .ok_or_else(|| {
                                    WorkbenchError::new("BAD_XML", "Unresolved DOCX merge owner.")
                                })?;
                            owner.rowspan += 1;
                            if has_content {
                                owner.blocks.extend(continuation);
                                self.document.loss("nonemptyMergeContinuation", 1);
                            }
                            next_active.insert(column, (owner_row, owner_cell));
                        }
                        column += span;
                        continue;
                    }
                    self.document.loss("unresolvedTableMerge", 1);
                }
                let mut cell = Cell {
                    blocks: self.blocks(cell_node)?,
                    column,
                    colspan: span,
                    rowspan: 1,
                    borders: Style::new(),
                    shading: None,
                };
                if let Some(cell_props) = cell_props {
                    cell.borders = self.borders(cell_props.child(W, "tcBorders"))?;
                    cell.shading = colour(attr(cell_props.child(W, "shd"), "fill"));
                    for prop in &cell_props.children {
                        if !["gridSpan", "vMerge", "tcBorders", "shd", "tcW"]
                            .into_iter()
                            .any(|tag| is(prop, W, tag))
                        {
                            self.document
                                .loss(&format!("cellProperty:{}", prop.name.local), 1);
                        }
                    }
                }
                if merge.is_some() {
                    next_active.insert(column, (row_index, cells.len()));
                }
                cells.push(cell);
                column += span;
                work += span;
                require(
                    work <= MAX_GRID_WORK,
                    "LIMIT",
                    "Table mapping exceeds work limit.",
                )?;
            }
            active = next_active;
            table.rows.push(cells);
            while table.widths.len() < column {
                table.widths.push(1440);
            }
        }
        if table.widths.is_empty() {
            table.widths.push(1440);
        }
        let row_count = table.rows.len();
        for (row_index, row) in table.rows.iter_mut().enumerate() {
            for cell in row {
                let mut inherited = Style::new();
                for (side, outer, inside) in [
                    ("top", row_index == 0, "insideH"),
                    ("bottom", row_index + cell.rowspan == row_count, "insideH"),
                    ("left", cell.column == 0, "insideV"),
                    (
                        "right",
                        cell.column + cell.colspan == table.widths.len(),
                        "insideV",
                    ),
                ] {
                    if let Some(border) = inherited_borders.get(if outer { side } else { inside }) {
                        inherited.insert(side.into(), border.clone());
                    }
                }
                inherited.extend(std::mem::take(&mut cell.borders));
                self.retain_style_bytes(style_payload_bytes(&inherited))?;
                cell.borders = inherited;
            }
        }
        if let Some(properties) = properties {
            for prop in &properties.children {
                if !["tblBorders", "tblW", "tblLayout"]
                    .into_iter()
                    .any(|tag| is(prop, W, tag))
                {
                    self.document
                        .loss(&format!("tableProperty:{}", prop.name.local), 1);
                }
            }
        }
        Ok(table)
    }

    fn blocks(&mut self, node: &Element) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();
        for child in &node.children {
            if is(child, W, "p") {
                blocks.push(Block::Paragraph(self.paragraph(child)?));
            } else if is(child, W, "tbl") {
                blocks.push(Block::Table(self.table(child)?));
            } else if ["sdt", "sdtContent", "customXml"]
                .into_iter()
                .any(|tag| is(child, W, tag))
            {
                self.document.loss("contentControls", 1);
                blocks.extend(self.blocks(child)?);
            }
        }
        Ok(blocks)
    }

    fn page(&mut self, node: Option<&Element>) -> Result<Style> {
        let mut page = default_page();
        let Some(node) = node else {
            return Ok(page);
        };
        if let Some(size) = node.child(W, "pgSz") {
            for key in ["width", "height"] {
                let attr_key = if key == "width" { "w" } else { "h" };
                let default = page.get(key).and_then(Value::as_i64).unwrap_or(0);
                page.insert(
                    key.into(),
                    json!(integer(attr(Some(size), attr_key), default, 1, 10_000_000)?),
                );
            }
            page.insert(
                "orientation".into(),
                json!(attr(Some(size), "orient").unwrap_or("portrait")),
            );
        }
        if let Some(margin) = node.child(W, "pgMar") {
            for key in [
                "left", "right", "top", "bottom", "header", "footer", "gutter",
            ] {
                let default = page.get(key).and_then(Value::as_i64).unwrap_or(0);
                page.insert(
                    key.into(),
                    json!(numeric(attr(Some(margin), key), default)?),
                );
            }
        }
        for item in &node.children {
            if ![
                "pgSz",
                "pgMar",
                "headerReference",
                "footerReference",
                "type",
            ]
            .into_iter()
            .any(|tag| is(item, W, tag))
            {
                self.document
                    .loss(&format!("sectionProperty:{}", item.name.local), 1);
            }
        }
        Ok(page)
    }

    fn read(mut self) -> Result<RichDocument> {
        let root = self
            .trees
            .get("word/document.xml")
            .filter(|node| is(node, W, "document"))
            .ok_or_else(|| WorkbenchError::new("NOT_DOCX", "Missing DOCX main document."))?;
        let body = root
            .child(W, "body")
            .ok_or_else(|| WorkbenchError::new("NOT_DOCX", "Missing DOCX body."))?;
        let mut section = Section::default();
        for child in &body.children {
            if is(child, W, "sectPr") {
                section.page = self.page(Some(child))?;
            } else if is(child, W, "p") || is(child, W, "tbl") {
                section.blocks.push(if is(child, W, "p") {
                    Block::Paragraph(self.paragraph(child)?)
                } else {
                    Block::Table(self.table(child)?)
                });
                if is(child, W, "p") {
                    if let Some(boundary) = child.path(W, &["pPr", "sectPr"]) {
                        section.page = self.page(Some(boundary))?;
                        self.document.sections.push(section);
                        section = Section::default();
                    }
                }
            } else {
                self.document
                    .loss(&format!("bodyElement:{}", child.name.local), 1);
                section.blocks.extend(self.blocks(child)?);
            }
        }
        if !section.blocks.is_empty() || self.document.sections.is_empty() {
            self.document.sections.push(section);
        }
        Ok(self.document)
    }
}

/// Map DOCX paragraphs, tables, styles, images and sections, with explicit loss counts.
pub fn read_docx(payload: &[u8]) -> Result<RichDocument> {
    let (parts, trees) = read_package(payload)?;
    let mut document = Reader::new(&parts, &trees)?.read()?;
    for section in &mut document.sections {
        section.blocks = split_page_breaks(std::mem::take(&mut section.blocks));
    }
    require(
        document
            .sections
            .iter()
            .flat_map(|section| paragraphs(&section.blocks))
            .count()
            <= MAX_NODES,
        "LIMIT",
        "Page breaks exceed the paragraph limit.",
    )?;
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_images_fail_before_copied_payloads_exceed_budget() {
        let payload =
            include_bytes!("../../../tests/data/convert_fixtures-package-60bd5b8e40094da8.docx");
        let (parts, trees) = read_package(payload).unwrap();
        let mut reader = Reader::new(&parts, &trees).unwrap();
        reader.image_budget = parts["word/media/pixel.png"].len();
        let error = reader.read().unwrap_err();
        assert_eq!(error.code, "LIMIT");
        assert_eq!(error.message, "Mapped image payloads exceed limits.");
    }

    #[test]
    fn repeated_styles_fail_before_retained_payloads_exceed_budget() {
        let payload =
            include_bytes!("../../../tests/data/convert_fixtures-docx-3bf7c69f1761e6c0.docx");
        let (parts, trees) = read_package(payload).unwrap();
        let mut reader = Reader::new(&parts, &trees).unwrap();
        reader.style_budget = 1;
        let error = reader.read().unwrap_err();
        assert_eq!(error.code, "LIMIT");
        assert_eq!(error.message, "Expanded conversion styles exceed limits.");
    }
}
