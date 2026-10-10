//! In-process desktop adapter for the shared native service.
use crate::{engine::{Engine, ToolAction}, Error, Result};
use hwp_core::{errors::WorkbenchError, workspace::Workspace};
use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}};

// Match the desktop IPC budgets, including preview responses with image bytes.
const MAX_REQUEST: usize = 4 * 1024 * 1024;
const MAX_LINK_REQUEST: usize = 2 * 1024 * 1024;
const MAX_OUTPUT: usize = 32 * 1024 * 1024;
const MAX_PREVIEW_OUTPUT: usize = 90 * 1024 * 1024;

pub struct RustEngine {
    source_root: PathBuf,
}
impl RustEngine {
    pub fn new(source_root: PathBuf) -> Self {
        Self { source_root }
    }
}
fn service_error(error: WorkbenchError) -> Error {
    Error::coded(error.code, error.message)
}
fn check_size(value: &Value, limit: usize, message: &str) -> Result<()> {
    if serde_json::to_vec(value)?.len() > limit {
        return Err(Error::new(message));
    }
    Ok(())
}
fn with_cancellation<T>(cancel: &AtomicBool, call: impl FnOnce() -> Result<T>) -> Result<T> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Error::new("Preview dropped"));
    }
    let result = call();
    if cancel.load(Ordering::SeqCst) {
        return Err(Error::new("Preview dropped"));
    }
    result
}
impl Engine for RustEngine {
    fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value> {
        check_size(&json!({"method":method,"args":args}), MAX_REQUEST, "Request too large")?;
        let result = hwp_workbench::service::invoke(root, method, &args).map_err(service_error)?;
        let limit = if method == "preview" { MAX_PREVIEW_OUTPUT } else { MAX_OUTPUT };
        check_size(&result, limit, "Response too large")?;
        Ok(result)
    }
    fn call_cancelled(&self, root: &Path, method: &str, args: Value, cancel: Arc<AtomicBool>) -> Result<Value> {
        with_cancellation(&cancel, || self.call(root, method, args))
    }
    fn agent_link(&self, root: &Path, name: &str, action: &str, args: Value) -> Result<Value> {
        if !matches!(action, "read" | "session" | "outcome") {
            return Err(Error::new("Unknown link action."));
        }
        check_size(&args, MAX_LINK_REQUEST, "Link request too large")?;
        // The Python desktop helper exposed messages without engine error codes.
        let invoke = || -> std::result::Result<Value, WorkbenchError> {
            hwp_workbench::agent_link::validate_document_name(name)?;
            let workspace = Workspace::new(root)?;
            hwp_workbench::agent_link::desktop_action(&workspace, name, action, &args)
        };
        let result = invoke().map_err(|error| Error::new(error.message))?;
        check_size(&result, MAX_OUTPUT, "Response too large")?;
        Ok(result)
    }
    fn tool(&self, action: ToolAction) -> Result<String> {
        match action {
            ToolAction::Status => {
                let value = hwp_workbench::skill_install::run("status", "all", false).map_err(service_error)?;
                Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
            }
            ToolAction::Install { target } => {
                if !matches!(target.as_str(), "claude" | "codex") {
                    return Err(Error::new("Invalid skill target"));
                }
                let value = hwp_workbench::skill_install::run("install", &target, false).map_err(service_error)?;
                Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
            }
            ToolAction::McpConfig { client, root } => {
                if !matches!(client.as_str(), "claude-code" | "codex") {
                    return Err(Error::new("Invalid MCP client"));
                }
                let text = hwp_workbench::mcp_config::setup(&client, root).map_err(service_error)?;
                Ok(format!("{text}\n"))
            }
            ToolAction::BuildBundle => {
                let output = self.source_root.join("dist/hwp-workbench.mcpb");
                hwp_workbench::mcpb::build(&std::env::current_exe()?, &output).map_err(service_error)?;
                Ok(format!("{}\n",output.display()))
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_after_a_completed_call_drops_both_results_and_errors() {
        for fail in [false,true] {
            let cancel = AtomicBool::new(false);
            let result = with_cancellation(&cancel, || {
                cancel.store(true,Ordering::SeqCst);
                if fail { Err(Error::new("Read failed")) } else { Ok(json!(true)) }
            });
            assert_eq!(result.unwrap_err().message, "Preview dropped");
        }
    }
}
