//! Explicit conversion routes, loss evidence and exclusive output publication.
use base64::{engine::general_purpose::STANDARD, Engine};
use hwp_convert::{
    libreoffice, mapper_report, merge_unsupported, package::read_package, read_docx, read_hwpx,
    text_check, write_docx, write_hwpx,
};
use hwp_core::{
    cfb::{CompoundFile, MAGIC},
    document::{sha, Document, HP},
    errors::{require, Result, WorkbenchError, MAX_FILE},
    workspace::Workspace,
    xml_io::xml_tree,
};
use hwp_verify::WorkerCommand;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[path = "mapper_package.rs"]
mod mapper_package;

pub fn detect(payload: &[u8]) -> Result<&'static str> {
    if payload.starts_with(&MAGIC) {
        let compound = CompoundFile::new(payload)?;
        if compound
            .streams
            .get("FileHeader")
            .is_some_and(|bytes| bytes.starts_with(b"HWP Document File"))
        {
            return Ok("hwp");
        }
        require(
            compound.streams.contains_key("WordDocument"),
            "UNSUPPORTED_FORMAT",
            "Unsupported compound document.",
        )?;
        return Ok("doc");
    }
    if payload.starts_with(b"PK") {
        let (parts, _) = read_package(payload)?;
        let mime = parts
            .get("mimetype")
            .map(|bytes| bytes.trim_ascii())
            .unwrap_or_default();
        if mime == b"application/hwp+zip" {
            return Ok("hwpx");
        }
        if mime == b"application/vnd.oasis.opendocument.text" {
            return Ok("odt");
        }
        require(
            parts.contains_key("word/document.xml") && parts.contains_key("[Content_Types].xml"),
            "UNSUPPORTED_FORMAT",
            "Unsupported ZIP document.",
        )?;
        return Ok("docx");
    }
    if payload.trim_ascii_start().starts_with(b"{\\rtf") {
        return Ok("rtf");
    }
    Err(WorkbenchError::new(
        "UNSUPPORTED_FORMAT",
        "Conversion accepts HWP 5, HWPX, DOCX, DOC, ODT or RTF.",
    ))
}

fn section_number(name: &str) -> usize {
    name.rsplit("section")
        .next()
        .unwrap_or(name)
        .rsplit("Section")
        .next()
        .unwrap_or(name)
        .split('.')
        .next()
        .unwrap_or(name)
        .parse()
        .unwrap_or(0)
}

/// Read paragraph sequences, including native controls and real empty paragraphs.
pub fn native_texts(payload: &[u8]) -> Result<Vec<String>> {
    let document = Document::new(payload, "")?;
    if document.format == "hwp" {
        let mut sections: Vec<_> = document.hwp_sections.iter().collect();
        sections.sort_by_key(|(name, _)| section_number(name));
        let mut result = Vec::new();
        for (_, section) in sections {
            for paragraph in section.paragraphs.values() {
                let mut text = String::new();
                let mut table_carrier = false;
                for index in &paragraph.children {
                    let record = section.records.get(*index).ok_or_else(|| {
                        WorkbenchError::new(
                            "BAD_HWP_RECORD",
                            "Record payload exceeds stream bounds.",
                        )
                    })?;
                    if record.tag == 71 && record.data.get(..4) == Some(b" lbt") {
                        table_carrier = true;
                    }
                    if record.tag == 67 {
                        append_native_text(&record.data, &mut text)?;
                    }
                }
                if !table_carrier || !text.is_empty() {
                    result.push(text);
                }
            }
        }
        return Ok(result);
    }
    let mut grouped: BTreeMap<(&str, usize), String> = BTreeMap::new();
    let mut parts = BTreeSet::new();
    for node in &document.nodes {
        grouped
            .entry((&node.part, node.paragraph))
            .or_default()
            .push_str(&node.text);
        parts.insert(node.part.as_str());
    }
    let mut parts: Vec<_> = parts.into_iter().collect();
    parts.sort_by_key(|name| section_number(name));
    let mut result = Vec::new();
    for part in parts {
        let bytes = document
            .parts
            .get(part)
            .ok_or_else(|| WorkbenchError::new("NOT_HWPX", "Missing section part."))?;
        let root = xml_tree(bytes)?;
        for (index, paragraph) in root
            .iter()
            .filter(|element| {
                element.name.namespace.as_deref() == Some(HP) && element.name.local == "p"
            })
            .enumerate()
        {
            let text = grouped.remove(&(part, index + 1)).unwrap_or_default();
            let has_table = paragraph
                .children
                .iter()
                .filter(|run| run.name.expanded().ends_with("}run"))
                .flat_map(|run| &run.children)
                .any(|item| item.name.expanded().ends_with("}tbl"));
            if !has_table || !text.is_empty() {
                result.push(text);
            }
        }
    }
    Ok(result)
}

fn read_unit(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..offset.saturating_add(2))
        .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
        .ok_or_else(|| WorkbenchError::new("BAD_HWP_TEXT", "Misaligned UTF-16 text record."))?;
    Ok(u16::from_le_bytes(bytes))
}
fn append_native_text(data: &[u8], text: &mut String) -> Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let code = read_unit(data, offset)?;
        if code < 32 {
            // Native extended controls occupy eight UTF-16 units.
            let width = if matches!(code, 1..=9 | 11..=12 | 14..=23) {
                16
            } else {
                2
            };
            require(
                offset + width <= data.len(),
                "BAD_HWP_TEXT",
                "Truncated native text control.",
            )?;
            match code {
                9 => text.push('\t'),
                10 => text.push('\n'),
                24 => text.push('\u{ad}'),
                25 => text.push(' '),
                30 => text.push('\u{a0}'),
                31 => text.push('\u{2007}'),
                _ => {}
            }
            offset += width;
        } else {
            let mut units = Vec::new();
            while offset < data.len() && read_unit(data, offset)? >= 32 {
                units.push(read_unit(data, offset)?);
                offset += 2;
            }
            text.push_str(&String::from_utf16(&units).map_err(|_| {
                WorkbenchError::new("BAD_HWP_TEXT", "Invalid UTF-16 in paragraph text.")
            })?);
        }
    }
    Ok(())
}

fn native_unsupported(payload: &[u8]) -> Result<Map<String, Value>> {
    let document = Document::new(payload, "")?;
    if document.format == "hwpx" {
        return Ok(read_hwpx(payload)?.unsupported);
    }
    let mut counts = Map::new();
    if document.block_save {
        counts.insert("activeOrSignedContent".into(), json!(1));
    }
    for section in document.hwp_sections.values() {
        for record in &section.records {
            if record.tag != 71 {
                continue;
            }
            let Some(bytes) = record.data.get(..4) else {
                continue;
            };
            let control: Vec<u8> = bytes.iter().rev().copied().collect();
            let feature = match control.as_slice() {
                b"head" => "headers",
                b"foot" => "footers",
                b"fn  " => "footnotes",
                b"en  " => "endnotes",
                b"%hlk" | b"%dte" | b"%pag" => "fields",
                b"eqed" => "equations",
                b"gso " => "shapes",
                b"tcmt" => "comments",
                b"secd" | b"cold" | b"tbl " | b"atno" | b"nwno" => continue,
                _ => {
                    let id: String = control
                        .iter()
                        .map(|byte| {
                            if byte.is_ascii() {
                                char::from(*byte)
                            } else {
                                '\u{fffd}'
                            }
                        })
                        .collect();
                    increment(&mut counts, &format!("nativeControl:{id}"));
                    continue;
                }
            };
            increment(&mut counts, feature);
        }
    }
    Ok(counts)
}
fn increment(counts: &mut Map<String, Value>, feature: &str) {
    let count = counts.get(feature).and_then(Value::as_u64).unwrap_or(0);
    counts.insert(feature.into(), json!(count + 1));
}

fn export(
    payload: &[u8],
    target: &str,
    reports: &mut Vec<Value>,
    invoke: &mut impl FnMut(&[u8], &str) -> Result<Value>,
) -> Result<Option<Vec<u8>>> {
    let exported = invoke(payload, target)?;
    if exported.get("available") == Some(&Value::Bool(false)) {
        let mut report = json!({"engine":"rhwp"});
        if let (Some(report), Some(exported)) = (report.as_object_mut(), exported.as_object()) {
            report.extend(exported.clone());
        }
        reports.push(report);
        return Ok(None);
    }
    let encoded = exported
        .get("bytesBase64")
        .and_then(Value::as_str)
        .filter(|encoded| encoded.len() <= MAX_FILE.div_ceil(3) * 4)
        .ok_or_else(|| {
            WorkbenchError::new("INDEPENDENT_PROTOCOL", "Invalid or oversized rhwp export.")
        })?;
    let loss = exported
        .get("lossReport")
        .filter(|loss| loss.is_object() || loss.is_array())
        .ok_or_else(|| {
            WorkbenchError::new(
                "INDEPENDENT_PROTOCOL",
                "rhwp export omitted its loss report.",
            )
        })?;
    let result = STANDARD
        .decode(encoded)
        .map_err(|_| WorkbenchError::new("INDEPENDENT_PROTOCOL", "Invalid rhwp export bytes."))?;
    if target == "hwpx" {
        read_package(&result)?;
    }
    require(
        Document::new(&result, "")?.format == target,
        "VERIFY_FAILED",
        "rhwp exported the wrong format.",
    )?;
    reports.push(json!({"engine":"rhwp","to":target,"lossReport":loss,"scope":"rhwp export losses; does not cover mapper import losses or certify layout"}));
    Ok(Some(result))
}

pub fn convert(workspace: &Workspace, args: &Value) -> Result<Value> {
    convert_with_worker(workspace, args, &crate::service::worker_command()?)
}
pub fn convert_with_worker(
    workspace: &Workspace,
    args: &Value,
    worker: &WorkerCommand,
) -> Result<Value> {
    convert_with_backends(
        workspace,
        args,
        |payload, format| {
            hwp_verify::export(payload, format, worker)
                .map_err(|error| WorkbenchError::new(error.code, error.message))
        },
        libreoffice::convert,
    )
}

/// Trusted library seam for external backends; never exposed through RPC or MCP.
#[doc(hidden)]
pub fn convert_with_backends(
    workspace: &Workspace,
    args: &Value,
    mut export_backend: impl FnMut(&[u8], &str) -> Result<Value>,
    mut legacy_backend: impl FnMut(&[u8], &str, &str) -> Result<(Option<Vec<u8>>, Value)>,
) -> Result<Value> {
    let fields = args
        .as_object()
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Unknown conversion argument."))?;
    require(
        fields
            .keys()
            .all(|field| ["path", "to", "output", "acknowledgeRisks"].contains(&field.as_str())),
        "BAD_ARGUMENTS",
        "Unknown conversion argument.",
    )?;
    let target = fields
        .get("to")
        .and_then(Value::as_str)
        .filter(|target| matches!(*target, "hwp" | "hwpx" | "docx"))
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "to must be hwp, hwpx or docx."))?;
    require(
        fields.get("acknowledgeRisks").is_none_or(Value::is_boolean),
        "BAD_ARGUMENTS",
        "acknowledgeRisks must be a boolean.",
    )?;
    let output = if let Some(output) = fields.get("output") {
        let output = output
            .as_str()
            .filter(|output| {
                Path::new(output)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(target))
            })
            .ok_or_else(|| {
                WorkbenchError::new(
                    "OUTPUT_FORMAT",
                    "Conversion output must use the requested target extension.",
                )
            })?;
        workspace.relative_path(output)?;
        Some(output)
    } else {
        None
    };
    let path = fields
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkbenchError::new("BAD_PATH", "Invalid file path."))?;
    let original = workspace.read(path)?;
    let source = detect(&original)?;
    let mut content = original.clone();
    let mut current = source;
    let mut route = Vec::new();
    let mut reports = Vec::new();
    let mut unsupported = Map::new();
    let mut check_source = "source document";
    if matches!(source, "doc" | "odt" | "rtf") {
        route.push(format!("{source} → docx (LibreOffice)"));
        let (converted, report) = legacy_backend(&original, source, "docx")?;
        reports.push(report);
        let Some(converted) = converted else {
            return Ok(report_result(
                source,
                target,
                route,
                reports,
                unsupported,
                json!({"performed":false}),
                false,
            ));
        };
        content = converted;
        current = "docx";
        check_source = "LibreOffice DOCX intermediate; original legacy import losses cannot be independently measured";
        unsupported.insert("legacyImportUnverified".into(), json!(1));
    }
    let model = if current == "docx" {
        Some(read_docx(&content)?)
    } else {
        None
    };
    let before = if let Some(model) = &model {
        model.paragraph_texts()
    } else {
        unsupported.extend(native_unsupported(&content)?);
        native_texts(&content)?
    };
    if current == target {
        route.push(format!("{current} → {target} (validated byte copy)"));
        reports.push(
            json!({"engine":"byte copy","lossReport":{"losses":[]},"originalBytesRetained":true}),
        );
        unsupported.clear();
        if matches!(source, "doc" | "odt" | "rtf") {
            unsupported.insert("legacyImportUnverified".into(), json!(1));
        }
    } else {
        if current == "hwp" {
            route.push("hwp → hwpx (rhwp)".into());
            let Some(converted) = export(&content, "hwpx", &mut reports, &mut export_backend)?
            else {
                return Ok(report_result(
                    source,
                    target,
                    route,
                    reports,
                    unsupported,
                    json!({"performed":false}),
                    false,
                ));
            };
            content = converted;
            current = "hwpx";
        }
        if current == "docx" {
            route.push("docx → hwpx (direct mapper)".into());
            let model = model
                .as_ref()
                .ok_or_else(|| WorkbenchError::new("NOT_DOCX", "Missing DOCX model."))?;
            unsupported.extend(model.unsupported.clone());
            content = mapper_package::normalize(&write_hwpx(model)?, "hwpx")?;
            reports.push(mapper_report("docx", "hwpx", model));
        }
        if target == "docx" {
            route.push("hwpx → docx (direct mapper)".into());
            let model = read_hwpx(&content)?;
            merge_unsupported(&mut unsupported, &model.unsupported);
            content = mapper_package::normalize(&write_docx(&model)?, "docx")?;
            reports.push(mapper_report("hwpx", "docx", &model));
        } else if target == "hwp" {
            route.push("hwpx → hwp (rhwp)".into());
            let Some(converted) = export(&content, "hwp", &mut reports, &mut export_backend)?
            else {
                return Ok(report_result(
                    source,
                    target,
                    route,
                    reports,
                    unsupported,
                    json!({"performed":false}),
                    false,
                ));
            };
            content = converted;
        }
    }
    let after = if target == "docx" {
        read_docx(&content)?.paragraph_texts()
    } else {
        native_texts(&content)?
    };
    let mut result = report_result(
        source,
        target,
        route,
        reports,
        unsupported,
        text_check(&before, &after, check_source),
        true,
    );
    if let Some(output) =
        output.filter(|_| fields.get("acknowledgeRisks") == Some(&Value::Bool(true)))
    {
        require(
            sha(&workspace.read(path)?) == sha(&original),
            "STALE_DOCUMENT",
            "Source changed during conversion; output was not published.",
        )?;
        workspace.write_new(output, &content)?;
        if let Some(result) = result.as_object_mut() {
            result.insert("output".into(), json!(output));
            result.insert("outputSha256".into(), json!(sha(&content)));
        }
    }
    Ok(result)
}
fn report_result(
    source: &str,
    target: &str,
    route: Vec<String>,
    reports: Vec<Value>,
    unsupported: Map<String, Value>,
    text_check: Value,
    available: bool,
) -> Value {
    json!({"from":source,"to":target,"route":route,"engineReports":reports,"textCheck":text_check,"unsupported":unsupported,"layoutCertified":false,"available":available})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_text_controls_retain_the_readable_sequence() {
        let mut bytes = Vec::new();
        for code in [65u16, 9, 10, 24, 25, 30, 31, 13, 66] {
            bytes.extend_from_slice(&code.to_le_bytes());
            if code == 9 {
                bytes.extend_from_slice(&[0; 14]);
            }
        }
        let mut text = String::new();
        append_native_text(&bytes, &mut text).unwrap();
        assert_eq!(text, "A\t\n\u{ad} \u{a0}\u{2007}B");
        let error = append_native_text(&9u16.to_le_bytes(), &mut String::new()).unwrap_err();
        assert_eq!(error.code, "BAD_HWP_TEXT");
        assert_eq!(error.message, "Truncated native text control.");
    }
}
