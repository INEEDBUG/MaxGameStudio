//! Normalize the persisted LiteCut project body using the v2 editor contract.

use serde_json::{json, Map, Number, Value};
use std::collections::HashSet;

type Object = Map<String, Value>;
type Check = fn(&Object) -> Result<(), &'static str>;

#[derive(Clone, Copy)]
enum Rule {
    Text(usize, usize),
    Integer(i64, i64),
    JsonInteger,
    Float(f64, f64, bool),
    Bool,
    Choice(&'static [&'static str]),
    Version,
    Object,
    Model(&'static Model),
    List(&'static Rule, usize),
}

#[derive(Clone, Copy)]
enum DefaultValue {
    Required,
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(&'static str),
    List,
    Object,
}

struct Field {
    name: &'static str,
    rule: Rule,
    default: DefaultValue,
}

struct Model {
    fields: &'static [Field],
    check: Option<Check>,
}

use DefaultValue as D;
use Rule as R;

const TEXT: Rule = R::Text(0, usize::MAX);
const ID: Rule = R::Text(1, 160);
const TIME: Rule = float(0.0, 86400.0);
const UNIT: Rule = float(0.0, 1.0);
const VOLUME: Rule = float(0.0, 5.0);
const SCALE: Rule = positive(20.0);
const FREE_FLOAT: Rule = float(f64::NEG_INFINITY, f64::INFINITY);
const KEYFRAMES: Rule = R::List(&R::Object, 500);

const fn float(min: f64, max: f64) -> Rule {
    R::Float(min, max, false)
}

const fn positive(max: f64) -> Rule {
    R::Float(0.0, max, true)
}

const fn field(name: &'static str, rule: Rule, default: DefaultValue) -> Field {
    Field {
        name,
        rule,
        default,
    }
}

const fn model(fields: &'static [Field]) -> Model {
    Model {
        fields,
        check: None,
    }
}

const OUTPUT: Model = Model {
    fields: &[
        field("dir", TEXT, D::Text("")),
        field("filename", TEXT, D::Text("lite_cut_export.mp4")),
        field("width", R::Integer(16, 7680), D::Int(1920)),
        field("height", R::Integer(16, 4320), D::Int(1080)),
        field("fps", R::Integer(1, 240), D::Int(60)),
        field(
            "encoder",
            R::Choice(&["auto", "h264_nvenc", "h264_qsv", "h264_amf", "libx264"]),
            D::Text("auto"),
        ),
        field(
            "encoder_tier",
            R::Choice(&["quality", "fast"]),
            D::Text("quality"),
        ),
        field(
            "canvas_fit",
            R::Choice(&["contain", "cover", "blur"]),
            D::Text("contain"),
        ),
        field("background_color", TEXT, D::Text("#000000")),
        field("blur_amount", R::Integer(0, 100), D::Int(24)),
        field(
            "range_mode",
            R::Choice(&["full", "custom"]),
            D::Text("full"),
        ),
        field("range_start_sec", TIME, D::Float(0.0)),
        field("range_end_sec", TIME, D::Null),
    ],
    check: Some(check_output_range),
};

const TRANSITION: Model = model(&[
    field("type", R::Text(1, 64), D::Text("cut")),
    field("duration_sec", float(0.0, 10.0), D::Float(0.5)),
]);

const COLOR: Model = model(&[
    field("brightness", float(-100.0, 100.0), D::Float(0.0)),
    field("contrast", float(-100.0, 100.0), D::Float(0.0)),
    field("saturation", float(-100.0, 100.0), D::Float(0.0)),
    field("filter_preset", TEXT, D::Null),
]);

const CLIP_TRANSFORM: Model = model(&[
    field("x", float(-10.0, 10.0), D::Float(0.5)),
    field("y", float(-10.0, 10.0), D::Float(0.5)),
    field("scale", SCALE, D::Float(1.0)),
    field("rotation", float(-3600.0, 3600.0), D::Float(0.0)),
    field("width", SCALE, D::Float(1.0)),
    field("height", SCALE, D::Float(1.0)),
    field("opacity", UNIT, D::Float(1.0)),
]);

const CROP: Model = Model {
    fields: &[
        field("x", UNIT, D::Float(0.0)),
        field("y", UNIT, D::Float(0.0)),
        field("width", positive(1.0), D::Float(1.0)),
        field("height", positive(1.0), D::Float(1.0)),
    ],
    check: Some(check_crop),
};

const CLIP: Model = Model {
    fields: &[
        field("id", ID, D::Required),
        field(
            "source_type",
            R::Choice(&["recorded_clip", "file", "text", "template_asset"]),
            D::Text("recorded_clip"),
        ),
        field("source_id", R::JsonInteger, D::Null),
        field("file_path", TEXT, D::Null),
        field("timeline_start", TIME, D::Float(0.0)),
        field("trim_in", TIME, D::Float(0.0)),
        field("trim_out", TIME, D::Null),
        field("transition_in", R::Model(&TRANSITION), D::Null),
        field("transition_out", R::Model(&TRANSITION), D::Null),
        field("color", R::Model(&COLOR), D::Null),
        field("transform", R::Model(&CLIP_TRANSFORM), D::Null),
        field("keyframes", KEYFRAMES, D::List),
        field("crop", R::Model(&CROP), D::Null),
        field(
            "canvas_fit",
            R::Choice(&["inherit", "contain", "cover", "blur"]),
            D::Null,
        ),
        field("flip_horizontal", R::Bool, D::Bool(false)),
        field("flip_vertical", R::Bool, D::Bool(false)),
        field("speed", float(0.25, 4.0), D::Float(1.0)),
        field("speed_keyframes", KEYFRAMES, D::List),
        field("preserve_pitch", R::Bool, D::Bool(true)),
        field("reverse", R::Bool, D::Bool(false)),
        field("freeze_frame_sec", float(0.0, 30.0), D::Float(0.0)),
        field("volume", VOLUME, D::Float(1.0)),
        field("audio_keyframes", KEYFRAMES, D::List),
        field("muted", R::Bool, D::Bool(false)),
        field("fade_in_sec", TIME, D::Float(0.0)),
        field("fade_out_sec", TIME, D::Float(0.0)),
        field("meta", R::Object, D::Null),
    ],
    check: Some(check_clip),
};

const TRACK: Model = model(&[
    field("id", ID, D::Required),
    field(
        "type",
        R::Choice(&["video", "overlay", "audio"]),
        D::Required,
    ),
    field("label", TEXT, D::Required),
    field("name", R::Text(0, 60), D::Null),
    field("locked", R::Bool, D::Bool(false)),
    field("hidden", R::Bool, D::Bool(false)),
    field("muted", R::Bool, D::Bool(false)),
    field("solo", R::Bool, D::Bool(false)),
    field("volume", VOLUME, D::Float(1.0)),
    field("clips", R::List(&R::Model(&CLIP), 500), D::List),
]);

const OVERLAY_TEXT: Model = model(&[
    field("content", TEXT, D::Text("")),
    field("font_family", TEXT, D::Text("sans-serif")),
    field("font_file", TEXT, D::Null),
    field("font_size", R::JsonInteger, D::Int(48)),
    field("preset_id", TEXT, D::Null),
    field("anim_in", TEXT, D::Null),
    field("anim_out", TEXT, D::Null),
]);

const OVERLAY_TRANSFORM: Model = model(&[
    field("x", FREE_FLOAT, D::Float(0.5)),
    field("y", FREE_FLOAT, D::Float(0.5)),
    field("scale", SCALE, D::Float(1.0)),
    field("rotation", FREE_FLOAT, D::Float(0.0)),
    field("width", SCALE, D::Float(0.33)),
    field("height", SCALE, D::Float(0.33)),
    field("opacity", UNIT, D::Float(1.0)),
]);

const OVERLAY_KEYFRAME: Model = model(&[
    field("time_sec", TIME, D::Float(0.0)),
    field("transform", R::Model(&OVERLAY_TRANSFORM), D::Object),
]);

const OVERLAY: Model = model(&[
    field("id", ID, D::Required),
    field(
        "type",
        R::Choice(&["text", "sticker", "webm", "name_card"]),
        D::Required,
    ),
    field("timeline_start", TIME, D::Float(0.0)),
    field("duration", positive(86400.0), D::Float(3.0)),
    field("fade_in_sec", TIME, D::Float(0.0)),
    field("fade_out_sec", TIME, D::Float(0.0)),
    field("transition_in", R::Model(&TRANSITION), D::Null),
    field("transition_out", R::Model(&TRANSITION), D::Null),
    field("transform", R::Model(&OVERLAY_TRANSFORM), D::Object),
    field(
        "keyframes",
        R::List(&R::Model(&OVERLAY_KEYFRAME), 500),
        D::List,
    ),
    field("flip_horizontal", R::Bool, D::Bool(false)),
    field("flip_vertical", R::Bool, D::Bool(false)),
    field("text", R::Model(&OVERLAY_TEXT), D::Null),
    field("asset_path", TEXT, D::Null),
    field("meta", R::Object, D::Null),
]);

const BGM: Model = model(&[
    field("path", TEXT, D::Text("")),
    field("name", TEXT, D::Null),
    field("asset_id", R::JsonInteger, D::Null),
    field("duration_sec", FREE_FLOAT, D::Null),
    field("volume", float(0.0, 2.0), D::Float(1.0)),
    field("start_sec", TIME, D::Float(0.0)),
    field("fade_in_sec", TIME, D::Float(0.0)),
    field("fade_out_sec", TIME, D::Float(0.0)),
    field("ducking_enabled", R::Bool, D::Bool(false)),
    field("ducking_volume", UNIT, D::Float(0.35)),
]);

const AUDIO: Model = model(&[
    field("bgm", R::Model(&BGM), D::Null),
    field("master_volume", VOLUME, D::Float(1.0)),
]);

const MARKER: Model = model(&[
    field("id", TEXT, D::Required),
    field("time_sec", FREE_FLOAT, D::Float(0.0)),
    field("label", TEXT, D::Text("")),
    field("color", TEXT, D::Text("#f59e0b")),
]);

const PROJECT: Model = model(&[
    field("schema_version", R::Version, D::Int(2)),
    field("output", R::Model(&OUTPUT), D::Object),
    field("tracks", R::List(&R::Model(&TRACK), 32), D::List),
    field("overlays", R::List(&R::Model(&OVERLAY), 32), D::List),
    field("overlay_tracks", R::List(&R::Object, 32), D::List),
    field("markers", R::List(&R::Model(&MARKER), 500), D::List),
    field("audio", R::Model(&AUDIO), D::Object),
    field("template_id", TEXT, D::Null),
    field("created_from_template", R::Bool, D::Bool(false)),
]);

pub(super) fn normalize(input: Option<Value>) -> Result<Value, String> {
    let mut input = input.unwrap_or(Value::Null);
    if input.is_null() || input.as_object().is_some_and(Map::is_empty) {
        input = json!({
            "tracks": [
                {"id":"v1","type":"video","label":"V1"},
                {"id":"a1","type":"audio","label":"A1"},
            ]
        });
    }
    let project = normalize_model(input, &PROJECT, "body")?;
    let tracks = list(&project["tracks"]);
    check_ids(tracks.iter(), "track")?;
    check_ids(
        tracks.iter().flat_map(|track| list(&track["clips"])),
        "clip",
    )?;
    check_ids(list(&project["overlays"]).iter(), "overlay")?;
    check_ids(list(&project["markers"]).iter(), "marker")?;
    Ok(project)
}

fn normalize_model(input: Value, spec: &Model, path: &str) -> Result<Value, String> {
    let Value::Object(mut input) = input else {
        return Err(at(path, "expected an object"));
    };
    let mut output = Map::new();
    for field in spec.fields {
        let path = format!("{path}.{}", field.name);
        let value = match input.remove(field.name) {
            Some(value) => value,
            None => match field.default {
                D::Required => return Err(at(&path, "field is required")),
                D::Null => Value::Null,
                D::Bool(value) => Value::Bool(value),
                D::Int(value) => Value::from(value),
                D::Float(value) => Value::from(value),
                D::Text(value) => Value::from(value),
                D::List => Value::Array(Vec::new()),
                D::Object => Value::Object(Map::new()),
            },
        };
        let value = if value.is_null() && matches!(field.default, D::Null) {
            Value::Null
        } else {
            normalize_value(value, field.rule, &path)?
        };
        output.insert(field.name.to_owned(), value);
    }
    if let Some(check) = spec.check {
        check(&output).map_err(|reason| at(path, reason))?;
    }
    Ok(Value::Object(output))
}

fn normalize_value(value: Value, rule: Rule, path: &str) -> Result<Value, String> {
    match rule {
        R::Model(spec) => normalize_model(value, spec, path),
        R::List(item, limit) => {
            let Value::Array(items) = value else {
                return Err(at(path, "expected an array"));
            };
            if items.len() > limit {
                return Err(at(path, &format!("expected at most {limit} items")));
            }
            items
                .into_iter()
                .enumerate()
                .map(|(index, value)| normalize_value(value, *item, &format!("{path}.{index}")))
                .collect()
        }
        R::Object if value.is_object() => Ok(value),
        R::Object => Err(at(path, "expected an object")),
        R::Text(min, max) => {
            let size = value
                .as_str()
                .ok_or_else(|| at(path, "expected a string"))?
                .chars()
                .count();
            if (min..=max).contains(&size) {
                Ok(value)
            } else {
                Err(at(path, "string length is out of range"))
            }
        }
        R::Choice(choices) if value.as_str().is_some_and(|text| choices.contains(&text)) => {
            Ok(value)
        }
        R::Choice(_) => Err(at(path, "unsupported value")),
        R::Version if value.as_f64() == Some(2.0) => Ok(json!(2)),
        R::Version => Err(at(path, "expected schema version 2")),
        R::Bool => boolean(&value)
            .map(Value::Bool)
            .ok_or_else(|| at(path, "expected a boolean")),
        R::Integer(min, max) => {
            let integer = numeric_integer(&value).ok_or_else(|| at(path, "expected an integer"))?;
            if (i128::from(min)..=i128::from(max)).contains(&integer) {
                Ok(Value::from(integer as i64))
            } else {
                Err(at(path, "integer is out of range"))
            }
        }
        R::JsonInteger => {
            let integer = numeric_integer(&value).ok_or_else(|| at(path, "expected an integer"))?;
            if let Ok(signed) = i64::try_from(integer) {
                Ok(Value::from(signed))
            } else {
                u64::try_from(integer)
                    .map(Value::from)
                    .map_err(|_| at(path, "integer is out of range"))
            }
        }
        R::Float(min, max, exclusive_min) => {
            let number = numeric_float(&value).ok_or_else(|| at(path, "expected a number"))?;
            if number < min || number > max || (exclusive_min && number == min) {
                return Err(at(path, "number is out of range"));
            }
            Number::from_f64(number)
                .map(Value::Number)
                .ok_or_else(|| at(path, "expected a finite number"))
        }
    }
}

fn at(path: &str, reason: &str) -> String {
    format!("{path} - {reason}")
}

fn strip(text: &str) -> &str {
    text.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch))
}

fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) if n.as_f64() == Some(0.0) => Some(false),
        Value::Number(n) if n.as_f64() == Some(1.0) => Some(true),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn numeric_text(text: &str) -> Option<std::borrow::Cow<'_, str>> {
    let text = strip(text);
    if !text.contains('_') {
        return Some(text.into());
    }
    let bytes = text.as_bytes();
    for (index, ch) in bytes.iter().enumerate() {
        if *ch == b'_'
            && !(index > 0
                && bytes[index - 1].is_ascii_digit()
                && bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
        {
            return None;
        }
    }
    Some(text.replace('_', "").into())
}

fn numeric_integer(value: &Value) -> Option<i128> {
    match value {
        Value::Bool(value) => Some(i128::from(*value)),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                return Some(i128::from(value));
            }
            if let Some(value) = value.as_u64() {
                return Some(i128::from(value));
            }
            let value = value.as_f64()?;
            (value.fract() == 0.0
                && (-9223372036854775808.0..18446744073709551616.0).contains(&value))
            .then_some(value as i128)
        }
        Value::String(value) => {
            let text = numeric_text(value)?;
            let whole = if let Some((whole, fractional)) = text.split_once('.') {
                if fractional.is_empty() || !fractional.bytes().all(|ch| ch == b'0') {
                    return None;
                }
                whole
            } else {
                &text
            };
            whole.parse().ok()
        }
        _ => None,
    }
}

fn numeric_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        Value::Number(value) => value.as_f64(),
        Value::String(value) => numeric_text(value)?.parse().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

pub(super) fn list(value: &Value) -> &[Value] {
    value.as_array().expect("normalized model list")
}

fn check_ids<'a>(mut values: impl Iterator<Item = &'a Value>, kind: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    values.try_for_each(|value| {
        let key = strip(value["id"].as_str().expect("normalized id"));
        if key.is_empty() {
            Err(format!("{kind} id must not be blank"))
        } else if !seen.insert(key) {
            Err(format!("duplicate {kind} id"))
        } else {
            Ok(())
        }
    })
}

fn check_output_range(output: &Object) -> Result<(), &'static str> {
    if output["range_mode"] == "custom"
        && output["range_end_sec"].as_f64().is_some_and(|end| {
            end <= output["range_start_sec"]
                .as_f64()
                .expect("normalized start")
        })
    {
        return Err("output range_end_sec must be greater than range_start_sec");
    }
    Ok(())
}

fn check_crop(crop: &Object) -> Result<(), &'static str> {
    let number = |key: &str| crop[key].as_f64().expect("normalized crop");
    if number("x") + number("width") > 1.000001 || number("y") + number("height") > 1.000001 {
        return Err("crop rectangle must stay within the source frame");
    }
    Ok(())
}

fn check_clip(clip: &Object) -> Result<(), &'static str> {
    if clip["trim_out"]
        .as_f64()
        .is_some_and(|end| end <= clip["trim_in"].as_f64().expect("normalized trim start"))
    {
        return Err("clip trim_out must be greater than trim_in");
    }
    for (collection, time, amount, min, max) in [
        ("speed_keyframes", "source_sec", "speed", 0.25, 4.0),
        ("audio_keyframes", "time_sec", "volume", 0.0, 5.0),
    ] {
        for keyframe in list(&clip[collection]) {
            let timing = numeric_float(&keyframe[time]).ok_or("keyframe time must be numeric")?;
            let value = numeric_float(&keyframe[amount]).ok_or("keyframe value must be numeric")?;
            if !(0.0..=86400.0).contains(&timing) || !(min..=max).contains(&value) {
                return Err("keyframe time or value is out of range");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured with the released Python models using the regression's recorded clip,
    // overlay asset, and marker examples as an unversioned project body.
    const UNVERSIONED: &str = r##"{"output":{"canvas_fit":"blur","width":"1080","height":1920,"range_mode":"custom","range_start_sec":1,"range_end_sec":7},"tracks":[{"id":"v1","type":"video","label":"V1","clips":[{"id":"clip-r","source_type":"recorded_clip","source_id":"1","timeline_start":0,"trim_in":0,"trim_out":4,"canvas_fit":"cover","preserve_pitch":false,"reverse":true,"speed":1.25,"speed_keyframes":[{"source_sec":"1.25","speed":"1","easing":"linear"}],"meta":{"duration_sec":4,"future":{"label":"\u526a\u8f91"}},"crop":{"x":0.1,"y":0.2,"width":0.9,"height":0.8},"transform":{"scale":1.5},"future_clip_field":"ignored"}]}],"overlays":[{"id":"ov-asset","type":"sticker","timeline_start":1.25,"duration":3,"asset_path":"C:/clips/logo.png","transform":{"x":0.2,"y":0.8,"width":0.3,"height":0.3,"scale":1,"rotation":0,"opacity":0.75},"meta":{"asset_id":42,"name":"logo.png","kind":"image","duration_sec":3}}],"overlay_tracks":[{"id":"ot1","vendor":{"order":[1,2]}}],"markers":[{"id":"m1","time_sec":4.25,"label":"beat","color":"#f59e0b"}]}"##;
    const NORMALIZED: &str = r##"{"schema_version":2,"output":{"dir":"","filename":"lite_cut_export.mp4","width":1080,"height":1920,"fps":60,"encoder":"auto","encoder_tier":"quality","canvas_fit":"blur","background_color":"#000000","blur_amount":24,"range_mode":"custom","range_start_sec":1.0,"range_end_sec":7.0},"tracks":[{"id":"v1","type":"video","label":"V1","name":null,"locked":false,"hidden":false,"muted":false,"solo":false,"volume":1.0,"clips":[{"id":"clip-r","source_type":"recorded_clip","source_id":1,"file_path":null,"timeline_start":0.0,"trim_in":0.0,"trim_out":4.0,"transition_in":null,"transition_out":null,"color":null,"transform":{"x":0.5,"y":0.5,"scale":1.5,"rotation":0.0,"width":1.0,"height":1.0,"opacity":1.0},"keyframes":[],"crop":{"x":0.1,"y":0.2,"width":0.9,"height":0.8},"canvas_fit":"cover","flip_horizontal":false,"flip_vertical":false,"speed":1.25,"speed_keyframes":[{"source_sec":"1.25","speed":"1","easing":"linear"}],"preserve_pitch":false,"reverse":true,"freeze_frame_sec":0.0,"volume":1.0,"audio_keyframes":[],"muted":false,"fade_in_sec":0.0,"fade_out_sec":0.0,"meta":{"duration_sec":4,"future":{"label":"\u526a\u8f91"}}}]}],"overlays":[{"id":"ov-asset","type":"sticker","timeline_start":1.25,"duration":3.0,"fade_in_sec":0.0,"fade_out_sec":0.0,"transition_in":null,"transition_out":null,"transform":{"x":0.2,"y":0.8,"scale":1.0,"rotation":0.0,"width":0.3,"height":0.3,"opacity":0.75},"keyframes":[],"flip_horizontal":false,"flip_vertical":false,"text":null,"asset_path":"C:/clips/logo.png","meta":{"asset_id":42,"name":"logo.png","kind":"image","duration_sec":3}}],"overlay_tracks":[{"id":"ot1","vendor":{"order":[1,2]}}],"markers":[{"id":"m1","time_sec":4.25,"label":"beat","color":"#f59e0b"}],"audio":{"bgm":null,"master_volume":1.0},"template_id":null,"created_from_template":false}"##;
    const EMPTY: &str = r##"{"schema_version":2,"output":{"dir":"","filename":"lite_cut_export.mp4","width":1920,"height":1080,"fps":60,"encoder":"auto","encoder_tier":"quality","canvas_fit":"contain","background_color":"#000000","blur_amount":24,"range_mode":"full","range_start_sec":0.0,"range_end_sec":null},"tracks":[{"id":"v1","type":"video","label":"V1","name":null,"locked":false,"hidden":false,"muted":false,"solo":false,"volume":1.0,"clips":[]},{"id":"a1","type":"audio","label":"A1","name":null,"locked":false,"hidden":false,"muted":false,"solo":false,"volume":1.0,"clips":[]}],"overlays":[],"overlay_tracks":[],"markers":[],"audio":{"bgm":null,"master_volume":1.0},"template_id":null,"created_from_template":false}"##;

    #[test]
    fn absent_and_empty_bodies_receive_the_released_default_tracks() {
        let expected: Value = serde_json::from_str(EMPTY).unwrap();
        for input in [None, Some(Value::Null), Some(json!({}))] {
            assert_eq!(normalize(input).unwrap(), expected);
        }
        let explicit = normalize(Some(json!({"tracks": []}))).unwrap();
        assert_eq!(explicit["tracks"], json!([]));
    }

    #[test]
    fn legacy_unversioned_timeline_has_the_exact_released_normalization() {
        let raw = serde_json::from_str(UNVERSIONED).unwrap();
        let golden: Value = serde_json::from_str(NORMALIZED).unwrap();
        assert_eq!(normalize(Some(raw)).unwrap(), golden);
        assert_eq!(normalize(Some(golden.clone())).unwrap(), golden);
    }

    #[test]
    fn validation_preserves_bounds_and_does_not_report_submitted_paths() {
        let clip = |mut fields: Value| {
            fields["id"] = json!("clip-1");
            json!({"tracks":[{"id":"v1","type":"video","label":"V1","clips":[fields]}]})
        };
        let invalid = [
            (json!({"output":{"fps":0}}), "fps"),
            (
                json!({"output":{"range_mode":"custom","range_start_sec":3,"range_end_sec":3}}),
                "range_end_sec",
            ),
            (json!({"output":{"canvas_fit":"invalid"}}), "canvas_fit"),
            (json!({"output":{"width":"1__920"}}), "width"),
            (json!({"schema_version":1}), "schema_version"),
            (json!({"schema_version":"2"}), "schema_version"),
            (json!({"tracks":[{"id":"v1","type":"video"}]}), "label"),
            (clip(json!({"trim_in":4,"trim_out":3})), "trim_out"),
            (clip(json!({"crop":{"x":0.5,"width":0.6}})), "crop"),
            (clip(json!({"transform":{"scale":0}})), "scale"),
            (
                clip(json!({"speed_keyframes":[{"source_sec":1,"speed":6}]})),
                "keyframe",
            ),
            (clip(json!({"audio_keyframes":[{"volume":2}]})), "keyframe"),
            (clip(json!({"keyframes":[true]})), "keyframes.0"),
            (json!({"markers":[{"id":"  "}]}), "marker id"),
            (
                json!({"overlay_tracks": vec![json!({}); 33]}),
                "overlay_tracks",
            ),
            (
                json!({"tracks":[
                    {"id":"v1","type":"video","label":"V1","clips":[{"id":" C:/private/fixture.mp4 "}]},
                    {"id":"v2","type":"video","label":"V2","clips":[{"id":"C:/private/fixture.mp4"}]},
                ]}),
                "duplicate clip id",
            ),
        ];
        for (input, expected) in invalid {
            let error = normalize(Some(input)).unwrap_err();
            assert!(
                error.contains(expected),
                "{expected:?} not found in {error:?}"
            );
            assert!(!error.contains("C:/private/fixture.mp4"));
        }
    }

    #[test]
    fn common_released_numeric_and_boolean_coercions_are_preserved() {
        let body = normalize(Some(json!({
            "schema_version":2.0,
            "output":{"width":"1_920","fps":" 29.0 "},
            "created_from_template":"YES",
        })))
        .unwrap();
        assert_eq!(body["schema_version"], 2);
        assert_eq!(body["output"]["width"], 1920);
        assert_eq!(body["output"]["fps"], 29);
        assert_eq!(body["created_from_template"], true);
        assert!(normalize(Some(json!({"output":{"fps":"6e1"}}))).is_err());
        assert!(normalize(Some(json!({"output":{"fps":"28."}}))).is_err());
        assert!(normalize(Some(json!({"created_from_template":" yes " }))).is_err());
    }
}
