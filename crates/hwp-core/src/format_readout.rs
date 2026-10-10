//! Read native HWPX formatting without conversion defaults or document writes.
use crate::document::{Document, HH, HP};
use crate::errors::{require, Result, WorkbenchError};
use crate::xml_io::{xml_tree, Element};
use serde_json::{json, Map, Value};

// Native page measurements use 7200 HWP units per inch, and 25.4 mm per inch.
const HWP_UNITS_PER_MM: f64 = 7200.0 / 25.4;

/// Read a finite native numeric field; absent or unreadable fields stay unknown.
pub fn number(value: Option<&str>, divisor: f64) -> Option<f64> {
    let value: String = value?.trim().chars().map(ascii_decimal_digit).collect();
    // Accept underscores only between decimal digits.
    let bytes = value.as_bytes();
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte == b'_' {
            let previous = offset.checked_sub(1).and_then(|index| bytes.get(index));
            let next = offset.checked_add(1).and_then(|index| bytes.get(index));
            if !previous.is_some_and(u8::is_ascii_digit) || !next.is_some_and(u8::is_ascii_digit) {
                return None;
            }
        }
    }
    let result = value.replace('_', "").parse::<f64>().ok()? / divisor;
    result.is_finite().then_some(result)
}

pub(crate) fn ascii_decimal_digit(character: char) -> char {
    if character.is_ascii() {
        return character;
    }
    // Each Unicode decimal digit block contains ten consecutive code points.
    // Accept these decimal digits, including the Unicode 16 blocks.
    const DECIMAL_ZEROES: &[u32] = &[
        0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66, 0xde6,
        0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
        0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
        0xff10, 0x104a0, 0x10d30, 0x10d40, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450,
        0x114d0, 0x11650, 0x116c0, 0x116d0, 0x116da, 0x11730, 0x118e0, 0x11950, 0x11bf0, 0x11c50,
        0x11d50, 0x11da0, 0x11f50, 0x16130, 0x16a60, 0x16ac0, 0x16b50, 0x16d70, 0x1ccf0, 0x1d7ce,
        0x1d7d8, 0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0, 0x1e4f0, 0x1e5f1, 0x1e950, 0x1fbf0,
    ];
    DECIMAL_ZEROES
        .iter()
        .find_map(|zero| {
            (character as u32)
                .checked_sub(*zero)
                .filter(|digit| *digit < 10)
        })
        .and_then(|digit| char::from_digit(digit, 10))
        .unwrap_or(character)
}

fn matches(element: &Element, namespace: &str, tag: &str) -> bool {
    element.name.namespace.as_deref() == Some(namespace) && element.name.local == tag
}

fn find<'a>(element: &'a Element, namespace: &str, tag: &str) -> Option<&'a Element> {
    element
        .children
        .iter()
        .find(|child| matches(child, namespace, tag))
}

fn descendant<'a>(element: &'a Element, namespace: &str, tag: &str) -> Option<&'a Element> {
    element
        .iter()
        .skip(1)
        .find(|child| matches(child, namespace, tag))
}

fn reference<'a>(header: &'a Element, kind: &str, id: Option<&str>) -> Option<&'a Element> {
    // ElementTree's reference dictionary retains the last duplicate ID.
    header
        .iter()
        .filter(|item| matches(item, HH, kind) && item.get("id") == id)
        .last()
}

fn native_part(document: &Document, name: &str) -> Result<Element> {
    let payload = document.part(name).ok_or_else(|| {
        WorkbenchError::new("NOT_HWPX", "Missing HWPX header or content manifest.")
    })?;
    xml_tree(payload)
}

fn text_parents(body: &Element, ordinal: usize) -> Option<(Option<&Element>, Option<&Element>)> {
    let mut pending = vec![(body, None, None)];
    let mut text_index = 0;
    while let Some((element, mut run, mut paragraph)) = pending.pop() {
        if element.name.local == "run" {
            run = Some(element);
        }
        if element.name.local == "p" {
            paragraph = Some(element);
        }
        if matches(element, HP, "t") {
            if text_index == ordinal {
                return Some((run, paragraph));
            }
            text_index += 1;
        }
        pending.extend(
            element
                .children
                .iter()
                .rev()
                .map(|child| (child, run, paragraph)),
        );
    }
    None
}

fn colour(value: Option<&str>) -> Option<String> {
    let value = value?;
    let value = value.strip_prefix('#').unwrap_or(value);
    (value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_uppercase())
}

fn character(header: &Element, run: Option<&Element>) -> Value {
    let Some(property) = run.and_then(|run| reference(header, "charPr", run.get("charPrIDRef")))
    else {
        return json!({});
    };
    let font_id = find(property, HH, "fontRef").and_then(|item| item.get("hangul"));
    let font = if find(property, HH, "fontRef").is_some() {
        header
            .iter()
            .filter(|face| matches(face, HH, "fontface") && face.get("lang") == Some("HANGUL"))
            .flat_map(|face| &face.children)
            .filter(|item| item.get("id") == font_id)
            .last()
            .and_then(|font| font.get("face"))
    } else {
        None
    };
    json!({
        "font": font,
        "size": number(property.get("height"), 100.0),
        "widthRatio": number(find(property, HH, "ratio").and_then(|item| item.get("hangul")), 1.0),
        "letterSpacing": number(find(property, HH, "spacing").and_then(|item| item.get("hangul")), 1.0),
        "bold": find(property, HH, "bold").is_some(),
        "italic": find(property, HH, "italic").is_some(),
        "underline": find(property, HH, "underline").is_some_and(|item| item.get("type") != Some("NONE")),
        "colour": colour(property.get("textColor"))
    })
}

// The display formatter rounds to six significant digits and switches
// to exponent notation below 1e-4 or at 1e6, after rounding.
fn general_number(value: f64) -> String {
    let scientific = format!("{value:.5e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let exponent = exponent.parse::<i32>().unwrap_or(0);
    if !(-4..6).contains(&exponent) {
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        format!("{mantissa}e{exponent:+03}")
    } else {
        let precision = usize::try_from((5 - exponent).max(0)).unwrap_or(0);
        let decimal = format!("{value:.precision$}");
        if decimal.contains('.') {
            decimal
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
        } else {
            decimal
        }
    }
}

fn paragraph(header: &Element, paragraph: Option<&Element>) -> Value {
    let Some(property) =
        paragraph.and_then(|item| reference(header, "paraPr", item.get("paraPrIDRef")))
    else {
        return json!({});
    };
    let scope = descendant(property, HP, "default").unwrap_or(property);
    let mut result = Map::new();
    result.insert(
        "alignment".to_owned(),
        json!(find(property, HH, "align").and_then(|item| item.get("horizontal"))),
    );
    if let Some(margin) = descendant(scope, HH, "margin") {
        for item in &margin.children {
            let key = match item.name.local.as_str() {
                "intent" => "indent",
                "left" => "left",
                "right" => "right",
                "prev" => "before",
                "next" => "after",
                _ => continue,
            };
            if item.get("unit") == Some("HWPUNIT") {
                result.insert(key.to_owned(), json!(number(item.get("value"), 100.0)));
            }
        }
    }
    if let Some(spacing) = descendant(scope, HH, "lineSpacing") {
        if let Some(value) = number(spacing.get("value"), 1.0) {
            let spacing = match spacing.get("type") {
                Some("PERCENT") => Some(format!("{}%", general_number(value))),
                Some("FIXED" | "AT_LEAST") => Some(format!("{} pt", general_number(value / 100.0))),
                _ => None,
            };
            if let Some(spacing) = spacing {
                result.insert("lineSpacing".to_owned(), json!(spacing));
            }
        }
    }
    Value::Object(result)
}

fn page(body: &Element) -> Value {
    let Some(page) = body.iter().find(|item| matches(item, HP, "pagePr")) else {
        return json!({});
    };
    let mut result = Map::new();
    for key in ["width", "height"] {
        result.insert(
            key.to_owned(),
            json!(number(page.get(key), HWP_UNITS_PER_MM)),
        );
    }
    let orientation = match page.get("landscape") {
        Some("WIDELY") => Some("landscape"),
        Some("NARROWLY") => Some("portrait"),
        _ => None,
    };
    result.insert("orientation".to_owned(), json!(orientation));
    if let Some(margin) = find(page, HP, "margin") {
        for key in [
            "left", "right", "top", "bottom", "header", "footer", "gutter",
        ] {
            result.insert(
                key.to_owned(),
                json!(number(margin.get(key), HWP_UNITS_PER_MM)),
            );
        }
    }
    Value::Object(result)
}

/// Read native header properties for a selected node, or the first text node.
pub fn read_format(document: &Document, node_id: Option<&str>) -> Result<Value> {
    let selected = node_id.and_then(|id| document.nodes.iter().find(|node| node.id == id));
    require(
        node_id.is_none() || selected.is_some(),
        "BAD_ARGUMENTS",
        "Unknown text node.",
    )?;
    let empty = json!({"character": {}, "paragraph": {}, "page": {}, "style": {},
        "fonts": document.fonts, "source": "native-header"});
    if document.format != "hwpx" {
        return Ok(empty);
    }
    let header = native_part(document, "Contents/header.xml")?;
    let Some(selected) = selected.or_else(|| document.nodes.first()) else {
        return Ok(empty);
    };
    let body = native_part(document, &selected.part)?;
    let ordinal = document
        .nodes
        .iter()
        .filter(|node| node.part == selected.part)
        .position(|node| node.id == selected.id)
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Unknown text node."))?;
    let (run, para) = text_parents(&body, ordinal)
        .ok_or_else(|| WorkbenchError::new("BAD_XML", "Missing text node in HWPX section."))?;
    let style_name = para
        .and_then(|item| reference(&header, "style", item.get("styleIDRef")))
        .and_then(|style| style.get("name"));
    Ok(
        json!({"character": character(&header, run), "paragraph": paragraph(&header, para),
        "page": page(&body), "style": {"name": style_name},
        "fonts": document.fonts, "source": "native-header"}),
    )
}
