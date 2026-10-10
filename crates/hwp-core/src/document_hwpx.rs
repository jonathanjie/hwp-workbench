//! Namespace-aware HWPX reads and surgical edits in the original XML bytes.
use super::{sha, Document, Position, TextNode, TextToken, HP};
use crate::errors::{require, Result, WorkbenchError, MAX_EXPANDED, MAX_NODES, MAX_PART};
use crate::package_io::ZipPackage;
use crate::xml_io::{xml_tree, Element, TokenKind, TokenReader};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Hancom 한글 writes this script pair into ordinary HWPX files: UTF-16LE
/// document bindings and an empty `OnDocument_New` callback with a `//todo`
/// comment. Only this exact pair is retained without blocking edits.
pub const DEFAULT_HWPX_SCRIPT_HASHES: &[(&str, &str)] = &[
    (
        "Scripts/headerScripts.js",
        "d24eb743b49a913f449c0a4e9ee6e6a9465c890298c769145dba831e7b13b4b0",
    ),
    (
        "Scripts/sourceScripts.js",
        "6f558fdcd17d07a7d2f29478a6eb3d628c7e3c9c64f049dce4959426c17360d0",
    ),
];

impl Document {
    fn block_protected_hwpx(&mut self) {
        self.block_save = true;
        self.warn(
            "PROTECTED_OR_ACTIVE_CONTENT",
            "error",
            "Signatures, encryption metadata or script content detected. Editing is blocked; these payloads are never executed.",
        );
    }

    /// True only when the script parts are exactly the known default pair, byte for byte.
    fn is_default_hwpx_script_stub(&self, script_names: &[String]) -> bool {
        script_names.len() == DEFAULT_HWPX_SCRIPT_HASHES.len()
            && DEFAULT_HWPX_SCRIPT_HASHES.iter().all(|(name, digest)| {
                script_names.iter().any(|candidate| candidate == name)
                    && self
                        .part(name)
                        .is_some_and(|payload| sha(payload) == *digest)
            })
    }

    pub(super) fn read_hwpx(&mut self) -> Result<()> {
        let mut archive = ZipPackage::new(&self.original)?;
        let names: Vec<String> = archive.names().map(str::to_owned).collect();
        let mut total = 0usize;
        for (index, name) in names.iter().enumerate() {
            let payload = archive.read_member(index, total, MAX_PART, MAX_EXPANDED)?;
            total += payload.len();
            self.parts.insert(name.clone(), payload);
        }
        self.part_order = names;
        require(
            self.part("mimetype")
                .is_some_and(|value| trim_bytes(value) == b"application/hwp+zip"),
            "NOT_HWPX",
            "Missing or invalid HWPX mimetype.",
        )?;
        require(
            self.parts.contains_key("Contents/header.xml")
                && self.parts.contains_key("Contents/content.hpf"),
            "NOT_HWPX",
            "Missing HWPX header or content manifest.",
        )?;
        let mut section_names: Vec<(String, String)> = self
            .part_order
            .iter()
            .filter_map(|name| {
                section_number(name, "Contents/section", ".xml")
                    .map(|number| (name.clone(), number))
            })
            .collect();
        section_names.sort_by(|a, b| (a.1.len(), &a.1).cmp(&(b.1.len(), &b.1)));
        require(
            !section_names.is_empty(),
            "NOT_HWPX",
            "No body sections found.",
        )?;
        let mut trees = BTreeMap::new();
        let mut script_names: Vec<String> = Vec::new();
        for name in self.part_order.clone() {
            let payload = self
                .part(&name)
                .ok_or_else(|| WorkbenchError::new("NOT_HWPX", "Missing HWPX part."))?;
            if [".xml", ".hpf", ".rels"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
            {
                trees.insert(name.clone(), xml_tree(payload)?);
            }
            let lower = name.to_lowercase();
            if lower.contains("scripts/") {
                script_names.push(name.clone());
            } else if [
                "signature",
                "_xmlsignatures",
                "vbascript",
                "drm/",
                "encrypt",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
            {
                self.block_protected_hwpx();
            }
        }
        if !script_names.is_empty() {
            if self.is_default_hwpx_script_stub(&script_names) {
                self.warn(
                    "DEFAULT_SCRIPT_STUB",
                    "info",
                    "Exact known Hancom default script stub retained byte-identically; never executed.",
                );
            } else {
                self.block_protected_hwpx();
            }
        }
        let header = trees.get("Contents/header.xml").ok_or_else(|| {
            WorkbenchError::new("NOT_HWPX", "Missing HWPX header or content manifest.")
        })?;
        let mut styles = BTreeMap::new();
        let mut fonts = BTreeSet::new();
        for element in header.iter() {
            if element.name.local == "charPr" {
                let size = integer_value(element.get("height").unwrap_or("1000"))
                    .map(|height| (height / 100.0).clamp(5.0, 96.0))
                    .unwrap_or(10.0);
                styles.insert(
                    element.get("id").unwrap_or("0").to_owned(),
                    json!({"size":size,
                    "bold":element.children.iter().any(|child| child.name.local == "bold"),
                    "italic":element.children.iter().any(|child| child.name.local == "italic")}),
                );
            }
            if element.name.local == "font" {
                if let Some(face) = element.get("face").filter(|face| !face.is_empty()) {
                    fonts.insert(face.to_owned());
                }
            }
        }
        self.fonts = fonts.into_iter().collect();
        self.warn("FONT_METRICS_UNVERIFIED", "warning", "Requested fonts are listed, but installed font identity and text metrics are not verified. No fonts are bundled.");
        for (section_index, (part, _)) in section_names.iter().enumerate() {
            let root = trees
                .get(part)
                .ok_or_else(|| WorkbenchError::new("NOT_HWPX", "Missing body section."))?;
            let payload = self
                .part(part)
                .ok_or_else(|| WorkbenchError::new("NOT_HWPX", "Missing body section."))?;
            let tokens = text_tokens(payload, part)?;
            let text_count = root.iter().filter(|element| is_hp(element, "t")).count();
            require(
                text_count == tokens.len(),
                "BAD_XML",
                "Text token mapping did not match namespace-aware parse.",
            )?;
            let token_by_element: BTreeMap<usize, TextToken> = tokens
                .into_iter()
                .map(|token| (token.element_index, token))
                .collect();
            let mut context = ScanContext {
                document: self,
                part,
                styles: &styles,
                tokens: &token_by_element,
                paragraph: 0,
                ancestors: vec![],
                node_indexes: BTreeMap::new(),
            };
            scan(root, &mut context, &json!({}))?;
            anchors(root, section_index, &context.node_indexes, context.document);
            self.view.push(json!({"type":"section","label":part,"children":visual(root, &token_by_element, 0)}));
        }
        self.warn("PREVIEW_CACHE_RETAINED", "info", "Embedded preview images/text and line-layout caches are retained unchanged. They may be stale after edits; reopen in Hancom to refresh.");
        Ok(())
    }

    pub(super) fn rewrite_hwpx_part(&self, part: &str, items: &[(&str, &str)]) -> Result<Vec<u8>> {
        let source = self
            .part(part)
            .ok_or_else(|| WorkbenchError::new("VERIFY_FAILED", "Missing edited XML part."))?;
        let mut replacements = vec![];
        for (id, value) in items {
            let token = match self.positions.get(*id) {
                Some(Position::Hwpx(token)) => token,
                _ => {
                    return Err(WorkbenchError::new(
                        "VERIFY_FAILED",
                        "Missing XML text position.",
                    ))
                }
            };
            let escaped = value
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            if token.self_closing {
                let opening = source.get(token.start..token.inner_start).ok_or_else(|| {
                    WorkbenchError::new("VERIFY_FAILED", "Invalid XML text position.")
                })?;
                let cut = opening.len().checked_sub(2).ok_or_else(|| {
                    WorkbenchError::new("VERIFY_FAILED", "Invalid XML empty tag.")
                })?;
                let mut replacement = opening
                    .get(..cut)
                    .ok_or_else(|| WorkbenchError::new("VERIFY_FAILED", "Invalid XML empty tag."))?
                    .to_vec();
                replacement.extend_from_slice(format!(">{escaped}</{}>", token.qname).as_bytes());
                replacements.push((token.start, token.end, replacement));
            } else {
                replacements.push((token.inner_start, token.inner_end, escaped.into_bytes()));
            }
        }
        replacements.sort_by(|a, b| (b.0, b.1, &b.2).cmp(&(a.0, a.1, &a.2)));
        let mut raw = source.to_vec();
        for (start, end, replacement) in replacements {
            require(
                start <= end && end <= raw.len(),
                "VERIFY_FAILED",
                "Invalid XML text position.",
            )?;
            let new_size = raw
                .len()
                .checked_sub(end - start)
                .and_then(|size| size.checked_add(replacement.len()))
                .ok_or_else(|| WorkbenchError::new("LIMIT", "XML part exceeds 32 MiB."))?;
            require(new_size <= MAX_PART, "LIMIT", "XML part exceeds 32 MiB.")?;
            raw.splice(start..end, replacement);
        }
        xml_tree(&raw)?;
        Ok(raw)
    }
}

fn trim_bytes(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .map(|index| index + 1)
        .unwrap_or(start);
    value.get(start..end).unwrap_or(&[])
}

/// Sort arbitrary-length numeric section names without integer overflow.
pub(super) fn section_number(name: &str, prefix: &str, suffix: &str) -> Option<String> {
    let original = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    let number: String = original
        .chars()
        .map(crate::format_readout::ascii_decimal_digit)
        .collect();
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let normalized = number.trim_start_matches('0');
    Some(
        if normalized.is_empty() {
            "0"
        } else {
            normalized
        }
        .to_owned(),
    )
}

fn integer_value(value: &str) -> Option<f64> {
    let normalized: String = value
        .chars()
        .map(crate::format_readout::ascii_decimal_digit)
        .collect();
    let value = normalized.trim();
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    value.replace('_', "").parse::<f64>().ok()
}

fn is_hp(element: &Element, name: &str) -> bool {
    element.name.namespace.as_deref() == Some(HP) && element.name.local == name
}

fn text_tokens(payload: &[u8], part: &str) -> Result<Vec<TextToken>> {
    let mut reader = TokenReader::new(payload)?;
    let mut tokens: Vec<TextToken> = vec![];
    let mut active: Vec<usize> = vec![];
    while let Some(event) = reader.next_token()? {
        match event.kind {
            TokenKind::Start {
                name,
                self_closing,
                element_index,
                ..
            } => {
                for index in &active {
                    if let Some(token) = tokens.get_mut(*index) {
                        token.plain = false;
                    }
                }
                if name.namespace.as_deref() == Some(HP) && name.local == "t" {
                    let index = tokens.len();
                    tokens.push(TextToken {
                        id: format!("{part}#text:{index}"),
                        element_index,
                        start: event.start,
                        inner_start: event.end,
                        inner_end: event.end,
                        end: event.end,
                        plain: true,
                        self_closing,
                        qname: name.qualified,
                    });
                    if !self_closing {
                        active.push(index);
                    }
                }
            }
            TokenKind::End { name, .. }
                if name.namespace.as_deref() == Some(HP) && name.local == "t" =>
            {
                let index = active
                    .pop()
                    .ok_or_else(|| WorkbenchError::new("BAD_XML", "Unmatched XML text end tag."))?;
                let token = tokens.get_mut(index).ok_or_else(|| {
                    WorkbenchError::new(
                        "BAD_XML",
                        "Text token mapping did not match namespace-aware parse.",
                    )
                })?;
                token.inner_end = event.start;
                token.end = event.end;
            }
            _ => {}
        }
    }
    Ok(tokens)
}

struct ScanContext<'a> {
    document: &'a mut Document,
    part: &'a str,
    styles: &'a BTreeMap<String, Value>,
    tokens: &'a BTreeMap<usize, TextToken>,
    paragraph: usize,
    ancestors: Vec<String>,
    node_indexes: BTreeMap<usize, usize>,
}
fn scan(element: &Element, context: &mut ScanContext<'_>, inherited_style: &Value) -> Result<()> {
    require(
        context.ancestors.len() < 128,
        "LIMIT",
        "XML nesting exceeds 128 levels.",
    )?;
    let tag = element.name.local.as_str();
    let feature = match tag {
        "tbl" => Some("TABLE_LAYOUT"),
        "pic" => Some("IMAGES"),
        "equation" => Some("EQUATIONS"),
        "ole" => Some("OLE_OBJECTS"),
        "chart" => Some("CHARTS"),
        "header" | "footer" => Some("HEADERS_FOOTERS"),
        "footNote" | "endNote" => Some("FOOTNOTES"),
        "fieldBegin" => Some("FIELDS"),
        "rect" | "line" | "ellipse" | "polygon" | "curve" | "container" => Some("SHAPES"),
        "pageNum" => Some("PAGE_NUMBERING"),
        "bookmark" => Some("BOOKMARKS"),
        _ => None,
    };
    if let Some(code) = feature {
        context.document.warn(code, "warning", &format!("{tag} content is retained in the package; its full layout/behavior is not editable or certified in this alpha."));
    }
    if tag == "p" {
        context.paragraph += 1;
    }
    let empty_style = json!({});
    let style = if tag == "run" {
        context
            .styles
            .get(element.get("charPrIDRef").unwrap_or("0"))
            .unwrap_or(&empty_style)
    } else {
        inherited_style
    };
    if let Some(token) = context.tokens.get(&element.element_index) {
        let role = if context.ancestors.iter().any(|name| name == "tc") {
            "table-cell"
        } else if context
            .ancestors
            .iter()
            .any(|name| matches!(name.as_str(), "header" | "footer"))
        {
            "header-footer"
        } else {
            "body"
        };
        let editable = token.plain
            && !context.ancestors.iter().any(|name| {
                matches!(
                    name.as_str(),
                    "equation" | "chart" | "ole" | "fieldBegin" | "fieldEnd"
                )
            });
        context
            .node_indexes
            .insert(element.element_index, context.document.nodes.len());
        context.document.nodes.push(TextNode {
            id: token.id.clone(),
            text: hwpx_text(element),
            part: context.part.to_owned(),
            role: role.to_owned(),
            editable,
            paragraph: context.paragraph,
            style: style.clone(),
            anchor: Value::Null,
            paragraph_record_index: None,
            text_start: None,
            text_end: None,
        });
        require(
            context.document.nodes.len() <= MAX_NODES,
            "LIMIT",
            "Too many text nodes.",
        )?;
        context
            .document
            .positions
            .insert(token.id.clone(), Position::Hwpx(token.clone()));
    }
    context.ancestors.push(tag.to_owned());
    for child in &element.children {
        scan(child, context, style)?;
    }
    context.ancestors.pop();
    Ok(())
}

fn hwpx_text(element: &Element) -> String {
    let mut text = element.text.clone().unwrap_or_default();
    for child in &element.children {
        let inline = if child.self_closing && child.name.namespace.as_deref() == Some(HP) {
            match child.name.local.as_str() {
                "lineBreak" | "columnBreak" => Some("\n"),
                "tab" => Some("\t"),
                "nbSpace" => Some("\u{a0}"),
                "fwSpace" => Some("\u{2007}"),
                "hyphen" => Some("\u{ad}"),
                _ => None,
            }
        } else {
            None
        };
        if let Some(inline) = inline {
            text.push_str(inline);
        } else {
            text.push_str(&hwpx_text(child));
        }
        if let Some(tail) = &child.tail {
            text.push_str(tail);
        }
    }
    text
}

fn anchors(
    root: &Element,
    section: usize,
    indexes: &BTreeMap<usize, usize>,
    document: &mut Document,
) {
    for index in indexes.values() {
        if let Some(node) = document.nodes.get_mut(*index) {
            node.anchor = json!({"section":section,"paragraph":0,"charOffset":Value::Null,"inCell":node.role == "table-cell"});
        }
    }
    let paragraphs: Vec<&Element> = if is_hp(root, "p") {
        vec![root]
    } else {
        root.children
            .iter()
            .filter(|child| is_hp(child, "p"))
            .collect()
    };
    for (paragraph, element) in paragraphs.into_iter().enumerate() {
        let mut offset = 0usize;
        visit_anchor(
            element,
            element.element_index,
            section,
            paragraph,
            true,
            false,
            None,
            &mut offset,
            indexes,
            document,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn visit_anchor(
    element: &Element,
    owner: usize,
    section: usize,
    paragraph: usize,
    top_level: bool,
    in_cell: bool,
    parent: Option<&str>,
    offset: &mut usize,
    indexes: &BTreeMap<usize, usize>,
    document: &mut Document,
) {
    let top_level = top_level && !(element.element_index != owner && is_hp(element, "p"));
    let in_cell = in_cell || is_hp(element, "tc");
    if let Some(index) = indexes.get(&element.element_index) {
        if let Some(node) = document.nodes.get_mut(*index) {
            node.anchor = json!({"section":section,"paragraph":paragraph,"charOffset":if top_level {json!(*offset)} else {Value::Null},"inCell":in_cell});
            if top_level {
                *offset += node.text.chars().count();
            } else if node.role == "body" {
                node.role = "text-box".to_owned();
            }
        }
        return;
    }
    let tag = element.name.local.as_str();
    if top_level
        && ((tag == "autoNum" && parent == Some("ctrl"))
            || (element.self_closing
                && element.name.namespace.as_deref() == Some(HP)
                && matches!(tag, "lineBreak" | "columnBreak" | "softHyphen" | "tab")))
    {
        *offset += 1;
    }
    for child in &element.children {
        visit_anchor(
            child,
            owner,
            section,
            paragraph,
            top_level,
            in_cell,
            Some(tag),
            offset,
            indexes,
            document,
        );
    }
}

fn visual(element: &Element, tokens: &BTreeMap<usize, TextToken>, depth: usize) -> Vec<Value> {
    if depth > 32 {
        return vec![];
    }
    let tag = element.name.local.as_str();
    if tag == "t" {
        if let Some(token) = tokens.get(&element.element_index) {
            return vec![json!({"type":"text","node":token.id})];
        }
    }
    if matches!(
        tag,
        "pic" | "ole" | "chart" | "equation" | "rect" | "line" | "ellipse" | "polygon" | "curve"
    ) {
        return vec![json!({"type":"opaque","label":tag})];
    }
    let children: Vec<Value> = element
        .children
        .iter()
        .flat_map(|child| visual(child, tokens, depth + 1))
        .collect();
    match tag {
        "tbl" => vec![json!({"type":"table","children":children})],
        "tr" => vec![json!({"type":"row","children":children})],
        "p" => vec![json!({"type":"paragraph","children":children})],
        "tc" => {
            let span = element
                .children
                .iter()
                .find(|child| child.name.local == "cellSpan");
            let rows = span
                .map(|span| span.get("rowSpan").unwrap_or("1"))
                .unwrap_or("1");
            let columns = span
                .map(|span| span.get("colSpan").unwrap_or("1"))
                .unwrap_or("1");
            let (rows_value, columns_value) = match (integer_value(rows), integer_value(columns)) {
                (Some(rows), Some(columns)) => (
                    rows.clamp(1.0, 100.0) as u64,
                    columns.clamp(1.0, 100.0) as u64,
                ),
                _ => (1, 1),
            };
            let address = element
                .children
                .iter()
                .find(|child| child.name.local == "cellAddr")
                .map(|child| json!({"row":child.get("rowAddr"),"column":child.get("colAddr")}));
            let native_span = span.map(|_| json!({"rows":rows,"columns":columns}));
            vec![
                json!({"type":"cell","colspan":columns_value,"rowspan":rows_value,"address":address,"nativeSpan":native_span,"children":children}),
            ]
        }
        _ => children,
    }
}
