//! Install and remove only application-marked agent skills through directory descriptors.
use hwp_core::errors::{require, Result, WorkbenchError};
use hwp_core::workspace::Workspace;
use regex::Regex;
use rustix::fd::OwnedFd;
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;
use serde_json::{json, Value};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Bound ownership-marker reads to 2 MiB; skills are not document inputs.
const MAX_SKILL_FILE: usize = 2 * 1024 * 1024;
const TARGETS: [&str; 2] = ["claude", "codex"];
const VERSION: &str = "0.1.0-alpha.1";
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
fn error(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("REQUEST_FAILED", error.to_string())
}
fn unsafe_path() -> WorkbenchError {
    WorkbenchError::new(
        "UNSAFE_PATH",
        "Parent directory is absent or contains a symlink.",
    )
}
fn selected(target: &str) -> Result<Vec<&str>> {
    require(
        matches!(target, "claude" | "codex" | "all"),
        "BAD_ARGUMENTS",
        "Skill target must be claude, codex or all.",
    )?;
    Ok(if target == "all" {
        TARGETS.to_vec()
    } else {
        vec![target]
    })
}
fn folder(target: &str) -> String {
    format!(".{target}/skills/hwp-workbench")
}
fn parent(workspace: &Workspace, target: &str, create: bool) -> Result<Option<OwnedFd>> {
    let mut directory =
        fs::open(workspace.root(), DIRECTORY_FLAGS, Mode::empty()).map_err(error)?;
    for name in [format!(".{target}"), "skills".to_owned()] {
        if create {
            match fs::mkdirat(&directory, name.as_str(), Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(_) => return Err(unsafe_path()),
            }
        }
        directory = match fs::openat(&directory, name.as_str(), DIRECTORY_FLAGS, Mode::empty()) {
            Ok(next) => next,
            Err(Errno::NOENT) if !create => return Ok(None),
            Err(_) => return Err(unsafe_path()),
        };
    }
    Ok(Some(directory))
}
fn version(parent: Option<&OwnedFd>) -> Result<(bool, Option<String>)> {
    let Some(parent) = parent else {
        return Ok((false, None));
    };
    let info = match fs::statat(parent, "hwp-workbench", AtFlags::SYMLINK_NOFOLLOW) {
        Ok(info) => info,
        Err(Errno::NOENT) => return Ok((false, None)),
        Err(failure) => return Err(error(failure)),
    };
    if FileType::from_raw_mode(info.st_mode) != FileType::Directory {
        return Ok((true, None));
    }
    let directory =
        fs::openat(parent, "hwp-workbench", DIRECTORY_FLAGS, Mode::empty()).map_err(error)?;
    let descriptor = match fs::openat(
        &directory,
        "SKILL.md",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(_) => return Ok((true, None)),
    };
    let mut file = File::from(descriptor);
    let metadata = file.metadata().map_err(error)?;
    if !metadata.is_file() || metadata.len() > MAX_SKILL_FILE as u64 {
        return Ok((true, None));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_SKILL_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > MAX_SKILL_FILE {
        return Ok((true, None));
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok((true, None));
    };
    let marker =
        Regex::new(r"<!-- installed by hwp-workbench ([A-Za-z0-9.+-]+) -->").map_err(error)?;
    Ok((
        true,
        marker
            .captures(&text)
            .and_then(|captures| captures.get(1))
            .map(|version| version.as_str().to_owned()),
    ))
}
/// Report installed versions without following a user-supplied skill link.
pub fn status(workspace: &Workspace) -> Value {
    let mut result = serde_json::Map::new();
    for target in TARGETS {
        let version = parent(workspace, target, false)
            .and_then(|parent| version(parent.as_ref()))
            .ok()
            .and_then(|(_, version)| version);
        result.insert(target.to_owned(),json!({"installed":version.is_some(),"version":version,"path":workspace.root().join(folder(target)).to_string_lossy()}));
    }
    Value::Object(result)
}
fn skill_source() -> Result<PathBuf> {
    Ok(crate::mcp_config::RuntimePaths::current()?
        .source_root
        .join("skills/hwp-workbench"))
}
fn content() -> Result<Vec<(String, Vec<u8>)>> {
    let source = skill_source()?;
    let launcher = crate::mcp_config::RuntimePaths::current()?
        .command
        .to_string_lossy()
        .into_owned();
    let mut files = Vec::new();
    for name in ["references/patch-format.md", "SKILL.md"] {
        let text = std::fs::read_to_string(source.join(name))
            .map_err(error)?
            .replace("{{HWP_WORKBENCH}}", &launcher);
        let text = if name == "SKILL.md" {
            format!(
                "{}\n\n<!-- installed by hwp-workbench {VERSION} -->\n",
                text.trim_end()
            )
        } else {
            text
        };
        files.push((name.to_owned(), text.into_bytes()));
    }
    Ok(files)
}
fn random_name() -> Result<String> {
    let mut bytes = [0; 12];
    getrandom::fill(&mut bytes).map_err(error)?;
    Ok(format!(".hwp-workbench-backup-{}", hex::encode(bytes)))
}
fn backup(workspace: &Workspace, target: &str, parent: &OwnedFd) -> Result<String> {
    let name = random_name()?;
    fs::mkdirat(parent, name.as_str(), Mode::from_raw_mode(0o700)).map_err(error)?;
    let directory =
        fs::openat(parent, name.as_str(), DIRECTORY_FLAGS, Mode::empty()).map_err(error)?;
    fs::renameat(parent, "hwp-workbench", &directory, "skill").map_err(error)?;
    Ok(workspace
        .root()
        .join(format!(".{target}/skills/{name}/skill"))
        .to_string_lossy()
        .into_owned())
}
/// Preflight all targets, back up owned folders and write the marker last.
pub fn install(workspace: &Workspace, target: &str, dry_run: bool) -> Result<Value> {
    let targets = selected(target)?;
    let files = content()?;
    for target in &targets {
        let parent = parent(workspace, target, false)?;
        let (exists, version) = version(parent.as_ref())?;
        require(
            !exists || version.is_some(),
            "SKILL_NOT_OWNED",
            &format!(
                "Refusing to replace unmarked skill folder: {}",
                workspace.root().join(folder(target)).display()
            ),
        )?;
    }
    let mut backups = Vec::new();
    if !dry_run {
        for target in &targets {
            let parent = parent(workspace, target, true)?.ok_or_else(unsafe_path)?;
            let (exists, version) = version(Some(&parent))?;
            require(
                !exists || version.is_some(),
                "SKILL_NOT_OWNED",
                "Skill ownership changed; no replacement allowed.",
            )?;
            if exists {
                backups.push(backup(workspace, target, &parent)?);
            }
            fs::mkdirat(&parent, "hwp-workbench", Mode::from_raw_mode(0o700)).map_err(error)?;
            let folder = folder(target);
            workspace.ensure_directory(&format!("{folder}/references"))?;
            for (name, bytes) in &files {
                workspace.write_new(&format!("{folder}/{name}"), bytes)?;
            }
        }
    }
    let mut result = json!({"dryRun":dry_run,"targets":targets,"backups":backups});
    if let (Some(result), Some(status)) = (result.as_object_mut(), status(workspace).as_object()) {
        result.extend(status.clone());
    }
    Ok(result)
}
fn remove_tree(parent: &OwnedFd, name: &str) -> Result<()> {
    let directory = fs::openat(parent, name, DIRECTORY_FLAGS, Mode::empty()).map_err(error)?;
    let entries = fs::Dir::read_from(&directory).map_err(error)?;
    for entry in entries {
        let entry = entry.map_err(error)?;
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let info = fs::statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        if FileType::from_raw_mode(info.st_mode) == FileType::Directory {
            let name = name.to_str().map_err(error)?;
            remove_tree(&directory, name)?;
        } else {
            fs::unlinkat(&directory, name, AtFlags::empty()).map_err(error)?;
        }
    }
    fs::unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(error)
}
/// Remove application-owned skills; preserve all unmarked folders.
pub fn uninstall(workspace: &Workspace, target: &str) -> Result<Value> {
    let mut removed = Vec::new();
    for target in selected(target)? {
        let parent = match parent(workspace, target, false) {
            Ok(parent) => parent,
            Err(error) if error.code == "UNSAFE_PATH" => continue,
            Err(error) => return Err(error),
        };
        if let Some(parent) = parent {
            let (exists, version) = version(Some(&parent))?;
            if exists && version.is_some() {
                remove_tree(&parent, "hwp-workbench")?;
                removed.push(target);
            }
        }
    }
    let mut result = json!({"removed":removed});
    if let (Some(result), Some(status)) = (result.as_object_mut(), status(workspace).as_object()) {
        result.extend(status.clone());
    }
    Ok(result)
}
/// The environment override is retained so tests never write the actual home.
pub fn run(command: &str, target: &str, dry_run: bool) -> Result<Value> {
    let home = match std::env::var_os("HWP_WORKBENCH_SKILL_HOME") {
        Some(path) => {
            require(
                !path.is_empty(),
                "BAD_ARGUMENTS",
                "Skill home override must not be empty.",
            )?;
            crate::mcp_config::expand(Path::new(&path))?
        }
        None => crate::mcp_config::home()?,
    };
    let workspace = Workspace::new(home)?;
    match command {
        "status" => Ok(status(&workspace)),
        "install" => install(&workspace, target, dry_run),
        "uninstall" => uninstall(&workspace, target),
        _ => Err(WorkbenchError::new(
            "BAD_ARGUMENTS",
            "Unknown skill command.",
        )),
    }
}
