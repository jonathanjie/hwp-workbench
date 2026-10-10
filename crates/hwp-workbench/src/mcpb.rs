//! Build the native Claude Desktop bundle from three explicit sources.
use hwp_core::errors::{require, Result, WorkbenchError};
use rustix::fs::{self, Mode, OFlags};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const MANIFEST: &str = include_str!("../mcpb/manifest.json");
const ICON: &[u8] = include_bytes!("../mcpb/icon.png");

/// Must match the Tauri bundle identifier; also names the per-user data folder.
pub const APP_IDENTIFIER: &str = "io.github.jonathanjie.hwp-workbench";

/// Resolve the current server version in an explicit application data folder.
pub fn bundle_path_in(data_home: &Path) -> PathBuf {
    data_home.join(format!("hwp-workbench-{}.mcpb", crate::service::VERSION))
}

/// Keep generated bundles outside the signed app. The override is the complete
/// application data folder, so test processes can use a private temporary folder.
pub fn bundle_path() -> Result<PathBuf> {
    let data_home = match std::env::var_os("HWP_WORKBENCH_DATA_HOME") {
        Some(directory) => PathBuf::from(directory),
        None => default_data_home()?,
    };
    require(
        data_home.is_absolute(),
        "BAD_ROOT",
        "Application data directory must be an absolute path.",
    )?;
    Ok(bundle_path_in(&data_home))
}

fn default_data_home() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(|directory| PathBuf::from(directory).join(APP_IDENTIFIER))
            .ok_or_else(|| {
                WorkbenchError::new("BAD_ROOT", "Application data directory unavailable.")
            })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = crate::mcp_config::home()?;
        #[cfg(target_os = "macos")]
        let base = home.join("Library/Application Support");
        #[cfg(not(target_os = "macos"))]
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        Ok(base.join(APP_IDENTIFIER))
    }
}

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
