//! Native document text model retaining opaque payloads and source byte ranges.
use crate::cfb::CompoundFile;
use crate::errors::{require, Result, WorkbenchError, MAX_FILE, MAX_NODES};
use crate::hwp_records::{Section, TextPosition};
use crate::package_io::ZipPackage;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[path = "document_hwp.rs"]
mod hwp;
#[path = "document_hwpx.rs"]
mod hwpx;

pub const HP: &str = "http://www.hancom.co.kr/hwpml/2011/paragraph";
pub const HC: &str = "http://www.hancom.co.kr/hwpml/2011/core";
pub const HH: &str = "http://www.hancom.co.kr/hwpml/2011/head";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextNode {
    pub id: String,
    pub text: String,
    pub part: String,
    pub role: String,
    pub editable: bool,
    pub paragraph: usize,
    pub style: Value,
    pub anchor: Value,
    pub paragraph_record_index: Option<usize>,
    pub text_start: Option<usize>,
    pub text_end: Option<usize>,
}

pub struct Document {
    pub original: Vec<u8>,
    pub name: String,
    pub hash: String,
    pub format: String,
    pub nodes: Vec<TextNode>,
    pub fonts: Vec<String>,
    pub warnings: Vec<Value>,
    pub view: Vec<Value>,
    pub block_save: bool,
    pub parts: BTreeMap<String, Vec<u8>>,
    pub sections: BTreeMap<String, Vec<u8>>,
    pub hwp_sections: BTreeMap<String, Section>,
    pub edits: Vec<(String, String)>,
    part_order: Vec<String>,
    positions: BTreeMap<String, Position>,
    by_id: BTreeMap<String, usize>,
    references: BTreeMap<String, String>,
    cfb: Option<CompoundFile>,
    compressed: bool,
    flags: u32,
    stream_trailers: BTreeMap<String, bool>,
}
#[derive(Clone)]
enum Position {
    Hwp(TextPosition),
    Hwpx(TextToken),
}
#[derive(Clone)]
struct TextToken {
    id: String,
    element_index: usize,
    start: usize,
    inner_start: usize,
    inner_end: usize,
    end: usize,
    plain: bool,
    self_closing: bool,
    qname: String,
}
/// Maximum text length in Unicode scalars per patch value.
pub const MAX_TEXT: usize = 200_000;
/// Maximum number of operations in a complete patch.
pub const MAX_OPERATIONS: usize = 1000;

impl Document {
    pub fn new(bytes: impl AsRef<[u8]>, name: impl Into<String>) -> Result<Self> {
        let bytes = bytes.as_ref();
        require(
            !bytes.is_empty() && bytes.len() <= MAX_FILE,
            "LIMIT",
            "File must be between 1 byte and 64 MiB.",
        )?;
        let format = if bytes.starts_with(&crate::cfb::MAGIC) {
            "hwp"
        } else if bytes.starts_with(b"PK") {
            "hwpx"
        } else {
            return Err(WorkbenchError::new(
                "UNSUPPORTED_FORMAT",
                "Only HWP 5.x CFB and HWPX ZIP documents are supported. HWP 3.x is not supported.",
            ));
        };
        let mut document = Self {
            original: bytes.to_vec(),
            name: name.into(),
            hash: sha(bytes),
            format: format.into(),
            nodes: vec![],
            fonts: vec![],
            warnings: vec![],
            view: vec![],
            block_save: false,
            parts: BTreeMap::new(),
            part_order: vec![],
            sections: BTreeMap::new(),
            hwp_sections: BTreeMap::new(),
            edits: vec![],
            positions: BTreeMap::new(),
            by_id: BTreeMap::new(),
            references: BTreeMap::new(),
            cfb: None,
            compressed: false,
            flags: 0,
            stream_trailers: BTreeMap::new(),
        };
        if format == "hwpx" {
            document.read_hwpx()?;
        } else {
            document.read_hwp()?;
        }
        require(
            document.nodes.len() <= MAX_NODES,
            "LIMIT",
            "Too many text nodes.",
        )?;
        let mut section_numbers = BTreeMap::new();
        let mut section_counts = BTreeMap::new();
        for (index, node) in document.nodes.iter_mut().enumerate() {
            document.by_id.insert(node.id.clone(), index);
            let next_section = section_numbers.len();
            let section = section_numbers
                .entry(node.part.clone())
                .or_insert(next_section);
            let count = section_counts.entry(node.part.clone()).or_insert(0usize);
            document
                .references
                .insert(format!("s{section}:t{count}"), node.id.clone());
            *count += 1;
            if document.block_save {
                node.editable = false;
            }
        }
        document.warn("LAYOUT_UNVERIFIED", "warning", "The workspace preview is approximate. Hancom layout, pagination and font fidelity have not been certified.");
        Ok(document)
    }

    pub fn part(&self, name: &str) -> Option<&[u8]> {
        self.parts.get(name).map(Vec::as_slice)
    }

    pub fn node(&self, id: &str) -> Option<&TextNode> {
        self.by_id.get(id).and_then(|index| self.nodes.get(*index))
    }

    pub fn current_text<'a>(&'a self, node: &'a TextNode) -> &'a str {
        self.edits
            .iter()
            .find(|(id, _)| id == &node.id)
            .map(|(_, text)| text.as_str())
            .unwrap_or(&node.text)
    }

    pub fn inspect(&self) -> Value {
        let nodes: Vec<Value> = self.nodes.iter().map(|node| {
            // Struct field order is the Python dataclass order.
            serde_json::json!({"id":node.id,"text":self.current_text(node),"part":node.part,"role":node.role,
                "editable":node.editable,"paragraph":node.paragraph,"style":node.style,"anchor":node.anchor,
                "paragraphRecordIndex":node.paragraph_record_index,"textStart":node.text_start,"textEnd":node.text_end})
        }).collect();
        let parts: Vec<Value> = self.part_order.iter().filter_map(|name| {
            self.parts.get(name).map(|payload| serde_json::json!({"name":name,"size":payload.len(),"sha256":sha(payload)}))
        }).collect();
        serde_json::json!({"schemaVersion":1,"format":self.format,"name":self.name,"sha256":self.hash,
            "nodes":nodes,"warnings":self.warnings,"view":self.view,"fonts":self.fonts,
            "capabilities":{"textEdit":!self.block_save,"lengthChangingText":!self.block_save,"structuralEdit":false,
                "formattingEdit":false,"certifiedLayout":false,"saveAs":!self.block_save},
            "parts":parts,"pendingChanges":self.edits.len()})
    }

    pub fn diff(&self) -> Value {
        let changes: Vec<Value> = self
            .edits
            .iter()
            .filter_map(|(id, text)| {
                self.node(id).map(|node| {
            serde_json::json!({"nodeId":id,"before":node.text,"after":text,"part":node.part})
        })
            })
            .collect();
        serde_json::json!({"baseSha256":self.hash,"changes":changes,"warnings":self.warnings,"layoutCertified":false})
    }

    pub fn resolve_node_id(&self, node_id: &str) -> Result<String> {
        let resolved = self
            .references
            .get(node_id)
            .map(String::as_str)
            .unwrap_or(node_id);
        require(
            self.by_id.contains_key(resolved),
            "NODE_NOT_FOUND",
            &format!("Unknown node ID: {node_id}"),
        )?;
        Ok(resolved.to_owned())
    }

    /// Validate every operation before replacing the pending edit batch.
    pub fn patch(&mut self, patch: &Value) -> Result<Value> {
        require(
            patch.is_object()
                && patch.get("schemaVersion").is_some_and(|version| {
                    version.as_f64() == Some(1.0) || version.as_bool() == Some(true)
                }),
            "BAD_PATCH",
            "Expected patch schemaVersion 1.",
        )?;
        require(
            patch.get("baseSha256").and_then(Value::as_str) == Some(&self.hash),
            "STALE_DOCUMENT",
            "Patch hash differs from source document; re-inspect before editing.",
        )?;
        let operations = patch.get("operations").and_then(Value::as_array);
        require(
            operations.is_some_and(|items| !items.is_empty() && items.len() <= MAX_OPERATIONS),
            "BAD_PATCH",
            "Supply 1–1000 operations.",
        )?;
        require(
            !self.block_save,
            "SAVE_BLOCKED",
            "Protected/active content blocks edits.",
        )?;
        let operations = operations
            .ok_or_else(|| WorkbenchError::new("BAD_PATCH", "Supply 1–1000 operations."))?;
        let mut pending = self.edits.clone();
        let mut seen = HashSet::new();
        for operation in operations {
            require(
                operation.as_object().is_some_and(|fields| {
                    fields.len() == 4
                        && ["op", "nodeId", "expectedText", "text"]
                            .iter()
                            .all(|key| fields.contains_key(*key))
                }),
                "BAD_PATCH",
                "Each operation requires exactly op, nodeId, expectedText, text.",
            )?;
            require(
                operation.get("op").and_then(Value::as_str) == Some("set_text")
                    && operation.get("nodeId").is_some_and(Value::is_string),
                "BAD_PATCH",
                "Only set_text is implemented.",
            )?;
            let supplied_id = operation
                .get("nodeId")
                .and_then(Value::as_str)
                .ok_or_else(|| WorkbenchError::new("BAD_PATCH", "Only set_text is implemented."))?;
            let id = self.resolve_node_id(supplied_id)?;
            require(
                seen.insert(id.clone()),
                "BAD_PATCH",
                "Duplicate target in patch.",
            )?;
            let node = self.node(&id).ok_or_else(|| {
                WorkbenchError::new("NODE_NOT_FOUND", format!("Unknown node ID: {supplied_id}"))
            })?;
            require(
                node.editable,
                "UNSUPPORTED_ELEMENT",
                "This text is not safely editable.",
            )?;
            let before = pending
                .iter()
                .find(|(key, _)| key == &id)
                .map(|(_, text)| text.as_str())
                .unwrap_or(&node.text);
            require(
                operation.get("expectedText").and_then(Value::as_str) == Some(before),
                "STALE_TEXT",
                &format!("Expected text no longer matches {id}"),
            )?;
            let value = operation.get("text").ok_or_else(|| {
                WorkbenchError::new(
                    "BAD_PATCH",
                    "Each operation requires exactly op, nodeId, expectedText, text.",
                )
            })?;
            if self.format == "hwp" {
                if let Some(text) = value.as_str() {
                    require(text.chars().all(|c| c >= ' '), "UNSUPPORTED_ELEMENT", "Plain text only: do not touch native controls, tabs, fields or paragraph breaks.")?;
                }
            }
            check_text(value)?;
            let text = value.as_str().ok_or_else(|| {
                WorkbenchError::new(
                    "BAD_PATCH",
                    "Text must be a string of at most 200,000 characters.",
                )
            })?;
            if self.format == "hwp" {
                let section = self.hwp_sections.get(&node.part).ok_or_else(|| WorkbenchError::new("UNSUPPORTED_ELEMENT", "Text has no complete paragraph header. Re-inspect a supported HWP 5 paragraph."))?;
                let position = match self.positions.get(&id) {
                    Some(Position::Hwp(position)) => position,
                    _ => {
                        return Err(WorkbenchError::new(
                            "UNSUPPORTED_ELEMENT",
                            "Select an existing plain text run inside one paragraph.",
                        ))
                    }
                };
                crate::hwp_records::validate_text_edit(section, position)?;
            }
            let index = pending.iter().position(|(key, _)| key == &id);
            if text == node.text {
                if let Some(index) = index {
                    pending.remove(index);
                }
            } else if let Some(index) = index {
                if let Some(item) = pending.get_mut(index) {
                    item.1 = text.to_owned();
                }
            } else {
                pending.push((id, text.to_owned()));
            }
        }
        self.edits = pending;
        Ok(self.diff())
    }

    pub fn serialize(&self, acknowledge_risks: bool) -> Result<Vec<u8>> {
        if self.edits.is_empty() {
            return Ok(self.original.clone());
        }
        require(!self.block_save, "SAVE_BLOCKED", "Saving is blocked.")?;
        require(acknowledge_risks, "RISK_ACK_REQUIRED", "Edited output requires explicit acknowledgement of compatibility and stale-preview risks.")?;
        let mut by_part: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
        for (id, text) in &self.edits {
            let node = self
                .node(id)
                .ok_or_else(|| WorkbenchError::new("VERIFY_FAILED", "Unknown edited node."))?;
            by_part.entry(&node.part).or_default().push((id, text));
        }
        let mut changed = BTreeMap::new();
        let mut expected_sections = BTreeMap::new();
        for (part, items) in by_part {
            if self.format == "hwp" {
                let section = self
                    .hwp_sections
                    .get(part)
                    .ok_or_else(|| WorkbenchError::new("VERIFY_FAILED", "Missing HWP section."))?;
                let mut edits = vec![];
                for (id, text) in items {
                    match self.positions.get(id) {
                        Some(Position::Hwp(position)) => edits.push((
                            position.clone(),
                            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                        )),
                        _ => {
                            return Err(WorkbenchError::new(
                                "VERIFY_FAILED",
                                "Missing HWP text position.",
                            ))
                        }
                    }
                }
                let raw = crate::hwp_records::rewrite_section(section, &edits)?;
                expected_sections.insert(part.to_owned(), raw.clone());
                changed.insert(part.to_owned(), self.encode_hwp_stream(part, &raw)?);
            } else {
                changed.insert(part.to_owned(), self.rewrite_hwpx_part(part, &items)?);
            }
        }
        let result = if self.format == "hwp" {
            self.cfb
                .as_ref()
                .ok_or_else(|| WorkbenchError::new("VERIFY_FAILED", "Missing compound file."))?
                .rewrite(&changed)?
        } else {
            ZipPackage::new(&self.original)?.rewrite(&changed)?
        };
        let check = Self::new(&result, &self.name)?;
        require(
            check.parts.keys().eq(self.parts.keys()),
            "VERIFY_FAILED",
            "Package inventory changed.",
        )?;
        for (name, payload) in &self.parts {
            if !changed.contains_key(name) {
                require(
                    check.part(name) == Some(payload.as_slice()),
                    "VERIFY_FAILED",
                    &format!("Opaque part changed: {name}"),
                )?;
            }
        }
        if self.format == "hwp" {
            for (part, expected) in expected_sections {
                require(
                    check.sections.get(&part) == Some(&expected),
                    "VERIFY_FAILED",
                    &format!("Edited HWP section differs on reopen: {part}"),
                )?;
            }
        } else {
            for (id, text) in &self.edits {
                require(
                    check.node(id).is_some_and(|node| &node.text == text),
                    "VERIFY_FAILED",
                    &format!("Edit was not preserved on reopen: {id}"),
                )?;
            }
        }
        Ok(result)
    }

    fn warn(&mut self, code: &str, severity: &str, message: &str) {
        if !self
            .warnings
            .iter()
            .any(|warning| warning.get("code").and_then(Value::as_str) == Some(code))
        {
            self.warnings
                .push(serde_json::json!({"code":code,"severity":severity,"message":message}));
        }
    }
}

pub fn sha(data: &[u8]) -> String {
    crate::workspace::sha256(data)
}
pub fn patch_input(value: &Value) -> &Value {
    if value.is_object() && value.get("schemaVersion").is_none() {
        if let Some(patch) = value.get("patch") {
            return patch;
        }
    }
    value
}
pub fn check_text(value: &Value) -> Result<()> {
    require(
        value
            .as_str()
            .is_some_and(|text| text.chars().count() <= MAX_TEXT),
        "BAD_PATCH",
        "Text must be a string of at most 200,000 characters.",
    )?;
    require(value.as_str().is_some_and(|text| text.chars().all(|c| matches!(c as u32, 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff))), "CONTROL_TEXT", "Use plain text only; native tabs, newlines and control characters are not supported by this operation.")
}
pub fn local(tag: &str) -> &str {
    tag.rsplit('}').next().unwrap_or(tag)
}
