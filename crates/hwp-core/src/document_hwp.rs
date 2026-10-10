//! HWP 5 stream classification, plain runs and bounded native compression.
use super::{hwpx::section_number, sha, Document, Position, TextNode};
use crate::cfb::CompoundFile;
use crate::errors::{require, Result, WorkbenchError, MAX_EXPANDED, MAX_NODES, MAX_PART};
use crate::hwp_records::{parse_section, Record, Section, TextPosition, PARA_HEADER, PARA_TEXT};
use flate2::{write::DeflateEncoder, Compression, Crc, Decompress, FlushDecompress, Status};
use serde_json::{json, Value};
use std::io::Write;

// Exact observed Hancom default stub hashes. Only native bytes are hashed;
// scripts are never decoded, interpreted or executed.
pub const DEFAULT_SCRIPT_HASHES: &[(&str, &str)] = &[
    (
        "Scripts/DefaultJScript",
        "e1f35ff38336598f79448c84b41bcb508d53a552808454a76ee12691cb2c97e4",
    ),
    (
        "Scripts/JScriptVersion",
        "7c9fa136d4413fa6173637e883b6998d32e1d675f88cddff9dcbcf331820f4b8",
    ),
];
// Native control slots occupy eight UTF-16 units; other controls occupy one.
const EXTENDED_CONTROL_BYTES: usize = 16;
// Inflate in fixed chunks, independently of untrusted compressed lengths.
const INFLATE_CHUNK: usize = 64 * 1024;
const COMPRESSION_MESSAGE: &str = "Invalid or oversized compressed HWP stream.";

impl Document {
    pub(super) fn read_hwp(&mut self) -> Result<()> {
        let cfb = CompoundFile::new(&self.original)?;
        for (name, bytes) in cfb.ordered_streams() {
            self.part_order.push(name.to_owned());
            self.parts.insert(name.to_owned(), bytes.to_vec());
        }
        self.cfb = Some(cfb);
        let head = self.part("FileHeader").unwrap_or(&[]);
        require(
            head.len() >= 40 && head.starts_with(b"HWP Document File"),
            "NOT_HWP",
            "Missing HWP FileHeader.",
        )?;
        require(
            head.get(35) == Some(&5),
            "UNSUPPORTED_FORMAT",
            "Only HWP 5.x is supported.",
        )?;
        let flags = read_u32(head, 36)?;
        // Password, distribution and DRM flags are not bypassed by this reader.
        let protected_flags = (1 << 1) | (1 << 2) | (1 << 4) | (1 << 8) | (1 << 9);
        require(
            flags & protected_flags == 0,
            "PROTECTED_HWP",
            "Password, distribution or DRM protected HWP cannot be opened by this alpha.",
        )?;
        self.flags = flags;
        self.compressed = flags & 1 != 0;
        self.classify_hwp_scripts()?;
        let mut section_names: Vec<(String, String)> = self
            .part_order
            .iter()
            .filter_map(|name| {
                section_number(name, "BodyText/Section", "").map(|number| (name.clone(), number))
            })
            .collect();
        section_names.sort_by(|a, b| (a.1.len(), &a.1).cmp(&(b.1.len(), &b.1)));
        let mut expanded_total = 0usize;
        for (section_index, (part, _)) in section_names.iter().enumerate() {
            let raw = self.hwp_stream(part)?;
            expanded_total += raw.len();
            require(
                expanded_total <= MAX_EXPANDED,
                "LIMIT",
                "Expanded HWP sections exceed limit.",
            )?;
            self.read_hwp_section(part, &raw, section_index)?;
            self.sections.insert(part.clone(), raw);
        }
        require(
            !self.sections.is_empty(),
            "NOT_HWP",
            "No HWP body sections found.",
        )?;
        self.warn("HWP_REFLOW_UNVERIFIED", "warning", "Text length may change in existing plain text runs. Stored line starts are adjusted, but layout is stale; readers must recompute line breaks. Hancom reflow has not been verified.");
        self.warn("HWP_LAYOUT_NOT_RENDERED", "warning", "HWP is shown as structured text, not a page-faithful rendering. Table/control/style streams are retained but not visually reconstructed.");
        self.warn(
            "PREVIEW_CACHE_RETAINED",
            "info",
            "Stored preview content remains unchanged and may be stale after editing.",
        );
        Ok(())
    }

    fn classify_hwp_scripts(&mut self) -> Result<()> {
        let script_names: Vec<String> = self
            .parts
            .keys()
            .filter(|name| name.to_lowercase().contains("script"))
            .cloned()
            .collect();
        let mut known_stub = script_names.len() == DEFAULT_SCRIPT_HASHES.len()
            && DEFAULT_SCRIPT_HASHES
                .iter()
                .all(|(name, _)| script_names.iter().any(|candidate| candidate == name));
        if known_stub {
            // Read both streams even when the first differs, as the oracle does.
            for (name, digest) in DEFAULT_SCRIPT_HASHES {
                let payload = self.hwp_stream(name)?;
                known_stub &= sha(&payload) == *digest;
            }
        }
        if known_stub {
            self.warn(
                "DEFAULT_SCRIPT_STUB",
                "info",
                "Exact known Hancom default script stub retained byte-identically; never executed.",
            );
        }
        let has_scripts = !script_names.is_empty() || self.flags & (1 << 3) != 0;
        let has_signature = self.flags & ((1 << 6) | (1 << 7)) != 0;
        if has_signature || (has_scripts && !known_stub) {
            self.block_save = true;
            self.warn(
                "PROTECTED_OR_ACTIVE_CONTENT",
                "error",
                "Script or digital-signature metadata detected; editing is blocked.",
            );
        }
        Ok(())
    }

    fn hwp_stream(&mut self, name: &str) -> Result<Vec<u8>> {
        let payload = self
            .part(name)
            .ok_or_else(|| WorkbenchError::new("NOT_HWP", "Missing HWP stream."))?;
        if !self.compressed {
            require(
                payload.len() <= MAX_PART,
                "LIMIT",
                "HWP stream exceeds limit.",
            )?;
            let raw = payload.to_vec();
            self.stream_trailers.insert(name.to_owned(), false);
            return Ok(raw);
        }
        let (raw, trailer) = inflate(payload)?;
        self.stream_trailers.insert(name.to_owned(), trailer);
        Ok(raw)
    }

    pub(super) fn encode_hwp_stream(&self, name: &str, raw: &[u8]) -> Result<Vec<u8>> {
        if !self.compressed {
            return Ok(raw.to_vec());
        }
        let mut encoder = DeflateEncoder::new(vec![], Compression::new(6));
        encoder
            .write_all(raw)
            .map_err(|error| WorkbenchError::new("BAD_HWP_COMPRESSION", error.to_string()))?;
        let mut output = encoder
            .finish()
            .map_err(|error| WorkbenchError::new("BAD_HWP_COMPRESSION", error.to_string()))?;
        if self.stream_trailers.get(name) == Some(&true) {
            output.extend_from_slice(&checksum(raw).to_le_bytes());
            let length = u32::try_from(raw.len())
                .map_err(|_| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
            output.extend_from_slice(&length.to_le_bytes());
        }
        Ok(output)
    }

    fn read_hwp_section(&mut self, name: &str, raw: &[u8], section_index: usize) -> Result<()> {
        let section = parse_section(raw)?;
        let mut paragraph = 0usize;
        let mut top_count = 0usize;
        let mut paragraph_level = 0u32;
        let mut controls: Vec<Control> = vec![];
        let mut children = vec![];
        for item in &section.records {
            while controls
                .last()
                .is_some_and(|control| item.level <= control.level)
            {
                controls.pop();
            }
            if item.tag == 71 && item.data.len() >= 4 {
                let id = item.data.get(..4).ok_or_else(|| {
                    WorkbenchError::new("BAD_HWP_RECORD", "Record payload exceeds stream bounds.")
                })?;
                controls.push(Control {
                    level: item.level,
                    id: id.iter().rev().copied().collect(),
                    table_seen: false,
                    list_role: None,
                });
            }
            if let Some(table) = controls.last_mut().filter(|control| control.id == b"tbl ") {
                if item.level == table.level + 1 {
                    if item.tag == 77 {
                        table.table_seen = true;
                        table.list_role = None;
                    } else if item.tag == 72 {
                        table.list_role = Some(if table.table_seen { "cell" } else { "caption" });
                    }
                }
            }
            if item.tag == PARA_HEADER {
                paragraph += 1;
                paragraph_level = item.level;
                if item.level == 0 {
                    top_count += 1;
                }
            }
            if item.tag != PARA_TEXT {
                continue;
            }
            require(
                item.data.len() % 2 == 0,
                "BAD_HWP_TEXT",
                "Misaligned UTF-16 text record.",
            )?;
            let control = controls.last();
            let in_cell = control
                .is_some_and(|control| control.id == b"tbl " && control.list_role == Some("cell"));
            let nested = paragraph_level > 0;
            let role = if nested {
                if controls
                    .iter()
                    .any(|control| control.id == b"head" || control.id == b"foot")
                {
                    "header-footer"
                } else if control.is_some_and(|control| {
                    control.id == b"tbl " && control.list_role == Some("caption")
                }) {
                    "table-caption"
                } else if in_cell {
                    "table-cell"
                } else {
                    "text-box"
                }
            } else {
                "text-run"
            };
            let mut context = RunContext {
                part: name,
                section_index,
                paragraph,
                top_paragraph: top_count.saturating_sub(1),
                nested,
                in_cell,
                role,
                section: &section,
                item,
                segment: 0,
                char_offset: 0,
            };
            let mut offset = 0usize;
            while offset < item.data.len() {
                let code = read_u16(&item.data, offset)?;
                if code < 32 {
                    let width = if matches!(code, 1..=9 | 11..=12 | 14..=23) {
                        EXTENDED_CONTROL_BYTES
                    } else {
                        2
                    };
                    require(
                        offset + width <= item.data.len(),
                        "BAD_HWP_TEXT",
                        "Truncated HWP control sequence.",
                    )?;
                    if matches!(code, 9 | 10 | 18 | 24 | 25 | 30 | 31) {
                        context.char_offset += 1;
                    }
                    if code == 13 {
                        if context.segment == 0 {
                            self.append_hwp_run(
                                &context,
                                offset,
                                offset,
                                String::new(),
                                &mut children,
                            )?;
                        }
                        break;
                    }
                    offset += width;
                    continue;
                }
                let begin = offset;
                while offset < item.data.len() && read_u16(&item.data, offset)? >= 32 {
                    offset += 2;
                }
                let bytes = item.data.get(begin..offset).ok_or_else(|| {
                    WorkbenchError::new("BAD_HWP_TEXT", "Invalid UTF-16 in paragraph text.")
                })?;
                let units: Result<Vec<u16>> = bytes
                    .chunks_exact(2)
                    .map(|pair| read_u16(pair, 0))
                    .collect();
                let text = String::from_utf16(&units?).map_err(|_| {
                    WorkbenchError::new("BAD_HWP_TEXT", "Invalid UTF-16 in paragraph text.")
                })?;
                let scalar_length = text.chars().count();
                self.append_hwp_run(&context, begin, offset, text, &mut children)?;
                context.char_offset += scalar_length;
                context.segment += 1;
            }
        }
        self.view
            .push(json!({"type":"section","label":name,"children":children}));
        self.hwp_sections.insert(name.to_owned(), section);
        Ok(())
    }

    fn append_hwp_run(
        &mut self,
        context: &RunContext<'_>,
        begin: usize,
        end: usize,
        text: String,
        children: &mut Vec<Value>,
    ) -> Result<()> {
        let id = format!(
            "{}#record:{}:text:{}",
            context.part, context.item.index, context.segment
        );
        let paragraph_record_index = context.section.owners.get(&context.item.index).copied();
        let base = context
            .section
            .text_offsets
            .get(&context.item.index)
            .copied()
            .unwrap_or(0);
        let text_start = base + begin / 2;
        let text_end = base + end / 2;
        self.nodes.push(TextNode { id:id.clone(),text,part:context.part.to_owned(),role:context.role.to_owned(),editable:true,paragraph:context.paragraph,style:json!({}),
            anchor:json!({"section":context.section_index,"paragraph":context.top_paragraph,"charOffset":if context.nested {Value::Null} else {json!(context.char_offset)},"inCell":context.in_cell}),
            paragraph_record_index,text_start:Some(text_start),text_end:Some(text_end) });
        require(
            self.nodes.len() <= MAX_NODES,
            "LIMIT",
            "Too many text nodes.",
        )?;
        self.positions.insert(
            id.clone(),
            Position::Hwp(TextPosition {
                paragraph_record_index,
                record: context.item.index,
                start: context.item.payload_start + begin,
                end: context.item.payload_start + end,
                text_start,
                text_end,
            }),
        );
        children.push(json!({"type":"paragraph","children":[{"type":"text","node":id}]}));
        Ok(())
    }
}

struct Control {
    level: u32,
    id: Vec<u8>,
    table_seen: bool,
    list_role: Option<&'static str>,
}
struct RunContext<'a> {
    part: &'a str,
    section_index: usize,
    paragraph: usize,
    top_paragraph: usize,
    nested: bool,
    in_cell: bool,
    role: &'a str,
    section: &'a Section,
    item: &'a Record,
    segment: usize,
    char_offset: usize,
}
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|value| <[u8; 2]>::try_from(value).ok())
        .ok_or_else(|| WorkbenchError::new("BAD_HWP_TEXT", "Misaligned UTF-16 text record."))?;
    Ok(u16::from_le_bytes(value))
}
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|value| <[u8; 4]>::try_from(value).ok())
        .ok_or_else(|| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
    Ok(u32::from_le_bytes(value))
}
fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(bytes);
    crc.sum()
}

fn inflate(payload: &[u8]) -> Result<(Vec<u8>, bool)> {
    let mut decompressor = Decompress::new(false);
    let mut raw = vec![];
    let mut buffer = [0u8; INFLATE_CHUNK];
    loop {
        let input_offset = usize::try_from(decompressor.total_in())
            .map_err(|_| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
        let input = payload
            .get(input_offset..)
            .ok_or_else(|| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
        let before_in = decompressor.total_in();
        let before_out = decompressor.total_out();
        let remaining = (MAX_PART + 1).saturating_sub(raw.len());
        let length = remaining.min(buffer.len());
        let target = buffer
            .get_mut(..length)
            .ok_or_else(|| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
        let status = decompressor
            .decompress(input, target, FlushDecompress::None)
            .map_err(|error| WorkbenchError::new("BAD_HWP_COMPRESSION", error.to_string()))?;
        let written = usize::try_from(decompressor.total_out() - before_out)
            .map_err(|_| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
        raw.extend_from_slice(
            buffer
                .get(..written)
                .ok_or_else(|| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?,
        );
        require(raw.len() <= MAX_PART, "LIMIT", COMPRESSION_MESSAGE)?;
        if status == Status::StreamEnd {
            break;
        }
        require(
            decompressor.total_in() > before_in || decompressor.total_out() > before_out,
            "LIMIT",
            COMPRESSION_MESSAGE,
        )?;
    }
    let consumed = usize::try_from(decompressor.total_in())
        .map_err(|_| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
    let trailer = payload
        .get(consumed..)
        .ok_or_else(|| WorkbenchError::new("LIMIT", COMPRESSION_MESSAGE))?;
    if trailer.is_empty() {
        return Ok((raw, false));
    }
    require(trailer.len() == 8, "LIMIT", COMPRESSION_MESSAGE)?;
    require(
        read_u32(trailer, 0)? == checksum(&raw) && read_u32(trailer, 4)? as usize == raw.len(),
        "LIMIT",
        COMPRESSION_MESSAGE,
    )?;
    Ok((raw, true))
}
