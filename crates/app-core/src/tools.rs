//! Fixed agent-tool actions. The caller performs clipboard and Finder operations.
use crate::{
    engine::{Engine, ToolAction},
    Error, Result,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn bundle_available(bundle: &Path) -> Result<bool> {
    let directory = bundle
        .parent()
        .ok_or_else(|| Error::new("Unsafe bundle path"))?;
    match (
        fs::symlink_metadata(directory),
        fs::symlink_metadata(bundle),
    ) {
        (Ok(folder), Ok(file)) if folder.is_dir() && file.is_file() => Ok(true),
        (Err(error), _) | (_, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(false)
        }
        (Err(error), _) | (_, Err(error)) => Err(error.into()),
        _ => Err(Error::new("Unsafe bundle path")),
    }
}

pub fn status(engine: &dyn Engine, document_root: Option<&Path>) -> Result<Value> {
    let mut result: Value = serde_json::from_str(&engine.tool(ToolAction::Status)?)?;
    let fields = result
        .as_object_mut()
        .ok_or_else(|| Error::new("Invalid engine response"))?;
    fields.insert(
        "claudeDesktop".into(),
        json!({"installed":null,"bundleAvailable":bundle_available(&engine.bundle_path()?)?}),
    );
    fields.insert("hasDocument".into(), Value::Bool(document_root.is_some()));
    Ok(result)
}

pub fn install(engine: &dyn Engine, document_root: Option<&Path>, target: &str) -> Result<Value> {
    if !matches!(target, "claude" | "codex") {
        return Err(Error::new("Invalid skill target"));
    }
    engine.tool(ToolAction::Install {
        target: target.to_owned(),
    })?;
    status(engine, document_root)
}

pub fn copy_mcp(engine: &dyn Engine, document_root: Option<&Path>, client: &str) -> Result<String> {
    if !matches!(client, "claude-code" | "codex") {
        return Err(Error::new("Invalid MCP client"));
    }
    let root = document_root.ok_or_else(|| Error::new("No document"))?;
    Ok(engine
        .tool(ToolAction::McpConfig {
            client: client.to_owned(),
            root: root.to_owned(),
        })?
        .trim()
        .to_owned())
}

pub fn reveal_bundle(engine: &dyn Engine) -> Result<PathBuf> {
    let bundle = engine.bundle_path()?;
    if !bundle_available(&bundle)? {
        let folder = bundle
            .parent()
            .ok_or_else(|| Error::new("Unsafe bundle path"))?;
        match fs::symlink_metadata(folder) {
            Ok(info) if !info.is_dir() => return Err(Error::new("Unsafe bundle directory")),
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        engine.tool(ToolAction::BuildBundle)?;
    }
    if !bundle_available(&bundle)? {
        return Err(Error::new("Bundle unavailable"));
    }
    Ok(bundle)
}
