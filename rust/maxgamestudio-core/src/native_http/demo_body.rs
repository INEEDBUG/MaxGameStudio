use crate::wire_json::integer_from_text;
use serde_json::{json, Map, Value};

pub(super) fn metadata(input: Value, field: &str, max_length: usize) -> Result<String, Vec<Value>> {
    let object = model(&input)?;
    let Some(value) = object.get(field) else {
        return Ok(String::new());
    };
    text(value, field, 0, max_length)
        .map(str::to_owned)
        .map_err(|error| vec![error])
}

pub(super) fn watch_path(input: Value) -> Result<(String, usize), Vec<Value>> {
    let object = model(&input)?;
    let path = match object.get("path") {
        Some(value) => text(value, "path", 1, 4096),
        None => Err(field_issue(
            "path",
            &input,
            "missing",
            "Field required",
            None,
        )),
    };
    let depth = object.get("max_depth").map(depth_value).unwrap_or(Ok(2));
    match (path, depth) {
        (Ok(path), Ok(depth)) => Ok((path.to_owned(), depth)),
        (Err(path), Err(depth)) => Err(vec![path, depth]),
        (Err(error), _) | (_, Err(error)) => Err(vec![error]),
    }
}

pub(super) fn model(input: &Value) -> Result<&Map<String, Value>, Vec<Value>> {
    input.as_object().ok_or_else(|| vec![json!({
        "type": "model_attributes_type", "loc": ["body"],
        "msg": "Input should be a valid dictionary or object to extract fields from", "input": input,
    })])
}

pub(super) fn text<'a>(
    value: &'a Value,
    field: &str,
    min: usize,
    max: usize,
) -> Result<&'a str, Value> {
    let input = value.as_str().ok_or_else(|| {
        field_issue(
            field,
            value,
            "string_type",
            "Input should be a valid string",
            None,
        )
    })?;
    let length = input.chars().count();
    if length < min {
        return Err(field_issue(
            field,
            value,
            "string_too_short",
            "String should have at least 1 character",
            Some(json!({"min_length": min})),
        ));
    }
    if length > max {
        return Err(field_issue(
            field,
            value,
            "string_too_long",
            &format!("String should have at most {max} characters"),
            Some(json!({"max_length": max})),
        ));
    }
    Ok(input)
}

fn depth_value(value: &Value) -> Result<usize, Value> {
    let issue = |kind, message, context| field_issue("max_depth", value, kind, message, context);
    let number = match value {
        Value::Bool(value) => f64::from(u8::from(*value)),
        Value::String(value) => integer_from_text(value)
            .and_then(|number| number.as_f64())
            .ok_or_else(|| {
                issue(
                    "int_parsing",
                    "Input should be a valid integer, unable to parse string as an integer",
                    None,
                )
            })?,
        Value::Number(number) => {
            let value = number.as_f64().unwrap_or(f64::NAN);
            if value.fract() != 0.0 {
                return Err(issue(
                    "int_from_float",
                    "Input should be a valid integer, got a number with a fractional part",
                    None,
                ));
            }
            value
        }
        _ => return Err(issue("int_type", "Input should be a valid integer", None)),
    };
    if number < 0.0 {
        return Err(issue(
            "greater_than_equal",
            "Input should be greater than or equal to 0",
            Some(json!({"ge": 0})),
        ));
    }
    if number > 32.0 {
        return Err(issue(
            "less_than_equal",
            "Input should be less than or equal to 32",
            Some(json!({"le": 32})),
        ));
    }
    Ok(number as usize)
}

pub(super) fn field_issue(
    field: &str,
    input: &Value,
    kind: &str,
    message: &str,
    context: Option<Value>,
) -> Value {
    let mut value = json!({"type": kind, "loc": ["body", field], "msg": message, "input": input});
    if let Some(context) = context {
        value["ctx"] = context;
    }
    value
}
