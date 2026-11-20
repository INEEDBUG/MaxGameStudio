use maxgamestudio_league_core::resg::{
    is_resg_content, is_resg_page, resg_champion_url, select_resg_champion,
};
use serde_json::Value;

const BEHAVIOR: &str = include_str!("resg-behavior.json");

#[test]
fn preserves_resg_selection_and_navigation_contract() {
    let fixture: Value = serde_json::from_str(BEHAVIOR).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let actual = select_resg_champion(
            case["phase"].as_str(),
            &case["session"],
            &case["champSelect"],
            case["puuid"].as_str(),
        );
        assert_eq!(
            actual.map(u64::from),
            case["expected"].as_u64(),
            "{}",
            case["name"]
        );
    }
    for destination in fixture["destinations"].as_array().unwrap() {
        let url = destination["url"].as_str().unwrap();
        let parsed = url::Url::parse(url).ok();
        assert_eq!(
            parsed.as_ref().is_some_and(is_resg_page),
            destination["page"].as_bool().unwrap(),
            "{url}"
        );
        assert_eq!(
            parsed.as_ref().is_some_and(is_resg_content),
            destination["content"].as_bool().unwrap(),
            "{url}"
        );
    }
    for chart in fixture["charts"].as_array().unwrap() {
        let actual = resg_champion_url(chart["champion"].as_u64().unwrap() as u32);
        assert_eq!(
            actual.as_ref().map(|url| url.as_str()),
            chart["expected"].as_str()
        );
    }
}
