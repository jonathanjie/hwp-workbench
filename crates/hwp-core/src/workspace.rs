//! Workspace-constrained POSIX I/O with descriptor walks and guarded publication.
use crate::errors::{require, Result, WorkbenchError, MAX_FILE};
use rustix::fd::OwnedFd;
use rustix::fs::{self, AtFlags, Mode, OFlags};
use rustix::io::Errno;
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use time::OffsetDateTime;

// Test seams compile away completely; they cannot be called by production users.
#[cfg(test)]
type TestHook = Box<dyn FnMut(&str) -> Result<()>>;
#[cfg(test)]
thread_local! {
    static TEST_HOOK: std::cell::RefCell<Option<TestHook>> = const { std::cell::RefCell::new(None) };
    static TEST_RANDOM: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static TEST_TIMESTAMP: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
macro_rules! step {
    ($name:literal) => {{
        #[cfg(test)]
        TEST_HOOK.with(|hook| match hook.borrow_mut().as_mut() {
            Some(hook) => hook($name),
            None => Ok(()),
        })?;
    }};
}

fn write_error(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("WRITE_FAILED", error.to_string())
}
fn read_error(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("READ_FAILED", error.to_string())
}

/// Compute the lowercase SHA-256 used by hash-bound source and output checks.
pub fn sha256(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn random_hex(bytes: usize) -> Result<String> {
    #[cfg(test)]
    if let Some(value) = TEST_RANDOM.with(|random| random.borrow().clone()) {
        return Ok(value);
    }
    let mut buffer = vec![0; bytes];
    getrandom::fill(&mut buffer).map_err(write_error)?;
    Ok(hex::encode(buffer))
}

fn timestamp() -> Result<String> {
    #[cfg(test)]
    if let Some(value) = TEST_TIMESTAMP.with(|timestamp| timestamp.borrow().clone()) {
        return Ok(value);
    }
    let now = OffsetDateTime::now_utc();
    // Compact UTC names sort lexically and do not depend on the local time zone.
    Ok(format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    ))
}

fn same_inode(first: &Metadata, second: &Metadata) -> bool {
    first.dev() == second.dev() && first.ino() == second.ino()
}

fn source_identity(root: &Path, relative: &str) -> String {
    // Match os.fsencode on POSIX, including roots with non-UTF-8 bytes.
    hex::encode(Sha256::digest(root.join(relative).as_os_str().as_bytes()))
        .chars()
        .take(8)
        .collect()
}

fn matches_backup(name: &str, prefix: &str, suffix: &str) -> bool {
    let Some(middle) = name
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
    else {
        return false;
    };
    let bytes = middle.as_bytes();
    if bytes.len() != 23 {
        return false;
    }
    let Some(date) = bytes.get(..8) else {
        return false;
    };
    let Some(clock) = bytes.get(9..15) else {
        return false;
    };
    let Some(random) = bytes.get(17..) else {
        return false;
    };
    date.iter().all(u8::is_ascii_digit)
        && bytes.get(8) == Some(&b'T')
        && clock.iter().all(u8::is_ascii_digit)
        && bytes.get(15) == Some(&b'Z')
        && bytes.get(16) == Some(&b'-')
        && random
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

/// An approved canonical workspace; all descendants are opened without links.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}
impl Workspace {
    /// Canonicalize an existing root and require it to be a directory.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let supplied = root.as_ref();
        let root = if supplied == Path::new("~") || supplied.starts_with("~/") {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                WorkbenchError::new("BAD_ROOT", "Workspace root must be a directory.")
            })?;
            let tail = supplied.strip_prefix("~").map_err(read_error)?;
            PathBuf::from(home).join(tail)
        } else {
            supplied.to_owned()
        };
        let root = root.canonicalize().map_err(read_error)?;
        require(
            root.is_dir(),
            "BAD_ROOT",
            "Workspace root must be a directory.",
        )?;
        Ok(Self { root })
    }

    /// Return the canonical root selected by the caller.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn parts(&self, path: &str) -> Result<Vec<String>> {
        require(
            !path.is_empty() && !path.contains('\0'),
            "BAD_PATH",
            "Invalid file path.",
        )?;
        let supplied = Path::new(path);
        let relative = if supplied.is_absolute() {
            supplied.strip_prefix(&self.root).map_err(|_| {
                WorkbenchError::new(
                    "OUTSIDE_WORKSPACE",
                    "Path is outside the allowed workspace.",
                )
            })?
        } else {
            supplied
        };
        let mut parts = vec![];
        for component in relative.components() {
            match component {
                Component::Normal(name) => parts.push(
                    name.to_str()
                        .ok_or_else(|| WorkbenchError::new("BAD_PATH", "Invalid file path."))?
                        .to_owned(),
                ),
                Component::CurDir => {} // pathlib removes harmless dot and repeated separators.
                _ => {
                    return Err(WorkbenchError::new(
                        "OUTSIDE_WORKSPACE",
                        "Relative traversal is prohibited.",
                    ))
                }
            }
        }
        require(
            !parts.is_empty(),
            "OUTSIDE_WORKSPACE",
            "Relative traversal is prohibited.",
        )?;
        Ok(parts)
    }

    /// Validate a path and return its normalized workspace-relative spelling.
    pub fn relative_path(&self, path: &str) -> Result<String> {
        Ok(self.parts(path)?.join("/"))
    }

    fn directory(
        &self,
        parts: &[String],
        create: bool,
        missing_ok: bool,
    ) -> Result<Option<OwnedFd>> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directory = fs::open(&self.root, flags, Mode::empty()).map_err(read_error)?;
        for name in parts {
            if create {
                match fs::mkdirat(&directory, name.as_str(), Mode::from_raw_mode(0o700)) {
                    Ok(()) | Err(Errno::EXIST) => {}
                    Err(_) => {
                        return Err(WorkbenchError::new(
                            "UNSAFE_PATH",
                            "Parent directory is absent or contains a symlink.",
                        ));
                    }
                }
            }
            match fs::openat(&directory, name.as_str(), flags, Mode::empty()) {
                Ok(next) => directory = next,
                Err(Errno::NOENT) if missing_ok => return Ok(None),
                Err(_) => {
                    return Err(WorkbenchError::new(
                        "UNSAFE_PATH",
                        "Parent directory is absent or contains a symlink.",
                    ))
                }
            }
        }
        Ok(Some(directory))
    }

    fn parent(&self, path: &str, missing_ok: bool) -> Result<(Option<OwnedFd>, String)> {
        let mut parts = self.parts(path)?;
        let name = parts
            .pop()
            .ok_or_else(|| WorkbenchError::new("BAD_PATH", "Invalid file path."))?;
        Ok((self.directory(&parts, false, missing_ok)?, name))
    }

    /// Create missing directories with private modes using a no-follow walk.
    pub fn ensure_directory(&self, path: &str) -> Result<()> {
        self.directory(&self.parts(path)?, true, false)?;
        Ok(())
    }

    /// List up to limit names accepted by a full-name predicate, then sort them.
    pub fn list_files(
        &self,
        path: &str,
        name_pattern: impl Fn(&str) -> bool,
        limit: usize,
    ) -> Result<Vec<String>> {
        let Some(directory) = self.directory(&self.parts(path)?, false, true)? else {
            return Ok(vec![]);
        };
        let mut names = vec![];
        let entries = fs::Dir::read_from(&directory).map_err(read_error)?;
        for entry in entries {
            let entry = entry.map_err(read_error)?;
            let Ok(name) = entry.file_name().to_str() else {
                continue;
            };
            if matches!(name, "." | "..") || !name_pattern(name) {
                continue;
            }
            names.push(name.to_owned());
            // Keep Python's semantics: limit zero collects all matches.
            if names.len() == limit {
                break;
            }
        }
        names.sort();
        Ok(names)
    }

    /// Read a regular file without symlinks, bounded by the normal file limit.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.read_with_options(path, MAX_FILE, false)?
            .ok_or_else(|| WorkbenchError::new("READ_FAILED", "File does not exist."))
    }

    /// Read at most max_bytes and optionally return None for an absent path.
    pub fn read_with_options(
        &self,
        path: &str,
        max_bytes: usize,
        missing_ok: bool,
    ) -> Result<Option<Vec<u8>>> {
        let (parent, name) = self.parent(path, missing_ok)?;
        let Some(parent) = parent else {
            return Ok(None);
        };
        let descriptor = match fs::openat(
            &parent,
            name.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(descriptor) => descriptor,
            Err(Errno::NOENT) if missing_ok => return Ok(None),
            Err(error) => return Err(read_error(error)),
        };
        let mut file = File::from(descriptor);
        let info = file.metadata().map_err(read_error)?;
        require(
            info.is_file(),
            "UNSAFE_PATH",
            "Only regular files are allowed.",
        )?;
        let message = format!("File exceeds {max_bytes} bytes.");
        require(info.len() <= max_bytes as u64, "LIMIT", &message)?;
        let data = bounded_read(&mut file, max_bytes).map_err(read_error)?;
        require(data.len() <= max_bytes, "LIMIT", &message)?;
        Ok(Some(data))
    }

    /// Publish a private new file atomically without replacing any destination.
    pub fn write_new(&self, path: &str, data: &[u8]) -> Result<()> {
        require(data.len() <= MAX_FILE, "LIMIT", "Output exceeds 64 MiB.")?;
        let (parent, name) = self.parent(path, false)?;
        let parent = parent.ok_or_else(|| write_error("Missing parent directory."))?;
        let temporary = temporary_name()?;
        let flags =
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let descriptor = fs::openat(
            &parent,
            temporary.as_str(),
            flags,
            Mode::from_raw_mode(0o600),
        )
        .map_err(new_write_error)?;
        // This cleanup scope starts only after successful exclusive creation.
        let result = (|| {
            let mut file = File::from(descriptor);
            file.write_all(data).map_err(write_error)?;
            step!("new_file_sync");
            file.sync_all().map_err(write_error)?;
            step!("new_link");
            fs::linkat(
                &parent,
                temporary.as_str(),
                &parent,
                name.as_str(),
                AtFlags::empty(),
            )
            .map_err(new_write_error)?;
            step!("new_directory_sync");
            fs::fsync(&parent).map_err(write_error)
        })();
        let cleanup = fs::unlinkat(&parent, temporary.as_str(), AtFlags::empty());
        match cleanup {
            Ok(()) | Err(Errno::NOENT) => result,
            Err(error) => Err(write_error(error)),
        }
    }

    fn replacement_source(
        &self,
        parent: &OwnedFd,
        name: &str,
        expected: &str,
    ) -> Result<(Vec<u8>, Metadata)> {
        let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let descriptor = fs::openat(parent, name, flags, Mode::empty()).map_err(|error| {
            if error == Errno::ACCESS || error == Errno::PERM {
                WorkbenchError::new("READ_ONLY_DOCUMENT", "Document is not writable.")
            } else {
                WorkbenchError::new(
                    "UNSAFE_PATH",
                    "Document must be an existing regular file without symlinks.",
                )
            }
        })?;
        let mut file = File::from(descriptor);
        let info = file.metadata().map_err(write_error)?;
        require(
            info.is_file(),
            "UNSAFE_PATH",
            "Only regular files are allowed.",
        )?;
        require(
            info.mode() & 0o222 != 0,
            "READ_ONLY_DOCUMENT",
            "Document has no write permission.",
        )?;
        require(
            info.len() <= MAX_FILE as u64,
            "LIMIT",
            "Document exceeds 64 MiB.",
        )?;
        let data = bounded_read(&mut file, MAX_FILE).map_err(write_error)?;
        require(data.len() <= MAX_FILE, "LIMIT", "Document exceeds 64 MiB.")?;
        require(
            sha256(&data) == expected,
            "SOURCE_CHANGED",
            "Document changed on disk; read it again before saving.",
        )?;
        Ok((data, info))
    }

    fn private_file(&self, parent: &OwnedFd, data: &[u8], mode: u32) -> Result<String> {
        let name = temporary_name()?;
        let descriptor = fs::openat(
            parent,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(write_error)?;
        let result = (|| {
            let mut file = File::from(descriptor);
            file.write_all(data).map_err(write_error)?;
            fs::fchmod(&file, Mode::from_raw_mode(mode as _)).map_err(write_error)?;
            step!("private_sync");
            file.sync_all().map_err(write_error)
        })();
        if let Err(error) = result {
            fs::unlinkat(parent, name.as_str(), AtFlags::empty()).map_err(write_error)?;
            return Err(error);
        }
        Ok(name)
    }

    fn retain_recovery(&self, path: &str, parent: &OwnedFd, recovery: &str) -> Result<String> {
        let relative = self.relative_path(path)?;
        let document = Path::new(&relative);
        let directory = document
            .parent()
            .unwrap_or(Path::new(""))
            .join(".hwp-workbench/backups");
        let directory = directory
            .to_str()
            .ok_or_else(|| write_error("Invalid backup path."))?;
        self.ensure_directory(directory)?;
        let backup_parent = self
            .directory(&self.parts(directory)?, false, false)?
            .ok_or_else(|| write_error("Missing backup directory."))?;
        let name = format!(
            "{}.recovery-{}-{}",
            document
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| write_error("Invalid source name."))?,
            timestamp()?,
            random_hex(6)?
        );
        let descriptor = fs::openat(
            parent,
            recovery,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(write_error)?;
        fs::fchmod(&descriptor, Mode::from_raw_mode(0o600)).map_err(write_error)?;
        step!("retain_recovery");
        fs::linkat(
            parent,
            recovery,
            &backup_parent,
            name.as_str(),
            AtFlags::empty(),
        )
        .map_err(write_error)?;
        let backup = format!("{directory}/{name}");
        let cleanup = (|| {
            step!("retention_backup_sync");
            fs::fsync(&backup_parent).map_err(write_error)?;
            step!("retention_cleanup");
            fs::unlinkat(parent, recovery, AtFlags::empty()).map_err(write_error)?;
            step!("retention_source_sync");
            fs::fsync(parent).map_err(write_error)
        })();
        cleanup.map_err(|error| retention_error(error, &backup))?;
        Ok(format!("{directory}/{name}"))
    }

    fn owns_output(&self, parent: &OwnedFd, name: &str, hash: &str, published: &Metadata) -> bool {
        self.replacement_source(parent, name, hash)
            .ok()
            .is_some_and(|(_, info)| same_inode(&info, published))
    }

    fn rollback_exchange(
        &self,
        parent: &OwnedFd,
        name: &str,
        displaced: &str,
        hash: &str,
        published: &Metadata,
    ) -> Result<bool> {
        step!("rollback");
        if !self.owns_output(parent, name, hash, published) {
            return Ok(false);
        }
        step!("rollback_exchange");
        // Exchange captures the exact revision present at the syscall, closing
        // the destructive check-then-rename gap in the Python implementation.
        fs::renameat_with(parent, displaced, parent, name, fs::RenameFlags::EXCHANGE)
            .map_err(write_error)?;
        if !self.owns_output(parent, displaced, hash, published) {
            // A writer raced the first exchange. Restore that captured revision;
            // retain whatever the second exchange displaces for explicit recovery.
            fs::renameat_with(parent, displaced, parent, name, fs::RenameFlags::EXCHANGE)
                .map_err(write_error)?;
            fs::fsync(parent).map_err(write_error)?;
            return Ok(false);
        }
        fs::fsync(parent).map_err(write_error)?;
        Ok(true)
    }

    fn retain_displaced(&self, path: &str, parent: &OwnedFd, displaced: &str) -> Result<String> {
        // Copy captured source bytes into a private inode rather than chmod an
        // original inode that may still have user-owned hard links elsewhere.
        let descriptor = fs::openat(
            parent,
            displaced,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(write_error)?;
        let mut file = File::from(descriptor);
        require(
            file.metadata().map_err(write_error)?.is_file(),
            "UNSAFE_PATH",
            "Only regular files are allowed.",
        )?;
        let bytes = bounded_read(&mut file, MAX_FILE).map_err(write_error)?;
        require(bytes.len() <= MAX_FILE, "LIMIT", "Document exceeds 64 MiB.")?;
        let copy = self.private_file(parent, &bytes, 0o600)?;
        let backup = self.retain_recovery(path, parent, &copy)?;
        fs::unlinkat(parent, displaced, AtFlags::empty())
            .map_err(|error| retention_error(write_error(error), &backup))?;
        Ok(backup)
    }

    /// Replace a hash-bound source and return the SHA-256 read back from disk.
    pub fn replace_existing(
        &self,
        path: &str,
        data: &[u8],
        expected_sha256: &str,
    ) -> Result<String> {
        require(data.len() <= MAX_FILE, "LIMIT", "Output exceeds 64 MiB.")?;
        let (parent, name) = self.parent(path, false)?;
        let parent = parent.ok_or_else(|| write_error("Missing parent directory."))?;
        let mut temporary: Option<String> = None;
        let mut recovery: Option<String> = None;
        let mut displaced: Option<String> = None;
        let mut published: Option<Metadata> = None;
        let mut source_conflict = false;
        let output_hash = sha256(data);
        let result = (|| {
            let (original, original_info) =
                self.replacement_source(&parent, &name, expected_sha256)?;
            let mode = original_info.mode() & 0o7777;
            let temporary_name = self.private_file(&parent, data, mode)?;
            temporary = Some(temporary_name.clone());
            recovery = Some(self.private_file(&parent, &original, mode)?);
            step!("recovery_written");
            let (_, current_info) = self.replacement_source(&parent, &name, expected_sha256)?;
            require(
                same_inode(&current_info, &original_info),
                "SOURCE_CHANGED",
                "Document was replaced during saving.",
            )?;
            let descriptor = fs::openat(
                &parent,
                temporary_name.as_str(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(write_error)?;
            let published_info = File::from(descriptor).metadata().map_err(write_error)?;
            step!("publish");
            fs::renameat_with(
                &parent,
                temporary_name.as_str(),
                &parent,
                name.as_str(),
                fs::RenameFlags::EXCHANGE,
            )
            .map_err(write_error)?;
            temporary = None;
            displaced = Some(temporary_name.clone());
            published = Some(published_info.clone());
            step!("after_publish");
            // Verify the source that was actually displaced by publication.
            let source_check = self.replacement_source(&parent, &temporary_name, expected_sha256);
            source_conflict = !source_check
                .as_ref()
                .is_ok_and(|(_, info)| same_inode(info, &original_info));
            require(
                !source_conflict,
                "SOURCE_CHANGED",
                "Document was replaced during saving.",
            )?;
            step!("publication_sync");
            fs::fsync(&parent).map_err(write_error)?;
            step!("readback");
            let (saved, saved_info) = self.replacement_source(&parent, &name, &output_hash)?;
            require(
                same_inode(&saved_info, &published_info),
                "SOURCE_CHANGED",
                "Document was replaced after saving.",
            )?;
            step!("recovery_cleanup");
            let recovery_name = recovery
                .as_deref()
                .ok_or_else(|| write_error("Missing recovery file."))?;
            fs::unlinkat(&parent, recovery_name, AtFlags::empty()).map_err(write_error)?;
            recovery = None;
            step!("displaced_cleanup");
            fs::unlinkat(&parent, temporary_name.as_str(), AtFlags::empty())
                .map_err(write_error)?;
            displaced = None;
            Ok(sha256(&saved))
        })();
        let result = match result {
            Err(error) if published.is_some() => {
                let published_info = published
                    .as_ref()
                    .ok_or_else(|| write_error("Missing publication identity."))?;
                let captured = displaced
                    .as_deref()
                    .ok_or_else(|| write_error("Missing displaced source."))?;
                let restored = match self.rollback_exchange(
                    &parent,
                    &name,
                    captured,
                    &output_hash,
                    published_info,
                ) {
                    Ok(restored) => restored,
                    Err(rollback_error) => {
                        // Keep both sources on rollback failure; never let final
                        // cleanup turn a recoverable write error into data loss.
                        return Err(write_error(format!(
                            "{rollback_error}; recovery kept at {}",
                            recovery_relative_path(
                                self,
                                path,
                                recovery.as_deref().unwrap_or(captured)
                            )?
                        )));
                    }
                };
                if !restored || source_conflict {
                    let mut backups = vec![];
                    if let Some(retained) = recovery.take() {
                        let fallback = recovery_relative_path(self, path, &retained)?;
                        backups.push(retention_location(
                            self.retain_recovery(path, &parent, &retained),
                            fallback,
                        ));
                    }
                    if let Some(retained) = displaced.take() {
                        if self.owns_output(&parent, &retained, &output_hash, published_info) {
                            fs::unlinkat(&parent, retained.as_str(), AtFlags::empty())
                                .map_err(write_error)?;
                        } else {
                            let fallback = recovery_relative_path(self, path, &retained)?;
                            backups.push(retention_location(
                                self.retain_displaced(path, &parent, &retained),
                                fallback,
                            ));
                        }
                    }
                    Err(WorkbenchError::new(
                        "SOURCE_CHANGED",
                        "Document changed after publication; recovery kept at ".to_owned()
                            + &backups.join(", "),
                    ))
                } else {
                    Err(error)
                }
            }
            result => result,
        };
        for owned in [temporary, recovery, displaced].into_iter().flatten() {
            fs::unlinkat(&parent, owned.as_str(), AtFlags::empty()).map_err(write_error)?;
        }
        result
    }

    /// Keep private previous bytes and retain only this source's newest ten backups.
    pub fn backup_existing(
        &self,
        path: &str,
        data: &[u8],
        backup_directory: Option<&str>,
    ) -> Result<String> {
        let relative = self.relative_path(path)?;
        let document = Path::new(&relative);
        let default_directory = document
            .parent()
            .unwrap_or(Path::new(""))
            .join(".hwp-workbench/backups");
        let directory = backup_directory.unwrap_or(
            default_directory
                .to_str()
                .ok_or_else(|| write_error("Invalid backup path."))?,
        );
        self.ensure_directory(directory)?;
        let identity = source_identity(&self.root, &relative);
        let stem = document
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| write_error("Invalid source name."))?;
        let suffix = document
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        let prefix = format!("{stem}.{identity}.");
        let name = format!("{prefix}{}-{}{suffix}", timestamp()?, random_hex(3)?);
        let backup = format!("{directory}/{name}");
        self.write_new(&backup, data)?;
        require(
            self.read(&backup)? == data,
            "VERIFY_FAILED",
            "Previous-version backup differs from the source.",
        )?;
        self.prune_backups(directory, &prefix, &suffix, &name)?;
        self.relative_path(&backup)
    }

    fn prune_backups(
        &self,
        path: &str,
        prefix: &str,
        suffix: &str,
        newest_name: &str,
    ) -> Result<()> {
        let directory = self
            .directory(&self.parts(path)?, false, false)?
            .ok_or_else(|| write_error("Missing backup directory."))?;
        let operation = (|| -> std::result::Result<(), Errno> {
            // Two descriptor scans keep memory bounded to the ten retained names.
            let mut newest = vec![];
            for entry in fs::Dir::read_from(&directory)? {
                let entry = entry?;
                let Ok(name) = entry.file_name().to_str() else {
                    continue;
                };
                if !matches_backup(name, prefix, suffix) {
                    continue;
                }
                let info = fs::statat(&directory, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)?;
                if fs::FileType::from_raw_mode(info.st_mode) != fs::FileType::RegularFile {
                    continue;
                }
                newest.push((
                    name == newest_name,
                    info.st_mtime,
                    info.st_mtime_nsec,
                    name.to_owned(),
                ));
                newest.sort_unstable_by(|a, b| b.cmp(a));
                newest.truncate(10);
            }
            for entry in fs::Dir::read_from(&directory)? {
                let entry = entry?;
                let Ok(name) = entry.file_name().to_str() else {
                    continue;
                };
                if !matches_backup(name, prefix, suffix) || newest.iter().any(|item| item.3 == name)
                {
                    continue;
                }
                let info = fs::statat(&directory, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)?;
                if fs::FileType::from_raw_mode(info.st_mode) == fs::FileType::RegularFile {
                    fs::unlinkat(&directory, entry.file_name(), AtFlags::empty())?;
                }
            }
            fs::fsync(&directory)
        })();
        operation.map_err(|error| write_error(format!("Cannot prune previous versions: {error}")))
    }
}

fn temporary_name() -> Result<String> {
    Ok(format!(".hwp-workbench-{}.tmp", random_hex(12)?))
}
fn new_write_error(error: Errno) -> WorkbenchError {
    if error == Errno::EXIST {
        WorkbenchError::new(
            "OUTPUT_EXISTS",
            "Destination already exists; use a new filename. Original files are never overwritten.",
        )
    } else {
        write_error(error)
    }
}
fn bounded_read(file: &mut File, maximum: usize) -> std::io::Result<Vec<u8>> {
    let mut data = Vec::new();
    file.take((maximum as u64).saturating_add(1))
        .read_to_end(&mut data)?;
    Ok(data)
}
fn recovery_relative_path(workspace: &Workspace, source: &str, retained: &str) -> Result<String> {
    let source = workspace.relative_path(source)?;
    Ok(Path::new(&source)
        .parent()
        .unwrap_or(Path::new(""))
        .join(retained)
        .to_string_lossy()
        .into_owned())
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;

fn retention_error(mut error: WorkbenchError, backup: &str) -> WorkbenchError {
    error.details = Some(serde_json::json!({ "recoveryPath": backup }));
    error
}
fn retention_location(result: Result<String>, fallback: String) -> String {
    match result {
        Ok(path) => path,
        Err(error) => error
            .details
            .as_ref()
            .and_then(|details| details.get("recoveryPath"))
            .and_then(|path| path.as_str())
            .map(str::to_owned)
            .unwrap_or(fallback),
    }
}
