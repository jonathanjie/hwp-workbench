//! Stable errors, recovery actions and parsing limits shared by engine callers.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Maximum source or output file size (64 MiB).
pub const MAX_FILE: usize = 64 * 1024 * 1024;
/// Maximum total inflated payload (128 MiB).
pub const MAX_EXPANDED: usize = 128 * 1024 * 1024;
/// Maximum inflated part size (32 MiB).
pub const MAX_PART: usize = 32 * 1024 * 1024;
/// Maximum document text node count.
pub const MAX_NODES: usize = 100_000;
/// Codes that indicate a malformed native document.
pub const MALFORMED_DOCUMENT_CODES: &[&str] = &[
    "BAD_CFB",
    "BAD_HWP_COMPRESSION",
    "BAD_HWP_RECORD",
    "BAD_HWP_TEXT",
    "BAD_XML",
    "BAD_ZIP",
    "NOT_DOCX",
    "NOT_HWP",
    "NOT_HWPX",
    "UNSAFE_XML",
    "UNSAFE_ZIP",
    "XML_ENCODING",
];
/// Codes that indicate an unsupported or protected document.
pub const UNSUPPORTED_DOCUMENT_CODES: &[&str] =
    &["ENCRYPTED", "PROTECTED_HWP", "UNSUPPORTED_FORMAT"];
/// Engine result carrying a stable error code.
pub type Result<T> = std::result::Result<T, WorkbenchError>;
/// A corrective hint; retryable means retry after the correction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    pub hint: String,
    pub retryable: bool,
}
/// Return the Python oracle recovery hint and retry policy.
pub fn recovery(code: &str) -> Recovery {
    let (hint, retryable) = match code {
 "SOURCE_CHANGED" => ("Read or inspect the document again and rebuild the patch for its current SHA-256 before saving.", true),
 "READ_ONLY_DOCUMENT" => ("Use a writable source or apply the patch to a new output file.", true),
 "STALE_DOCUMENT" => ("Re-run read or inspect on the source and rebuild the patch with its current SHA-256.", true),
 "STALE_TEXT" => ("Re-run read or inspect and rebuild the patch with the current expectedText.", true),
 "NODE_NOT_FOUND" => ("Re-run read or search and use a full node ID from the current source.", true),
 "HWP_LENGTH_CHANGE_BLOCKED" => ("Use replacement text with the same UTF-16 code-unit length as the original node.", true),
 "SKILL_NOT_OWNED" => ("Keep the existing skill folder; install only into an absent or application-marked folder.", true),
 "OUTPUT_EXISTS" => ("Choose a new output filename; existing files cannot be replaced.", true),
 "OUTPUT_FORMAT" => ("Use the required extension: the source format for native save, or the requested target for conversion.", true),
 "RISK_ACK_REQUIRED" => ("Review the patch and compatibility warnings, then pass acknowledgeRisks: true to save.", true),
 "BAD_PATCH" => ("Rebuild a schemaVersion-1 patch with the current hash, expectedText and 1–1000 unique targets.", true),
 "BAD_ARGUMENTS" => ("Correct the arguments using CLI help or the MCP tools/list input schema.", true),
 "BAD_REGEX" => ("Correct the regex syntax, or use a literal search with regex: false.", true),
 "REGEX_LIMIT" => ("Use a regex pattern of at most 256 characters, or use literal search.", true),
 "REGEX_UNSUPPORTED" => ("Use literals, character classes, escapes, dots and anchors only, or use literal search.", true),
 "CONTROL_TEXT" => ("Use plain text without tabs, newlines, native controls or invalid Unicode characters.", true),
 "BAD_PATH" => ("Supply a nonempty regular-file path inside the configured workspace.", true),
 "BAD_ROOT" => ("Choose an existing document subfolder as the workspace root, such as ~/Documents/HWP.", true),
 "OPEN_FORMAT" => ("Choose an existing regular .hwp or .hwpx file inside the approved workspace.", true),
 "APP_UNAVAILABLE" => ("Install HWP Workbench.app in /Applications or ~/Applications, or install Electron in this source checkout.", true),
 "OPEN_FAILED" => ("Check the desktop app or Electron installation, then retry opening the document.", true),
 "OUTSIDE_WORKSPACE" => ("Use a path inside the configured workspace; relative traversal is prohibited.", true),
 "UNSAFE_PATH" => ("Use an existing parent directory and regular file path without symlink components.", true),
 "READ_FAILED" => ("Check that the source exists inside the workspace and is readable without symlinks.", true),
 "WRITE_FAILED" => ("Check output directory permissions and free space, then use a new output filename.", true),
 "LIMIT" => ("Reduce the file, request, pattern or result size to the documented limit.", true),
 "UNKNOWN_METHOD" => ("Use an operation listed by capabilities.", true),
 "REQUEST_FAILED" => ("Check the request JSON, required fields, workspace paths and file permissions, then retry.", true),
 "SAVE_BLOCKED" => ("Read the warnings and use an editable source without protected or active content.", false),
 "UNSUPPORTED_ELEMENT" => ("Select an editable plain-text node from read or inspect.", true),
 "VERIFY_FAILED" => ("Keep the original file and report the verification failure; do not publish the output.", false),
"BAD_CFB" | "BAD_HWP_COMPRESSION" | "BAD_HWP_RECORD" | "BAD_HWP_TEXT" | "BAD_XML" | "BAD_ZIP" | "NOT_DOCX" | "NOT_HWP" | "NOT_HWPX" | "UNSAFE_XML" | "UNSAFE_ZIP" | "XML_ENCODING" => ("Use an intact supported HWP/HWPX source; keep this file unchanged and report the parse failure.", false),
"ENCRYPTED" | "PROTECTED_HWP" | "UNSUPPORTED_FORMAT" => ("Use a supported, unprotected HWP 5.x or HWPX document; inspect the compatibility limits.", false),
_ => ("Check the request and compatibility limits; report the code if the problem continues.", false),
};
    Recovery {
        hint: hint.to_owned(),
        retryable,
    }
}
/// A stable engine error with optional internal context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkbenchError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}
impl WorkbenchError {
    /// Create an error without extra context.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }
    /// Serialize the exact code/message/hint/retryable detail used by CLI and MCP.
    pub fn as_json(&self) -> Value {
        let action = recovery(&self.code);
        serde_json::json!({"code": self.code, "message": self.message, "hint": action.hint, "retryable": action.retryable})
    }
}
impl fmt::Display for WorkbenchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for WorkbenchError {}
/// Serialize engine errors or classify a generic failure as REQUEST_FAILED.
pub fn error_details(error: &(dyn std::error::Error + 'static)) -> Value {
    if let Some(error) = error.downcast_ref::<WorkbenchError>() {
        error.as_json()
    } else {
        WorkbenchError::new("REQUEST_FAILED", error.to_string()).as_json()
    }
}
/// Require a condition or return a stable engine error.
pub fn require(condition: bool, code: &str, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(WorkbenchError::new(code, message))
    }
}
