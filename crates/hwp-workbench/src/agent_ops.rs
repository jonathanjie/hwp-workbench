//! Bounded native text operations and source-scoped short references.
use hwp_core::document::{check_text, Document, TextNode};
use hwp_core::errors::{require, Result, WorkbenchError};
use regex::{Regex, RegexBuilder};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use unicode_normalization::UnicodeNormalization;

/// Restrict expressions and output batches before scanning document text.
pub const MAX_PATTERN: usize = 256;
pub const MAX_SEARCH_MATCHES: usize = 1000;
pub const MAX_PATCH_OPERATIONS: usize = 1000;
/// Context is bounded even when a matched literal is very long.
pub const CONTEXT_PADDING: usize = 40;
/// Bound table mapping allocations and work independently of preview limits.
pub const MAX_TABLE_CELLS: usize = 100_000;
pub const MAX_TABLE_COLUMNS: usize = 1000;
pub const MAX_GRID_WORK: usize = 1_000_000;

fn integer(value: &Value, name: &str, minimum: usize, maximum: usize) -> Result<usize> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n >= minimum && *n <= maximum)
        .ok_or_else(|| {
            WorkbenchError::new(
                "BAD_ARGUMENTS",
                format!("{name} must be an integer from {minimum} to {maximum}."),
            )
        })
}
fn pattern(query: &Value, regex: &Value, case_sensitive: &Value) -> Result<Pattern> {
    let query = query
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| WorkbenchError::new("BAD_ARGUMENTS", "Supply nonempty search text."))?;
    require(
        regex.is_boolean() && case_sensitive.is_boolean(),
        "BAD_ARGUMENTS",
        "regex and caseSensitive must be booleans.",
    )?;
    let expression = if regex == &json!(true) {
        require(
            query.chars().count() <= MAX_PATTERN,
            "REGEX_LIMIT",
            "Regex patterns are limited to 256 characters.",
        )?;
        let mut in_class = false;
        let mut escaped = false;
        for character in query.chars() {
            if escaped {
                require(
                    !python_digit(character) && character != 'g' && character != 'N',
                    "REGEX_UNSUPPORTED",
                    "Regex backreferences and named escapes are not supported.",
                )?;
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if in_class {
                if character == ']' {
                    in_class = false;
                }
            } else if character == '[' {
                in_class = true;
            } else {
                require(
                    !"()*+?{}|".contains(character),
                    "REGEX_UNSUPPORTED",
                    "Regex repetitions, groups and branches are not supported.",
                )?;
            }
        }
        if in_class {
            return Err(WorkbenchError::new(
                "BAD_REGEX",
                format!(
                    "unterminated character set at position {}",
                    query.chars().position(|c| c == '[').unwrap_or(0)
                ),
            ));
        }
        if escaped {
            return Err(WorkbenchError::new(
                "BAD_REGEX",
                format!(
                    "bad escape (end of pattern) at position {}",
                    query.chars().count().saturating_sub(1)
                ),
            ));
        }
        query.to_owned()
    } else {
        require(
            query.chars().count() <= 200_000,
            "LIMIT",
            "Search text exceeds 200,000 characters.",
        )?;
        regex::escape(query)
    };
    let (expression, boundaries) = translate_pattern(&expression, case_sensitive == &json!(false))?;
    let regex = RegexBuilder::new(&expression)
        .case_insensitive(case_sensitive == &json!(false))
        .build()
        .map_err(|error| WorkbenchError::new("BAD_REGEX", error.to_string()))?;
    Ok(Pattern { regex, boundaries })
}

enum Assertion {
    Word(bool),
    FinalEnd,
}
struct Pattern {
    regex: Regex,
    boundaries: Vec<(usize, Assertion)>,
}
struct TextMatch<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}
impl TextMatch<'_> {
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
    fn as_str(&self) -> &str {
        self.text
    }
}
impl Pattern {
    fn find_matches<'a>(&self, text: &'a str, limit: usize) -> Vec<TextMatch<'a>> {
        let mut matches = Vec::new();
        let mut cursor = 0;
        while matches.len() < limit {
            let Some(captures) = self.regex.captures_at(text, cursor) else {
                break;
            };
            let Some(found) = captures.get(0) else {
                break;
            };
            let valid = self.boundaries.iter().all(|(group, expected)| {
                let Some(at) = captures.get(*group).map(|capture| capture.start()) else {
                    return false;
                };
                let before = text.get(..at).and_then(|prefix| prefix.chars().next_back());
                let after = text.get(at..).and_then(|suffix| suffix.chars().next());
                // Python Unicode word characters are letters, numbers and underscore.
                let boundary = before.is_some_and(python_word) != after.is_some_and(python_word);
                match expected {
                    Assertion::Word(expected) => boundary == *expected,
                    Assertion::FinalEnd => at == text.len() || text.get(at..) == Some("\n"),
                }
            });
            if valid {
                matches.push(TextMatch {
                    start: found.start(),
                    end: found.end(),
                    text: found.as_str(),
                });
            }
            let advance_from = if valid && found.end() > found.start() {
                found.end()
            } else {
                found.start()
            };
            if valid && found.end() > found.start() {
                cursor = advance_from;
            } else if let Some(character) = text
                .get(advance_from..)
                .and_then(|suffix| suffix.chars().next())
            {
                cursor = advance_from + character.len_utf8();
            } else {
                break;
            }
        }
        matches
    }
}
fn python_word(character: char) -> bool {
    // Alphabetic alone also includes combining marks, which Python excludes.
    let mut buffer = [0; 4];
    let value = character.encode_utf8(&mut buffer);
    static WORD: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    // This literal class is validated by the Unicode regex regression tests.
    WORD.get_or_init(|| Regex::new(r"^[\p{L}\p{N}_]$").expect("constant Unicode word class"))
        .is_match(value)
}
fn python_digit(character: char) -> bool {
    // Python isdigit includes decimal digits and these nondecimal digit blocks;
    // normalized superscripts and enclosed single digits use the ASCII check.
    if matches!(character, '\u{1369}'..='\u{1371}' | '\u{19da}' | '\u{2474}'..='\u{247c}' | '\u{2488}'..='\u{2490}' | '\u{24f5}'..='\u{24fd}' | '\u{24ff}' | '\u{2776}'..='\u{277e}' | '\u{2780}'..='\u{2788}' | '\u{278a}'..='\u{2792}' | '\u{10a40}'..='\u{10a43}' | '\u{10e60}'..='\u{10e68}' | '\u{11052}'..='\u{1105a}' | '\u{1f100}'..='\u{1f10a}')
    {
        return true;
    }
    let normalized = character.to_string().nfkc().collect::<String>();
    if normalized.len() == 1 && normalized.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    static DECIMAL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let mut buffer = [0; 4];
    // This constant character class is validated by the regex compiler.
    DECIMAL
        .get_or_init(|| Regex::new(r"^\p{Nd}$").expect("constant decimal class"))
        .is_match(character.encode_utf8(&mut buffer))
}
fn bad_escape(character: char, position: usize) -> WorkbenchError {
    WorkbenchError::new(
        "BAD_REGEX",
        format!("bad escape \\{character} at position {position}"),
    )
}
/// Translate only the permitted Python atoms. Boundary assertions are checked
/// separately because the Rust word class also includes combining marks.
fn translate_pattern(source: &str, insensitive: bool) -> Result<(String, Vec<(usize, Assertion)>)> {
    let characters: Vec<char> = source.chars().collect();
    let mut at = 0;
    let mut output = String::new();
    let mut boundaries = Vec::new();
    let mut in_class = false;
    let mut class_i = false;
    while let Some(character) = characters.get(at).copied() {
        if character == '\\' {
            let position = at;
            at += 1;
            let escaped = characters.get(at).copied().ok_or_else(|| {
                WorkbenchError::new(
                    "BAD_REGEX",
                    format!("bad escape (end of pattern) at position {position}"),
                )
            })?;
            match escaped {
                'b' | 'B' if !in_class => {
                    output.push_str("()");
                    boundaries.push((boundaries.len() + 1, Assertion::Word(escaped == 'b')));
                }
                'b' => output.push_str(r"\x08"),
                'a' => output.push_str(r"\x07"),
                'v' => output.push_str(r"\x0b"),
                'A' | 'Z' | 'z' if in_class => return Err(bad_escape(escaped, position)),
                'A' => output.push_str(r"\A"),
                'Z' | 'z' => output.push_str(r"\z"),
                'w' => output.push_str(r"[\p{L}\p{N}_]"),
                'W' => output.push_str(r"[^\p{L}\p{N}_]"),
                's' => output.push_str(r"[\s\x1c-\x1f]"),
                'S' => output.push_str(r"[^\s\x1c-\x1f]"),
                'd' | 'D' | 'n' | 'r' | 't' | 'f' | '\\' => {
                    output.push('\\');
                    output.push(escaped);
                }
                'x' | 'u' | 'U' => {
                    let width = match escaped {
                        'x' => 2,
                        'u' => 4,
                        _ => 8,
                    };
                    let end = at.saturating_add(width).saturating_add(1);
                    let digits = characters
                        .get(at + 1..end)
                        .filter(|digits| digits.iter().all(char::is_ascii_hexdigit));
                    let Some(digits) = digits else {
                        return Err(WorkbenchError::new(
                            "BAD_REGEX",
                            format!("incomplete escape \\{escaped} at position {position}"),
                        ));
                    };
                    let number = u32::from_str_radix(&digits.iter().collect::<String>(), 16)
                        .map_err(|_| bad_escape(escaped, position))?;
                    if let Some(scalar) = char::from_u32(number) {
                        if insensitive && matches!(scalar, 'i' | 'I' | 'İ' | 'ı') {
                            if in_class {
                                class_i = true;
                                output.push(scalar);
                            } else {
                                output.push_str("[iIİı]");
                            }
                        } else {
                            output.push_str(&regex::escape(&scalar.to_string()));
                        }
                    } else if number <= 0x10ffff {
                        // Native UTF-8 text cannot contain a surrogate scalar.
                        output.push_str(r"(?:a\A)");
                    } else {
                        return Err(WorkbenchError::new(
                            "BAD_REGEX",
                            format!(
                                "bad escape \\{escaped}{} at position {position}",
                                digits.iter().collect::<String>()
                            ),
                        ));
                    }
                    at += width;
                }
                escaped if escaped.is_ascii_alphabetic() => {
                    return Err(bad_escape(escaped, position))
                }
                escaped => {
                    if insensitive && matches!(escaped, 'İ' | 'ı') {
                        if in_class {
                            class_i = true;
                            output.push(escaped);
                        } else {
                            output.push_str("[iIİı]");
                        }
                    } else {
                        output.push_str(&regex::escape(&escaped.to_string()));
                    }
                }
            }
        } else {
            match character {
                '[' if in_class => output.push_str(r"\["),
                '[' => {
                    in_class = true;
                    class_i = insensitive && characters.get(at + 1..).is_some_and(|rest| {
                        let class: Vec<_> = rest.iter().copied().take_while(|c| *c != ']').collect();
                        class.iter().any(|c| matches!(c, 'i'|'I'|'İ'|'ı')) || class.windows(3).any(|range| {
                            matches!(range, [start, '-', end] if (*start <= 'i' && *end >= 'i') || (*start <= 'I' && *end >= 'I'))
                        })
                    });
                    output.push(character);
                    let next = characters.get(at + 1).copied();
                    let closing_literal = if next == Some('^') {
                        characters.get(at + 2).copied() == Some(']')
                    } else {
                        next == Some(']')
                    };
                    if closing_literal {
                        if next == Some('^') {
                            output.push('^');
                            at += 1;
                        }
                        output.push_str(r"\]");
                        at += 1;
                    }
                }
                ']' => {
                    if class_i {
                        output.push_str("iIİı");
                    }
                    in_class = false;
                    output.push(character);
                }
                '$' if !in_class => {
                    output.push_str("()");
                    boundaries.push((boundaries.len() + 1, Assertion::FinalEnd));
                }
                'i' | 'I' | 'İ' | 'ı' if insensitive && !in_class => output.push_str("[iIİı]"),
                '-' if in_class => {
                    if let (Some(start), Some(end)) = (
                        at.checked_sub(1).and_then(|p| characters.get(p)).copied(),
                        characters.get(at + 1).copied(),
                    ) {
                        if start != '[' && start != '^' && end != ']' && start > end {
                            return Err(WorkbenchError::new(
                                "BAD_REGEX",
                                format!(
                                    "bad character range {start}-{end} at position {}",
                                    at.saturating_sub(1)
                                ),
                            ));
                        }
                    }
                    output.push('-');
                }
                '&' | '~' if in_class => {
                    output.push('\\');
                    output.push(character);
                }
                character => output.push(character),
            }
        }
        at += 1;
    }
    if in_class {
        return Err(WorkbenchError::new(
            "BAD_REGEX",
            format!(
                "unterminated character set at position {}",
                source.chars().position(|c| c == '[').unwrap_or(0)
            ),
        ));
    }
    Ok((output, boundaries))
}

#[derive(Clone)]
struct Cell {
    row: usize,
    column: usize,
    width: usize,
    height: usize,
    nodes: Vec<String>,
    valid: bool,
}
fn children(view: &Value) -> impl Iterator<Item = &Value> {
    view.get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}
fn kind(view: &Value) -> &str {
    view.get("type").and_then(Value::as_str).unwrap_or("")
}
fn text_ids(view: &Value, ids: &mut Vec<String>) {
    match kind(view) {
        "text" => {
            if let Some(id) = view.get("node").and_then(Value::as_str) {
                ids.push(id.to_owned());
            }
        }
        "table" => {}
        _ => {
            for child in children(view) {
                text_ids(child, ids);
            }
        }
    }
}
fn native_integer(value: Option<&Value>) -> Option<usize> {
    value.and_then(|v| {
        v.as_str()
            .and_then(|s| s.trim().parse().ok())
            .or_else(|| v.as_u64().and_then(|n| usize::try_from(n).ok()))
    })
}
fn tables(doc: &Document) -> Result<Vec<Vec<Cell>>> {
    fn visit(
        view: &Value,
        tables: &mut Vec<Vec<Cell>>,
        count: &mut usize,
        work: &mut usize,
    ) -> Result<()> {
        if kind(view) == "table" {
            let mut cells = vec![];
            let mut occupied = BTreeMap::<usize, usize>::new();
            let mut valid = true;
            for (row_index, row) in children(view).filter(|v| kind(v) == "row").enumerate() {
                let mut column = 0usize;
                for cell in children(row).filter(|v| kind(v) == "cell") {
                    *count += 1;
                    require(
                        *count <= MAX_TABLE_CELLS,
                        "LIMIT",
                        "Table view exceeds 100,000 cells.",
                    )?;
                    let preview_width = native_integer(cell.get("colspan")).unwrap_or(1);
                    let preview_height = native_integer(cell.get("rowspan")).unwrap_or(1);
                    let (width, height) =
                        if let Some(span) = cell.get("nativeSpan").filter(|v| !v.is_null()) {
                            match (
                                native_integer(span.get("columns")),
                                native_integer(span.get("rows")),
                            ) {
                                (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
                                _ => {
                                    valid = false;
                                    (preview_width, preview_height)
                                }
                            }
                        } else {
                            (preview_width, preview_height)
                        };
                    require(
                        width <= MAX_TABLE_COLUMNS && height <= MAX_TABLE_CELLS,
                        "LIMIT",
                        "Native table span exceeds the bounded mapping limits.",
                    )?;
                    let mut cell_row = row_index;
                    if let Some(address) = cell.get("address").filter(|v| !v.is_null()) {
                        match (
                            native_integer(address.get("row")),
                            native_integer(address.get("column")),
                        ) {
                            (Some(r), Some(c)) => {
                                cell_row = r;
                                column = c;
                            }
                            _ => {
                                valid = false;
                                column = 0;
                            }
                        }
                    } else {
                        while (column..column.saturating_add(width))
                            .any(|c| occupied.get(&c).copied().unwrap_or(0) > cell_row)
                        {
                            column += 1;
                            *work += width;
                            require(
                                column < MAX_TABLE_COLUMNS && *work <= MAX_GRID_WORK,
                                "LIMIT",
                                "Table grid exceeds the bounded mapping limits.",
                            )?;
                        }
                    }
                    require(
                        cell_row < MAX_TABLE_CELLS
                            && column.saturating_add(width) <= MAX_TABLE_COLUMNS,
                        "LIMIT",
                        "Table addresses exceed 100,000 rows or 1000 columns.",
                    )?;
                    if (column..column.saturating_add(width))
                        .any(|c| occupied.get(&c).copied().unwrap_or(0) > cell_row)
                    {
                        valid = false;
                    }
                    let mut nodes = vec![];
                    text_ids(cell, &mut nodes);
                    cells.push(Cell {
                        row: cell_row,
                        column,
                        width,
                        height,
                        nodes,
                        valid: true,
                    });
                    *work += width;
                    require(
                        *work <= MAX_GRID_WORK,
                        "LIMIT",
                        "Table grid mapping exceeds its work limit.",
                    )?;
                    for col in column..column.saturating_add(width) {
                        occupied.insert(col, cell_row.saturating_add(height));
                    }
                    column += width;
                }
            }
            for cell in &mut cells {
                cell.valid = valid;
            }
            tables.push(cells);
        }
        for child in children(view) {
            visit(child, tables, count, work)?;
        }
        Ok(())
    }
    let mut mapped = vec![];
    let mut count = 0;
    let mut work = 0;
    for view in &doc.view {
        visit(view, &mut mapped, &mut count, &mut work)?;
    }
    Ok(mapped)
}

pub fn read(doc: &Document, offset: &Value, limit: &Value) -> Result<Value> {
    let offset = integer(offset, "offset", 0, 100_000)?;
    let limit = integer(limit, "limit", 1, 1000)?;
    let mut positions = BTreeMap::new();
    for (table, cells) in tables(doc)?.iter().enumerate() {
        for (cell_index, cell) in cells.iter().enumerate() {
            for id in &cell.nodes {
                positions.insert(id.clone(), (table, cell.row, cell_index));
            }
        }
    }
    let mut parts = BTreeMap::new();
    let mut counts = BTreeMap::new();
    let mut references = Map::new();
    let mut lines: Vec<String> = vec![];
    let mut previous = None;
    let stop = doc.nodes.len().min(offset + limit);
    for (index, node) in doc.nodes.iter().enumerate() {
        let section_count = parts.len();
        let section = parts.entry(node.part.clone()).or_insert(section_count);
        let count = counts.entry(node.part.clone()).or_insert(0usize);
        let reference = format!("s{section}:t{count}");
        *count += 1;
        if index < offset || index >= stop {
            continue;
        }
        references.insert(reference.clone(), json!(node.id));
        let text = node
            .text
            .replace('\\', "\\\\")
            .replace('|', "\\|")
            .replace('\r', "\\r")
            .replace('\n', "\\n");
        let marker = if node.editable { "" } else { "(read-only) " };
        let line = format!("[{reference}] {marker}{text}");
        let position = positions.get(&node.id).copied();
        if let Some((table, row, cell)) = position {
            if previous.is_some_and(|(t, r, _)| t == table && r == row) {
                if let Some(last) = lines.last_mut() {
                    if previous == Some((table, row, cell)) {
                        last.truncate(last.len().saturating_sub(2));
                        last.push(' ');
                        last.push_str(&line);
                        last.push_str(" |");
                    } else {
                        last.push(' ');
                        last.push_str(&line);
                        last.push_str(" |");
                    }
                }
            } else {
                lines.push(format!("| {line} |"));
            }
        } else {
            lines.push(line);
        }
        previous = position;
    }
    let warnings: Vec<Value> = doc
        .warnings
        .iter()
        .filter_map(|v| v.get("code").cloned())
        .collect();
    Ok(
        json!({"format":doc.format,"sha256":doc.hash,"total":doc.nodes.len(),"offset":offset,"limit":limit,"nextOffset":if stop<doc.nodes.len(){Some(stop)}else{None},"text":lines.join("\n"),"references":references,"warnings":warnings}),
    )
}
pub fn search(
    doc: &Document,
    query: &Value,
    regex: &Value,
    case_sensitive: &Value,
    limit: &Value,
) -> Result<Value> {
    let limit = integer(limit, "limit", 1, MAX_SEARCH_MATCHES)?;
    let pattern = pattern(query, regex, case_sensitive)?;
    let mut matches = vec![];
    for node in &doc.nodes {
        for m in pattern.find_matches(&node.text, limit.saturating_sub(matches.len())) {
            let start = node.text.get(..m.start()).unwrap_or("").chars().count();
            let end = start + m.as_str().chars().count();
            let context_start = start.saturating_sub(CONTEXT_PADDING);
            let context_end = (end + CONTEXT_PADDING).min(start + 2 * CONTEXT_PADDING);
            let context: String = node
                .text
                .chars()
                .skip(context_start)
                .take(context_end - context_start)
                .collect();
            matches.push(json!({"nodeId":node.id,"start":start,"end":end,"context":context,"editable":node.editable}));
            if matches.len() == limit {
                return Ok(
                    json!({"format":doc.format,"sha256":doc.hash,"matches":matches,"limitReached":true}),
                );
            }
        }
    }
    Ok(json!({"format":doc.format,"sha256":doc.hash,"matches":matches,"limitReached":false}))
}
fn patch(doc: &Document, operations: Vec<Value>) -> Value {
    json!({"schemaVersion":1,"baseSha256":doc.hash,"operations":operations})
}
fn operation(node: &TextNode, text: &str) -> Value {
    json!({"op":"set_text","nodeId":node.id,"expectedText":node.text,"text":text})
}
fn change(node: &TextNode, text: &str) -> Value {
    json!({"nodeId":node.id,"before":node.text,"after":text,"part":node.part})
}
fn edit_error(doc: &Document, node: &TextNode) -> Option<&'static str> {
    if doc.block_save {
        Some("SAVE_BLOCKED")
    } else if !node.editable {
        Some("UNSUPPORTED_ELEMENT")
    } else {
        None
    }
}
#[allow(clippy::too_many_arguments)]
pub fn replace(
    doc: &Document,
    find: &Value,
    replace_with: &Value,
    regex: &Value,
    case_sensitive: &Value,
    node_ids: Option<&Value>,
    limit: &Value,
) -> Result<Value> {
    let mut remaining = integer(limit, "limit", 1, MAX_PATCH_OPERATIONS)?;
    check_text(replace_with)?;
    let replacement = replace_with.as_str().ok_or_else(|| {
        WorkbenchError::new(
            "BAD_PATCH",
            "Text must be a string of at most 200,000 characters.",
        )
    })?;
    let pattern = pattern(find, regex, case_sensitive)?;
    let selected = if let Some(value) = node_ids {
        require(
            value.as_array().is_some_and(|ids| {
                ids.len() <= MAX_PATCH_OPERATIONS && ids.iter().all(Value::is_string)
            }),
            "BAD_ARGUMENTS",
            "nodeIds must contain at most 1000 full IDs or short references.",
        )?;
        let mut ids = HashSet::new();
        if let Some(array) = value.as_array() {
            for id in array {
                if let Some(id) = id.as_str() {
                    ids.insert(doc.resolve_node_id(id)?);
                }
            }
        }
        Some(ids)
    } else {
        None
    };
    let mut operations = vec![];
    let mut changes = vec![];
    let mut skipped = vec![];
    for node in &doc.nodes {
        if remaining == 0 {
            break;
        }
        if selected.as_ref().is_some_and(|ids| !ids.contains(&node.id)) {
            continue;
        }
        let matches = pattern.find_matches(&node.text, remaining);
        if matches.is_empty() {
            continue;
        }
        remaining -= matches.len();
        if let Some(code) = edit_error(doc, node) {
            skipped.push(json!({"nodeId":node.id,"code":code,"reason":"Read-only text or protected document."}));
            continue;
        }
        let removed: usize = matches.iter().map(|m| m.as_str().chars().count()).sum();
        let length = node
            .text
            .chars()
            .count()
            .saturating_sub(removed)
            .saturating_add(matches.len().saturating_mul(replacement.chars().count()));
        require(
            length <= 200_000,
            "BAD_PATCH",
            "Replacement text exceeds 200,000 characters in one node.",
        )?;
        let mut after = String::new();
        let mut end = 0;
        for m in matches {
            after.push_str(node.text.get(end..m.start()).unwrap_or(""));
            after.push_str(replacement);
            end = m.end();
        }
        after.push_str(node.text.get(end..).unwrap_or(""));
        check_text(&json!(after))?;
        if after != node.text {
            operations.push(operation(node, &after));
            changes.push(change(node, &after));
        }
    }
    Ok(json!({"patch":patch(doc,operations),"changes":changes,"skipped":skipped}))
}
fn hangul(c: char) -> bool {
    matches!(c,'\u{1100}'..='\u{11ff}'|'\u{3130}'..='\u{318f}'|'\u{ac00}'..='\u{d7a3}')
}
fn python_whitespace(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
}
fn normalise_label(label: &str) -> String {
    let label = label
        .nfc()
        .collect::<String>()
        .split(python_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let label = label
        .split(" / ")
        .next()
        .unwrap_or("")
        .trim_end_matches([':', '：', ' '])
        .trim();
    let mut output = String::new();
    let mut previous = None;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ' ' && previous.is_some_and(hangul) && chars.peek().copied().is_some_and(hangul) {
            continue;
        }
        output.push(c);
        previous = Some(c);
    }
    output
}
pub fn fill(doc: &Document, fields: &Value) -> Result<Value> {
    require(
        fields.as_object().is_some_and(|map| {
            map.len() <= MAX_PATCH_OPERATIONS
                && map.keys().all(|label| !normalise_label(label).is_empty())
        }),
        "BAD_ARGUMENTS",
        "fields must map at most 1000 nonempty labels to text values.",
    )?;
    let fields = fields.as_object().ok_or_else(|| {
        WorkbenchError::new(
            "BAD_ARGUMENTS",
            "fields must map at most 1000 nonempty labels to text values.",
        )
    })?;
    for value in fields.values() {
        check_text(value)?;
    }
    let tables = if doc.format == "hwpx" {
        tables(doc)?
    } else {
        vec![]
    };
    let mut labels: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (table_index, cells) in tables.iter().enumerate() {
        for (cell_index, cell) in cells.iter().enumerate() {
            let text: String = cell
                .nodes
                .iter()
                .filter_map(|id| doc.node(id))
                .map(|node| node.text.as_str())
                .collect();
            labels
                .entry(normalise_label(&text))
                .or_default()
                .push((table_index, cell_index));
        }
    }
    let mut operations = vec![];
    let mut filled = vec![];
    let mut unresolved = vec![];
    let mut details = vec![];
    let mut used = HashSet::new();
    for (label, value) in fields {
        let mut code = None;
        let mut target = None;
        let candidates = labels
            .get(&normalise_label(label))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if doc.format == "hwp" {
            code = Some("HWP_TABLE_MAPPING_UNAVAILABLE");
        } else if candidates.len() != 1 {
            code = Some(if candidates.is_empty() {
                "LABEL_NOT_FOUND"
            } else {
                "AMBIGUOUS_LABEL"
            });
        } else if let Some((table_index, cell_index)) = candidates.first() {
            if let Some(cells) = tables.get(*table_index) {
                if let Some(label_cell) = cells.get(*cell_index) {
                    if !label_cell.valid {
                        code = Some("TABLE_MAPPING_UNAVAILABLE");
                    } else {
                        let mut right: Vec<_> = cells
                            .iter()
                            .filter(|c| {
                                c.row == label_cell.row
                                    && c.column >= label_cell.column + label_cell.width
                            })
                            .collect();
                        right.sort_by_key(|c| c.column);
                        let below = cells.iter().filter(|c| {
                            c.row == label_cell.row + label_cell.height
                                && c.column == label_cell.column
                        });
                        for cell in right.into_iter().chain(below) {
                            if cell.nodes.len() == 1 {
                                if let Some(node) = cell.nodes.first().and_then(|id| doc.node(id)) {
                                    if edit_error(doc, node).is_none() {
                                        target = Some(node);
                                        break;
                                    }
                                }
                            }
                        }
                        if let Some(node) = target {
                            if used.contains(&node.id) {
                                code = Some("TARGET_CONFLICT");
                            }
                        } else {
                            code = Some("NO_EDITABLE_VALUE_CELL");
                        }
                    }
                }
            }
        }
        if let Some(code) = code {
            unresolved.push(json!(label));
            details.push(json!({"label":label,"code":code}));
            continue;
        }
        if let Some(node) = target {
            let text = value.as_str().unwrap_or("");
            used.insert(node.id.clone());
            if node.text != text {
                operations.push(operation(node, text));
            }
            filled.push(json!({"label":label,"nodeId":node.id,"before":node.text,"after":text}));
        }
    }
    Ok(
        json!({"patch":patch(doc,operations),"filled":filled,"unresolved":unresolved,"unresolvedDetails":details}),
    )
}
pub fn set_text(doc: &Document, node_id: &Value, text: &Value) -> Result<Value> {
    let supplied = node_id.as_str().ok_or_else(|| match node_id {
        Value::Array(_) => WorkbenchError::new("REQUEST_FAILED", "unhashable type: 'list'"),
        Value::Object(_) => WorkbenchError::new("REQUEST_FAILED", "unhashable type: 'dict'"),
        Value::Null => WorkbenchError::new("NODE_NOT_FOUND", "Unknown node ID: None"),
        Value::Bool(value) => WorkbenchError::new(
            "NODE_NOT_FOUND",
            format!("Unknown node ID: {}", if *value { "True" } else { "False" }),
        ),
        _ => WorkbenchError::new("NODE_NOT_FOUND", format!("Unknown node ID: {node_id}")),
    })?;
    let id = doc.resolve_node_id(supplied)?;
    check_text(text)?;
    let node = doc.node(&id).ok_or_else(|| {
        WorkbenchError::new("NODE_NOT_FOUND", format!("Unknown node ID: {supplied}"))
    })?;
    if let Some(code) = edit_error(doc, node) {
        return Err(WorkbenchError::new(
            code,
            "This node cannot accept the requested text.",
        ));
    }
    let text = text.as_str().unwrap_or("");
    let changes = if text == node.text {
        vec![]
    } else {
        vec![change(node, text)]
    };
    Ok(json!({"patch":patch(doc,vec![operation(node,text)]),"changes":changes}))
}
