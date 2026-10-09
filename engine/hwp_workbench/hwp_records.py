"""Bounded HWP 5 records with exact source bytes and paragraph ownership.

Field layouts agree with rhwp parser/body_text.rs and
serializer/{record_writer,body_text}.rs. Unknown records remain opaque.
"""
from dataclasses import dataclass, field
import struct

from .errors import MAX_NODES, MAX_PART, require

PARA_HEADER = 66
PARA_TEXT = 67
PARA_CHAR_SHAPE = 68
PARA_LINE_SEG = 69
PARA_RANGE_TAG = 70
EXTENDED_SIZE = 0xfff


@dataclass(frozen=True)
class Record:
    index: int
    tag: int
    level: int
    data: bytes
    original: bytes
    payload_start: int


@dataclass
class Paragraph:
    header_index: int
    children: list[int] = field(default_factory=list)
    text_units: int = 0


@dataclass
class Section:
    records: list[Record]
    paragraphs: dict[int, Paragraph]
    owners: dict[int, int]
    text_offsets: dict[int, int]


def parse_section(raw):
    records = []
    paragraphs = {}
    owners = {}
    text_offsets = {}
    stack = []
    offset = 0
    while offset < len(raw):
        require(offset + 4 <= len(raw), 'BAD_HWP_RECORD', 'Truncated record header.')
        bits = struct.unpack_from('<I', raw, offset)[0]
        # Header: tag bits 0-9, level bits 10-19, size bits 20-31.
        tag = bits & 0x3ff
        level = (bits >> 10) & 0x3ff
        size = bits >> 20
        header_size = 4
        if size == EXTENDED_SIZE:
            require(offset + 8 <= len(raw), 'BAD_HWP_RECORD', 'Truncated extended record header.')
            size = struct.unpack_from('<I', raw, offset + 4)[0]
            header_size = 8
        start = offset + header_size
        end = start + size
        require(end <= len(raw), 'BAD_HWP_RECORD', 'Record payload exceeds stream bounds.')
        index = len(records)
        records.append(Record(index, tag, level, raw[start:end], raw[offset:end], start))
        require(len(records) <= MAX_NODES, 'LIMIT', 'Too many HWP records.')
        while stack and level <= records[stack[-1]].level:
            stack.pop()
        if tag == PARA_HEADER:
            paragraphs[index] = Paragraph(index)
            stack.append(index)
        elif stack and level == records[stack[-1]].level + 1:
            owner = paragraphs[stack[-1]]
            owner.children.append(index)
            owners[index] = owner.header_index
            if tag == PARA_TEXT:
                require(size % 2 == 0, 'BAD_HWP_TEXT', 'Misaligned UTF-16 text record.')
                text_offsets[index] = owner.text_units
                owner.text_units += size // 2
        offset = end
    return Section(records, paragraphs, owners, text_offsets)


def encode_record(record, data):
    if data == record.data:
        return record.original
    require(len(data) <= MAX_PART, 'LIMIT', 'HWP record exceeds 32 MiB.')
    size = len(data)
    header_size = min(size, EXTENDED_SIZE)
    bits = record.tag | (record.level << 10) | (header_size << 20)
    header = struct.pack('<I', bits)
    if size >= EXTENDED_SIZE:
        header += struct.pack('<I', size)
    return header + data


def validate_paragraph(section, header_index):
    require(header_index in section.paragraphs, 'UNSUPPORTED_ELEMENT',
            'Text has no complete paragraph header. Re-inspect a supported HWP 5 paragraph.')
    paragraph = section.paragraphs[header_index]
    header = section.records[header_index].data
    # PARA_HEADER: nChars(u32, bit 31 is a flag), control mask(u32),
    # paragraph shape(u16), style(u8), break(u8), three counts(u16), instance(u32).
    require(len(header) >= 22, 'UNSUPPORTED_ELEMENT', 'Incomplete HWP paragraph header; editing is unsafe.')
    count = struct.unpack_from('<I', header)[0] & 0x7fffffff
    require(count == paragraph.text_units, 'UNSUPPORTED_ELEMENT', 'HWP paragraph character count differs from text payload.')
    widths = {PARA_CHAR_SHAPE: 8, PARA_RANGE_TAG: 12, PARA_LINE_SEG: 36}
    counts = {tag: 0 for tag in widths}
    previous_shape_start = 0
    for index in paragraph.children:
        item = section.records[index]
        if item.tag not in widths:
            continue
        width = widths[item.tag]
        require(len(item.data) % width == 0, 'UNSUPPORTED_ELEMENT', 'Incomplete HWP paragraph position record.')
        counts[item.tag] += len(item.data) // width
        for offset in range(0, len(item.data), width):
            position = struct.unpack_from('<I', item.data, offset)[0]
            require(position <= count, 'UNSUPPORTED_ELEMENT', 'HWP paragraph position is outside text.')
            if item.tag == PARA_CHAR_SHAPE:
                require(position >= previous_shape_start, 'UNSUPPORTED_ELEMENT', 'HWP character shape starts are out of order.')
                previous_shape_start = position
            elif item.tag == PARA_RANGE_TAG:
                end = struct.unpack_from('<I', item.data, offset + 4)[0]
                require(position <= end <= count, 'UNSUPPORTED_ELEMENT', 'HWP range tag is outside text.')
    actual = (counts[PARA_CHAR_SHAPE], counts[PARA_RANGE_TAG], counts[PARA_LINE_SEG])
    require(struct.unpack_from('<3H', header, 12) == actual, 'UNSUPPORTED_ELEMENT',
            'HWP paragraph metadata counts differ from child records.')
    return paragraph


def validate_text_edit(section, position):
    paragraph = validate_paragraph(section, position['paragraphRecordIndex'])
    index = position['record']
    require(index in paragraph.children and section.records[index].tag == PARA_TEXT,
            'UNSUPPORTED_ELEMENT', 'Select an existing plain text run inside one paragraph.')
    item = section.records[index]
    start = position['start'] - item.payload_start
    end = position['end'] - item.payload_start
    hint = 'The edit touches a native control. Select one plain text run; leave tabs, fields and paragraph breaks unchanged.'
    require(0 <= start <= end <= len(item.data) and start % 2 == 0 and end % 2 == 0,
            'UNSUPPORTED_ELEMENT', hint)
    # Native controls are 1 or 8 UTF-16 units; no unit of a slot is editable.
    offset = 0
    while offset < len(item.data):
        code = struct.unpack_from('<H', item.data, offset)[0]
        width = 16 if (1 <= code <= 9 or 11 <= code <= 12 or 14 <= code <= 23) else 2
        if code < 32:
            require(offset + width <= len(item.data), 'BAD_HWP_TEXT', 'Truncated HWP control sequence.')
            require(end <= offset or start >= offset + width, 'UNSUPPORTED_ELEMENT', hint)
            if start == end:
                require(not offset < start < offset + width, 'UNSUPPORTED_ELEMENT', hint)
            offset += width
        else:
            offset += 2
    base = section.text_offsets[index]
    require(position['textStart'] == base + start // 2 and position['textEnd'] == base + end // 2,
            'UNSUPPORTED_ELEMENT', 'Text range does not match its native paragraph position.')


def shift_position(position, start, end, length):
    if position <= start:
        return position
    if position >= end:
        return position + length - (end - start)
    return min(position, start + length)


def rewrite_section(section, edits):
    """Apply original-address edits backwards and update only their direct records."""
    changed = {}
    section_size = sum(len(item.original) for item in section.records)
    by_paragraph = {}
    for position, replacement in edits:
        validate_text_edit(section, position)
        by_paragraph.setdefault(position['paragraphRecordIndex'], []).append((position, replacement))
    for header_index, items in by_paragraph.items():
        paragraph = section.paragraphs[header_index]
        header = bytearray(section.records[header_index].data)
        count_flag = struct.unpack_from('<I', header)[0] & 0x80000000
        count = paragraph.text_units
        for position, replacement in sorted(items, key=lambda item: item[0]['textStart'], reverse=True):
            index = position['record']
            item = section.records[index]
            start = position['start'] - item.payload_start
            end = position['end'] - item.payload_start
            data = changed.get(index, item.data)
            new_size = len(data) + len(replacement) - (end - start)
            section_size += new_size - len(data)
            section_size += (8 if new_size >= EXTENDED_SIZE else 4) - (8 if len(data) >= EXTENDED_SIZE else 4)
            require(new_size <= MAX_PART and section_size <= MAX_PART, 'LIMIT',
                    'Edited HWP record or section exceeds 32 MiB.')
            changed[index] = data[:start] + replacement + data[end:]
            text_start = position['textStart']
            text_end = position['textEnd']
            length = len(replacement) // 2
            delta = length - (text_end - text_start)
            count += delta
            require(0 <= count <= 0x7fffffff, 'LIMIT', 'HWP paragraph character count exceeds bounds.')
            if delta == 0:
                continue
            previous_shape_start = None
            previous_original_start = None
            for child_index in paragraph.children:
                child = section.records[child_index]
                if child.tag not in {PARA_CHAR_SHAPE, PARA_RANGE_TAG, PARA_LINE_SEG}:
                    continue
                data = changed.get(child_index, child.data)
                width = {PARA_CHAR_SHAPE: 8, PARA_RANGE_TAG: 12, PARA_LINE_SEG: 36}[child.tag]
                updated = bytearray()
                for offset in range(0, len(data), width):
                    entry = bytearray(data[offset:offset + width])
                    old_start = struct.unpack_from('<I', entry)[0]
                    new_start = shift_position(old_start, text_start, text_end, length)
                    struct.pack_into('<I', entry, 0, new_start)
                    if child.tag == PARA_CHAR_SHAPE:
                        # Entries stay ordered. Keep pre-existing duplicate starts.
                        # At a newly collapsed boundary, retain the first shape,
                        # which applied at the earlier position.
                        if new_start == previous_shape_start and old_start != previous_original_start:
                            continue
                        previous_shape_start = new_start
                        previous_original_start = old_start
                    elif child.tag == PARA_RANGE_TAG:
                        old_end = struct.unpack_from('<I', entry, 4)[0]
                        new_end = shift_position(old_end, text_start, text_end, length)
                        struct.pack_into('<I', entry, 4, new_end)
                    updated.extend(entry)
                changed[child_index] = bytes(updated)
        shape_count = sum(len(changed.get(index, section.records[index].data)) // 8
                          for index in paragraph.children if section.records[index].tag == PARA_CHAR_SHAPE)
        require(shape_count <= 0xffff, 'LIMIT', 'Too many HWP character shapes.')
        struct.pack_into('<I', header, 0, count_flag | count)
        struct.pack_into('<H', header, 12, shape_count)
        changed[header_index] = bytes(header)
    size = 0
    for item in section.records:
        data = changed.get(item.index, item.data)
        if data == item.data:
            size += len(item.original)
        else:
            size += len(data) + (8 if len(data) >= EXTENDED_SIZE else 4)
    require(size <= MAX_PART, 'LIMIT', 'Edited HWP section exceeds 32 MiB.')
    return b''.join(encode_record(item, changed.get(item.index, item.data)) for item in section.records)
