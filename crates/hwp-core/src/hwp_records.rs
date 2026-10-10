//! Bounded HWP 5 records, paragraph ownership and preservation-first text edits.
use crate::errors::{require, Result, WorkbenchError, MAX_NODES, MAX_PART};
use std::collections::BTreeMap;

pub const PARA_HEADER: u32 = 66;
pub const PARA_TEXT: u32 = 67;
pub const PARA_CHAR_SHAPE: u32 = 68;
pub const PARA_LINE_SEG: u32 = 69;
pub const PARA_RANGE_TAG: u32 = 70;
/// The 12-bit size marker that introduces a separate 32-bit payload length.
pub const EXTENDED_SIZE: usize = 0xfff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub index: usize,
    pub tag: u32,
    pub level: u32,
    pub data: Vec<u8>,
    pub original: Vec<u8>,
    pub payload_start: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub header_index: usize,
    pub children: Vec<usize>,
    pub text_units: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub records: Vec<Record>,
    pub paragraphs: BTreeMap<usize, Paragraph>,
    pub owners: BTreeMap<usize, usize>,
    pub text_offsets: BTreeMap<usize, usize>,
}

/// An original-source byte range and its native paragraph UTF-16 range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPosition {
    pub paragraph_record_index: Option<usize>,
    pub record: usize,
    pub start: usize,
    pub end: usize,
    pub text_start: usize,
    pub text_end: usize,
}

fn error(code: &str, message: &str) -> WorkbenchError {
    WorkbenchError::new(code, message)
}

fn bytes_at<'a>(
    data: &'a [u8],
    start: usize,
    length: usize,
    code: &str,
    message: &str,
) -> Result<&'a [u8]> {
    start
        .checked_add(length)
        .and_then(|end| data.get(start..end))
        .ok_or_else(|| error(code, message))
}

fn read_u32(data: &[u8], offset: usize, code: &str, message: &str) -> Result<u32> {
    let bytes = bytes_at(data, offset, 4, code, message)?;
    let array = <[u8; 4]>::try_from(bytes).map_err(|_| error(code, message))?;
    Ok(u32::from_le_bytes(array))
}

fn write_bytes(data: &mut [u8], offset: usize, value: &[u8]) -> Result<()> {
    let end = offset
        .checked_add(value.len())
        .ok_or_else(|| error("LIMIT", "HWP record exceeds 32 MiB."))?;
    let target = data.get_mut(offset..end).ok_or_else(|| {
        error(
            "UNSUPPORTED_ELEMENT",
            "Incomplete HWP paragraph position record.",
        )
    })?;
    target.copy_from_slice(value);
    Ok(())
}

fn section_record(section: &Section, index: usize) -> Result<&Record> {
    section.records.get(index).ok_or_else(|| {
        error(
            "UNSUPPORTED_ELEMENT",
            "Select an existing plain text run inside one paragraph.",
        )
    })
}

pub fn parse_section(raw: &[u8]) -> Result<Section> {
    let mut section = Section {
        records: Vec::new(),
        paragraphs: BTreeMap::new(),
        owners: BTreeMap::new(),
        text_offsets: BTreeMap::new(),
    };
    let mut stack = Vec::<usize>::new();
    let mut offset = 0;
    while offset < raw.len() {
        let bits = read_u32(raw, offset, "BAD_HWP_RECORD", "Truncated record header.")?;
        let tag = bits & 0x3ff;
        let level = (bits >> 10) & 0x3ff;
        let mut size = (bits >> 20) as usize;
        let mut header_size = 4;
        if size == EXTENDED_SIZE {
            let size_offset = offset
                .checked_add(4)
                .ok_or_else(|| error("BAD_HWP_RECORD", "Truncated extended record header."))?;
            size = read_u32(
                raw,
                size_offset,
                "BAD_HWP_RECORD",
                "Truncated extended record header.",
            )? as usize;
            header_size = 8;
        }
        let start = offset
            .checked_add(header_size)
            .ok_or_else(|| error("BAD_HWP_RECORD", "Record payload exceeds stream bounds."))?;
        let data = bytes_at(
            raw,
            start,
            size,
            "BAD_HWP_RECORD",
            "Record payload exceeds stream bounds.",
        )?;
        let end = start
            .checked_add(size)
            .ok_or_else(|| error("BAD_HWP_RECORD", "Record payload exceeds stream bounds."))?;
        let index = section.records.len();
        let original = raw
            .get(offset..end)
            .ok_or_else(|| error("BAD_HWP_RECORD", "Record payload exceeds stream bounds."))?;
        section.records.push(Record {
            index,
            tag,
            level,
            data: data.to_vec(),
            original: original.to_vec(),
            payload_start: start,
        });
        require(
            section.records.len() <= MAX_NODES,
            "LIMIT",
            "Too many HWP records.",
        )?;
        while let Some(owner) = stack.last() {
            if level > section_record(&section, *owner)?.level {
                break;
            }
            stack.pop();
        }
        if tag == PARA_HEADER {
            section.paragraphs.insert(
                index,
                Paragraph {
                    header_index: index,
                    children: Vec::new(),
                    text_units: 0,
                },
            );
            stack.push(index);
        } else if let Some(owner_index) = stack.last().copied() {
            if Some(level) == section_record(&section, owner_index)?.level.checked_add(1) {
                let owner = section.paragraphs.get_mut(&owner_index).ok_or_else(|| {
                    error("BAD_HWP_RECORD", "Record payload exceeds stream bounds.")
                })?;
                owner.children.push(index);
                section.owners.insert(index, owner_index);
                if tag == PARA_TEXT {
                    require(
                        size % 2 == 0,
                        "BAD_HWP_TEXT",
                        "Misaligned UTF-16 text record.",
                    )?;
                    section.text_offsets.insert(index, owner.text_units);
                    owner.text_units = owner.text_units.checked_add(size / 2).ok_or_else(|| {
                        error("LIMIT", "HWP paragraph character count exceeds bounds.")
                    })?;
                }
            }
        }
        offset = end;
    }
    Ok(section)
}

pub fn encode_record(record: &Record, data: &[u8]) -> Result<Vec<u8>> {
    if data == record.data {
        return Ok(record.original.clone());
    }
    require(
        data.len() <= MAX_PART,
        "LIMIT",
        "HWP record exceeds 32 MiB.",
    )?;
    let bits = record.tag | (record.level << 10) | ((data.len().min(EXTENDED_SIZE) as u32) << 20);
    let mut output = bits.to_le_bytes().to_vec();
    if data.len() >= EXTENDED_SIZE {
        output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    }
    output.extend_from_slice(data);
    Ok(output)
}

fn position_width(tag: u32) -> Option<usize> {
    match tag {
        PARA_CHAR_SHAPE => Some(8),
        PARA_RANGE_TAG => Some(12),
        PARA_LINE_SEG => Some(36),
        _ => None,
    }
}

pub fn validate_paragraph(section: &Section, header_index: Option<usize>) -> Result<&Paragraph> {
    let paragraph = header_index
        .and_then(|index| section.paragraphs.get(&index))
        .ok_or_else(|| {
            error(
                "UNSUPPORTED_ELEMENT",
                "Text has no complete paragraph header. Re-inspect a supported HWP 5 paragraph.",
            )
        })?;
    let header = &section_record(section, paragraph.header_index)?.data;
    require(
        header.len() >= 22,
        "UNSUPPORTED_ELEMENT",
        "Incomplete HWP paragraph header; editing is unsafe.",
    )?;
    let count = (read_u32(
        header,
        0,
        "UNSUPPORTED_ELEMENT",
        "Incomplete HWP paragraph header; editing is unsafe.",
    )? & 0x7fffffff) as usize;
    require(
        count == paragraph.text_units,
        "UNSUPPORTED_ELEMENT",
        "HWP paragraph character count differs from text payload.",
    )?;
    let mut counts = BTreeMap::<u32, usize>::new();
    let mut previous_shape_start = 0;
    for index in &paragraph.children {
        let item = section_record(section, *index)?;
        let Some(width) = position_width(item.tag) else {
            continue;
        };
        require(
            item.data.len() % width == 0,
            "UNSUPPORTED_ELEMENT",
            "Incomplete HWP paragraph position record.",
        )?;
        let entries = counts.entry(item.tag).or_default();
        *entries = entries
            .checked_add(item.data.len() / width)
            .ok_or_else(|| {
                error(
                    "UNSUPPORTED_ELEMENT",
                    "HWP paragraph metadata counts differ from child records.",
                )
            })?;
        for entry in item.data.chunks_exact(width) {
            let position = read_u32(
                entry,
                0,
                "UNSUPPORTED_ELEMENT",
                "Incomplete HWP paragraph position record.",
            )? as usize;
            require(
                position <= count,
                "UNSUPPORTED_ELEMENT",
                "HWP paragraph position is outside text.",
            )?;
            if item.tag == PARA_CHAR_SHAPE {
                require(
                    position >= previous_shape_start,
                    "UNSUPPORTED_ELEMENT",
                    "HWP character shape starts are out of order.",
                )?;
                previous_shape_start = position;
            } else if item.tag == PARA_RANGE_TAG {
                let end = read_u32(
                    entry,
                    4,
                    "UNSUPPORTED_ELEMENT",
                    "Incomplete HWP paragraph position record.",
                )? as usize;
                require(
                    position <= end && end <= count,
                    "UNSUPPORTED_ELEMENT",
                    "HWP range tag is outside text.",
                )?;
            }
        }
    }
    for (offset, tag) in [
        (12, PARA_CHAR_SHAPE),
        (14, PARA_RANGE_TAG),
        (16, PARA_LINE_SEG),
    ] {
        let bytes = bytes_at(
            header,
            offset,
            2,
            "UNSUPPORTED_ELEMENT",
            "Incomplete HWP paragraph header; editing is unsafe.",
        )?;
        let actual = u16::from_le_bytes(<[u8; 2]>::try_from(bytes).map_err(|_| {
            error(
                "UNSUPPORTED_ELEMENT",
                "Incomplete HWP paragraph header; editing is unsafe.",
            )
        })?) as usize;
        require(
            actual == counts.get(&tag).copied().unwrap_or(0),
            "UNSUPPORTED_ELEMENT",
            "HWP paragraph metadata counts differ from child records.",
        )?;
    }
    Ok(paragraph)
}

const CONTROL_HINT: &str = "The edit touches a native control. Select one plain text run; leave tabs, fields and paragraph breaks unchanged.";

fn relative_bounds(item: &Record, position: &TextPosition) -> Result<(usize, usize)> {
    let start = position
        .start
        .checked_sub(item.payload_start)
        .ok_or_else(|| error("UNSUPPORTED_ELEMENT", CONTROL_HINT))?;
    let end = position
        .end
        .checked_sub(item.payload_start)
        .ok_or_else(|| error("UNSUPPORTED_ELEMENT", CONTROL_HINT))?;
    require(
        start <= end && end <= item.data.len() && start % 2 == 0 && end % 2 == 0,
        "UNSUPPORTED_ELEMENT",
        CONTROL_HINT,
    )?;
    Ok((start, end))
}

pub fn validate_text_edit(section: &Section, position: &TextPosition) -> Result<()> {
    let paragraph = validate_paragraph(section, position.paragraph_record_index)?;
    let item = section_record(section, position.record)?;
    require(
        paragraph.children.contains(&position.record) && item.tag == PARA_TEXT,
        "UNSUPPORTED_ELEMENT",
        "Select an existing plain text run inside one paragraph.",
    )?;
    let (start, end) = relative_bounds(item, position)?;
    let mut offset = 0;
    while offset < item.data.len() {
        let bytes = bytes_at(
            &item.data,
            offset,
            2,
            "BAD_HWP_TEXT",
            "Truncated HWP control sequence.",
        )?;
        let code = u16::from_le_bytes(
            <[u8; 2]>::try_from(bytes)
                .map_err(|_| error("BAD_HWP_TEXT", "Truncated HWP control sequence."))?,
        );
        let width = if matches!(code, 1..=9 | 11..=12 | 14..=23) {
            16
        } else {
            2
        };
        if code < 32 {
            let control_end = offset
                .checked_add(width)
                .ok_or_else(|| error("BAD_HWP_TEXT", "Truncated HWP control sequence."))?;
            require(
                control_end <= item.data.len(),
                "BAD_HWP_TEXT",
                "Truncated HWP control sequence.",
            )?;
            require(
                end <= offset || start >= control_end,
                "UNSUPPORTED_ELEMENT",
                CONTROL_HINT,
            )?;
            if start == end {
                require(
                    !(offset < start && start < control_end),
                    "UNSUPPORTED_ELEMENT",
                    CONTROL_HINT,
                )?;
            }
        }
        offset = offset
            .checked_add(if code < 32 { width } else { 2 })
            .ok_or_else(|| error("BAD_HWP_TEXT", "Truncated HWP control sequence."))?;
    }
    let base = section
        .text_offsets
        .get(&position.record)
        .copied()
        .ok_or_else(|| {
            error(
                "UNSUPPORTED_ELEMENT",
                "Text range does not match its native paragraph position.",
            )
        })?;
    require(
        Some(position.text_start) == base.checked_add(start / 2)
            && Some(position.text_end) == base.checked_add(end / 2),
        "UNSUPPORTED_ELEMENT",
        "Text range does not match its native paragraph position.",
    )
}

pub fn shift_position(position: usize, start: usize, end: usize, length: usize) -> Result<usize> {
    if position <= start {
        return Ok(position);
    }
    let span = end
        .checked_sub(start)
        .ok_or_else(|| error("LIMIT", "HWP paragraph character count exceeds bounds."))?;
    let shifted = if position >= end {
        position
            .checked_sub(span)
            .and_then(|value| value.checked_add(length))
    } else {
        start.checked_add(length).map(|value| position.min(value))
    };
    shifted.ok_or_else(|| error("LIMIT", "HWP paragraph character count exceeds bounds."))
}

fn changed_data<'a>(changed: &'a BTreeMap<usize, Vec<u8>>, item: &'a Record) -> &'a [u8] {
    changed
        .get(&item.index)
        .map(Vec::as_slice)
        .unwrap_or(&item.data)
}

fn shift_metadata(
    section: &Section,
    paragraph: &Paragraph,
    changed: &mut BTreeMap<usize, Vec<u8>>,
    start: usize,
    end: usize,
    length: usize,
) -> Result<()> {
    let mut boundaries = BTreeMap::new();
    let mut start_counts = BTreeMap::<usize, usize>::new();
    for child_index in &paragraph.children {
        let child = section_record(section, *child_index)?;
        if child.tag != PARA_CHAR_SHAPE {
            continue;
        }
        for entry in changed_data(changed, child).chunks_exact(8) {
            let old_start = read_u32(
                entry,
                0,
                "UNSUPPORTED_ELEMENT",
                "Incomplete HWP paragraph position record.",
            )? as usize;
            boundaries.insert(shift_position(old_start, start, end, length)?, old_start);
            let count = start_counts.entry(old_start).or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| error("LIMIT", "Too many HWP character shapes."))?;
        }
    }
    for child_index in &paragraph.children {
        let child = section_record(section, *child_index)?;
        let Some(width) = position_width(child.tag) else {
            continue;
        };
        let mut updated = Vec::new();
        for data in changed_data(changed, child).chunks_exact(width) {
            let mut entry = data.to_vec();
            let old_start = read_u32(
                &entry,
                0,
                "UNSUPPORTED_ELEMENT",
                "Incomplete HWP paragraph position record.",
            )? as usize;
            let new_start = shift_position(old_start, start, end, length)?;
            let new_start_u32 = u32::try_from(new_start)
                .map_err(|_| error("LIMIT", "HWP paragraph character count exceeds bounds."))?;
            write_bytes(&mut entry, 0, &new_start_u32.to_le_bytes())?;
            if child.tag == PARA_CHAR_SHAPE {
                // Later original starts carry the style of surviving content.
                // Original duplicate groups remain, including across records.
                if boundaries.get(&new_start).copied() != Some(old_start)
                    && start_counts.get(&old_start).copied() == Some(1)
                {
                    continue;
                }
            } else if child.tag == PARA_RANGE_TAG {
                let old_end = read_u32(
                    &entry,
                    4,
                    "UNSUPPORTED_ELEMENT",
                    "Incomplete HWP paragraph position record.",
                )? as usize;
                let new_end = u32::try_from(shift_position(old_end, start, end, length)?)
                    .map_err(|_| error("LIMIT", "HWP paragraph character count exceeds bounds."))?;
                write_bytes(&mut entry, 4, &new_end.to_le_bytes())?;
            }
            updated.extend_from_slice(&entry);
        }
        changed.insert(*child_index, updated);
    }
    Ok(())
}

pub fn rewrite_section(section: &Section, edits: &[(TextPosition, Vec<u8>)]) -> Result<Vec<u8>> {
    rewrite_section_with_limit(section, edits, MAX_PART)
}

/// A bounded rewrite; a smaller limit supports allocation-bound regression tests.
pub fn rewrite_section_with_limit(
    section: &Section,
    edits: &[(TextPosition, Vec<u8>)],
    max_part: usize,
) -> Result<Vec<u8>> {
    // Test budgets may narrow the hard native part limit, never increase it.
    let max_part = max_part.min(MAX_PART);
    let mut changed = BTreeMap::<usize, Vec<u8>>::new();
    let mut section_size = section.records.iter().try_fold(0usize, |size, record| {
        size.checked_add(record.original.len())
            .ok_or_else(|| error("LIMIT", "Edited HWP section exceeds 32 MiB."))
    })?;
    let mut by_paragraph = BTreeMap::<usize, Vec<&(TextPosition, Vec<u8>)>>::new();
    for edit in edits {
        validate_text_edit(section, &edit.0)?;
        let header = edit.0.paragraph_record_index.ok_or_else(|| {
            error(
                "UNSUPPORTED_ELEMENT",
                "Text has no complete paragraph header. Re-inspect a supported HWP 5 paragraph.",
            )
        })?;
        by_paragraph.entry(header).or_default().push(edit);
    }
    for (header_index, mut items) in by_paragraph {
        let paragraph = validate_paragraph(section, Some(header_index))?;
        let mut header = section_record(section, header_index)?.data.clone();
        let flag = read_u32(
            &header,
            0,
            "UNSUPPORTED_ELEMENT",
            "Incomplete HWP paragraph header; editing is unsafe.",
        )? & 0x80000000;
        let mut count = paragraph.text_units;
        items.sort_by_key(|item| std::cmp::Reverse(item.0.text_start));
        for (position, replacement) in items {
            let item = section_record(section, position.record)?;
            let (start, end) = relative_bounds(item, position)?;
            let data = changed_data(&changed, item);
            let new_size = data
                .len()
                .checked_sub(end - start)
                .and_then(|value| value.checked_add(replacement.len()))
                .ok_or_else(|| error("LIMIT", "Edited HWP record or section exceeds 32 MiB."))?;
            section_size = section_size
                .checked_sub(data.len())
                .and_then(|value| value.checked_add(new_size))
                .and_then(|value| {
                    value.checked_sub(if data.len() >= EXTENDED_SIZE { 8 } else { 4 })
                })
                .and_then(|value| value.checked_add(if new_size >= EXTENDED_SIZE { 8 } else { 4 }))
                .ok_or_else(|| error("LIMIT", "Edited HWP record or section exceeds 32 MiB."))?;
            require(
                new_size <= max_part && section_size <= max_part,
                "LIMIT",
                "Edited HWP record or section exceeds 32 MiB.",
            )?;
            let mut updated = Vec::with_capacity(new_size);
            updated.extend_from_slice(
                data.get(..start)
                    .ok_or_else(|| error("UNSUPPORTED_ELEMENT", CONTROL_HINT))?,
            );
            updated.extend_from_slice(replacement);
            updated.extend_from_slice(
                data.get(end..)
                    .ok_or_else(|| error("UNSUPPORTED_ELEMENT", CONTROL_HINT))?,
            );
            changed.insert(position.record, updated);
            let length = replacement.len() / 2;
            let removed = position
                .text_end
                .checked_sub(position.text_start)
                .ok_or_else(|| error("LIMIT", "HWP paragraph character count exceeds bounds."))?;
            count = count
                .checked_sub(removed)
                .and_then(|value| value.checked_add(length))
                .ok_or_else(|| error("LIMIT", "HWP paragraph character count exceeds bounds."))?;
            require(
                count <= 0x7fffffff,
                "LIMIT",
                "HWP paragraph character count exceeds bounds.",
            )?;
            if length != removed {
                shift_metadata(
                    section,
                    paragraph,
                    &mut changed,
                    position.text_start,
                    position.text_end,
                    length,
                )?;
            }
        }
        let shape_count = paragraph.children.iter().try_fold(0usize, |count, index| {
            let child = section_record(section, *index)?;
            let added = if child.tag == PARA_CHAR_SHAPE {
                changed_data(&changed, child).len() / 8
            } else {
                0
            };
            count
                .checked_add(added)
                .ok_or_else(|| error("LIMIT", "Too many HWP character shapes."))
        })?;
        require(
            shape_count <= 0xffff,
            "LIMIT",
            "Too many HWP character shapes.",
        )?;
        write_bytes(&mut header, 0, &(flag | count as u32).to_le_bytes())?;
        write_bytes(&mut header, 12, &(shape_count as u16).to_le_bytes())?;
        changed.insert(header_index, header);
    }
    let size = section.records.iter().try_fold(0usize, |size, item| {
        let data = changed_data(&changed, item);
        let length = if data == item.data {
            item.original.len()
        } else {
            data.len()
                .checked_add(if data.len() >= EXTENDED_SIZE { 8 } else { 4 })
                .ok_or_else(|| error("LIMIT", "Edited HWP section exceeds 32 MiB."))?
        };
        size.checked_add(length)
            .ok_or_else(|| error("LIMIT", "Edited HWP section exceeds 32 MiB."))
    })?;
    require(
        size <= max_part,
        "LIMIT",
        "Edited HWP section exceeds 32 MiB.",
    )?;
    let mut output = Vec::with_capacity(size);
    for item in &section.records {
        output.extend_from_slice(&encode_record(item, changed_data(&changed, item))?);
    }
    Ok(output)
}
