//! Loss reports and readable-paragraph evidence for the shared service.
use crate::model::RichDocument;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

/// Maximum paragraph-pair work before the bounded fallback comparison.
pub const MAX_TEXT_DIFF_WORK: usize = 1_000_000;
/// Maximum entries per detail array; total counts remain complete.
pub const MAX_TEXT_DIFF_DETAILS: usize = 1000;

pub fn mapper_report(source: &str, target: &str, document: &RichDocument) -> Value {
    // Retain the engine label for compatibility with existing service clients.
    json!({"engine":"direct mapper","from":source,"to":target,"unsupported":document.unsupported,"layoutCertified":false,"scope":"supported properties only; fonts, pagination and Hancom layout are unqualified"})
}
/// A native object can appear in both source and pivot reports. Count it once.
pub fn merge_unsupported(target: &mut Map<String, Value>, incoming: &Map<String, Value>) {
    for (feature, count) in incoming {
        let old = target.get(feature).and_then(Value::as_u64).unwrap_or(0);
        target.insert(
            feature.clone(),
            Value::from(old.max(count.as_u64().unwrap_or(0))),
        );
    }
}
fn normalize(text: &str) -> String {
    text.nfc()
        .collect::<String>()
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
        })
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn preview(text: &str) -> String {
    text.chars().take(120).collect()
}
#[derive(Clone, Copy, Debug)]
struct Match {
    a: usize,
    b: usize,
    size: usize,
}
/// SequenceMatcher without junk heuristics. The longest-match ties select the
/// earliest source position, then earliest output position, as difflib does.
fn matching_blocks(source: &[String], output: &[String]) -> Vec<Match> {
    let mut positions: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, text) in output.iter().enumerate() {
        positions.entry(text).or_default().push(index);
    }
    let mut pending = vec![(0, source.len(), 0, output.len())];
    let mut found = Vec::new();
    while let Some((alo, ahi, blo, bhi)) = pending.pop() {
        let mut best = Match {
            a: alo,
            b: blo,
            size: 0,
        };
        let mut previous: HashMap<usize, usize> = HashMap::new();
        for (i, text) in source.iter().enumerate().take(ahi).skip(alo) {
            let mut current = HashMap::new();
            if let Some(indices) = positions.get(text.as_str()) {
                for &j in indices.iter().filter(|j| **j >= blo && **j < bhi) {
                    let size = j
                        .checked_sub(1)
                        .and_then(|j| previous.get(&j))
                        .copied()
                        .unwrap_or(0)
                        + 1;
                    current.insert(j, size);
                    if size > best.size {
                        best = Match {
                            a: i + 1 - size,
                            b: j + 1 - size,
                            size,
                        };
                    }
                }
            }
            previous = current;
        }
        if best.size > 0 {
            if alo < best.a && blo < best.b {
                pending.push((alo, best.a, blo, best.b));
            }
            if best.a + best.size < ahi && best.b + best.size < bhi {
                pending.push((best.a + best.size, ahi, best.b + best.size, bhi));
            }
            found.push(best);
        }
    }
    found.sort_by_key(|item| (item.a, item.b, item.size));
    let mut merged: Vec<Match> = Vec::new();
    for item in found {
        if let Some(last) = merged.last_mut() {
            if last.a + last.size == item.a && last.b + last.size == item.b {
                last.size += item.size;
                continue;
            }
        }
        merged.push(item);
    }
    merged.push(Match {
        a: source.len(),
        b: output.len(),
        size: 0,
    });
    merged
}
pub fn text_check(before: &[String], after: &[String], source: &str) -> Value {
    let source_texts: Vec<String> = before.iter().map(|text| normalize(text)).collect();
    let output_texts: Vec<String> = after.iter().map(|text| normalize(text)).collect();
    let (mut missing, mut changed, mut added) = (Vec::new(), Vec::new(), Vec::new());
    let comparison;
    if before
        .len()
        .checked_mul(after.len())
        .is_some_and(|work| work <= MAX_TEXT_DIFF_WORK)
    {
        let (mut a, mut b) = (0, 0);
        for block in matching_blocks(&source_texts, &output_texts) {
            let paired = (block.a - a).min(block.b - b);
            for (offset, (old, new)) in before
                .iter()
                .skip(a)
                .zip(after.iter().skip(b))
                .take(paired)
                .enumerate()
            {
                changed.push(json!({"sourceIndex":a + offset,"outputIndex":b + offset,"before":preview(old),"after":preview(new)}));
            }
            for (i, text) in before.iter().enumerate().take(block.a).skip(a + paired) {
                missing.push(json!({"sourceIndex":i,"text":preview(text)}));
            }
            for (j, text) in after.iter().enumerate().take(block.b).skip(b + paired) {
                added.push(json!({"outputIndex":j,"text":preview(text)}));
            }
            a = block.a + block.size;
            b = block.b + block.size;
        }
        comparison = "aligned paragraph sequence";
    } else {
        let mut remaining: HashMap<&str, usize> = HashMap::new();
        for text in &output_texts {
            *remaining.entry(text).or_default() += 1;
        }
        for (index, text) in source_texts.iter().enumerate() {
            let count = remaining.entry(text).or_default();
            if *count > 0 {
                *count -= 1;
            } else if let Some(text) = before.get(index) {
                missing.push(json!({"sourceIndex":index,"text":preview(text)}));
            }
        }
        for (index, ((old, new), (old_text, new_text))) in source_texts
            .iter()
            .zip(&output_texts)
            .zip(before.iter().zip(after))
            .enumerate()
        {
            if old != new {
                changed.push(json!({"sourceIndex":index,"outputIndex":index,"before":preview(old_text),"after":preview(new_text)}));
            }
        }
        for (index, text) in after.iter().enumerate().skip(before.len()) {
            added.push(json!({"outputIndex":index,"text":preview(text)}));
        }
        comparison = "bounded multiplicity and index comparison";
    }
    let counts = (missing.len(), changed.len(), added.len());
    missing.truncate(MAX_TEXT_DIFF_DETAILS);
    changed.truncate(MAX_TEXT_DIFF_DETAILS);
    added.truncate(MAX_TEXT_DIFF_DETAILS);
    json!({"performed":true,"source":source,"scope":"readable paragraphs; unsupported object semantics and layout are not checked","normalisation":"Unicode NFC and whitespace collapse within each paragraph","comparison":comparison,"missingParagraphs":missing,"changedParagraphs":changed,"addedParagraphs":added,"missingParagraphCount":counts.0,"changedParagraphCount":counts.1,"addedParagraphCount":counts.2,"detailsTruncated":counts.0 > MAX_TEXT_DIFF_DETAILS || counts.1 > MAX_TEXT_DIFF_DETAILS || counts.2 > MAX_TEXT_DIFF_DETAILS,"sourceParagraphs":before.len(),"outputParagraphs":after.len(),"sourceCharacters":before.iter().map(|text| text.chars().count()).sum::<usize>(),"outputCharacters":after.iter().map(|text| text.chars().count()).sum::<usize>(),"matches":source_texts == output_texts})
}
