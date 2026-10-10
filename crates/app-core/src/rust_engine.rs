//! In-process desktop adapter for the shared native service.
use crate::{
    engine::{Engine, ToolAction},
    Error, Result,
};
use hwp_core::{errors::WorkbenchError, workspace::Workspace};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

// Match the desktop IPC budgets, including preview responses with image bytes.
const MAX_REQUEST: usize = 4 * 1024 * 1024;
const MAX_LINK_REQUEST: usize = 2 * 1024 * 1024;
const MAX_OUTPUT: usize = 32 * 1024 * 1024;
const MAX_PREVIEW_OUTPUT: usize = 90 * 1024 * 1024;

pub struct RustEngine {
    data_home: Option<PathBuf>,
}
impl RustEngine {
    pub fn new(_source_root: PathBuf) -> Self {
        Self { data_home: None }
    }

    /// Explicit application data folder for isolated hosts and tests.
    pub fn with_data_home(data_home: PathBuf) -> Self {
        Self {
            data_home: Some(data_home),
        }
    }
}
fn service_error(error: WorkbenchError) -> Error {
    Error::coded(error.code, error.message)
}

/// Count serialized UTF-8 bytes without retaining a second copy of the value.
struct SizeWriter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}
impl Write for SizeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        // bytes never exceeds limit; subtraction also avoids addition overflow.
        if buffer.len() > self.limit - self.bytes {
            self.exceeded = true;
            return Err(io::Error::other("JSON size limit exceeded"));
        }
        self.bytes += buffer.len();
        Ok(buffer.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn check_size(value: &impl Serialize, limit: usize, message: &str) -> Result<()> {
    let mut writer = SizeWriter {
        bytes: 0,
        limit,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        return Err(Error::new(message));
    }
    result?;
    Ok(())
}

#[derive(Serialize)]
struct DesktopRequest<'a> {
    method: &'a str,
    args: &'a Value,
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
    fn bundle_path(&self) -> Result<PathBuf> {
        match &self.data_home {
            Some(directory) => Ok(hwp_workbench::mcpb::bundle_path_in(directory)),
            None => hwp_workbench::mcpb::bundle_path().map_err(service_error),
        }
    }
    fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value> {
        check_size(
            &DesktopRequest {
                method,
                args: &args,
            },
            MAX_REQUEST,
            "Request too large",
        )?;
        let result = hwp_workbench::service::invoke(root, method, &args).map_err(service_error)?;
        let limit = if method == "preview" {
            MAX_PREVIEW_OUTPUT
        } else {
            MAX_OUTPUT
        };
        check_size(&result, limit, "Response too large")?;
        Ok(result)
    }
    fn call_cancelled(
        &self,
        root: &Path,
        method: &str,
        args: Value,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value> {
        with_cancellation(&cancel, || self.call(root, method, args))
    }
    fn agent_link(&self, root: &Path, name: &str, action: &str, args: Value) -> Result<Value> {
        if !matches!(action, "read" | "session" | "outcome") {
            return Err(Error::new("Unknown link action."));
        }
        check_size(&args, MAX_LINK_REQUEST, "Link request too large")?;
        // Renderer errors contain messages without engine error codes.
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
                let value = hwp_workbench::skill_install::run("status", "all", false)
                    .map_err(service_error)?;
                Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
            }
            ToolAction::Install { target } => {
                if !matches!(target.as_str(), "claude" | "codex") {
                    return Err(Error::new("Invalid skill target"));
                }
                let value = hwp_workbench::skill_install::run("install", &target, false)
                    .map_err(service_error)?;
                Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
            }
            ToolAction::McpConfig { client, root } => {
                if !matches!(client.as_str(), "claude-code" | "codex") {
                    return Err(Error::new("Invalid MCP client"));
                }
                let text =
                    hwp_workbench::mcp_config::setup(&client, root).map_err(service_error)?;
                Ok(format!("{text}\n"))
            }
            ToolAction::BuildBundle => {
                let output = self.bundle_path()?;
                hwp_workbench::mcpb::build(&std::env::current_exe()?, &output)
                    .map_err(service_error)?;
                Ok(format!("{}\n", output.display()))
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::{SerializeSeq, Serializer};
    use serde_json::json;
    use std::cell::Cell;

    #[test]
    fn size_check_accepts_the_limit_and_refuses_one_byte_over() {
        for value in [
            json!(null),
            json!("한글\n\""),
            json!({"args":[],"method":"inspect"}),
        ] {
            let length = serde_json::to_vec(&value).unwrap().len();
            check_size(&value, length, "Response too large").unwrap();
            assert_eq!(
                check_size(&value, length - 1, "Response too large")
                    .unwrap_err()
                    .message,
                "Response too large"
            );
        }
    }

    #[test]
    fn size_check_stops_serializing_as_soon_as_the_limit_is_passed() {
        struct CountedSequence<'a>(&'a Cell<usize>);
        impl serde::Serialize for CountedSequence<'_> {
            fn serialize<S: Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                let mut sequence = serializer.serialize_seq(Some(100))?;
                for _ in 0..100 {
                    self.0.set(self.0.get() + 1);
                    sequence.serialize_element(&0)?;
                }
                sequence.end()
            }
        }
        let visited = Cell::new(0);
        let error = check_size(&CountedSequence(&visited), 5, "Request too large").unwrap_err();
        assert_eq!(error.message, "Request too large");
        assert_eq!(visited.get(), 3, "Do not serialize the remaining elements");
    }
    #[test]
    fn cancellation_after_a_completed_call_drops_both_results_and_errors() {
        for fail in [false, true] {
            let cancel = AtomicBool::new(false);
            let result = with_cancellation(&cancel, || {
                cancel.store(true, Ordering::SeqCst);
                if fail {
                    Err(Error::new("Read failed"))
                } else {
                    Ok(json!(true))
                }
            });
            assert_eq!(result.unwrap_err().message, "Preview dropped");
        }
    }
}
