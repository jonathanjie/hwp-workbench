//! Bounded CFB v3/v4 reader and same-directory stream rewriter.
use crate::errors::{require, Result, WorkbenchError, MAX_EXPANDED, MAX_FILE};
use std::collections::{BTreeMap, HashSet};

/// Compound-file signature for HWP 5 containers.
pub const MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
/// An unused allocation or directory pointer.
pub const FREE: u32 = 0xffff_ffff;
/// The terminal sector in an allocation chain.
pub const END: u32 = 0xffff_fffe;
/// A FAT allocation-table sector marker.
pub const FAT: u32 = 0xffff_fffd;
/// A DIFAT extension sector marker.
pub const DIF: u32 = 0xffff_fffc;
/// Directory entries are bounded independently of the container size.
pub const MAX_DIRECTORY_ENTRIES: usize = 8192;
/// CFB mini-sector size in bytes, fixed by the supported format.
pub const MINI_SECTOR_SIZE: usize = 64;
/// Streams below this size use mini-sector allocation.
pub const MINI_STREAM_CUTOFF: usize = 4096;
const DIRECTORY_ENTRY_SIZE: usize = 128;
const HEADER_FAT_ENTRIES: usize = 109;

fn bad_cfb(message: &str) -> WorkbenchError {
    WorkbenchError::new("BAD_CFB", message)
}
fn bytes_at<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N]> {
    let end = at
        .checked_add(N)
        .ok_or_else(|| bad_cfb("Truncated CFB field."))?;
    bytes
        .get(at..end)
        .ok_or_else(|| bad_cfb("Truncated CFB field."))?
        .try_into()
        .map_err(|_| bad_cfb("Truncated CFB field."))
}
/// Read a bounded little-endian 16-bit native field.
pub fn u16(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes_at(bytes, at)?))
}
/// Read a bounded little-endian 32-bit native field.
pub fn u32(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes_at(bytes, at)?))
}
/// Read a bounded little-endian 64-bit native field.
pub fn u64(bytes: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(bytes_at(bytes, at)?))
}
fn put(bytes: &mut [u8], at: usize, value: &[u8]) -> Result<()> {
    let end = at
        .checked_add(value.len())
        .ok_or_else(|| bad_cfb("Truncated CFB field."))?;
    bytes
        .get_mut(at..end)
        .ok_or_else(|| bad_cfb("Truncated CFB field."))?
        .copy_from_slice(value);
    Ok(())
}
/// Write a bounded little-endian 32-bit native field.
pub fn p32(bytes: &mut [u8], at: usize, value: u32) -> Result<()> {
    put(bytes, at, &value.to_le_bytes())
}
/// Write a bounded little-endian 64-bit native field.
pub fn p64(bytes: &mut [u8], at: usize, value: u64) -> Result<()> {
    put(bytes, at, &value.to_le_bytes())
}
/// Decode an aligned allocation table without unchecked field reads.
pub fn words(bytes: &[u8]) -> Result<Vec<u32>> {
    require(
        bytes.len() % 4 == 0,
        "BAD_CFB",
        "Misaligned allocation table.",
    )?;
    bytes.chunks_exact(4).map(|chunk| u32(chunk, 0)).collect()
}

/// A directory entry with retained class IDs, flags, topology and timestamps.
#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub raw: [u8; DIRECTORY_ENTRY_SIZE],
    pub name: String,
    pub kind: u8,
    pub left: u32,
    pub right: u32,
    pub child: u32,
    pub start: u32,
    pub size: u64,
}
impl DirectoryEntry {
    fn parse(raw: &[u8]) -> Result<Self> {
        let raw: [u8; DIRECTORY_ENTRY_SIZE] = raw
            .try_into()
            .map_err(|_| bad_cfb("Truncated directory."))?;
        // Fixed-size directory fields are checked even for inactive entries.
        let kind = *raw.get(66).ok_or_else(|| bad_cfb("Truncated directory."))?;
        let length = usize::from(u16(&raw, 64)?);
        require(
            matches!(kind, 0 | 1 | 2 | 5),
            "BAD_CFB",
            "Unknown directory entry kind.",
        )?;
        require(
            kind == 0 || ((2..=64).contains(&length) && length % 2 == 0),
            "BAD_CFB",
            "Invalid directory name length.",
        )?;
        let name = if kind == 0 {
            String::new()
        } else {
            let name_bytes = raw
                .get(..length.saturating_sub(2))
                .ok_or_else(|| bad_cfb("Invalid directory name length."))?;
            let encoded: Result<Vec<u16>> = name_bytes
                .chunks_exact(2)
                .map(|chunk| u16(chunk, 0))
                .collect();
            String::from_utf16(&encoded?)
                .map_err(|_| bad_cfb("Invalid directory name encoding."))?
        };
        Ok(Self {
            name,
            kind,
            left: u32(&raw, 68)?,
            right: u32(&raw, 72)?,
            child: u32(&raw, 76)?,
            start: u32(&raw, 116)?,
            size: u64(&raw, 120)?,
            raw,
        })
    }
}

/// A native compound file retaining original bytes and every opaque stream.
#[derive(Debug, Clone)]
pub struct CompoundFile {
    original: Vec<u8>,
    pub major: u16,
    pub sector_size: usize,
    sector_count: usize,
    fat: Vec<u32>,
    directory: Vec<u8>,
    pub entries: Vec<DirectoryEntry>,
    mini: Vec<u8>,
    minifat: Vec<u32>,
    pub paths: BTreeMap<String, usize>,
    path_order: Vec<String>,
    pub streams: BTreeMap<String, Vec<u8>>,
}
impl CompoundFile {
    /// Parse CFB metadata, bounded sector chains and ordered opaque streams.
    pub fn new(data: &[u8]) -> Result<Self> {
        require(
            (512..=MAX_FILE).contains(&data.len()) && data.starts_with(&MAGIC),
            "BAD_CFB",
            "Invalid CFB signature or size.",
        )?;
        require(
            u16(data, 28)? == 0xfffe,
            "BAD_CFB",
            "Unsupported byte order.",
        )?;
        let major = u16(data, 26)?;
        let shift = u16(data, 30)?;
        require(
            matches!((major, shift), (3, 9) | (4, 12)),
            "BAD_CFB",
            "Unsupported CFB version/sector size.",
        )?;
        require(
            u16(data, 32)? == 6 && u32(data, 56)? == MINI_STREAM_CUTOFF as u32,
            "BAD_CFB",
            "Unsupported mini stream parameters.",
        )?;
        let sector_size = 1usize << shift;
        require(
            data.len() % sector_size == 0,
            "BAD_CFB",
            "Truncated CFB sector.",
        )?;
        let sector_count = data.len() / sector_size - 1;
        require(sector_count > 0, "BAD_CFB", "CFB has no sectors.")?;
        let mut file = Self {
            original: data.to_vec(),
            major,
            sector_size,
            sector_count,
            fat: vec![],
            directory: vec![],
            entries: vec![],
            mini: vec![],
            minifat: vec![],
            paths: BTreeMap::new(),
            path_order: vec![],
            streams: BTreeMap::new(),
        };
        let mut fat_ids: Vec<u32> = words(
            data.get(76..512)
                .ok_or_else(|| bad_cfb("Truncated CFB header."))?,
        )?
        .into_iter()
        .filter(|id| *id != FREE)
        .collect();
        let mut difat = u32(data, 68)?;
        let difat_count = u32(data, 72)? as usize;
        require(
            difat_count <= sector_count,
            "BAD_CFB",
            "DIFAT count exceeds file bounds.",
        )?;
        let mut seen = HashSet::new();
        for _ in 0..difat_count {
            require(seen.insert(difat), "BAD_CFB", "Cyclic DIFAT.")?;
            let mut block = words(file.sector(difat)?)?;
            difat = block
                .pop()
                .ok_or_else(|| bad_cfb("Misaligned allocation table."))?;
            fat_ids.extend(block.into_iter().filter(|id| *id != FREE));
            // Do not allocate a forged DIFAT inventory larger than the source.
            require(
                fat_ids.len() <= sector_count,
                "BAD_CFB",
                "Invalid FAT count or duplicate FAT sector.",
            )?;
        }
        let fat_count = u32(data, 44)? as usize;
        require(
            fat_count <= sector_count
                && fat_ids.len() == fat_count
                && fat_ids.iter().collect::<HashSet<_>>().len() == fat_count,
            "BAD_CFB",
            "Invalid FAT count or duplicate FAT sector.",
        )?;
        for id in fat_ids {
            file.fat.extend(words(file.sector(id)?)?);
        }
        file.directory = file.read_chain(u32(data, 48)?, &file.fat, None, false)?;
        require(
            file.directory.len() % DIRECTORY_ENTRY_SIZE == 0,
            "BAD_CFB",
            "Truncated directory.",
        )?;
        require(
            file.directory.len() / DIRECTORY_ENTRY_SIZE <= MAX_DIRECTORY_ENTRIES,
            "LIMIT",
            "Too many CFB directory entries.",
        )?;
        file.entries = file
            .directory
            .chunks_exact(DIRECTORY_ENTRY_SIZE)
            .map(DirectoryEntry::parse)
            .collect::<Result<_>>()?;
        let root = file
            .entries
            .first()
            .ok_or_else(|| bad_cfb("Missing CFB root."))?;
        require(root.kind == 5, "BAD_CFB", "Missing CFB root.")?;
        require(
            root.size <= MAX_EXPANDED as u64,
            "BAD_CFB",
            "Mini stream too large.",
        )?;
        let root_start = root.start;
        let root_size = root.size;
        let root_child = root.child;
        file.mini = file.read_chain(root_start, &file.fat, Some(root_size), false)?;
        let mini_count = u32(data, 64)? as usize;
        require(mini_count <= sector_count, "BAD_CFB", "MiniFAT too large.")?;
        if mini_count > 0 {
            file.minifat = words(&file.read_chain(
                u32(data, 60)?,
                &file.fat,
                Some((mini_count * sector_size) as u64),
                false,
            )?)?;
        }
        let mut walked = HashSet::new();
        let mut path_keys = HashSet::new();
        let mut todo = vec![(root_child, String::new())];
        while let Some((index, prefix)) = todo.pop() {
            if index == FREE {
                continue;
            }
            require(
                walked.insert(index),
                "BAD_CFB",
                "Invalid or cyclic directory tree.",
            )?;
            let entry = file
                .entries
                .get(index as usize)
                .ok_or_else(|| bad_cfb("Invalid or cyclic directory tree."))?;
            require(
                matches!(entry.kind, 1 | 2),
                "BAD_CFB",
                "Invalid directory member.",
            )?;
            require(
                !entry.name.contains('/') && !matches!(entry.name.as_str(), "." | ".." | ""),
                "BAD_CFB",
                "Unsafe directory name.",
            )?;
            let path = prefix.clone() + &entry.name;
            require(
                path_keys.insert(crate::casefold::casefold(&path)),
                "BAD_CFB",
                "Duplicate stream name.",
            )?;
            file.paths.insert(path.clone(), index as usize);
            file.path_order.push(path.clone());
            todo.push((entry.right, prefix.clone()));
            todo.push((entry.left, prefix));
            if entry.kind == 1 {
                todo.push((entry.child, path + "/"));
            }
        }
        let mut total = 0u64;
        for path in &file.path_order {
            let index = file
                .paths
                .get(path)
                .copied()
                .ok_or_else(|| bad_cfb("Invalid directory member."))?;
            let entry = file
                .entries
                .get(index)
                .ok_or_else(|| bad_cfb("Invalid directory member."))?;
            if entry.kind != 2 {
                continue;
            }
            total = total
                .checked_add(entry.size)
                .ok_or_else(|| bad_cfb("Expanded streams exceed limit."))?;
            require(
                total <= MAX_EXPANDED as u64,
                "BAD_CFB",
                "Expanded streams exceed limit.",
            )?;
            let mini = entry.size < MINI_STREAM_CUTOFF as u64;
            let table = if mini { &file.minifat } else { &file.fat };
            let bytes = file.read_chain(entry.start, table, Some(entry.size), mini)?;
            file.streams.insert(path.clone(), bytes);
        }
        Ok(file)
    }

    /// Return the untouched compound-file bytes for no-op serialization.
    pub fn original(&self) -> &[u8] {
        &self.original
    }
    /// Iterate streams in the Python directory traversal order, not lexical order.
    pub fn ordered_streams(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.path_order.iter().filter_map(|path| {
            self.streams
                .get(path)
                .map(|bytes| (path.as_str(), bytes.as_slice()))
        })
    }
    /// Return a checked full-sized sector slice from the original source.
    pub fn sector(&self, id: u32) -> Result<&[u8]> {
        let id = id as usize;
        require(
            id < self.sector_count,
            "BAD_CFB",
            "Sector pointer is out of bounds.",
        )?;
        self.original
            .get((id + 1) * self.sector_size..(id + 2) * self.sector_size)
            .ok_or_else(|| bad_cfb("Sector pointer is out of bounds."))
    }
    /// Read a bounded regular or mini-sector chain and check its declared size.
    pub fn read_chain(
        &self,
        start: u32,
        table: &[u32],
        size: Option<u64>,
        mini: bool,
    ) -> Result<Vec<u8>> {
        if size == Some(0) {
            return Ok(vec![]);
        }
        let mut seen = HashSet::new();
        let mut id = start;
        let unit = if mini {
            MINI_SECTOR_SIZE
        } else {
            self.sector_size
        };
        let limit = if mini {
            self.mini.len() / MINI_SECTOR_SIZE
        } else {
            self.sector_count
        };
        let mut output = vec![];
        while id != END {
            let position = id as usize;
            require(
                seen.insert(id) && position < table.len() && position < limit,
                "BAD_CFB",
                "Invalid or cyclic sector chain.",
            )?;
            require(
                output.len() + unit <= MAX_EXPANDED,
                "BAD_CFB",
                "Sector chain exceeds limit.",
            )?;
            let block = if mini {
                self.mini
                    .get(position * unit..(position + 1) * unit)
                    .ok_or_else(|| bad_cfb("Invalid or cyclic sector chain."))?
            } else {
                self.sector(id)?
            };
            output.extend_from_slice(block);
            id = *table
                .get(position)
                .ok_or_else(|| bad_cfb("Invalid or cyclic sector chain."))?;
        }
        if let Some(size) = size {
            require(
                size <= output.len() as u64
                    && (output.len() as u64) < size.saturating_add(unit as u64),
                "BAD_CFB",
                "Stream size does not match its chain.",
            )?;
            output.truncate(size as usize);
        }
        Ok(output)
    }

    /// Rewrite only existing streams, retain directory metadata and verify every payload.
    pub fn rewrite(&self, changed: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
        for path in changed.keys() {
            require(
                self.streams.contains_key(path),
                "BAD_CFB",
                "Rewriter cannot add streams.",
            )?;
        }
        let changed: BTreeMap<_, _> = changed
            .iter()
            .filter(|(name, bytes)| self.streams.get(*name) != Some(*bytes))
            .collect();
        if changed.is_empty() {
            return Ok(self.original.clone());
        }
        let mut directory = self.directory.clone();
        let mut allocation = SectorBuilder::new(self.sector_size);
        let mut minidata = vec![];
        let mut minialloc = vec![];
        let mut paths: Vec<_> = self.paths.iter().collect();
        paths.sort_unstable_by_key(|(_, index)| **index);
        for (path, index) in paths {
            let Some(original) = self.streams.get(path) else {
                continue;
            };
            let payload = changed.get(path).copied().unwrap_or(original);
            require(
                payload.len() <= MAX_EXPANDED,
                "LIMIT",
                "Stream exceeds limit.",
            )?;
            let start = if !payload.is_empty() && payload.len() < MINI_STREAM_CUTOFF {
                let start = minialloc.len();
                let count = payload.len().div_ceil(MINI_SECTOR_SIZE);
                let size = minidata.len() + count * MINI_SECTOR_SIZE;
                minidata.extend_from_slice(payload);
                minidata.resize(size, 0);
                for offset in 0..count {
                    minialloc.push(if offset + 1 < count {
                        (start + offset + 1) as u32
                    } else {
                        END
                    });
                }
                start as u32
            } else {
                allocation.allocate(payload)?
            };
            p32(&mut directory, index * DIRECTORY_ENTRY_SIZE + 116, start)?;
            p64(
                &mut directory,
                index * DIRECTORY_ENTRY_SIZE + 120,
                payload.len() as u64,
            )?;
        }
        let mini_stream_start = allocation.allocate(&minidata)?;
        p32(&mut directory, 116, mini_stream_start)?;
        p64(&mut directory, 120, minidata.len() as u64)?;
        let (mini_start, mini_count) = if minialloc.is_empty() {
            (END, 0)
        } else {
            let mut bytes = encode_words(&minialloc);
            bytes.resize(
                bytes.len().div_ceil(self.sector_size) * self.sector_size,
                0xff,
            );
            let count = bytes.len() / self.sector_size;
            (allocation.allocate(&bytes)?, count)
        };
        let directory_start = allocation.allocate(&directory)?;
        let per_fat = self.sector_size / 4;
        let (mut fat_count, mut dif_count) = (0, 0);
        loop {
            let next_fat = (allocation.sectors.len() + fat_count + dif_count).div_ceil(per_fat);
            let next_dif = next_fat
                .saturating_sub(HEADER_FAT_ENTRIES)
                .div_ceil(per_fat - 1);
            if (next_fat, next_dif) == (fat_count, dif_count) {
                break;
            }
            (fat_count, dif_count) = (next_fat, next_dif);
        }
        require(
            (1 + allocation.sectors.len() + fat_count + dif_count) * self.sector_size <= MAX_FILE,
            "LIMIT",
            "Output file exceeds limit.",
        )?;
        let fat_start = allocation.sectors.len();
        let fat_ids: Vec<u32> = (fat_start..fat_start + fat_count)
            .map(|id| id as u32)
            .collect();
        let dif_ids: Vec<u32> = (fat_start + fat_count..fat_start + fat_count + dif_count)
            .map(|id| id as u32)
            .collect();
        let mut table = allocation.table;
        table.extend(std::iter::repeat_n(FAT, fat_count));
        table.extend(std::iter::repeat_n(DIF, dif_count));
        table.resize(fat_count * per_fat, FREE);
        for block in table.chunks_exact(per_fat) {
            allocation.sectors.push(encode_words(block));
        }
        let extra = fat_ids.get(HEADER_FAT_ENTRIES..).unwrap_or(&[]);
        for index in 0..dif_count {
            let start = index * (per_fat - 1);
            let end = (start + per_fat - 1).min(extra.len());
            let mut block = extra
                .get(start..end)
                .ok_or_else(|| bad_cfb("Invalid DIFAT allocation."))?
                .to_vec();
            block.resize(per_fat - 1, FREE);
            block.push(dif_ids.get(index + 1).copied().unwrap_or(END));
            allocation.sectors.push(encode_words(&block));
        }
        let mut header = self
            .original
            .get(..self.sector_size)
            .ok_or_else(|| bad_cfb("Truncated CFB header."))?
            .to_vec();
        p32(
            &mut header,
            40,
            if self.major == 4 {
                (directory.len() / self.sector_size) as u32
            } else {
                0
            },
        )?;
        p32(&mut header, 44, fat_count as u32)?;
        p32(&mut header, 48, directory_start)?;
        p32(&mut header, 60, mini_start)?;
        p32(&mut header, 64, mini_count as u32)?;
        p32(&mut header, 68, dif_ids.first().copied().unwrap_or(END))?;
        p32(&mut header, 72, dif_count as u32)?;
        let mut header_fat: Vec<_> = fat_ids.into_iter().take(HEADER_FAT_ENTRIES).collect();
        header_fat.resize(HEADER_FAT_ENTRIES, FREE);
        put(&mut header, 76, &encode_words(&header_fat))?;
        let mut output = header;
        for sector in allocation.sectors {
            output.extend(sector);
        }
        require(
            output.len() <= MAX_FILE,
            "LIMIT",
            "Output file exceeds limit.",
        )?;
        let check = Self::new(&output)?;
        require(
            check.streams.keys().eq(self.streams.keys()),
            "VERIFY_FAILED",
            "CFB stream inventory changed.",
        )?;
        for (name, old) in &self.streams {
            let expected = changed.get(name).copied().unwrap_or(old);
            require(
                check.streams.get(name) == Some(expected),
                "VERIFY_FAILED",
                &("CFB stream did not survive serialization: ".to_owned() + name),
            )?;
        }
        Ok(output)
    }
}
fn encode_words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
struct SectorBuilder {
    size: usize,
    sectors: Vec<Vec<u8>>,
    table: Vec<u32>,
}
impl SectorBuilder {
    fn new(size: usize) -> Self {
        Self {
            size,
            sectors: vec![],
            table: vec![],
        }
    }
    fn allocate(&mut self, payload: &[u8]) -> Result<u32> {
        if payload.is_empty() {
            return Ok(END);
        }
        let first = self.sectors.len();
        let count = payload.len().div_ceil(self.size);
        require(
            (first + count + 1) * self.size <= MAX_FILE,
            "LIMIT",
            "Output file exceeds limit.",
        )?;
        for (index, chunk) in payload.chunks(self.size).enumerate() {
            let mut sector = chunk.to_vec();
            sector.resize(self.size, 0);
            self.sectors.push(sector);
            self.table.push(if index + 1 < count {
                (first + index + 1) as u32
            } else {
                END
            });
        }
        Ok(first as u32)
    }
}
