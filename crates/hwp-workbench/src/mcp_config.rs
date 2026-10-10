//! Print client setup without changing client configuration.
use hwp_core::errors::{require, Result, WorkbenchError};
use serde_json::json;
use std::path::{Path, PathBuf};

pub(crate) fn project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// Resolve only the two supported runtime layouts: an app bundle or a checkout.
#[derive(Debug)]
pub(crate) struct RuntimePaths {
    pub source_root: PathBuf,
    pub command: PathBuf,
}
impl RuntimePaths {
    fn resolve(executable: &Path, project: &Path) -> Result<Self> {
        let directory = executable.parent().ok_or_else(cli_unavailable)?;
        if directory.file_name().is_some_and(|name| name == "MacOS") {
            if let Some(contents) = directory
                .parent()
                .filter(|path| path.file_name().is_some_and(|name| name == "Contents"))
            {
                return Ok(Self {
                    source_root: contents.join("Resources/app"),
                    command: contents.join("Resources/bin/hwp-workbench"),
                });
            }
        }
        // Cargo integration tests live in <profile>/deps; normal binaries are
        // directly in <profile>. This also supports a shared Cargo target folder.
        let profile = if directory.file_name().is_some_and(|name| name == "deps") {
            directory.parent().ok_or_else(cli_unavailable)?
        } else {
            directory
        };
        let profile_name = profile.file_name().unwrap_or_default();
        let candidates = [
            profile.join("hwp-workbench"),
            project
                .join("target")
                .join(profile_name)
                .join("hwp-workbench"),
            project.join("target/debug/hwp-workbench"),
            project.join("target/release/hwp-workbench"),
        ];
        let command = candidates
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(cli_unavailable)?;
        Ok(Self {
            source_root: project.to_owned(),
            command,
        })
    }

    pub(crate) fn current() -> Result<Self> {
        // macOS can report the Resources/bin symlink as current_exe. Resolve it
        // before testing the MacOS/Contents layout, or skills use the checkout.
        let executable = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .map_err(|_| cli_unavailable())?;
        Self::resolve(&executable, &project())
    }
}
fn cli_unavailable() -> WorkbenchError {
    WorkbenchError::new("CLI_UNAVAILABLE", "Built hwp-workbench CLI unavailable. Build the Rust CLI in target/<profile>/hwp-workbench before configuring an agent.")
}

pub(crate) fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        WorkbenchError::new(
            "BAD_ROOT",
            "Workspace must be an existing document directory.",
        )
    })
}
pub(crate) fn expand(path: &Path) -> Result<PathBuf> {
    if path == Path::new("~") || path.starts_with("~/") {
        Ok(home()?.join(
            path.strip_prefix("~")
                .map_err(|error| WorkbenchError::new("BAD_ROOT", error.to_string()))?,
        ))
    } else {
        Ok(path.to_owned())
    }
}
fn quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\"'\"'"))
    }
}
/// Print an absolute, shell-quoted setup command or JSON configuration.
pub fn setup(client: &str, root: impl AsRef<Path>) -> Result<String> {
    let workspace = expand(root.as_ref())?.canonicalize().map_err(|_| {
        WorkbenchError::new(
            "BAD_ROOT",
            "Workspace must be an existing document directory.",
        )
    })?;
    require(
        workspace.is_dir(),
        "BAD_ROOT",
        "Workspace must be an existing directory.",
    )?;
    let home = home()?
        .canonicalize()
        .map_err(|error| WorkbenchError::new("BAD_ROOT", error.to_string()))?;
    require(
        workspace != Path::new("/") && workspace != home,
        "BAD_ROOT",
        "The home directory and / grant too much access. Choose a documents subfolder.",
    )?;
    let paths = RuntimePaths::current()?;
    let command = paths.command.to_string_lossy().into_owned();
    let root = workspace.to_string_lossy().into_owned();
    let mut arguments = match client {
        "claude-code" => vec!["claude","mcp","add","hwp-workbench","--scope","user","--"],
        "codex" => vec!["codex","mcp","add","hwp-workbench","--"],
        "claude-desktop" => return Ok(crate::mcpb::bundle_path()?.to_string_lossy().into_owned()),
        _ => return serde_json::to_string_pretty(&json!({"mcpServers":{"hwp-workbench":{"command":command,"args":["mcp","--root",root]}}})).map_err(|error| WorkbenchError::new("REQUEST_FAILED",error.to_string())),
    };
    arguments.extend([&command, "mcp", "--root", &root]);
    Ok(arguments
        .into_iter()
        .map(quote)
        .collect::<Vec<_>>()
        .join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_paths_use_resources_app_and_the_cli_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let contents = directory.path().join("HWP Workbench.app/Contents");
        let executable = contents.join("MacOS/HWP Workbench");
        let skills = contents.join("Resources/app/skills/hwp-workbench");
        std::fs::create_dir_all(&skills).unwrap();
        std::fs::write(skills.join("SKILL.md"), "bundled skill").unwrap();
        let paths = RuntimePaths::resolve(&executable, directory.path()).unwrap();
        assert_eq!(paths.source_root, contents.join("Resources/app"));
        assert_eq!(paths.command, contents.join("Resources/bin/hwp-workbench"));
    }

    #[test]
    fn checkout_uses_the_current_built_profile_and_reports_a_missing_cli() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("checkout");
        let profile = directory.path().join("shared-target/release");
        std::fs::create_dir_all(&profile).unwrap();
        let executable = profile.join("hwp-workbench-app");
        let error = RuntimePaths::resolve(&executable, &project).unwrap_err();
        assert_eq!(error.code, "CLI_UNAVAILABLE");
        assert!(error.message.contains("Build the Rust CLI"));
        let cli = profile.join("hwp-workbench");
        std::fs::write(&cli, "native CLI").unwrap();
        assert_eq!(
            RuntimePaths::resolve(&executable, &project)
                .unwrap()
                .command,
            cli
        );
        std::fs::remove_file(&cli).unwrap();
        let cli = project.join("target/debug/hwp-workbench");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, "native CLI").unwrap();
        assert_eq!(
            RuntimePaths::resolve(&executable, &project)
                .unwrap()
                .command,
            cli
        );
    }
}
