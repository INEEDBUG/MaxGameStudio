use super::demo_body::{bounded_float, field_issue, model, text};
use crate::{replay_cache::ReplayRequest, wire_json::integer_from_text};
use serde_json::{json, Map, Value};

pub(super) fn parse(input: Value) -> Result<ReplayRequest, Vec<Value>> {
    let object = model(&input)?;
    let mut fields = ReplayFields {
        input: &input,
        object,
        errors: Vec::new(),
    };
    let path = fields.string("path", true, false);
    fields.string("map_name", false, false);
    let start_tick = fields.tick("start_tick", 0, false);
    let end_tick = fields.tick("end_tick", 0, true);
    let tick_rate = fields.rate("tick_rate", 0.0, 256.0, 64.0, true);
    let fps = fields.rate("fps", 1.0, 64.0, 32.0, false);
    fields.string("pov_player_name", false, true);
    fields.string("pov_steamid64", false, true);
    if !fields.errors.is_empty() {
        return Err(fields.errors);
    }
    Ok(ReplayRequest {
        path: path.unwrap(),
        start_tick,
        end_tick,
        tick_rate,
        fps,
    })
}

struct ReplayFields<'a> {
    input: &'a Value,
    object: &'a Map<String, Value>,
    errors: Vec<Value>,
}

impl ReplayFields<'_> {
    fn required(&mut self, name: &str) -> Option<&Value> {
        let value = self.object.get(name);
        if value.is_none() {
            self.errors.push(field_issue(
                name,
                self.input,
                "missing",
                "Field required",
                None,
            ));
        }
        value
    }

    fn string(&mut self, name: &str, required: bool, nullable: bool) -> Option<String> {
        let min = usize::from(required);
        let value = if required {
            self.required(name)?
        } else {
            self.object.get(name)?
        };
        if nullable && value.is_null() {
            return None;
        }
        match text(value, name, min, usize::MAX) {
            Ok(value) => Some(value.to_owned()),
            Err(error) => {
                self.errors.push(error);
                None
            }
        }
    }

    fn tick(&mut self, name: &str, minimum: i64, exclusive: bool) -> i64 {
        let Some(input) = self.required(name) else {
            return 0;
        };
        let issue =
            |kind: &str, message: &str, context| field_issue(name, input, kind, message, context);
        let parsed = match input {
            Value::Number(number) if number.is_i64() => Ok(number.as_i64().unwrap()),
            Value::Number(number) => match number.as_f64() {
                Some(value) if value.fract() != 0.0 => Err(issue(
                    "int_from_float",
                    "Input should be a valid integer, got a number with a fractional part",
                    None,
                )),
                Some(value) if value >= i64::MIN as f64 && value < i64::MAX as f64 => {
                    Ok(value as i64)
                }
                _ => Err(issue(
                    "int_parsing_size",
                    "Unable to parse input string as an integer, exceeded maximum size",
                    None,
                )),
            },
            Value::Bool(value) => Ok(i64::from(*value)),
            Value::String(text) => integer_from_text(text)
                .and_then(|number| number.as_i64())
                .ok_or_else(|| {
                    issue(
                        "int_parsing",
                        "Input should be a valid integer, unable to parse string as an integer",
                        None,
                    )
                }),
            _ => Err(issue("int_type", "Input should be a valid integer", None)),
        };
        let result = parsed.and_then(|tick| {
            if exclusive && tick <= minimum {
                Err(issue(
                    "greater_than",
                    &format!("Input should be greater than {minimum}"),
                    Some(json!({"gt": minimum})),
                ))
            } else if tick < minimum {
                Err(issue(
                    "greater_than_equal",
                    &format!("Input should be greater than or equal to {minimum}"),
                    Some(json!({"ge": minimum})),
                ))
            } else {
                Ok(tick)
            }
        });
        match result {
            Ok(value) => value,
            Err(error) => {
                self.errors.push(error);
                0
            }
        }
    }

    fn rate(&mut self, name: &str, min: f64, max: f64, default: f64, exclusive: bool) -> f64 {
        let Some(input) = self.object.get(name) else {
            return default;
        };
        match bounded_float(input, name, min, max, exclusive) {
            Ok(value) => value,
            Err(error) => {
                self.errors.push(error);
                default
            }
        }
    }
}
