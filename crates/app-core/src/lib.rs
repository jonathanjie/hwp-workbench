//! Desktop operations shared by the native shell and the renderer test host.
//! Renderer paths never grant access: only OS dialogs, handoffs and drops do.
mod backend;
pub mod engine;
pub mod files;
pub mod inbox;
pub mod preview;
pub mod queue;
pub mod recent;
pub mod tools;

pub use engine::Engine;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug, serde::Serialize)]
pub struct Error {
    pub code: Option<String>,
    pub message: String,
}
impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }
    pub fn coded(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: Some(code.into()),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}
pub fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| Error::new("Desktop state unavailable"))
}

pub trait Dialogs: Send + Sync {
    fn open_file(&self) -> Result<Option<PathBuf>>;
    fn save_file(&self, default: &Path, extensions: &[String]) -> Result<Option<PathBuf>>;
    fn message(&self, message: &str, buttons: &[&str], default: usize) -> Result<usize>;
}
pub trait Events: Send + Sync {
    fn emit(&self, event: &str, payload: Value) -> Result<()>;
    fn focus(&self) -> Result<()> {
        Ok(())
    }
    fn key_scheme(&self, _scheme: &str) -> Result<()> {
        Ok(())
    }
}
pub trait Shell: Send + Sync {
    fn reveal(&self, file: &Path) -> Result<()>;
    fn copy(&self, text: &str) -> Result<()>;
}
#[derive(Clone, Debug)]
pub struct Session {
    pub file: PathBuf,
    pub root: PathBuf,
    pub name: String,
    pub format: String,
    pub sha256: String,
    pub recent_id: Option<String>,
    pub base_bytes: Vec<u8>,
}

pub(crate) use backend::check_arguments;
pub use backend::{open_argument, validate_patch, Backend, PreviewRequest, COMMANDS};
