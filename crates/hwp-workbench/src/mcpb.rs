//! Build the native Claude Desktop bundle from three explicit sources.
use hwp_core::errors::{require, Result, WorkbenchError};
use rustix::fs::{self, Mode, OFlags};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const MANIFEST: &str = include_str!("../mcpb/manifest.json");
const ICON: &[u8] = include_bytes!("../mcpb/icon.png");

fn error(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("WRITE_FAILED", error.to_string())
}

/// Read every source first, then create an exclusive archive. Never replace output.
pub fn build(binary: &Path, output: &Path) -> Result<()> {
    let descriptor = fs::open(
        binary,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| {
        WorkbenchError::new(
            "BAD_ARGUMENTS",
            format!("Bundle source must be a regular file: {}", binary.display()),
        )
    })?;
    let mut source = File::from(descriptor);
    require(
        source.metadata().map_err(error)?.is_file(),
        "BAD_ARGUMENTS",
        &format!("Bundle source must be a regular file: {}", binary.display()),
    )?;
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes).map_err(error)?;
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(error)?;
    let filename = output
        .file_name()
        .ok_or_else(|| error("Invalid bundle path."))?;
    let directory = fs::open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(error)?;
    let descriptor = fs::openat(
        &directory,
        filename,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(error)?;
    let mut archive = ZipWriter::new(File::from(descriptor));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (name, content) in [("manifest.json", MANIFEST.as_bytes()), ("icon.png", ICON)] {
        archive.start_file(name, options).map_err(error)?;
        archive.write_all(content).map_err(error)?;
    }
    archive
        .start_file("server/hwp-workbench", options.unix_permissions(0o755))
        .map_err(error)?;
    archive.write_all(&bytes).map_err(error)?;
    archive.finish().map_err(error)?.sync_all().map_err(error)?;
    Ok(())
}
