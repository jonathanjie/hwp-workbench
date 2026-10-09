//! Bounded file reads and private profile writes. No document writer lives here.
use crate::{Error, Result};
use rustix::fs::{self as descriptor, Mode, OFlags};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

// The renderer receives at most the same 64 MiB native source as Electron.
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

fn io_error(error: rustix::io::Errno) -> Error {
    Error::from(std::io::Error::from(error))
}

pub fn validate_document(file: &Path) -> Result<PathBuf> {
    let extension = file
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !file.is_absolute()
        || !(extension.eq_ignore_ascii_case("hwp") || extension.eq_ignore_ascii_case("hwpx"))
    {
        return Err(Error::new("Unsupported document"));
    }
    let metadata = fs::symlink_metadata(file)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::new(
            "Document must be a regular file without symlinks",
        ));
    }
    canonical_document_path(file)
}

fn canonical_document_path(file: &Path) -> Result<PathBuf> {
    let parent = file
        .parent()
        .ok_or_else(|| Error::new("Unsupported document"))?;
    let name = file
        .file_name()
        .ok_or_else(|| Error::new("Unsupported document"))?;
    // Canonicalize only the parent. A leaf replaced after lstat must stay at
    // the granted path so the engine's no-follow open refuses the replacement.
    Ok(fs::canonicalize(parent)?.join(name))
}

fn open_document_no_follow(file: &Path) -> Result<File> {
    if !file.is_absolute() {
        return Err(Error::new("Source path changed"));
    }
    let mut parent = descriptor::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let components: Vec<_> = file
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(Ok(name)),
            Component::ParentDir => Some(Err(Error::new("Source path changed"))),
            _ => None,
        })
        .collect::<Result<_>>()?;
    let mut names = components.iter().peekable();
    while let Some(name) = names.next() {
        let flags = OFlags::RDONLY
            | OFlags::NONBLOCK
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | if names.peek().is_some() {
                OFlags::DIRECTORY
            } else {
                OFlags::empty()
            };
        parent = descriptor::openat(&parent, *name, flags, Mode::empty())
            .map_err(|_| Error::new("Source path changed"))?;
    }
    Ok(File::from(parent))
}

pub fn read_document_bytes(file: &Path, expected_hash: &str) -> Result<Vec<u8>> {
    let mut handle = open_document_no_follow(file)?;
    let metadata = handle.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::new("File exceeds 64 MiB"));
    }
    let size = usize::try_from(metadata.len()).map_err(|_| Error::new("File exceeds 64 MiB"))?;
    let mut bytes = vec![0; size];
    handle
        .read_exact(&mut bytes)
        .map_err(|_| Error::new("Source changed"))?;
    let mut extra = [0; 1];
    if handle.read(&mut extra)? != 0 || hex::encode(Sha256::digest(&bytes)) != expected_hash {
        return Err(Error::new("Source changed"));
    }
    Ok(bytes)
}

pub fn draft_path(profile: &Path, file: &Path, hash: &str) -> PathBuf {
    let identity = hex::encode(Sha256::digest(file.to_string_lossy().as_bytes()));
    let short = identity.get(..32).unwrap_or_default();
    profile.join("drafts").join(format!("{short}-{hash}.json"))
}

pub fn read_bounded(file: &Path, limit: usize) -> Result<Vec<u8>> {
    let descriptor = descriptor::open(
        file,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let handle = File::from(descriptor);
    let metadata = handle.metadata()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(Error::new("Recent data limit exceeded"));
    }
    let mut bytes = Vec::new();
    handle
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::new("Recent data limit exceeded"));
    }
    Ok(bytes)
}

pub fn ensure_private_directory(directory: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err(Error::new("Recent directory changed"));
    }
    Ok(())
}

pub fn atomic_write_json(file: &Path, value: &Value) -> Result<()> {
    atomic_write_bytes(file, &serde_json::to_vec_pretty(value)?)
}

pub fn atomic_write_bytes(file: &Path, bytes: &[u8]) -> Result<()> {
    let directory = file
        .parent()
        .ok_or_else(|| Error::new("Invalid profile path"))?;
    let destination = file
        .file_name()
        .ok_or_else(|| Error::new("Invalid profile path"))?;
    // Hold the directory descriptor while publishing; replacing a path component
    // cannot redirect the write after this point. Aliased parent folders are allowed.
    let parent = descriptor::open(
        directory,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let mut random = [0; 8];
    getrandom::fill(&mut random).map_err(|error| Error::new(error.to_string()))?;
    let temporary = format!(".profile-{}.tmp", hex::encode(random));
    let fd = descriptor::openat(
        &parent,
        temporary.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(io_error)?;
    let result = (|| {
        let mut output = File::from(fd);
        output.write_all(bytes)?;
        output.sync_all()?;
        descriptor::renameat(&parent, temporary.as_str(), &parent, destination)
            .map_err(io_error)?;
        Ok(())
    })();
    let _ = descriptor::unlinkat(&parent, temporary.as_str(), descriptor::AtFlags::empty());
    result
}

pub fn now_timestamp() -> String {
    let now = time::OffsetDateTime::now_utc();
    now.format(time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    ))
    .unwrap_or_else(|_| "1970-01-01T00:00:00.000Z".to_owned())
}

pub fn write_new(file: &Path, bytes: &[u8]) -> Result<()> {
    let parent_path = file
        .parent()
        .ok_or_else(|| Error::new("Invalid output path"))?;
    let name = file
        .file_name()
        .ok_or_else(|| Error::new("Invalid output path"))?;
    let parent = descriptor::open(
        parent_path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?;
    let fd = descriptor::openat(
        &parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(io_error)?;
    let mut output = File::from(fd);
    output.write_all(bytes)?;
    output.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_grants_do_not_follow_a_replaced_document_leaf() {
        let scratch = tempfile::tempdir().unwrap();
        let parent = fs::canonicalize(scratch.path()).unwrap();
        let approved = parent.join("approved");
        let outside = parent.join("outside");
        fs::create_dir(&approved).unwrap();
        fs::create_dir(&outside).unwrap();
        let granted = approved.join("source.hwpx");
        let other = outside.join("other.hwpx");
        fs::write(&granted, b"source").unwrap();
        fs::write(&other, b"outside").unwrap();
        assert!(fs::symlink_metadata(&granted).unwrap().is_file());
        // Model a replacement after the leaf check, before path normalization.
        fs::remove_file(&granted).unwrap();
        std::os::unix::fs::symlink(&other, &granted).unwrap();
        let normalized = canonical_document_path(&granted).unwrap();
        assert_eq!(normalized, granted);
        assert_ne!(normalized, other);
        assert!(read_document_bytes(&normalized, "hash").is_err());
    }
}
