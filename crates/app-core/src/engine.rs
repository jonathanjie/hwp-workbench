//! Shared desktop engine boundary for in-process native operations.
use crate::{Error, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub use crate::rust_engine::RustEngine;

pub type EngineError = Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolAction {
    Status,
    Install { target: String },
    McpConfig { client: String, root: PathBuf },
    BuildBundle,
}

pub trait Engine: Send + Sync {
    fn call(&self, root: &Path, method: &str, args: Value) -> Result<Value>;
    fn call_cancelled(
        &self,
        root: &Path,
        method: &str,
        args: Value,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value> {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::new("Preview dropped"));
        }
        self.call(root, method, args)
    }
    fn agent_link(&self, _root: &Path, _name: &str, _action: &str, _args: Value) -> Result<Value> {
        Err(Error::new("Agent link unavailable"))
    }
    fn tool(&self, _action: ToolAction) -> Result<String> {
        Err(Error::new("Agent tools unavailable"))
    }
}

/// Select the native engine shared by desktop shells.
pub fn desktop_engine(source_root: PathBuf) -> Arc<dyn Engine> {
    Arc::new(RustEngine::new(source_root))
}

/// Dispatch the native verification worker before a desktop host starts.
/// The CLI owns its protocol, exit codes and process streams.
pub fn worker_main() -> i32 {
    hwp_workbench::cli::main()
}
