use maxgamestudio_core::crosshair::{
    decode_crosshair, encode_crosshair, parse_crosshair_code, serialize_crosshair_code,
    CrosshairError, ParseOptions, SerializeOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct Capture<T> {
    name: String,
    input: T,
    output: Option<Value>,
    error: Option<String>,
}

impl<T> Capture<T> {
    fn check(&self, actual: Result<Value, CrosshairError>) {
        match (&self.output, &self.error) {
            (Some(expected), None) => assert_eq!(actual, Ok(expected.clone()), "{}", self.name),
            (None, Some(expected)) => assert_eq!(
                actual.map_err(|err| err.to_string()),
                Err(expected.clone()),
                "{}",
                self.name
            ),
            _ => panic!("invalid captured response for {}", self.name),
        }
    }
}

#[derive(Deserialize)]
struct ParseInput {
    code: String,
    strict: Option<bool>,
    allow_unknown: Option<bool>,
    require_primary: Option<bool>,
}

#[derive(Deserialize)]
struct Fixture {
    parse: Vec<Capture<ParseInput>>,
    encode: Vec<Capture<Value>>,
    decode: Vec<Capture<String>>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/crosshair.json")).unwrap()
}

#[test]
fn parses_and_serializes_the_original_native_codes() {
    for case in fixture().parse {
        let input = &case.input;
        case.check((|| {
            let ast = parse_crosshair_code(
                &input.code,
                ParseOptions {
                    strict: input.strict.unwrap_or(true),
                    allow_unknown: input.allow_unknown.unwrap_or(true),
                    require_primary: input.require_primary.unwrap_or(true),
                },
            )?;
            Ok(json!({
                "normal": serialize_crosshair_code(&ast, SerializeOptions::default())?,
                "canonical": serialize_crosshair_code(&ast, SerializeOptions {
                    canonical: true,
                    ..SerializeOptions::default()
                })?,
                "known": serialize_crosshair_code(&ast, SerializeOptions {
                    preserve_unknown: false,
                    ..SerializeOptions::default()
                })?,
            }))
        })());
    }
}

#[test]
fn encodes_the_same_codes_and_semantic_errors_as_the_original() {
    for case in fixture().encode {
        case.check(
            encode_crosshair(&case.input).map(|response| serde_json::to_value(response).unwrap()),
        );
    }
}

#[test]
fn decodes_the_same_editor_profiles_and_semantic_errors_as_the_original() {
    for case in fixture().decode {
        case.check(
            decode_crosshair(&case.input).map(|response| serde_json::to_value(response).unwrap()),
        );
    }
}
