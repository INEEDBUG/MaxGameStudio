use maxgamestudio_league_core::auto_select::{
    plan_auto_select, AutoSelectAction, AutoSelectInput, AutoSelectPlan,
};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Replay {
    base: Map<String, Value>,
    cases: Vec<Scenario>,
    just_show_without_pending: OriginalDiscrepancy,
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    overrides: Map<String, Value>,
    expected: AutoSelectPlan,
}

#[derive(Deserialize)]
struct OriginalDiscrepancy {
    overrides: Map<String, Value>,
    original: AutoSelectPlan,
}

fn fixture() -> Replay {
    serde_json::from_str(include_str!("fixtures/auto-select.json")).unwrap()
}

fn make_input(base: &Map<String, Value>, overrides: Map<String, Value>) -> AutoSelectInput {
    let mut input = base.clone();
    input.extend(overrides);
    serde_json::from_value(Value::Object(input)).unwrap()
}

#[test]
fn aram_and_kiwi_plans_match_the_staged_typescript_controllers() {
    let replay = fixture();
    for scenario in replay.cases {
        let input = make_input(&replay.base, scenario.overrides);
        assert_eq!(
            plan_auto_select(&input),
            scenario.expected,
            "{}",
            scenario.name
        );
    }
}

#[test]
fn just_show_preserves_an_existing_ban_even_without_a_pending_task() {
    let replay = fixture();
    let difference = replay.just_show_without_pending;
    assert!(matches!(
        difference.original,
        AutoSelectPlan::Schedule {
            action: AutoSelectAction::Ban {
                completed: false,
                ..
            },
            champion_id: 11,
            ..
        }
    ));
    let input = make_input(&replay.base, difference.overrides);
    assert_eq!(plan_auto_select(&input), AutoSelectPlan::Idle);
}

#[test]
fn spectator_never_receives_a_plan() {
    let replay = fixture();
    let valid = replay
        .cases
        .into_iter()
        .find(|scenario| scenario.name == "missing grid entry and hard pickability")
        .unwrap();
    let mut input = make_input(&replay.base, valid.overrides);
    assert!(matches!(
        plan_auto_select(&input),
        AutoSelectPlan::Schedule { .. }
    ));
    input.is_spectating = true;
    assert_eq!(plan_auto_select(&input), AutoSelectPlan::Idle);
}
