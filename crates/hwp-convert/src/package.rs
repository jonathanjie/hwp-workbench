//! Conversion packages use the shared bounded ZIP inflater and safe XML tree.
use hwp_core::errors::{require, Result, WorkbenchError, MAX_EXPANDED, MAX_FILE, MAX_PART};
use hwp_core::package_io::{ZipPackage, MAX_MEMBERS};
use hwp_core::xml_io::{xml_tree, Element};
use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// Maximum columns or a single table span, preventing sparse-grid allocation.
pub const MAX_COLUMNS: usize = 1000;
/// Maximum rows times columns, bounding table grid work.
pub const MAX_GRID_WORK: usize = 1_000_000;
pub type Parts = BTreeMap<String, Vec<u8>>;
pub type Trees = BTreeMap<String, Element>;

pub fn read_package(payload: &[u8]) -> Result<(Parts, Trees)> {
    read_package_with_limits(payload, MAX_PART, MAX_EXPANDED)
}
fn read_package_with_limits(
    payload: &[u8],
    part_limit: usize,
    total_limit: usize,
) -> Result<(Parts, Trees)> {
    require(
        !payload.is_empty() && payload.len() <= MAX_FILE,
        "LIMIT",
        "File must be 1 byte to 64 MiB.",
    )?;
    let mut archive = ZipPackage::new(payload).map_err(|error| {
        if error.code == "UNSAFE_ZIP"
            && matches!(
                error.message.as_str(),
                "Unsafe package path." | "Duplicate ZIP member."
            )
        {
            WorkbenchError::new("UNSAFE_ZIP", "Unsafe or duplicate ZIP member.")
        } else {
            error
        }
    })?;
    // The core preservation archive does not reject symlink modes. Conversion
    // follows the stricter Python package policy before reading any payload.
    let mut metadata = ZipArchive::new(Cursor::new(payload)).map_err(bad_zip)?;
    for index in 0..metadata.len() {
        let member = metadata.by_index_raw(index).map_err(bad_zip)?;
        require(
            member
                .unix_mode()
                .is_none_or(|mode| mode & 0o170000 != 0o120000),
            "UNSAFE_ZIP",
            "ZIP symlinks are prohibited.",
        )?;
    }
    let names: Vec<String> = archive.names().map(str::to_owned).collect();
    let mut parts = Parts::new();
    let mut expanded = 0;
    for (index, name) in names.into_iter().enumerate() {
        require(
            !name.is_empty(),
            "UNSAFE_ZIP",
            "Unsafe or duplicate ZIP member.",
        )?;
        let bytes = archive.read_member(index, expanded, part_limit, total_limit)?;
        expanded += bytes.len(); // Each successful read checked the combined budget.
        parts.insert(name, bytes);
    }
    let mut trees = Trees::new();
    for (name, bytes) in &parts {
        if name.ends_with(".xml") || name.ends_with(".rels") || name.ends_with(".hpf") {
            trees.insert(name.clone(), xml_tree(bytes)?);
        }
    }
    Ok((parts, trees))
}
fn bad_zip(error: impl std::fmt::Display) -> WorkbenchError {
    WorkbenchError::new("BAD_ZIP", error.to_string())
}
pub fn write_package(parts: &Parts) -> Result<Vec<u8>> {
    let names: Vec<String> = parts
        .get_key_value("mimetype")
        .into_iter()
        .chain(parts.iter().filter(|(name, _)| name.as_str() != "mimetype"))
        .map(|(name, _)| name.clone())
        .collect();
    write_package_ordered(parts, &names)
}

/// Write a complete package with an explicit member order and the same bounds.
pub fn write_package_ordered(parts: &Parts, names: &[String]) -> Result<Vec<u8>> {
    let unique: std::collections::HashSet<_> = names.iter().collect();
    require(
        names.len() == parts.len()
            && unique.len() == parts.len()
            && names.iter().all(|name| parts.contains_key(name)),
        "BAD_ZIP",
        "Invalid output ZIP member order.",
    )?;

    require(
        parts.len() <= MAX_MEMBERS,
        "LIMIT",
        "Too many output ZIP members.",
    )?;
    let mut total = 0usize;
    for bytes in parts.values() {
        total = total.saturating_add(bytes.len());
        require(
            bytes.len() <= MAX_PART && total <= MAX_EXPANDED,
            "LIMIT",
            "Expanded output exceeds limits.",
        )?;
    }
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    // Fixed timestamps make output independent of clock and host timezone.
    let timestamp = DateTime::from_date_and_time(2026, 1, 1, 0, 0, 0).map_err(bad_zip)?;
    for name in names {
        let bytes = parts
            .get(name)
            .ok_or_else(|| WorkbenchError::new("BAD_ZIP", "Invalid output ZIP member order."))?;
        let compression = if name == "mimetype" {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        writer
            .start_file(
                name,
                SimpleFileOptions::default()
                    .last_modified_time(timestamp)
                    .compression_method(compression),
            )
            .map_err(bad_zip)?;
        writer.write_all(bytes).map_err(bad_zip)?;
    }
    let result = writer.finish().map_err(bad_zip)?.into_inner();
    require(result.len() <= MAX_FILE, "LIMIT", "Output exceeds 64 MiB.")?;
    Ok(result)
}
pub fn integer(value: Option<&str>, default: i64, minimum: i64, maximum: i64) -> Result<i64> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = python_integer(value)?;
    require(
        value >= minimum && value <= maximum,
        "LIMIT",
        "Document numeric attribute exceeds limits.",
    )?;
    Ok(value)
}
// Decimal-zero code points from the frozen oracle's Unicode 16.0.0 tables.
// Each start is followed by the other nine decimal digits in that script.
const DECIMAL_ZERO: &[u32] = &[
    0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66,
    0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
    0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0, 0xff10,
    0x104a0, 0x10d30, 0x10d40, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0,
    0x11650, 0x116c0, 0x116d0, 0x116da, 0x11730, 0x118e0, 0x11950, 0x11bf0, 0x11c50, 0x11d50,
    0x11da0, 0x11f50, 0x16130, 0x16a60, 0x16ac0, 0x16b50, 0x16d70, 0x1ccf0, 0x1d7ce, 0x1d7d8,
    0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0, 0x1e4f0, 0x1e5f1, 0x1e950, 0x1fbf0,
];
// Python rejects decimal strings above its default conversion limit, including
// leading zeroes. Keep that malformed-number behavior before range validation.
const MAX_INTEGER_DIGITS: usize = 4300;
fn decimal_digit(character: char) -> Option<u32> {
    if character.is_ascii_digit() {
        return character.to_digit(10);
    }
    let code = u32::from(character);
    let index = DECIMAL_ZERO
        .partition_point(|start| *start <= code)
        .checked_sub(1)?;
    let difference = code.checked_sub(*DECIMAL_ZERO.get(index)?)?;
    (difference < 10).then_some(difference)
}
fn python_integer(text: &str) -> Result<i64> {
    let invalid = || WorkbenchError::new("BAD_XML", "Invalid numeric document attribute.");
    let text = text.trim();
    let (negative, digits) = if let Some(text) = text.strip_prefix('-') {
        (true, text)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    let mut magnitude = Some(0i128);
    let mut previous_digit = false;
    let mut digit_count = 0usize;
    for character in digits.chars() {
        if character == '_' && previous_digit {
            previous_digit = false;
            continue;
        }
        let digit = decimal_digit(character).ok_or_else(invalid)?;
        digit_count += 1;
        if digit_count > MAX_INTEGER_DIGITS {
            return Err(invalid());
        }
        previous_digit = true;
        magnitude = magnitude
            .and_then(|value| value.checked_mul(10))
            .and_then(|value| value.checked_add(i128::from(digit)));
    }
    if !previous_digit {
        return Err(invalid());
    }
    let value = magnitude
        .map(|value| if negative { -value } else { value })
        .and_then(|value| i64::try_from(value).ok());
    value.ok_or_else(|| WorkbenchError::new("LIMIT", "Document numeric attribute exceeds limits."))
}

pub fn member_target(part: &str, target: &str) -> Result<String> {
    require(
        !target.is_empty() && !target.contains(['\\', '\0', ':']),
        "UNSAFE_ZIP",
        "Unsafe internal relationship.",
    )?;
    require(
        !target.starts_with('/'),
        "UNSAFE_ZIP",
        "Relationship leaves package.",
    )?;
    let parent = part
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let mut components: Vec<&str> = parent
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                require(
                    components.pop().is_some(),
                    "UNSAFE_ZIP",
                    "Relationship leaves package.",
                )?;
            }
            other => components.push(other),
        }
    }
    Ok(if components.is_empty() {
        ".".into()
    } else {
        components.join("/")
    })
}

pub trait XmlExt {
    fn child(&self, namespace: &str, local: &str) -> Option<&Element>;
    fn path(&self, namespace: &str, locals: &[&str]) -> Option<&Element>;
    fn descendants(&self, namespace: &str, local: &str) -> Vec<&Element>;
    fn attr(&self, namespace: &str, local: &str) -> Option<&str>;
}
impl XmlExt for Element {
    fn child(&self, namespace: &str, local: &str) -> Option<&Element> {
        self.children.iter().find(|node| {
            node.name.namespace.as_deref() == Some(namespace) && node.name.local == local
        })
    }
    fn path(&self, namespace: &str, locals: &[&str]) -> Option<&Element> {
        let mut current = self;
        for local in locals {
            current = current.child(namespace, local)?;
        }
        Some(current)
    }
    fn descendants(&self, namespace: &str, local: &str) -> Vec<&Element> {
        self.iter()
            .filter(|node| {
                node.name.namespace.as_deref() == Some(namespace) && node.name.local == local
            })
            .collect()
    }
    fn attr(&self, namespace: &str, local: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| {
                attribute.name.namespace.as_deref() == Some(namespace)
                    && attribute.name.local == local
            })
            .map(|attribute| attribute.value.as_str())
    }
}

/// Small output tree with mixed text/tails for native line breaks and tabs.
#[derive(Clone, Debug)]
pub struct XmlNode {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub text: String,
    pub tail: String,
    pub children: Vec<XmlNode>,
}
impl XmlNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attributes: Vec::new(),
            text: String::new(),
            tail: String::new(),
            children: Vec::new(),
        }
    }
    pub fn attr(mut self, key: impl Into<String>, value: impl ToString) -> Self {
        self.set(key, value);
        self
    }
    pub fn set(&mut self, key: impl Into<String>, value: impl ToString) {
        let key = key.into();
        if let Some((_, existing)) = self.attributes.iter_mut().find(|(name, _)| name == &key) {
            *existing = value.to_string();
        } else {
            self.attributes.push((key, value.to_string()));
        }
    }
    pub fn child(mut self, child: XmlNode) -> Self {
        self.push(child);
        self
    }
    pub fn push(&mut self, child: XmlNode) {
        self.children.push(child);
    }
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }
}
fn escape(value: &str, attribute: bool) -> String {
    let mut escaped = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if attribute {
        escaped = escaped
            .replace('"', "&quot;")
            .replace('\r', "&#13;")
            .replace('\n', "&#10;")
            .replace('\t', "&#9;");
    }
    escaped
}
pub fn encoded(root: &XmlNode) -> Result<Vec<u8>> {
    fn serialize(node: &XmlNode, output: &mut String, depth: usize) -> Result<()> {
        require(
            depth <= hwp_core::xml_io::MAX_DEPTH,
            "LIMIT",
            "XML nesting exceeds limits.",
        )?;
        output.push('<');
        output.push_str(&node.name);
        for (key, value) in &node.attributes {
            output.push(' ');
            output.push_str(key);
            output.push_str("=\"");
            output.push_str(&escape(value, true));
            output.push('"');
        }
        if node.children.is_empty() && node.text.is_empty() {
            output.push_str(" />");
        } else {
            output.push('>');
            output.push_str(&escape(&node.text, false));
            for child in &node.children {
                serialize(child, output, depth + 1)?;
            }
            output.push_str("</");
            output.push_str(&node.name);
            output.push('>');
        }
        output.push_str(&escape(&node.tail, false));
        require(
            output.len() <= MAX_PART,
            "LIMIT",
            "XML part exceeds 32 MiB.",
        )
    }
    let mut output = String::from("<?xml version='1.0' encoding='utf-8'?>\n");
    serialize(root, &mut output, 1)?;
    Ok(output.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversion_package_uses_part_and_total_inflation_budgets() {
        let source =
            include_bytes!("../../../tests/data/convert_fixtures-docx-3bf7c69f1761e6c0.docx");
        assert_eq!(
            read_package_with_limits(source, 32, MAX_EXPANDED)
                .unwrap_err()
                .code,
            "LIMIT"
        );
        assert_eq!(
            read_package_with_limits(source, MAX_PART, 100)
                .unwrap_err()
                .code,
            "LIMIT"
        );
    }
    #[test]
    fn zip_symlink_metadata_is_rejected_before_mapping() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink("linked", "target", SimpleFileOptions::default())
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert_eq!(
            read_package(&bytes).unwrap_err().message,
            "ZIP symlinks are prohibited."
        );
    }
}
