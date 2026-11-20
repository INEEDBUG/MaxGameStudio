//! Compatibility helpers for the existing Python JSON and query contracts.

use serde_json::Number;

pub(crate) fn integer_from_text(text: &str) -> Option<Number> {
    let text = text.trim();
    let numeric = text.trim_start_matches(['-', '+']);
    if numeric.split('_').any(str::is_empty) {
        return None;
    }
    let replaced = text.replace('_', "");
    let whole = match replaced.split_once('.') {
        Some((whole, decimal))
            if !decimal.is_empty() && decimal.bytes().all(|byte| byte == b'0') =>
        {
            whole
        }
        Some(_) => return None,
        None => &replaced,
    };
    whole
        .parse::<i64>()
        .map(Number::from)
        .or_else(|_| whole.parse::<u64>().map(Number::from))
        .ok()
}

pub(crate) fn json_decode_error(text: &str, error: &serde_json::Error) -> (usize, &'static str) {
    let offset = if error.is_eof() {
        text.chars().count()
    } else {
        let before_line: usize = text
            .split_inclusive('\n')
            .take(error.line().saturating_sub(1))
            .map(str::len)
            .sum();
        let byte_offset = (before_line + error.column().saturating_sub(1)).min(text.len());
        text.char_indices()
            .take_while(|(index, _)| *index < byte_offset)
            .count()
    };
    let message = error.to_string();
    let description = if message.starts_with("trailing characters") {
        "Extra data"
    } else if message.starts_with("key must be a string") {
        "Expecting property name enclosed in double quotes"
    } else if message.starts_with("expected :") || message.starts_with("expected `:`") {
        "Expecting ':' delimiter"
    } else if message.starts_with("expected `,` or") {
        "Expecting ',' delimiter"
    } else {
        "Expecting value"
    };
    (offset, description)
}
