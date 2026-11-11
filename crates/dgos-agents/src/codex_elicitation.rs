use serde_json::{Map, Value};

const LIMIT: usize = 64 * 1024;
const SAFE_INTEGER: u64 = 9_007_199_254_740_991;

// Deliberately a bounded subset, shared with NativeElicitation. Unknown required
// constraints cannot be accepted; optional unsupported properties must be omitted.
pub(super) fn valid_response(params: &Value, response: &Value) -> bool {
    let Some(object) = response.as_object() else { return false; };
    if object.keys().any(|key| !matches!(key.as_str(), "action" | "content" | "_meta"))
        || !bounded(response, 0).is_some_and(|size| size <= LIMIT)
        || response.get("_meta").is_some_and(|meta| !meta.is_null() && !meta.is_object()) { return false; }
    let content = response.get("content").filter(|value| !value.is_null());
    match response.get("action").and_then(Value::as_str) {
        Some("decline" | "cancel") => content.is_none(),
        Some("accept") => match params.get("mode").map_or(Some("form"), Value::as_str).unwrap_or("") {
            "url" => content.is_none(),
            "form" => params.get("requestedSchema").zip(content).is_some_and(|(schema, content)| valid_form(schema, content)),
            _ => false,
        },
        _ => false,
    }
}

fn bounded(value: &Value, depth: usize) -> Option<usize> {
    if depth > 16 { return None; }
    let mut size = 2usize;
    match value {
        Value::String(text) => size = text.len().checked_mul(6)?.checked_add(2)?,
        Value::Array(values) => for value in values { size = size.checked_add(bounded(value, depth + 1)? + 1)?; if size > LIMIT { return None; } },
        Value::Object(values) => for (key, value) in values { size = size.checked_add(key.len().checked_mul(6)? + bounded(value, depth + 1)? + 4)?; if size > LIMIT { return None; } },
        _ => size = 32,
    }
    (size <= LIMIT).then_some(size)
}

fn valid_form(schema: &Value, content: &Value) -> bool {
    if bounded(schema, 0).is_none() || bounded(content, 0).is_none() { return false; }
    let (Some(schema), Some(values)) = (schema.as_object(), content.as_object()) else { return false; };
    if !keys(schema, &["type", "properties", "required", "additionalProperties", "title", "description", "$schema"])
        || schema.get("type").and_then(Value::as_str) != Some("object") || !annotations(schema)
        || schema.get("$schema").is_some_and(|value| !value.is_null() && !value.is_string())
        || schema.get("additionalProperties").is_some_and(|value| !value.is_boolean()) { return false; }
    let Some(fields) = schema.get("properties").and_then(Value::as_object).filter(|fields| fields.len() <= 64) else { return false; };
    if let Some(required) = schema.get("required").filter(|value| !value.is_null()) {
        let Some(required) = required.as_array() else { return false; };
        let mut names = std::collections::HashSet::new();
        for name in required {
            let Some(name) = name.as_str() else { return false; };
            if !names.insert(name) || !fields.contains_key(name) || !values.contains_key(name) { return false; }
        }
    }
    values.iter().all(|(key, value)| fields.get(key).is_some_and(|field| valid_field(field, value)))
}

fn keys(object: &Map<String, Value>, allowed: &[&str]) -> bool { object.keys().all(|key| allowed.contains(&key.as_str())) }
fn annotations(field: &Map<String, Value>) -> bool { ["title", "description"].iter().all(|key| field.get(*key).is_none_or(|value| value.is_null() || value.is_string())) }
fn count(field: &Map<String, Value>, key: &str) -> Option<Option<u64>> {
    match field.get(key).filter(|value| !value.is_null()) { None => Some(None), Some(value) => value.as_u64().filter(|value| *value <= SAFE_INTEGER).map(Some) }
}
fn limits(field: &Map<String, Value>, min: &str, max: &str, actual: u64) -> bool {
    let (Some(min), Some(max)) = (count(field, min), count(field, max)) else { return false; };
    min.zip(max).is_none_or(|(min, max)| min <= max) && min.is_none_or(|min| actual >= min) && max.is_none_or(|max| actual <= max)
}
fn choices(field: &Map<String, Value>) -> Option<Vec<&str>> {
    let variants = ["enum", "oneOf", "anyOf"].into_iter().filter(|key| field.get(*key).is_some_and(|value| !value.is_null())).collect::<Vec<_>>();
    if variants.len() != 1 { return None; }
    let key = variants[0];
    let rows = field.get(key)?.as_array()?.as_slice();
    if rows.is_empty() || rows.len() > 256 { return None; }
    let mut choices = Vec::with_capacity(rows.len());
    for row in rows {
        let value = if key == "enum" { row.as_str()? } else {
            let row = row.as_object()?;
            if !keys(row, &["const", "title"]) || row.get("title").is_some_and(|title| !title.is_string()) { return None; }
            row.get("const")?.as_str()?
        };
        if choices.contains(&value) { return None; }
        choices.push(value);
    }
    if let Some(labels) = field.get("enumNames").filter(|value| !value.is_null()) {
        let labels = labels.as_array()?;
        if key != "enum" || labels.len() != choices.len() || !labels.iter().all(Value::is_string) { return None; }
    }
    Some(choices)
}
fn valid_field(field: &Value, value: &Value) -> bool {
    let Some(field) = field.as_object() else { return false; };
    if !annotations(field) { return false; }
    match field.get("type").and_then(Value::as_str) {
        Some("boolean") => keys(field, &["type", "title", "description", "default"]) && value.is_boolean(),
        Some("number" | "integer") => {
            if !keys(field, &["type", "title", "description", "default", "minimum", "maximum"]) { return false; }
            let Some(number) = value.as_f64().filter(|value| value.is_finite()) else { return false; };
            if field.get("type").and_then(Value::as_str) == Some("integer") && number.fract() != 0.0 { return false; }
            let bound = |key| match field.get(key).filter(|value| !value.is_null()) { None => Some(None), Some(value) => value.as_f64().filter(|value| value.is_finite()).map(Some) };
            let (Some(min), Some(max)) = (bound("minimum"), bound("maximum")) else { return false; };
            min.zip(max).is_none_or(|(min, max)| min <= max) && min.is_none_or(|min| number >= min) && max.is_none_or(|max| number <= max)
        },
        Some("array") => {
            if !keys(field, &["type", "title", "description", "default", "items", "minItems", "maxItems", "uniqueItems"])
                || field.get("uniqueItems").is_some_and(|value| !value.is_boolean()) { return false; }
            let Some(items) = field.get("items").and_then(Value::as_object) else { return false; };
            if !keys(items, &["type", "enum", "enumNames", "oneOf", "anyOf"]) || items.get("type").is_some_and(|value| value != "string") { return false; }
            let (Some(choices), Some(values)) = (choices(items), value.as_array()) else { return false; };
            limits(field, "minItems", "maxItems", values.len() as u64) && values.iter().enumerate().all(|(index, value)| value.as_str().is_some_and(|value| choices.contains(&value)) && !values[..index].contains(value))
        },
        Some("string") => {
            if !keys(field, &["type", "title", "description", "default", "enum", "enumNames", "oneOf", "anyOf", "minLength", "maxLength", "format"]) { return false; }
            let Some(text) = value.as_str() else { return false; };
            if !limits(field, "minLength", "maxLength", text.chars().count() as u64) { return false; }
            if ["enum", "oneOf", "anyOf", "enumNames"].iter().any(|key| field.contains_key(*key)) && !choices(field).is_some_and(|choices| choices.contains(&text)) { return false; }
            match field.get("format").filter(|value| !value.is_null()).and_then(Value::as_str) {
                None => field.get("format").is_none_or(|value| value.is_null()),
                Some("email") => text.split_once('@').is_some_and(|(local, domain)| !local.is_empty() && !domain.contains('@') && domain.split_once('.').is_some_and(|(left, right)| !left.is_empty() && !right.is_empty())) && !text.chars().any(char::is_whitespace),
                Some("uri") => reqwest::Url::parse(text).is_ok(),
                Some("date") => date(text),
                Some("date-time") => date_time(text),
                _ => false,
            }
        },
        _ => false,
    }
}
fn date(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' || bytes.iter().enumerate().any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit()) { return false; }
    let year = text[..4].parse::<u32>().unwrap_or(0);
    let month = text[5..7].parse::<usize>().unwrap_or(0);
    let day = text[8..].parse::<u32>().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    month > 0 && month <= 12 && day > 0 && day <= days[month - 1]
}
fn date_time(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || !text.get(..10).is_some_and(date) || !matches!(bytes[10], b'T' | b't') || bytes[13] != b':' || bytes[16] != b':' { return false; }
    let pair = |offset: usize, max: u8| bytes[offset..offset + 2].iter().all(u8::is_ascii_digit) && (bytes[offset] - b'0') * 10 + bytes[offset + 1] - b'0' <= max;
    if !pair(11, 23) || !pair(14, 59) || !pair(17, 59) { return false; }
    let mut offset = 19;
    if bytes.get(offset) == Some(&b'.') { offset += 1; let start = offset; while bytes.get(offset).is_some_and(u8::is_ascii_digit) { offset += 1; } if start == offset { return false; } }
    match bytes.get(offset) {
        Some(b'Z' | b'z') => offset + 1 == bytes.len(),
        Some(b'+' | b'-') => offset + 6 == bytes.len() && bytes[offset + 3] == b':' && pair(offset + 1, 23) && pair(offset + 4, 59),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn accepts_only_supported_constraints_and_omittable_unknown_fields() {
        let schema = json!({"type":"object","properties":{"name":{"type":"string","minLength":2,"maxLength":4},"count":{"type":"integer","minimum":1,"maximum":3},"later":{"type":"string","pattern":"^x$"}},"required":["name","count"]});
        let pending = json!({"mode":"form","requestedSchema":schema});
        assert!(valid_response(&pending, &json!({"action":"accept","content":{"name":"ab","count":2}})));
        for content in [json!({"name":"a","count":2}), json!({"name":"ab","count":2.5}), json!({"name":"ab","count":2,"later":"x"}), json!({"name":"ab","count":2,"extra":true})] { assert!(!valid_response(&pending, &json!({"action":"accept","content":content}))); }
        let mut required = pending.clone(); required["requestedSchema"]["required"] = json!(["later"]);
        assert!(!valid_response(&required, &json!({"action":"accept","content":{"later":"x"}})));
        assert!(valid_response(&required, &json!({"action":"decline"})));
        assert!(!valid_response(&required, &json!({"action":"decline","content":{"later":"x"}})));
    }
    #[test]
    fn validates_enums_formats_root_constraints_and_bounds() {
        let schema = json!({"type":"object","properties":{"choice":{"type":"string","oneOf":[{"const":"a","title":"A"},{"const":"b","title":"B"}]},"tags":{"type":"array","items":{"anyOf":[{"const":"x","title":"X"}]},"minItems":1,"maxItems":2},"date":{"type":"string","format":"date"},"time":{"type":"string","format":"date-time"}},"required":["choice","tags","date","time"]});
        let content = json!({"choice":"a","tags":["x"],"date":"2024-02-29","time":"2024-02-29T23:59:01.123+08:00"});
        assert!(valid_form(&schema, &content));
        for (key, value) in [("choice",json!("c")),("tags",json!(["x","x"])),("date",json!("2023-02-29")),("time",json!("2024-02-29T25:00:00Z"))] { let mut invalid = content.clone(); invalid[key] = value; assert!(!valid_form(&schema, &invalid)); }
        for key in ["allOf", "patternProperties", "unevaluatedProperties"] { let mut invalid = schema.clone(); invalid[key] = json!([]); assert!(!valid_form(&invalid, &content)); }
        assert!(!valid_form(&json!({"type":"object","properties":{"x":{"type":"string","minLength":-1}},"required":["x"]}), &json!({"x":"a"})));
        assert!(!valid_form(&schema, &json!({"choice":"x".repeat(LIMIT)})));
        assert!(valid_response(&json!({"mode":"url"}), &json!({"action":"accept"})));
        assert!(!valid_response(&json!({"mode":"verification"}), &json!({"action":"accept"})));
    }
}
