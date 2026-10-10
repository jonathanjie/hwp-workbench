//! The recent-file index stores opaque grants, never renderer-provided paths.
use crate::{files, Error, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

// Match the desktop convenience cache: eight documents and 128 KiB per image.
const MAX_RECENT: usize = 8;
const MAX_PNG_BYTES: usize = 128 * 1024;
const PNG_PREFIX: &str = "data:image/png;base64,";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentEntry {
    id: String,
    file: PathBuf,
    name: Option<String>,
    format: String,
    opened_at: String,
}

pub struct RecentStore {
    directory: PathBuf,
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_document_path(file: &Path) -> bool {
    file.is_absolute()
        && file
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("hwp") || extension.eq_ignore_ascii_case("hwpx")
            })
}

impl RecentStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn entries(&self) -> Result<Vec<RecentEntry>> {
        files::ensure_private_directory(&self.directory)?;
        let index = self.directory.join("recent.json");
        if !index.try_exists()? {
            // A dangling symlink is invalid data, not an absent index.
            if fs::symlink_metadata(&index)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                return Ok(Vec::new());
            }
        }
        let bytes = files::read_bounded(&index, 64 * 1024)?;
        let entries: Vec<RecentEntry> =
            serde_json::from_slice(&bytes).map_err(|_| Error::new("Invalid recent data"))?;
        if entries.len() > MAX_RECENT
            || entries.iter().any(|entry| {
                !valid_id(&entry.id)
                    || !valid_document_path(&entry.file)
                    || !matches!(entry.format.as_str(), "hwp" | "hwpx")
            })
        {
            return Err(Error::new("Invalid recent data"));
        }
        Ok(entries)
    }

    pub fn record(&self, file: &Path, format: &str) -> Result<String> {
        if !valid_document_path(file) || !matches!(format, "hwp" | "hwpx") {
            return Err(Error::new("Invalid recent data"));
        }
        let entries = self.entries()?;
        let id = if let Some(previous) = entries.iter().find(|entry| entry.file == file) {
            previous.id.clone()
        } else {
            let mut random = [0; 16];
            getrandom::fill(&mut random).map_err(|error| Error::new(error.to_string()))?;
            hex::encode(random)
        };
        let new_entry = RecentEntry {
            id: id.clone(),
            file: file.to_owned(),
            name: file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            format: format.to_owned(),
            opened_at: files::now_timestamp(),
        };
        let next: Vec<_> = std::iter::once(new_entry)
            .chain(entries.iter().filter(|entry| entry.file != file).cloned())
            .take(MAX_RECENT)
            .collect();
        files::atomic_write_json(
            &self.directory.join("recent.json"),
            &serde_json::to_value(&next)?,
        )?;
        for removed in entries
            .iter()
            .filter(|entry| !next.iter().any(|current| current.id == entry.id))
        {
            let thumbs = self.directory.join("thumbs");
            files::ensure_private_directory(&thumbs)?;
            match fs::remove_file(thumbs.join(format!("{}.png", removed.id))) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into())
                }
                _ => {}
            }
        }
        Ok(id)
    }

    fn entry_for(&self, id: &str) -> Result<RecentEntry> {
        if !valid_id(id) {
            return Err(Error::new("Invalid recent document"));
        }
        self.entries()?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| Error::new("Invalid recent document"))
    }

    pub fn file_for(&self, id: &str) -> Result<PathBuf> {
        files::validate_document(&self.entry_for(id)?.file)
    }

    pub fn list(&self) -> Result<Value> {
        let entries = self.entries()?;
        let thumbs = self.directory.join("thumbs");
        files::ensure_private_directory(&thumbs)?;
        Ok(Value::Array(entries.into_iter().map(|entry| {
            let thumbnail = files::read_bounded(&thumbs.join(format!("{}.png", entry.id)), MAX_PNG_BYTES).ok().map(|bytes| format!("{PNG_PREFIX}{}", STANDARD.encode(bytes)));
            json!({"id":entry.id,"name":entry.file.file_name().map(|name| name.to_string_lossy().into_owned()),"format":entry.format,"openedAt":entry.opened_at,"thumbnail":thumbnail})
        }).collect()))
    }

    pub fn cache_thumbnail(&self, id: &str, data: &str) -> Result<bool> {
        self.entry_for(id)?;
        // Encoded byte cap includes the prefix and base64 padding.
        if data.len() > MAX_PNG_BYTES * 4 / 3 + 30 {
            return Err(Error::new("Invalid thumbnail"));
        }
        let encoded = data
            .strip_prefix(PNG_PREFIX)
            .ok_or_else(|| Error::new("Invalid thumbnail"))?;
        let png = STANDARD
            .decode(encoded)
            .map_err(|_| Error::new("Invalid thumbnail"))?;
        let width = png
            .get(16..20)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_be_bytes)
            .unwrap_or(0);
        let height = png
            .get(20..24)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_be_bytes)
            .unwrap_or(0);
        if png.len() < 24
            || png.len() > MAX_PNG_BYTES
            || png.get(..8) != Some(PNG_SIGNATURE)
            || png.get(12..16) != Some(b"IHDR")
            || width == 0
            || height == 0
            || width > 256
            || height > 384
        {
            return Err(Error::new("Invalid thumbnail"));
        }
        let thumbs = self.directory.join("thumbs");
        files::ensure_private_directory(&thumbs)?;
        files::atomic_write_bytes(&thumbs.join(format!("{id}.png")), &png)?;
        Ok(true)
    }
}
