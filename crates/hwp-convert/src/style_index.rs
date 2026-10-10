//! Value-keyed style indexes retain first-use order without quadratic scans.
use hwp_core::errors::{Result, WorkbenchError};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

pub fn immutable_value<T: Serialize>(value: &T) -> Result<String> {
    let mut value = serde_json::to_value(value)
        .map_err(|error| WorkbenchError::new("BAD_ARGUMENTS", error.to_string()))?;
    canonicalize(&mut value);
    serde_json::to_string(&value)
        .map_err(|error| WorkbenchError::new("BAD_ARGUMENTS", error.to_string()))
}
fn canonicalize(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for item in object.values_mut() {
                canonicalize(item);
            }
            object.sort_keys();
        }
        Value::Array(items) => {
            for item in items {
                canonicalize(item);
            }
        }
        Value::Number(number) => {
            if let Some(float) = number.as_f64() {
                if number.is_f64()
                    && float.fract() == 0.0
                    && float >= i64::MIN as f64
                    && float < i64::MAX as f64
                {
                    *value = Value::from(float as i64);
                }
            }
        }
        _ => {}
    }
}
pub fn indexed_value<T: Serialize + Clone>(
    items: &mut Vec<T>,
    indices: &mut HashMap<String, usize>,
    value: T,
) -> Result<usize> {
    let key = immutable_value(&value)?;
    if let Some(index) = indices.get(&key) {
        return Ok(*index);
    }
    let index = items.len();
    indices.insert(key, index);
    items.push(value);
    Ok(index)
}
