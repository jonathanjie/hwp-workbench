//! ZIP reads bounded by actual inflated bytes and preservation-oriented writes.
use crate::errors::{require, Result, WorkbenchError, MAX_FILE};
use flate2::{Crc, Decompress, FlushDecompress, Status};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read, Write};
use zip::{CompressionMethod, ZipArchive};

/// Bound both compressed input and each inflation output allocation to 64 KiB.
pub const ZIP_CHUNK: usize = 64 * 1024;
/// Bound the native package member inventory before parsing its payloads.
pub const MAX_MEMBERS: usize = 2048;

fn bad_zip(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("BAD_ZIP", error.to_string())
}

fn field16(bytes: &[u8], at: usize) -> Result<u16> {
    let value = bytes
        .get(at..at.saturating_add(2))
        .ok_or_else(|| bad_zip("Truncated ZIP header."))?;
    Ok(u16::from_le_bytes(value.try_into().map_err(bad_zip)?))
}

/// A package retaining its original bytes and ordered ZIP metadata for later edits.
pub struct ZipPackage<'a> {
    original: &'a [u8],
    archive: ZipArchive<Cursor<Cow<'a, [u8]>>>,
    layout: ArchiveLayout,
}
impl<'a> ZipPackage<'a> {
    /// Open a bounded archive and validate local names and nonoverlapping members.
    pub fn new(original: &'a [u8]) -> Result<Self> {
        require(
            original.len() <= MAX_FILE,
            "LIMIT",
            "File must be between 1 byte and 64 MiB.",
        )?;
        let mut layout = ArchiveLayout::parse(original)?;
        let archive = metadata_archive(original, &layout)?;
        require(
            archive.len() == layout.entries.len(),
            "BAD_ZIP",
            "ZIP inventory differs from its directory.",
        )?;
        let mut package = Self {
            original,
            archive,
            layout: ArchiveLayout::empty(),
        };
        let mut ranges = vec![];
        for (index, entry) in layout.entries.iter_mut().enumerate() {
            let member = package.archive.by_index_raw(index).map_err(bad_zip)?;
            let offset = usize::try_from(member.header_start()).map_err(bad_zip)?;
            let header = slice(original, offset, 30)?;
            require(
                header.starts_with(b"PK\x03\x04"),
                "BAD_ZIP",
                "Bad magic number for file header.",
            )?;
            let name_size = usize::from(field16(header, 26)?);
            let extra_size = usize::from(field16(header, 28)?);
            let name_start = offset + 30;
            let data_start = name_start + name_size + extra_size;
            require(
                slice(original, name_start, name_size)?
                    == slice(
                        original,
                        entry.central_start + 46,
                        usize::from(field16(original, entry.central_start + 28)?),
                    )?,
                "BAD_ZIP",
                "File name in directory and header differ.",
            )?;
            let size = usize::try_from(member.compressed_size()).map_err(bad_zip)?;
            let data_end = data_start
                .checked_add(size)
                .ok_or_else(|| bad_zip("Truncated ZIP member."))?;
            require(
                data_end <= layout.central_start,
                "BAD_ZIP",
                "Truncated or overlapping ZIP member.",
            )?;
            entry.local_start = offset;
            entry.data_start = data_start;
            entry.data_end = data_end;
            ranges.push((offset, data_end, index));
        }
        ranges.sort_unstable();
        for position in 0..ranges.len() {
            let current = ranges
                .get(position)
                .ok_or_else(|| bad_zip("Invalid ZIP inventory."))?;
            let next_start = ranges
                .get(position + 1)
                .map(|next| next.0)
                .unwrap_or(layout.central_start);
            require(
                current.1 <= next_start,
                "BAD_ZIP",
                "Overlapped entries: possible zip bomb.",
            )?;
            let entry = layout
                .entries
                .get_mut(current.2)
                .ok_or_else(|| bad_zip("Invalid ZIP inventory."))?;
            entry.local_end = next_start;
        }
        package.layout = layout;
        Ok(package)
    }

    /// Return the original package bytes for no-op serialization.
    pub fn original(&self) -> &'a [u8] {
        self.original
    }
    /// Return the number of entries in central-directory order.
    pub fn len(&self) -> usize {
        self.archive.len()
    }
    /// Report whether the archive has no entries.
    pub fn is_empty(&self) -> bool {
        self.archive.is_empty()
    }
    /// Return ordered entry names without decoding their payloads.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        (0..self.len()).filter_map(|i| self.archive.name_for_index(i))
    }
    /// Resolve a member name to its ordered index.
    pub fn index_for_name(&self, name: &str) -> Option<usize> {
        self.archive.index_for_name(name)
    }
    /// Return the archive comment verbatim.
    pub fn comment(&self) -> &[u8] {
        self.archive.comment()
    }

    /// Inflate a complete member, counting actual output against both budgets.
    pub fn read_member(
        &mut self,
        index: usize,
        expanded: usize,
        part_limit: usize,
        total_limit: usize,
    ) -> Result<Vec<u8>> {
        inflate_zip_member(&mut self.archive, index, expanded, part_limit, total_limit)
    }

    /// Copy unchanged compressed data raw and write changed entries in original order.
    pub fn rewrite(&self, changed: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
        for name in changed.keys() {
            require(
                self.archive.index_for_name(name).is_some(),
                "BAD_ZIP",
                "Rewriter cannot add ZIP members.",
            )?;
        }
        if changed.is_empty() {
            return Ok(self.original.to_vec());
        }
        let mut source = self.archive.clone();
        let first_local = self
            .layout
            .entries
            .iter()
            .map(|entry| entry.local_start)
            .min()
            .unwrap_or(self.layout.central_start);
        let mut output = slice(self.original, 0, first_local)?.to_vec();
        let mut central_records = vec![];
        for (index, entry) in self.layout.entries.iter().enumerate() {
            let member = source.by_index_raw(index).map_err(bad_zip)?;
            let mut central = slice(
                self.original,
                entry.central_start,
                entry.central_end - entry.central_start,
            )?
            .to_vec();
            // ZIP offsets exclude a possible self-extracting prefix.
            let local_offset = output
                .len()
                .checked_sub(self.layout.archive_offset)
                .ok_or_else(|| bad_zip("Invalid ZIP archive offset."))?;
            patch_central_offset(&mut central, local_offset)?;
            if let Some(data) = changed.get(member.name()) {
                require(data.len() <= MAX_FILE, "LIMIT", "Output exceeds 64 MiB.")?;
                let compressed = match member.compression() {
                    CompressionMethod::Stored => data.clone(),
                    CompressionMethod::Deflated => {
                        let mut encoder = flate2::write::DeflateEncoder::new(
                            Vec::new(),
                            flate2::Compression::new(6),
                        );
                        encoder.write_all(data).map_err(bad_zip)?;
                        encoder.finish().map_err(bad_zip)?
                    }
                    _ => {
                        return Err(WorkbenchError::new(
                            "UNSAFE_ZIP",
                            "Unsupported ZIP compression.",
                        ))
                    }
                };
                let mut crc = Crc::new();
                crc.update(data);
                let mut local = slice(
                    self.original,
                    entry.local_start,
                    entry.data_start - entry.local_start,
                )?
                .to_vec();
                patch_sizes(&mut local, false, crc.sum(), compressed.len(), data.len())?;
                patch_sizes(&mut central, true, crc.sum(), compressed.len(), data.len())?;
                append_output(&mut output, &local)?;
                append_output(&mut output, &compressed)?;
                let tail = slice(
                    self.original,
                    entry.data_end,
                    entry.local_end - entry.data_end,
                )?;
                if field16(&local, 6)? & 8 != 0 {
                    let mut tail = tail.to_vec();
                    patch_descriptor(
                        &mut tail,
                        &local,
                        (member.crc32(), member.compressed_size(), member.size()),
                        (crc.sum(), compressed.len(), data.len()),
                    )?;
                    append_output(&mut output, &tail)?;
                } else {
                    append_output(&mut output, tail)?;
                }
            } else {
                // Keep the entire original local record, extras and descriptor verbatim.
                append_output(
                    &mut output,
                    slice(
                        self.original,
                        entry.local_start,
                        entry.local_end - entry.local_start,
                    )?,
                )?;
            }
            central_records.push(central);
        }
        let central_start = output.len();
        for record in central_records {
            append_output(&mut output, &record)?;
        }
        let central_end = output.len();
        let mut trailer = slice(
            self.original,
            self.layout.records_end,
            self.original.len() - self.layout.records_end,
        )?
        .to_vec();
        self.layout
            .patch_trailer(&mut trailer, central_start, central_end)?;
        append_output(&mut output, &trailer)?;
        Ok(output)
    }
}

/// Read one raw ZIP member without declared-size truncation by the ZIP decoder.
pub fn read_zip_member(
    package: &mut ZipPackage<'_>,
    index: usize,
    expanded: usize,
    part_limit: usize,
    total_limit: usize,
) -> Result<Vec<u8>> {
    package.read_member(index, expanded, part_limit, total_limit)
}

fn inflate_zip_member<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    index: usize,
    expanded: usize,
    part_limit: usize,
    total_limit: usize,
) -> Result<Vec<u8>> {
    let mut member = archive.by_index_raw(index).map_err(bad_zip)?;
    let declared = usize::try_from(member.size()).map_err(bad_zip)?;
    check_budget(declared, expanded, part_limit, total_limit)?;
    require(
        !member.encrypted(),
        "ENCRYPTED",
        "Encrypted ZIP members are not supported.",
    )?;
    let mut decoder = match member.compression() {
        CompressionMethod::Stored => None,
        CompressionMethod::Deflated => Some(Decompress::new(false)),
        _ => return Err(bad_zip("Unsupported ZIP compression method.")),
    };
    let mut remaining = member.compressed_size();
    let mut output = Vec::new();
    let mut crc = Crc::new();
    let mut input = [0u8; ZIP_CHUNK];
    let mut inflated = [0u8; ZIP_CHUNK];
    let mut complete = false;
    while remaining > 0 {
        let requested = remaining.min(ZIP_CHUNK as u64) as usize;
        let compressed = input
            .get_mut(..requested)
            .ok_or_else(|| bad_zip("Invalid compressed read size."))?;
        let count = member.read(compressed).map_err(bad_zip)?;
        require(count > 0, "BAD_ZIP", "Truncated ZIP member.")?;
        remaining -= count as u64;
        let compressed = compressed
            .get(..count)
            .ok_or_else(|| bad_zip("Invalid compressed read size."))?;
        if let Some(decoder) = &mut decoder {
            require(!complete, "BAD_ZIP", "Data follows the ZIP deflate stream.")?;
            let mut offset = 0;
            loop {
                let before_in = decoder.total_in();
                let before_out = decoder.total_out();
                let pending = compressed
                    .get(offset..)
                    .ok_or_else(|| bad_zip("Invalid deflate position."))?;
                let status = decoder
                    .decompress(pending, &mut inflated, FlushDecompress::None)
                    .map_err(bad_zip)?;
                let used = (decoder.total_in() - before_in) as usize;
                let produced = (decoder.total_out() - before_out) as usize;
                offset += used;
                append(
                    &mut output,
                    &mut crc,
                    inflated
                        .get(..produced)
                        .ok_or_else(|| bad_zip("Invalid deflate output size."))?,
                    expanded,
                    part_limit,
                    total_limit,
                )?;
                if status == Status::StreamEnd {
                    complete = true;
                    require(
                        offset == compressed.len() && remaining == 0,
                        "BAD_ZIP",
                        "Data follows the ZIP deflate stream.",
                    )?;
                    break;
                }
                if offset == compressed.len() && produced < ZIP_CHUNK {
                    break;
                }
                require(
                    used > 0 || produced > 0,
                    "BAD_ZIP",
                    "Incomplete ZIP deflate stream.",
                )?;
            }
        } else {
            append(
                &mut output,
                &mut crc,
                compressed,
                expanded,
                part_limit,
                total_limit,
            )?;
        }
    }
    require(
        decoder.is_none() || complete,
        "BAD_ZIP",
        "Incomplete ZIP deflate stream.",
    )?;
    require(
        output.len() == declared,
        "BAD_ZIP",
        "Inflated ZIP length differs from its header.",
    )?;
    require(
        crc.sum() == member.crc32(),
        "BAD_ZIP",
        "ZIP member CRC mismatch.",
    )?;
    Ok(output)
}

fn check_budget(
    actual: usize,
    expanded: usize,
    part_limit: usize,
    total_limit: usize,
) -> Result<()> {
    require(
        actual <= part_limit
            && expanded
                .checked_add(actual)
                .is_some_and(|total| total <= total_limit),
        "LIMIT",
        "Expanded ZIP exceeds limits.",
    )
}

fn append(
    output: &mut Vec<u8>,
    crc: &mut Crc,
    chunk: &[u8],
    expanded: usize,
    part_limit: usize,
    total_limit: usize,
) -> Result<()> {
    let actual = output
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| WorkbenchError::new("LIMIT", "Expanded ZIP exceeds limits."))?;
    check_budget(actual, expanded, part_limit, total_limit)?;
    crc.update(chunk);
    output.extend_from_slice(chunk);
    Ok(())
}

fn slice(bytes: &[u8], at: usize, length: usize) -> Result<&[u8]> {
    let end = at
        .checked_add(length)
        .ok_or_else(|| bad_zip("Truncated ZIP header."))?;
    bytes
        .get(at..end)
        .ok_or_else(|| bad_zip("Truncated ZIP header."))
}
fn field32(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        slice(bytes, at, 4)?.try_into().map_err(bad_zip)?,
    ))
}
fn field64(bytes: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        slice(bytes, at, 8)?.try_into().map_err(bad_zip)?,
    ))
}
fn put_field(bytes: &mut [u8], at: usize, value: &[u8]) -> Result<()> {
    let end = at
        .checked_add(value.len())
        .ok_or_else(|| bad_zip("Truncated ZIP header."))?;
    bytes
        .get_mut(at..end)
        .ok_or_else(|| bad_zip("Truncated ZIP header."))?
        .copy_from_slice(value);
    Ok(())
}
fn put32(bytes: &mut [u8], at: usize, value: usize) -> Result<()> {
    let value = u32::try_from(value).map_err(bad_zip)?;
    put_field(bytes, at, &value.to_le_bytes())
}
fn put64(bytes: &mut [u8], at: usize, value: usize) -> Result<()> {
    put_field(bytes, at, &(value as u64).to_le_bytes())
}
fn append_output(output: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    require(
        output
            .len()
            .checked_add(bytes.len())
            .is_some_and(|length| length <= MAX_FILE),
        "LIMIT",
        "Output exceeds 64 MiB.",
    )?;
    output.extend_from_slice(bytes);
    Ok(())
}

#[derive(Default)]
struct RawEntry {
    central_start: usize,
    central_end: usize,
    local_start: usize,
    data_start: usize,
    data_end: usize,
    local_end: usize,
}
struct ArchiveLayout {
    entries: Vec<RawEntry>,
    archive_offset: usize,
    central_start: usize,
    records_end: usize,
    footer: usize,
    zip64: Option<usize>,
}
impl ArchiveLayout {
    fn empty() -> Self {
        Self {
            entries: vec![],
            archive_offset: 0,
            central_start: 0,
            records_end: 0,
            footer: 0,
            zip64: None,
        }
    }
    fn parse(bytes: &[u8]) -> Result<Self> {
        // A standard end record plus the longest legal ZIP comment is 65,557 bytes.
        let begin = bytes.len().saturating_sub(65_557);
        let footer = (begin..bytes.len().saturating_sub(21))
            .rev()
            .find(|at| {
                slice(bytes, *at, 4).is_ok_and(|signature| signature == b"PK\x05\x06")
                    && field16(bytes, *at + 20)
                        .is_ok_and(|length| *at + 22 + usize::from(length) == bytes.len())
            })
            .ok_or_else(|| bad_zip("File is not a zip file"))?;
        let locator = footer
            .checked_sub(20)
            .filter(|at| slice(bytes, *at, 4).is_ok_and(|signature| signature == b"PK\x06\x07"));
        let zip64 = if let Some(locator) = locator {
            let found = (0..locator.saturating_sub(11)).rev().find(|at| {
                slice(bytes, *at, 4).is_ok_and(|s| s == b"PK\x06\x06")
                    && field64(bytes, *at + 4).is_ok_and(|size| {
                        size >= 44
                            && (*at as u64)
                                .checked_add(12)
                                .and_then(|n| n.checked_add(size))
                                == Some(locator as u64)
                    })
            });
            Some(found.ok_or_else(|| bad_zip("Truncated ZIP64 directory."))?)
        } else {
            None
        };
        let (directory_end, central_size, central_offset) = if let Some(at) = zip64 {
            (
                at,
                usize::try_from(field64(bytes, at + 40)?).map_err(bad_zip)?,
                usize::try_from(field64(bytes, at + 48)?).map_err(bad_zip)?,
            )
        } else {
            (
                footer,
                field32(bytes, footer + 12)? as usize,
                field32(bytes, footer + 16)? as usize,
            )
        };
        let central_start = directory_end
            .checked_sub(central_size)
            .ok_or_else(|| bad_zip("Invalid ZIP central directory size."))?;
        let archive_offset = central_start
            .checked_sub(central_offset)
            .ok_or_else(|| bad_zip("Invalid ZIP archive offset."))?;
        let mut at = central_start;
        let mut entries = vec![];
        let mut names = HashSet::new();
        while at < directory_end && slice(bytes, at, 4)? == b"PK\x01\x02" {
            require(
                entries.len() < MAX_MEMBERS,
                "LIMIT",
                "Too many ZIP members.",
            )?;
            let header = slice(bytes, at, 46)?;
            let name_length = usize::from(field16(header, 28)?);
            let extra_length = usize::from(field16(header, 30)?);
            let comment_length = usize::from(field16(header, 32)?);
            let end = at + 46 + name_length + extra_length + comment_length;
            require(
                end <= directory_end,
                "BAD_ZIP",
                "Truncated ZIP central directory.",
            )?;
            let name = slice(bytes, at + 46, name_length)?;
            require(
                !name.starts_with(b"/")
                    && !name.contains(&b'\\')
                    && !name.contains(&0)
                    && !name.split(|byte| *byte == b'/').any(|part| part == b".."),
                "UNSAFE_ZIP",
                "Unsafe package path.",
            )?;
            require(
                names.insert(name.to_vec()),
                "UNSAFE_ZIP",
                "Duplicate ZIP member.",
            )?;
            require(
                field16(header, 8)? & 1 == 0,
                "ENCRYPTED",
                "Encrypted ZIP members are not supported.",
            )?;
            require(
                matches!(field16(header, 10)?, 0 | 8),
                "UNSAFE_ZIP",
                "Unsupported ZIP compression.",
            )?;
            entries.push(RawEntry {
                central_start: at,
                central_end: end,
                ..RawEntry::default()
            });
            at = end;
        }
        Ok(Self {
            entries,
            archive_offset,
            central_start,
            records_end: at,
            footer,
            zip64,
        })
    }
    fn patch_trailer(&self, trailer: &mut [u8], new_start: usize, new_end: usize) -> Result<()> {
        let footer = self.footer - self.records_end;
        let offset = new_start
            .checked_sub(self.archive_offset)
            .ok_or_else(|| bad_zip("Invalid ZIP archive offset."))?;
        let old_end = self.zip64.unwrap_or(self.footer);
        let tail_size = old_end - self.records_end;
        let central_size = new_end - new_start + tail_size;
        if field32(trailer, footer + 12)? != u32::MAX {
            put32(trailer, footer + 12, central_size)?;
        }
        if field32(trailer, footer + 16)? != u32::MAX {
            put32(trailer, footer + 16, offset)?;
        }
        if let Some(zip64) = self.zip64 {
            let at = zip64 - self.records_end;
            put64(trailer, at + 40, central_size)?;
            put64(trailer, at + 48, offset)?;
            let new_zip64 = new_end + at - self.archive_offset;
            put64(trailer, footer - 20 + 8, new_zip64)?;
        }
        Ok(())
    }
}

fn zip64_extra(header: &[u8], central: bool) -> Result<std::ops::Range<usize>> {
    let (base, name_at, extra_at) = if central { (46, 28, 30) } else { (30, 26, 28) };
    let mut at = base + usize::from(field16(header, name_at)?);
    let end = at + usize::from(field16(header, extra_at)?);
    while at + 4 <= end {
        let id = field16(header, at)?;
        let size = usize::from(field16(header, at + 2)?);
        require(
            at + 4 + size <= end,
            "BAD_ZIP",
            "Invalid ZIP extra field length.",
        )?;
        if id == 1 {
            return Ok(at + 4..at + 4 + size);
        }
        at += 4 + size;
    }
    Err(bad_zip("Missing ZIP64 extra field."))
}
fn patch_sizes(
    header: &mut [u8],
    central: bool,
    crc: u32,
    compressed: usize,
    expanded: usize,
) -> Result<()> {
    let (crc_at, compressed_at, expanded_at) = if central { (16, 20, 24) } else { (14, 18, 22) };
    let big_expanded = field32(header, expanded_at)? == u32::MAX;
    let big_compressed = field32(header, compressed_at)? == u32::MAX;
    put_field(header, crc_at, &crc.to_le_bytes())?;
    if !big_compressed {
        put32(header, compressed_at, compressed)?;
    }
    if !big_expanded {
        put32(header, expanded_at, expanded)?;
    }
    if big_expanded || big_compressed {
        let bounds = zip64_extra(header, central)?;
        let mut at = bounds.start;
        if big_expanded {
            patch_zip64_field(header, at, &bounds, expanded)?;
            at += 8;
        }
        if big_compressed {
            patch_zip64_field(header, at, &bounds, compressed)?;
        }
    }
    Ok(())
}
fn patch_central_offset(central: &mut [u8], offset: usize) -> Result<()> {
    if field32(central, 42)? != u32::MAX {
        return put32(central, 42, offset);
    }
    let bounds = zip64_extra(central, true)?;
    let mut at = bounds.start;
    if field32(central, 24)? == u32::MAX {
        at += 8;
    }
    if field32(central, 20)? == u32::MAX {
        at += 8;
    }
    patch_zip64_field(central, at, &bounds, offset)
}
fn patch_zip64_field(
    header: &mut [u8],
    at: usize,
    bounds: &std::ops::Range<usize>,
    value: usize,
) -> Result<()> {
    require(
        at >= bounds.start && at.checked_add(8).is_some_and(|end| end <= bounds.end),
        "BAD_ZIP",
        "Truncated ZIP64 extra field.",
    )?;
    put64(header, at, value)
}
fn patch_descriptor(
    tail: &mut [u8],
    local: &[u8],
    original: (u32, u64, u64),
    updated: (u32, usize, usize),
) -> Result<()> {
    let wide = field32(local, 18)? == u32::MAX || field32(local, 22)? == u32::MAX;
    let matches = |start| -> bool {
        let sizes = if wide {
            field64(tail, start + 4)
                .ok()
                .zip(field64(tail, start + 12).ok())
        } else {
            field32(tail, start + 4)
                .ok()
                .zip(field32(tail, start + 8).ok())
                .map(|(a, b)| (u64::from(a), u64::from(b)))
        };
        field32(tail, start).ok() == Some(original.0) && sizes == Some((original.1, original.2))
    };
    // An unsigned descriptor's CRC can itself equal the optional signature.
    let start = if tail.starts_with(b"PK\x07\x08") && matches(4) {
        4
    } else if matches(0) {
        0
    } else {
        return Err(bad_zip("Invalid ZIP data descriptor."));
    };
    put_field(tail, start, &updated.0.to_le_bytes())?;
    if wide {
        put64(tail, start + 4, updated.1)?;
        put64(tail, start + 12, updated.2)
    } else {
        put32(tail, start + 4, updated.1)?;
        put32(tail, start + 8, updated.2)
    }
}

fn metadata_archive<'a>(
    original: &'a [u8],
    layout: &ArchiveLayout,
) -> Result<ZipArchive<Cursor<Cow<'a, [u8]>>>> {
    let mut bytes = Cow::Borrowed(original);
    // Python identifies members by the raw header name and ignores Info-ZIP
    // Unicode Path aliases. Prevent zip's alias decoder from changing that key;
    // this shadow header is never serialized or exposed as document content.
    for entry in &layout.entries {
        let name_size = usize::from(field16(original, entry.central_start + 28)?);
        let extra_size = usize::from(field16(original, entry.central_start + 30)?);
        let mut at = entry.central_start + 46 + name_size;
        let end = at + extra_size;
        while at + 4 <= end {
            let id = field16(original, at)?;
            let size = usize::from(field16(original, at + 2)?);
            require(
                at + 4 + size <= end,
                "BAD_ZIP",
                "Invalid ZIP extra field length.",
            )?;
            if id == 0x7075 {
                put_field(bytes.to_mut(), at, &0xf075u16.to_le_bytes())?;
            }
            at += 4 + size;
        }
    }
    ZipArchive::new(Cursor::new(bytes)).map_err(bad_zip)
}
