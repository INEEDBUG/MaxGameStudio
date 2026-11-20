use maxgamestudio_league_core::{
    update::{league_update, league_update_feed, UpdateError},
    window_demand::{
        companion_window_demand, CompanionWindowKind, CompanionWindowState, WindowDemand,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordedBehavior {
    feeds: Vec<FeedCase>,
    updates: Vec<UpdateCase>,
    window_demands: Vec<WindowCase>,
}

#[derive(Deserialize)]
struct FeedCase {
    current: String,
    expected: Value,
}

#[derive(Deserialize)]
struct UpdateCase {
    name: String,
    manifest: Value,
    current: String,
    expected: Value,
}

#[derive(Deserialize)]
struct WindowCase {
    name: String,
    kind: CompanionWindowKind,
    state: CompanionWindowState,
    expected: WindowDemand,
}

fn recorded_behavior() -> RecordedBehavior {
    serde_json::from_str(include_str!("typescript-behavior.json")).unwrap()
}

fn capture(result: Result<impl Serialize, UpdateError>) -> Value {
    match result {
        Ok(value) => json!({ "value": value }),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

#[test]
fn replay_league_feed_selection() {
    for case in recorded_behavior().feeds {
        assert_eq!(
            capture(league_update_feed(&case.current)),
            case.expected,
            "installed version {:?}",
            case.current
        );
    }
}

#[test]
fn replay_league_update_validation() {
    for case in recorded_behavior().updates {
        assert_eq!(
            capture(league_update(&case.manifest, &case.current)),
            case.expected,
            "{}",
            case.name
        );
    }
}

#[test]
fn replay_companion_window_demands() {
    for case in recorded_behavior().window_demands {
        assert_eq!(
            companion_window_demand(case.kind, &case.state),
            case.expected,
            "{:?} {}",
            case.kind,
            case.name
        );
    }
}
