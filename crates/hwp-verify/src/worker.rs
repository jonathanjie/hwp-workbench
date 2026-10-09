//! Byte-only bridge protocol. This module runs only in the child process.

use std::io::{self, Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};

use base64::prelude::*;
use rhwp::DocumentCore;
use serde_json::{json, Value};

use crate::{Error, MAX_DOCUMENT, MAX_JSON, MAX_PARAGRAPHS, RHWP_VERSION};

#[derive(Debug, PartialEq, Eq)]
pub enum WorkerStatus {
    Success,
    Failure,
}

/// Read one framed request, emit one JSON line, and return its exit status.
/// Parsing and rendering panics are converted to RHWP_FAILED responses.
pub fn run_worker(input: impl Read, mut output: impl Write) -> io::Result<WorkerStatus> {
    let result = catch_response(|| {
        let (request, payload) = read_request(input)?;
        dispatch(&request, &payload)
    });
    let (response, status) = match result {
        Ok(response) => (response, WorkerStatus::Success),
        Err(error) => (json!({"error": error}), WorkerStatus::Failure),
    };
    serde_json::to_writer(&mut output, &response)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(status)
}

fn catch_response(action: impl FnOnce() -> Result<Value, Error>) -> Result<Value, Error> {
    match catch_unwind(AssertUnwindSafe(action)) {
        Ok(result) => result,
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("rhwp panicked.");
            Err(Error::new("RHWP_FAILED", message))
        }
    }
}

fn bad_request(message: &str) -> Error {
    Error::new("BAD_REQUEST", message)
}

fn rhwp_error(error: impl std::fmt::Display) -> Error {
    Error::new("RHWP_FAILED", error.to_string())
}

fn read_request(input: impl Read) -> Result<(Value, Vec<u8>), Error> {
    // One extra byte distinguishes an oversized stream without reading it all.
    let limit = MAX_DOCUMENT + MAX_JSON + 4;
    let mut bytes = Vec::new();
    input
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(rhwp_error)?;
    if bytes.len() > limit {
        return Err(bad_request("Request exceeds the input limit."));
    }
    let header: [u8; 4] = bytes
        .get(..4)
        .and_then(|header| header.try_into().ok())
        .ok_or_else(|| bad_request("Missing JSON length header."))?;
    let length = usize::try_from(u32::from_le_bytes(header))
        .map_err(|_| bad_request("Invalid JSON frame length."))?;
    if length == 0 || length > MAX_JSON {
        return Err(bad_request("Invalid JSON frame length."));
    }
    let encoded = bytes
        .get(4..4 + length)
        .ok_or_else(|| bad_request("Invalid JSON frame length."))?;
    // TextDecoder in the JS bridge removes a leading UTF-8 BOM.
    let encoded = encoded.strip_prefix(b"\xef\xbb\xbf").unwrap_or(encoded);
    let request: Value =
        serde_json::from_slice(encoded).map_err(|error| bad_request(&error.to_string()))?;
    validate_request(&request)?;
    let payload = bytes
        .get(4 + length..)
        .ok_or_else(|| bad_request("Invalid JSON frame length."))?;
    if payload.is_empty() || payload.len() > MAX_DOCUMENT {
        return Err(bad_request("Document must be between 1 byte and 64 MiB."));
    }
    Ok((request, payload.to_vec()))
}

fn validate_request(request: &Value) -> Result<(), Error> {
    let object = request
        .as_object()
        .ok_or_else(|| bad_request("Expected a request object."))?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "op" | "page" | "format"))
    {
        return Err(bad_request("Unknown request field."));
    }
    let op = object.get("op").and_then(Value::as_str).unwrap_or("");
    if !matches!(op, "probe" | "render" | "export") {
        return Err(bad_request("Unknown operation."));
    }
    if op != "render" && object.contains_key("page") {
        return Err(bad_request("Page is only valid for render."));
    }
    if op != "export" && object.contains_key("format") {
        return Err(bad_request("Format is only valid for export."));
    }
    if op == "render" {
        let page = object.get("page").and_then(Value::as_f64);
        if !page.is_some_and(|page| page >= 0.0 && page.fract() == 0.0) {
            return Err(bad_request("Page must be a nonnegative integer."));
        }
    }
    if op == "export"
        && !matches!(
            object.get("format").and_then(Value::as_str),
            Some("hwp" | "hwpx")
        )
    {
        return Err(bad_request("Export format must be hwp or hwpx."));
    }
    Ok(())
}

fn dispatch(request: &Value, payload: &[u8]) -> Result<Value, Error> {
    // Native methods return HwpError. Calling a JsValue error adapter on a
    // native target can abort instead of returning an ordinary error.
    let document = DocumentCore::from_bytes(payload).map_err(rhwp_error)?;
    match request.get("op").and_then(Value::as_str) {
        Some("probe") => probe_document(&document),
        Some("render") => {
            let page = request
                .get("page")
                .and_then(Value::as_f64)
                .ok_or_else(|| bad_request("Page must be a nonnegative integer."))?;
            if page >= f64::from(document.page_count()) {
                return Err(bad_request("Page is outside the document."));
            }
            // Validation and the page-count check bound this cast to u32.
            let page = page as u32;
            let svg = document.render_page_svg_native(page).map_err(rhwp_error)?;
            Ok(json!({"page": page, "svg": svg}))
        }
        Some("export") => {
            let format = request
                .get("format")
                .and_then(Value::as_str)
                .ok_or_else(|| bad_request("Export format must be hwp or hwpx."))?;
            let exported = if format == "hwp" {
                document.export_hwp_with_adapter_snapshot_with_report()
            } else {
                document.export_hwpx_native_with_report()
            }
            .map_err(rhwp_error)?;
            let loss_report: Value =
                serde_json::from_str(&exported.content_loss().to_json()).map_err(rhwp_error)?;
            // SerializedDocument always owns its bytes, like hasBytes() before
            // the first takeBytes() in the JS bridge, even for an empty vector.
            Ok(
                json!({"format": format, "lossReport": loss_report, "bytesBase64": BASE64_STANDARD.encode(exported.bytes())}),
            )
        }
        _ => Err(bad_request("Unknown operation.")),
    }
}

fn collect_sections(document: &DocumentCore, limit: usize) -> Result<(Vec<Value>, bool), Error> {
    let mut sections = Vec::new();
    let mut count = 0;
    let mut truncated = false;
    for (section_index, section) in document.document().sections.iter().enumerate() {
        let mut paragraphs = Vec::new();
        for paragraph_index in 0..section.paragraphs.len() {
            if count == limit {
                truncated = true;
                break;
            }
            let length = document
                .get_paragraph_length_native(section_index, paragraph_index)
                .map_err(rhwp_error)?;
            let text = document
                .get_text_range_native(section_index, paragraph_index, 0, length)
                .map_err(rhwp_error)?;
            paragraphs.push(json!({"index": paragraph_index, "text": text}));
            count += 1;
        }
        sections.push(json!({"index": section_index, "paragraphs": paragraphs}));
        if truncated {
            break;
        }
    }
    Ok((sections, truncated))
}

fn probe_document(document: &DocumentCore) -> Result<Value, Error> {
    let (sections, truncated) = collect_sections(document, MAX_PARAGRAPHS)?;
    let full_text: Value =
        serde_json::from_str(&document.text_file_unicode_json()).map_err(rhwp_error)?;
    if !full_text.is_string() {
        return Err(bad_request("Unicode export must decode to text."));
    }
    let mut result = json!({"engine": "rhwp", "version": RHWP_VERSION, "pageCount": document.page_count(), "sections": sections, "fullText": full_text});
    if truncated {
        if let Some(object) = result.as_object_mut() {
            object.insert("truncated".into(), Value::Bool(true));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> DocumentCore {
        DocumentCore::from_bytes(include_bytes!(
            "../../../fixtures/generated/review-form.hwpx"
        ))
        .unwrap()
    }

    #[test]
    fn catches_parser_panics_as_rhwp_failed() {
        struct PanicReader;
        impl Read for PanicReader {
            fn read(&mut self, _output: &mut [u8]) -> io::Result<usize> {
                panic!("parser panic")
            }
        }
        let mut output = Vec::new();
        assert_eq!(
            run_worker(PanicReader, &mut output).unwrap(),
            WorkerStatus::Failure
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap(),
            json!({"error": {"code": "RHWP_FAILED", "message": "parser panic"}})
        );
    }

    #[test]
    fn marks_truncation_only_when_a_paragraph_was_omitted() {
        let document = document();
        let count = document
            .document()
            .sections
            .iter()
            .map(|section| section.paragraphs.len())
            .sum();
        let (complete, truncated) = collect_sections(&document, count).unwrap();
        assert!(!truncated);
        let (limited, truncated) = collect_sections(&document, 1).unwrap();
        assert!(truncated);
        assert_eq!(limited[0]["paragraphs"].as_array().unwrap().len(), 1);
        assert_eq!(limited[0]["paragraphs"][0], complete[0]["paragraphs"][0]);
    }

    #[test]
    fn rejects_invalid_request_shapes_and_fields() {
        for (request, message) in [
            (json!([]), "Expected a request object."),
            (json!({}), "Unknown operation."),
            (
                json!({"op": "probe", "format": null}),
                "Format is only valid for export.",
            ),
            (
                json!({"op": "probe", "page": null}),
                "Page is only valid for render.",
            ),
            (
                json!({"op": "render", "page": -1}),
                "Page must be a nonnegative integer.",
            ),
            (
                json!({"op": "render", "page": 0.5}),
                "Page must be a nonnegative integer.",
            ),
            (
                json!({"op": "render", "page": true}),
                "Page must be a nonnegative integer.",
            ),
            (
                json!({"op": "export", "format": "docx"}),
                "Export format must be hwp or hwpx.",
            ),
        ] {
            assert_eq!(
                validate_request(&request).unwrap_err(),
                bad_request(message)
            );
        }
        validate_request(&json!({"op": "render", "page": 0.0})).unwrap();
    }

    #[test]
    fn frame_size_and_utf8_policy_match_text_decoder() {
        for (input, message) in [
            (0_u32.to_le_bytes().to_vec(), "Invalid JSON frame length."),
            (4_u32.to_le_bytes().to_vec(), "Invalid JSON frame length."),
            (
                [14_u32.to_le_bytes().as_slice(), b"{\"op\":\"probe\"}"].concat(),
                "Document must be between 1 byte and 64 MiB.",
            ),
        ] {
            assert_eq!(
                read_request(input.as_slice()).unwrap_err(),
                bad_request(message)
            );
        }
        let encoded = b"\xef\xbb\xbf{\"op\":\"probe\"}";
        let input = [
            (encoded.len() as u32).to_le_bytes().as_slice(),
            encoded,
            b"x",
        ]
        .concat();
        assert_eq!(read_request(input.as_slice()).unwrap().1, b"x");
        let input = [1_u32.to_le_bytes().as_slice(), b"\xff", b"x"].concat();
        assert_eq!(
            read_request(input.as_slice()).unwrap_err().code,
            "BAD_REQUEST"
        );
    }
}
