//! Shared document operations for the CLI, MCP and desktop RPC.
use base64::prelude::*;
use hwp_core::document::{patch_input, sha, Document};
use hwp_core::errors::{require, Result, WorkbenchError};
use hwp_core::workspace::Workspace;
use hwp_verify::WorkerCommand;
use serde_json::{json, Value};
use std::path::Path;

use crate::{agent_link, agent_ops, convert_operation};

/// Maximum native bytes returned by the desktop-only memory preview (64 MiB).
pub const MAX_PREVIEW_BYTES: usize = 64 * 1024 * 1024;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const OPERATIONS: &[&str] = &[
    "inspect", "extract", "validate", "plan", "apply", "save", "read", "search", "replace", "fill",
    "set", "propose", "session", "convert",
];

pub fn capabilities() -> Value {
    json!({
        "version": VERSION,
        "formats": {
            "hwp": "HWP 5.x: existing plain text runs; variable length; Hancom reflow unverified",
            "hwpx": "Existing plain text nodes; variable length; same-format save"
        },
        "operations": OPERATIONS,
        "notImplemented": ["native full-page rendering", "formatting and structural edits",
            "new document authoring", "PDF export", "comments/track changes", "full Hancom fidelity"],
        "conversion": {"targets":["hwp","hwpx","docx"],"pivot":"hwpx","lossy":true,
            "nativeBackend":"native rhwp", "legacyImportBackend":"optional LibreOffice",
            "dryRunUnlessOutputAndAcknowledgement":true},
        "safety": {"overwrite":"save operation only, verified, with backup",
            "externalNetwork":false,"execDocumentCode":false,"workspaceRestricted":true,
            "requireRiskAcknowledgement":true}
    })
}

/// The trusted worker is the current CLI or app executable. The app must also
/// dispatch this hidden command before it starts its GUI runtime.
pub fn worker_command() -> Result<WorkerCommand> {
    let executable = std::env::current_exe()
        .map_err(|error| WorkbenchError::new("REQUEST_FAILED", error.to_string()))?;
    Ok(WorkerCommand::new(executable, vec!["__rhwp-worker".into()]))
}

pub fn verification_error(error: hwp_verify::Error) -> WorkbenchError {
    WorkbenchError::new(error.code, error.message)
}

// This private boundary permits failure injection without environment switches
// or test-only configuration in the public IPC contract.
trait OperationBackend {
    fn serialize(&self, document: &Document) -> Result<Vec<u8>> {
        document.serialize(true)
    }
    fn independent_check(&self, bytes: &[u8], document: &Document) -> Value;
    fn before_backup(&self, _workspace: &Workspace, _path: &str) -> Result<()> {
        Ok(())
    }
    fn backup(
        &self,
        workspace: &Workspace,
        path: &str,
        document: &Document,
        directory: Option<&str>,
    ) -> Result<String> {
        workspace.backup_existing(path, &document.original, directory)
    }
    fn replace(
        &self,
        workspace: &Workspace,
        path: &str,
        bytes: &[u8],
        source_hash: &str,
    ) -> Result<String> {
        workspace.replace_existing(path, bytes, source_hash)
    }
    fn preview_limit(&self) -> usize {
        MAX_PREVIEW_BYTES
    }
}

struct NativeBackend;
impl OperationBackend for NativeBackend {
    fn independent_check(&self, bytes: &[u8], document: &Document) -> Value {
        let reader_edits: Vec<_> = document
            .edits
            .iter()
            .map(|(id, text)| (id.clone(), document.reader_text(id, text)))
            .collect();
        match worker_command() {
            Ok(worker) => hwp_verify::check_output(
                bytes,
                reader_edits.iter().map(|(id, text)| (id, text)),
                &worker,
            ),
            Err(error) => json!({"engine":"rhwp 0.8.7","opened":false,"error":error.as_json()}),
        }
    }
}

fn extension_matches(path: &str, format: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.to_lowercase() == format)
}

fn publish(
    workspace: &Workspace,
    path: &str,
    document: &Document,
    args: &Value,
    mut result: Value,
    backend: &impl OperationBackend,
) -> Result<Value> {
    let output = args.get("output").and_then(Value::as_str);
    require(
        output.is_some_and(|path| extension_matches(path, &document.format)),
        "OUTPUT_FORMAT",
        "Output must use the same native extension as the source.",
    )?;
    require(
        args.get("acknowledgeRisks") == Some(&Value::Bool(true)),
        "RISK_ACK_REQUIRED",
        "Pass acknowledgeRisks: true to save edited output.",
    )?;
    let output = output.ok_or_else(|| {
        WorkbenchError::new(
            "OUTPUT_FORMAT",
            "Output must use the same native extension as the source.",
        )
    })?;
    let content = backend.serialize(document)?;
    let independent_check = backend.independent_check(&content, document);
    require(
        sha(&workspace.read(path)?) == document.hash,
        "STALE_DOCUMENT",
        "Source changed during processing; output was not published.",
    )?;
    workspace.write_new(output, &content)?;
    let fields = result.as_object_mut().ok_or_else(|| {
        WorkbenchError::new("REQUEST_FAILED", "Operation result must be an object.")
    })?;
    fields.insert("output".into(), json!(output));
    fields.insert("outputSha256".into(), json!(sha(&content)));
    fields.insert("reopenedAndVerified".into(), json!(true));
    fields.insert("sourceUntouched".into(), json!(true));
    fields.insert("independentCheck".into(), independent_check);
    Ok(result)
}

fn nonempty(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::Bool(true)) => true,
    }
}

fn save(
    workspace: &Workspace,
    path: &str,
    document: &mut Document,
    args: &Value,
    backend: &impl OperationBackend,
) -> Result<Value> {
    require(
        extension_matches(path, &document.format),
        "OUTPUT_FORMAT",
        "Save must retain the source native extension and format.",
    )?;
    require(
        args.get("acknowledgeRisks") == Some(&Value::Bool(true)),
        "RISK_ACK_REQUIRED",
        "Pass acknowledgeRisks: true to replace the opened document.",
    )?;
    require(
        args.get("output").is_none(),
        "BAD_ARGUMENTS",
        "Save replaces path; use apply for a new output.",
    )?;
    document.patch(patch_input(args.get("patch").unwrap_or(&Value::Null)))?;
    let content = backend.serialize(document)?;
    require(
        Document::new(&content, &document.name)?.format == document.format,
        "VERIFY_FAILED",
        "Verified output must keep the source native format.",
    )?;
    let independent_check = backend.independent_check(&content, document);
    require(
        independent_check.get("opened") != Some(&Value::Bool(false))
            && independent_check.get("error").is_none()
            && !nonempty(independent_check.get("missing")),
        "VERIFY_FAILED",
        "Independent reader failed to reopen the output or reported missing edited text.",
    )?;
    // Empty substring searches cannot prove a deletion; native self-reparse
    // verifies that edit, while the independent uncertainty remains explicit.
    require(
        independent_check.get("editedTextsFound") != Some(&Value::Bool(false))
            || nonempty(independent_check.get("unverified")),
        "VERIFY_FAILED",
        "Independent reader could not find the edited text.",
    )?;
    backend.before_backup(workspace, path)?;
    require(
        sha(&workspace.read(path)?) == document.hash,
        "SOURCE_CHANGED",
        "Document changed during verification; no replacement was made.",
    )?;
    let backup_directory = args.get("backupDirectory").filter(|value| !value.is_null());
    require(
        backup_directory.is_none_or(Value::is_string),
        "BAD_PATH",
        "Invalid file path.",
    )?;
    let backup = backend.backup(
        workspace,
        path,
        document,
        backup_directory.and_then(Value::as_str),
    )?;
    let saved_hash = backend.replace(workspace, path, &content, &document.hash)?;
    Ok(
        json!({"saved":true,"path":path,"previousSha256":document.hash,"sha256":saved_hash,"backup":backup,"independentCheck":independent_check,"warnings":document.warnings}),
    )
}

fn agent_edit(document: &Document, method: &str, args: &Value) -> Result<Value> {
    match method {
        "replace" => agent_ops::replace(
            document,
            args.get("find").unwrap_or(&Value::Null),
            args.get("replaceWith").unwrap_or(&Value::Null),
            args.get("regex").unwrap_or(&Value::Bool(false)),
            args.get("caseSensitive").unwrap_or(&Value::Bool(true)),
            args.get("nodeIds").filter(|value| !value.is_null()),
            args.get("limit").unwrap_or(&json!(1000)),
        ),
        "fill" => agent_ops::fill(document, args.get("fields").unwrap_or(&Value::Null)),
        _ => agent_ops::set_text(
            document,
            args.get("nodeId").unwrap_or(&Value::Null),
            args.get("text").unwrap_or(&Value::Null),
        ),
    }
}

pub fn invoke(root: impl AsRef<Path>, method: &str, args: &Value) -> Result<Value> {
    invoke_with_backend(root, method, args, &NativeBackend)
        .map_err(|error| source_error(error, args))
}

// Report descriptor-relative OS failures with the source leaf name.
// Keep that public text without changing the shared workspace's error codes.
fn source_error(mut error: WorkbenchError, args: &Value) -> WorkbenchError {
    if error.code == "READ_FAILED" {
        if let Some((message, number)) = error.message.rsplit_once(" (os error ") {
            if let (Some(number), Some(path)) = (
                number.strip_suffix(')'),
                args.get("path").and_then(Value::as_str),
            ) {
                if number.parse::<i32>().is_ok() {
                    let leaf = Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path);
                    let quote = if leaf.contains('\'') && !leaf.contains('"') {
                        '"'
                    } else {
                        '\''
                    };
                    let escaped = leaf
                        .replace('\\', "\\\\")
                        .replace(quote, &format!("\\{quote}"));
                    error.message = format!("[Errno {number}] {message}: {quote}{escaped}{quote}");
                }
            }
        }
    }
    error
}

fn inspect(document: &Document) -> Value {
    let mut result = document.inspect();
    if let Some(nodes) = result.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            if let Some(style) = node.get_mut("style").and_then(Value::as_object_mut) {
                // Keep the integer representation at a clamp bound.
                if let Some(size) = style.get("size").and_then(Value::as_f64) {
                    if size == 5.0 {
                        style.insert("size".into(), json!(5));
                    } else if size == 96.0 {
                        style.insert("size".into(), json!(96));
                    }
                }
            }
        }
    }
    result
}

fn invoke_with_backend(
    root: impl AsRef<Path>,
    method: &str,
    args: &Value,
    backend: &impl OperationBackend,
) -> Result<Value> {
    require(
        args.is_object(),
        "BAD_ARGUMENTS",
        "Arguments must be an object.",
    )?;
    if method == "capabilities" {
        return Ok(capabilities());
    }
    require(
        OPERATIONS.contains(&method) || matches!(method, "preview" | "format"),
        "UNKNOWN_METHOD",
        &format!("Unknown operation: {method}"),
    )?;
    let workspace = Workspace::new(root)?;
    if method == "convert" {
        return convert_operation::convert(&workspace, args);
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkbenchError::new("BAD_PATH", "Invalid file path."))?;
    let source = workspace.read(path)?;
    if method == "save" {
        let patch = patch_input(args.get("patch").unwrap_or(&Value::Null));
        require(
            patch.is_object()
                && patch.get("schemaVersion").is_some_and(|value| {
                    value.as_f64() == Some(1.0) || value.as_bool() == Some(true)
                }),
            "BAD_PATCH",
            "Expected patch schemaVersion 1.",
        )?;
        require(
            patch.get("baseSha256").and_then(Value::as_str) == Some(sha(&source).as_str()),
            "SOURCE_CHANGED",
            "Document differs from the opened revision; read it again before saving.",
        )?;
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let mut document = Document::new(&source, name)?;
    match method {
        "format" => {
            let node = args.get("nodeId");
            require(
                node.is_none_or(|value| value.is_null() || value.is_string()),
                "BAD_ARGUMENTS",
                "Unknown text node.",
            )?;
            return hwp_core::format_readout::read_format(&document, node.and_then(Value::as_str));
        }
        "session" => return agent_link::session(&workspace, path, &document),
        "propose" => return agent_link::propose(&workspace, path, &mut document, args),
        "inspect" => {
            let compact = args.get("compact").unwrap_or(&Value::Bool(false));
            require(
                compact.is_boolean(),
                "BAD_ARGUMENTS",
                "compact must be a boolean.",
            )?;
            let mut result = inspect(&document);
            if compact.as_bool() == Some(true) {
                if let Some(fields) = result.as_object_mut() {
                    fields.remove("view");
                    fields.remove("parts");
                }
            }
            return Ok(result);
        }
        "read" => {
            return agent_ops::read(
                &document,
                args.get("offset").unwrap_or(&json!(0)),
                args.get("limit").unwrap_or(&json!(200)),
            )
        }
        "search" => {
            return agent_ops::search(
                &document,
                args.get("query").unwrap_or(&Value::Null),
                args.get("regex").unwrap_or(&Value::Bool(false)),
                args.get("caseSensitive").unwrap_or(&Value::Bool(false)),
                args.get("limit").unwrap_or(&json!(100)),
            )
        }
        "extract" => {
            return Ok(
                json!({"format":document.format,"sha256":document.hash,"text":document.nodes.iter().map(|node|node.text.as_str()).collect::<Vec<_>>().join("\n"),"warnings":document.warnings}),
            )
        }
        "validate" => {
            return Ok(
                json!({"valid":true,"scope":"bounded container/XML/record parsing only; not Hancom rendering certification","sha256":document.hash,"format":document.format,"textNodes":document.nodes.len(),"warnings":document.warnings}),
            )
        }
        "save" => return save(&workspace, path, &mut document, args, backend),
        "replace" | "fill" | "set" => {
            let mut result = agent_edit(&document, method, args)?;
            let patch = result
                .get("patch")
                .ok_or_else(|| WorkbenchError::new("BAD_PATCH", "Missing generated patch."))?;
            let has_changes = patch
                .get("operations")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty());
            if has_changes {
                document.patch(patch)?;
            }
            if let (Some(fields), Some(diff)) =
                (result.as_object_mut(), document.diff().as_object())
            {
                fields.extend(diff.clone());
            }
            if args.get("output").is_none() {
                return Ok(result);
            }
            require(
                has_changes,
                "BAD_PATCH",
                "No editable changes were selected; no output was written.",
            )?;
            return publish(&workspace, path, &document, args, result, backend);
        }
        _ => {}
    }
    document.patch(patch_input(args.get("patch").unwrap_or(&Value::Null)))?;
    let result = document.diff();
    if method == "plan" {
        return Ok(result);
    }
    if method == "preview" {
        let content = backend.serialize(&document)?;
        require(
            content.len() <= backend.preview_limit(),
            "LIMIT",
            "Preview exceeds 64 MiB.",
        )?;
        require(
            sha(&workspace.read(path)?) == document.hash,
            "STALE_DOCUMENT",
            "Source changed during preview; reopen the document.",
        )?;
        return Ok(
            json!({"sha256":sha(&content),"bytesBase64":BASE64_STANDARD.encode(content),"warnings":document.warnings}),
        );
    }
    publish(&workspace, path, &document, args, result, backend)
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
