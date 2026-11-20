use maxgamestudio_core::training_input::{
    analyze_input_session, recommend_sensitivity, TrainingInputError,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
struct Capture {
    name: String,
    input: Value,
    output: Option<Value>,
    error: Option<Value>,
}

#[derive(Deserialize)]
struct Fixture {
    sensitivity: Vec<Capture>,
    keyboard: Vec<Capture>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/training_input.json")).unwrap()
}

fn replay<Request, Response>(
    cases: Vec<Capture>,
    calculate: impl Fn(&Request) -> Result<Response, TrainingInputError>,
) where
    Request: DeserializeOwned,
    Response: Serialize,
{
    for case in cases {
        let request: Request =
            serde_json::from_value(case.input).unwrap_or_else(|err| panic!("{}: {err}", case.name));
        let actual = calculate(&request).map(|output| serde_json::to_value(output).unwrap());
        match (case.output, case.error) {
            (Some(expected), None) => assert_eq!(actual, Ok(expected), "{}", case.name),
            (None, Some(expected)) => assert_eq!(
                actual.map_err(|error| serde_json::to_value(error).unwrap()),
                Err(expected),
                "{}",
                case.name
            ),
            _ => panic!("invalid captured response for {}", case.name),
        }
    }
}

#[test]
fn sensitivity_scores_and_validation_match_original_python() {
    replay(fixture().sensitivity, recommend_sensitivity);
}

#[test]
fn keyboard_calibration_and_validation_match_original_python() {
    replay(fixture().keyboard, analyze_input_session);
}
