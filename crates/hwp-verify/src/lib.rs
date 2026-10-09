//! Independent reopen, render and export checks through the rhwp library.
//!
//! Callers supply a trusted worker command. Document bytes never become paths
//! or command arguments. Only the worker process calls rhwp.

mod process;
pub mod worker;

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Map, Value};

// Keep these limits aligned with independent.py and rhwp-bridge.mjs.
pub const MAX_DOCUMENT: usize = 64 * 1024 * 1024; // Maximum source file bytes.
pub const MAX_OUTPUT: usize = 64 * 1024 * 1024; // Combined stdout and stderr bytes.
pub const MAX_JSON: usize = 4096; // Maximum request JSON frame bytes.
pub const MAX_PARAGRAPHS: usize = 100_000; // Maximum top-level probe paragraphs.
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
pub(crate) const CHUNK_SIZE: usize = 64 * 1024; // Fair pipe read/write chunks.
pub const RHWP_VERSION: &str = "0.8.7";

/// Local boundary error; the service maps this to its shared error type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl Error {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn check_details(&self) -> Value {
        // independent.py includes recovery fields in its opened:false result.
        let (hint, retryable) = if self.code == "LIMIT" {
            (
                "Reduce the file, request, pattern or result size to the documented limit.",
                true,
            )
        } else {
            ("Check the request and compatibility limits; report the code if the problem continues.", false)
        };
        json!({"code": self.code, "message": self.message, "hint": hint, "retryable": retryable})
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// A trusted executable plus fixed arguments, without shell interpretation.
/// An app can use its own executable and a hidden worker subcommand here.
#[derive(Clone, Debug)]
pub struct WorkerCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl WorkerCommand {
    pub fn new(program: impl Into<PathBuf>, args: Vec<OsString>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

pub fn invoke(
    payload: &[u8],
    op: &str,
    page: Option<u32>,
    format: Option<&str>,
    worker: &WorkerCommand,
) -> Result<Value, Error> {
    process::invoke_with_limits(
        payload,
        op,
        page,
        format,
        worker,
        process::Limits {
            timeout: TIMEOUT,
            max_output: MAX_OUTPUT,
        },
    )
}

pub fn probe(payload: &[u8], worker: &WorkerCommand) -> Result<Value, Error> {
    invoke(payload, "probe", None, None, worker)
}

pub fn render_page_svg(payload: &[u8], page: u32, worker: &WorkerCommand) -> Result<Value, Error> {
    invoke(payload, "render", Some(page), None, worker)
}

/// Explicit conversion export, never a preservation save operation.
pub fn export(payload: &[u8], format: &str, worker: &WorkerCommand) -> Result<Value, Error> {
    invoke(payload, "export", None, Some(format), worker)
}

/// Text-presence evidence only. Empty edits cannot confirm deletion.
/// Iteration order follows the caller's edit sequence, including missing IDs.
pub fn check_output<'a>(
    payload: &[u8],
    edited_texts: impl IntoIterator<Item = (&'a String, &'a String)>,
    worker: &WorkerCommand,
) -> Value {
    let result =
        probe(payload, worker).and_then(|observed| check_observed(&observed, edited_texts));
    match result {
        Ok(observed) => observed,
        Err(error) => {
            json!({"engine": "rhwp 0.8.7", "opened": false, "error": error.check_details()})
        }
    }
}

fn check_observed<'a>(
    observed: &Value,
    edited_texts: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> Result<Value, Error> {
    if observed.get("available") == Some(&Value::Bool(false)) {
        return Ok(observed.clone());
    }
    let malformed = || Error::new("INDEPENDENT_PROTOCOL", "Invalid rhwp probe response.");
    let sections = observed
        .get("sections")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    let mut paragraphs = Vec::new();
    for section in sections {
        let texts = section
            .get("paragraphs")
            .and_then(Value::as_array)
            .ok_or_else(malformed)?;
        for paragraph in texts {
            paragraphs.push(
                paragraph
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(malformed)?,
            );
        }
    }
    let full_text = observed
        .get("fullText")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = observed
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(malformed)?;
    let page_count = observed
        .get("pageCount")
        .filter(|count| count.is_u64())
        .ok_or_else(malformed)?;
    let mut methods = Map::new();
    let mut missing = Vec::new();
    let mut unverified = Vec::new();
    for (node_id, text) in edited_texts {
        let method = if text.is_empty() {
            unverified.push(node_id);
            "unverified-empty"
        } else if paragraphs
            .iter()
            .any(|paragraph| paragraph.contains(text.as_str()))
        {
            "paragraphs"
        } else if full_text.contains(text.as_str()) {
            "fullText"
        } else {
            missing.push(node_id);
            "missing"
        };
        methods.insert(node_id.clone(), json!(method));
    }
    let mut result = json!({"engine": format!("rhwp {version}"), "opened": true, "pageCount": page_count, "editedTextsFound": missing.is_empty() && unverified.is_empty(), "missing": missing, "editMethods": methods});
    if !unverified.is_empty() {
        if let Some(object) = result.as_object_mut() {
            object.insert("unverified".into(), json!(unverified));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod process_tests;
