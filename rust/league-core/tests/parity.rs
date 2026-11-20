use maxgamestudio_league_core::{
    update::{
        league_feed_identity, league_update, league_update_feed, league_update_from_feed,
        UpdateError,
    },
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
fn native_feed_rejects_the_other_channel_and_non_league_assets() {
    let prior = "1.0.0-beta.9";
    let version = "1.0.0-beta.10";
    let tag = format!("league-v{version}");
    let url = format!("https://github.com/INEEDBUG/MaxGameStudio/releases/download/{tag}/MaxGameStudioLeague_{version}_x64-setup.exe");
    let published = json!({
        "product": "maxgamestudio-league", "channel": "beta", "version": version,
        "tag": tag, "url": url, "sha256": "1".repeat(64),
    });
    let beta = league_feed_identity(prior).unwrap();
    assert_eq!(beta.channel, "beta");
    assert_eq!(beta.product, "maxgamestudio-league");
    assert_eq!(
        beta.url,
        "https://raw.githubusercontent.com/INEEDBUG/MaxGameStudio/updater/league-beta.json"
    );
    let update = league_update_from_feed(&published, prior).unwrap().unwrap();
    assert_eq!(update.version, version);
    assert_eq!(update.url, url);
    assert_eq!(league_update_from_feed(&published, version).unwrap(), None);
    assert_eq!(
        league_update_from_feed(&published, "1.0.0-beta.11").unwrap(),
        None
    );
    assert_eq!(
        league_update_from_feed(&published, "1.0.0"),
        Err(UpdateError::Channel)
    );
    for (field, invalid) in [
        ("product", "maxgamestudio"),
        ("channel", "stable"),
        ("tag", "v1.0.0-beta.10"),
        ("url", "https://github.com/INEEDBUG/MaxGameStudio/releases/download/v1.0.0-beta.10/MaxGameStudio_1.0.0-beta.10_x64-setup.exe"),
        ("sha256", "bad digest"),
    ] {
        let mut wrong = published.clone();
        wrong[field] = json!(invalid);
        assert!(league_update_from_feed(&wrong, prior).is_err(), "{field}");
    }
    let mut no_digest = published;
    no_digest.as_object_mut().unwrap().remove("sha256");
    assert!(league_update_from_feed(&no_digest, prior).is_err());
}

#[test]
fn native_stable_feed_identity_and_eligible_download_match_the_installed_channel() {
    let stable = league_feed_identity("1.0.0").unwrap();
    assert_eq!(stable.channel, "stable");
    assert_eq!(stable.product, "maxgamestudio-league");
    assert_eq!(
        stable.url,
        "https://raw.githubusercontent.com/INEEDBUG/MaxGameStudio/updater/league-latest.json"
    );
    let published = json!({
        "product": "maxgamestudio-league", "channel": "stable", "version": "1.0.1",
        "tag": "league-v1.0.1", "sha256": "abcd".repeat(16),
        "url": "https://github.com/INEEDBUG/MaxGameStudio/releases/download/league-v1.0.1/MaxGameStudioLeague_1.0.1_x64-setup.exe",
    });
    assert_eq!(
        league_update_from_feed(&published, "1.0.0")
            .unwrap()
            .unwrap()
            .version,
        "1.0.1"
    );
    assert_eq!(
        league_update_from_feed(&published, "1.0.0-beta.10"),
        Err(UpdateError::Channel)
    );
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
