use serde_json::{json, Map, Number, Value};
use std::sync::LazyLock;

pub(super) type Object = Map<String, Value>;

pub(super) static DEFAULTS: LazyLock<Object> = LazyLock::new(|| {
    serde_json::from_str(include_str!("defaults.json")).expect("built-in profile defaults")
});

// Request ordering is observable in FastAPI's validation responses.
const PATCH_FIELDS: &[&str] = &[
    "obs",
    "llm",
    "ffmpeg_path",
    "montage_encoder",
    "cs2_path",
    "demo_watch_paths",
    "demo_watch_scan_depth",
    "ai_mode",
    "obs_agent_auto_prepare",
    "locale",
    "close_to_tray",
    "close_action",
    "expected_parse_players",
    "recording_global_pacing",
    "default_record_warmup",
    "cs2_extra_launch_args",
    "cs2_extra_launch_args_user_configured",
    "record_inject_console_lines",
    "record_inject_console_lines_user_configured",
    "obs_transition_enabled",
    "obs_transition_name",
    "obs_transition_duration_ms",
    "kb_overlay_enabled",
    "kb_overlay_tick_offset",
    "kb_overlay_position",
    "kill_fx_enabled",
    "kill_fx_tick_offset",
    "experimental",
    "steam_api_key",
    "steam_id64",
    "steam_game_auth_code",
    "steam_known_share_code",
    "match_mode",
    "match_count",
    "update_check_frequency",
    "last_update_check_at",
];

pub(super) fn normalize_config(input: &Object) -> Option<Object> {
    let mut validator = Validator::default();
    let object = validator.fields(input, &DEFAULTS, DEFAULTS.keys().map(String::as_str), true);
    validator.errors.is_empty().then_some(object)
}

pub(super) fn validate_patch(input: Value) -> Result<Object, Vec<Value>> {
    let mut validator = Validator {
        location: vec![json!("body")],
        errors: Vec::new(),
    };
    let Some(input) = input.as_object() else {
        validator.model_error(&input, "ConfigPayload");
        return Err(validator.errors);
    };
    let object = validator.fields(input, &DEFAULTS, PATCH_FIELDS.iter().copied(), false);
    if validator.errors.is_empty() {
        Ok(object)
    } else {
        Err(validator.errors)
    }
}

#[derive(Default)]
struct Validator {
    location: Vec<Value>,
    errors: Vec<Value>,
}

impl Validator {
    fn fields<'a>(
        &mut self,
        input: &Object,
        defaults: &Object,
        fields: impl Iterator<Item = &'a str>,
        fill_defaults: bool,
    ) -> Object {
        let mut output = if fill_defaults {
            defaults.clone()
        } else {
            Object::new()
        };
        for name in fields {
            let Some(value) = input.get(name) else {
                continue;
            };
            if value.is_null() && !fill_defaults {
                continue;
            }
            self.location.push(json!(name));
            let validated = self.value(name, value, &defaults[name], fill_defaults);
            self.location.pop();
            if let Some(value) = validated {
                output.insert(name.to_owned(), value);
            }
        }
        output
    }

    fn value(
        &mut self,
        name: &str,
        input: &Value,
        default: &Value,
        fill_defaults: bool,
    ) -> Option<Value> {
        if let Some((model, fields)) = nested_model(name, fill_defaults) {
            let Some(input) = input.as_object() else {
                self.model_error(input, model);
                return None;
            };
            let defaults = default.as_object().expect("nested model defaults");
            let fill_defaults = fill_defaults || name != "experimental";
            return Some(Value::Object(self.fields(
                input,
                defaults,
                fields.iter().copied(),
                fill_defaults,
            )));
        }

        let validated = match name {
            "close_action" => {
                if matches!(input.as_str(), Some("ask" | "tray" | "exit")) {
                    Ok(input.clone())
                } else {
                    self.error(
                        input,
                        "literal_error",
                        "Input should be 'ask', 'tray' or 'exit'",
                        Some(json!({"expected": "'ask', 'tray' or 'exit'"})),
                    );
                    return None;
                }
            }
            "base_url" if input.is_null() => Ok(Value::Null),
            "base_url" => input.as_str().map(normalize_base_url).ok_or(STRING_TYPE),
            "pov_goto_delay_extra_sec" if input.is_null() => Ok(Value::Null),
            "pov_goto_delay_extra_sec" => number(input),
            _ if default.is_string() => input.as_str().map(|_| input.clone()).ok_or(STRING_TYPE),
            _ if default.is_boolean() => boolean(input),
            _ if default.is_array() => {
                let Some(items) = input.as_array() else {
                    self.error(input, "list_type", "Input should be a valid list", None);
                    return None;
                };
                for (index, item) in items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| !item.is_string())
                {
                    self.location.push(json!(index));
                    self.error(item, STRING_TYPE.0, STRING_TYPE.1, None);
                    self.location.pop();
                }
                Ok(input.clone())
            }
            _ if default.is_object() => input
                .as_object()
                .map(|_| input.clone())
                .ok_or(("dict_type", "Input should be a valid dictionary")),
            _ if default.is_f64() => number(input),
            _ => integer(input),
        };

        match validated {
            Ok(value) => {
                self.check_bounds(name, input, &value);
                Some(value)
            }
            Err((kind, message)) => {
                self.error(input, kind, message, None);
                None
            }
        }
    }

    fn check_bounds(&mut self, name: &str, input: &Value, value: &Value) {
        let (min, max) = match name {
            "demo_watch_scan_depth" => (json!(0), json!(32)),
            "demo_timescale" => (json!(0.01), json!(1.0)),
            "max_retries" => (json!(1), json!(16)),
            "per_retry_timeout_sec" => (json!(0.05), json!(5.0)),
            "settle_sec" => (json!(0.0), json!(2.0)),
            "pov_goto_delay_extra_sec" => (json!(0.0), json!(20.0)),
            _ => return,
        };
        if let Some(value) = value.as_f64() {
            if value < min.as_f64().unwrap() {
                self.error(
                    input,
                    "greater_than_equal",
                    &format!("Input should be greater than or equal to {min}"),
                    Some(json!({"ge": min})),
                );
            } else if value > max.as_f64().unwrap() {
                self.error(
                    input,
                    "less_than_equal",
                    &format!("Input should be less than or equal to {max}"),
                    Some(json!({"le": max})),
                );
            }
        }
    }

    fn model_error(&mut self, input: &Value, class: &str) {
        self.error(
            input,
            "model_type",
            &format!("Input should be a valid dictionary or instance of {class}"),
            Some(json!({"class_name": class})),
        );
    }

    fn error(&mut self, input: &Value, kind: &str, message: &str, context: Option<Value>) {
        let mut value = json!({"type": kind, "loc": self.location, "msg": message, "input": input});
        if let Some(context) = context {
            value["ctx"] = context;
        }
        self.errors.push(value);
    }
}

fn nested_model(
    name: &str,
    fill_defaults: bool,
) -> Option<(&'static str, &'static [&'static str])> {
    match name {
        "obs" => Some((
            "OBSConfig",
            &[
                "host",
                "port",
                "password",
                "obs_path",
                "obs_config_verified",
            ],
        )),
        "llm" => Some(("LLMConfig", &["provider", "model", "api_key", "base_url"])),
        "experimental" => Some((
            if fill_defaults {
                "ExperimentalConfig"
            } else {
                "ExperimentalPayload"
            },
            &["pov_enabled"],
        )),
        "spec_player_verify" => Some((
            "SpecPlayerVerifyConfig",
            &[
                "demo_timescale",
                "max_retries",
                "per_retry_timeout_sec",
                "settle_sec",
                "pov_goto_delay_extra_sec",
            ],
        )),
        _ => None,
    }
}

type Invalid = (&'static str, &'static str);
const STRING_TYPE: Invalid = ("string_type", "Input should be a valid string");
const BOOLEAN_TYPE: Invalid = ("bool_type", "Input should be a valid boolean");
const BOOLEAN_PARSING: Invalid = (
    "bool_parsing",
    "Input should be a valid boolean, unable to interpret input",
);
const INTEGER_PARSING: Invalid = (
    "int_parsing",
    "Input should be a valid integer, unable to parse string as an integer",
);

fn boolean(input: &Value) -> Result<Value, Invalid> {
    match input {
        Value::Bool(_) => Ok(input.clone()),
        Value::Number(number) => match number.as_f64().unwrap_or(f64::NAN) {
            0.0 => Ok(json!(false)),
            1.0 => Ok(json!(true)),
            other if other.fract() == 0.0 => Err(BOOLEAN_PARSING),
            _ => Err(BOOLEAN_TYPE),
        },
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "0" | "off" | "f" | "false" | "n" | "no" => Ok(json!(false)),
            "1" | "on" | "t" | "true" | "y" | "yes" => Ok(json!(true)),
            _ => Err(BOOLEAN_PARSING),
        },
        _ => Err(BOOLEAN_TYPE),
    }
}

fn integer(input: &Value) -> Result<Value, Invalid> {
    match input {
        Value::Bool(value) => Ok(json!(i64::from(*value))),
        Value::Number(value) if value.is_i64() || value.is_u64() => Ok(input.clone()),
        Value::Number(value) => match value.as_f64().unwrap_or(f64::NAN) {
            v if v.fract() != 0.0 => Err((
                "int_from_float",
                "Input should be a valid integer, got a number with a fractional part",
            )),
            v if v >= i64::MIN as f64 && v < 0.0 => Ok(json!(v as i64)),
            v if v >= 0.0 && v < u64::MAX as f64 => Ok(json!(v as u64)),
            _ => Err((
                "int_parsing_size",
                "Unable to parse input string as an integer, exceeded maximum size",
            )),
        },
        Value::String(text) => {
            let text = text.trim();
            let numeric = text.trim_start_matches(['-', '+']);
            if numeric.split('_').any(|part| part.is_empty()) {
                return Err(INTEGER_PARSING);
            }
            let replaced = text.replace('_', "");
            let whole = match replaced.split_once('.') {
                Some((whole, decimal))
                    if !decimal.is_empty() && decimal.bytes().all(|byte| byte == b'0') =>
                {
                    whole
                }
                Some(_) => return Err(INTEGER_PARSING),
                None => &replaced,
            };
            whole
                .parse::<i64>()
                .map(Number::from)
                .or_else(|_| whole.parse::<u64>().map(Number::from))
                .map(Value::Number)
                .map_err(|_| INTEGER_PARSING)
        }
        _ => Err(("int_type", "Input should be a valid integer")),
    }
}

fn number(input: &Value) -> Result<Value, Invalid> {
    let value = match input {
        Value::Bool(value) => f64::from(u8::from(*value)),
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(value) => value.trim().replace('_', "").parse::<f64>().map_err(|_| {
            (
                "float_parsing",
                "Input should be a valid number, unable to parse string as a number",
            )
        })?,
        _ => return Err(("float_type", "Input should be a valid number")),
    };
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or(("finite_number", "Input should be a finite number"))
}

fn normalize_base_url(input: &str) -> Value {
    let mut url = input.trim().trim_end_matches('/');
    for suffix in ["/chat/completions", "/completions"] {
        if let Some(base) = url.strip_suffix(suffix) {
            url = base.trim_end_matches('/');
            break;
        }
    }
    if url.is_empty() {
        Value::Null
    } else {
        json!(url)
    }
}
