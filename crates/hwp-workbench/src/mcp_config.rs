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
    let project = project();
    let command = project
        .join("scripts/hwp-workbench")
        .to_string_lossy()
        .into_owned();
    let root = workspace.to_string_lossy().into_owned();
    let mut arguments = match client {
        "claude-code" => vec!["claude","mcp","add","hwp-workbench","--scope","user","--"],
        "codex" => vec!["codex","mcp","add","hwp-workbench","--"],
        "claude-desktop" => return Ok(project.join("dist/hwp-workbench.mcpb").to_string_lossy().into_owned()),
        _ => return serde_json::to_string_pretty(&json!({"mcpServers":{"hwp-workbench":{"command":command,"args":["mcp","--root",root]}}})).map_err(|error| WorkbenchError::new("REQUEST_FAILED",error.to_string())),
    };
    arguments.extend([&command, "mcp", "--root", &root]);
    Ok(arguments
        .into_iter()
        .map(quote)
        .collect::<Vec<_>>()
        .join(" "))
}
