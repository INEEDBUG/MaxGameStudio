//! Select the public RESG champion page from the current local League session.

use serde_json::Value;
use url::Url;

pub const RESG_NAMESPACE: &str = "window-manager-main/resg-window";
const MAX_CHAMPION_ID: u32 = 100_000;
const RESG_HOME: &str = "https://www.resg.top";

pub fn is_resg_page(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(url.host_str(), Some("resg.top" | "www.resg.top"))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

pub fn is_resg_content(url: &Url) -> bool {
    if !is_resg_page(url) {
        return false;
    }
    let Some(path) = url.path().strip_prefix('/') else {
        return false;
    };
    let mut segments = path.strip_suffix('/').unwrap_or(path).split('/');
    matches!(
        (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ),
        (Some("v"), Some(version), Some("champions"), Some(id), None)
            if !version.is_empty() && !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
    )
}

pub fn resg_champion_url(champion: u32) -> Option<Url> {
    if !(1..MAX_CHAMPION_ID).contains(&champion) {
        return None;
    }
    Some(
        Url::parse(&format!("{RESG_HOME}/v/latest/champions/{champion}"))
            .expect("RESG destination is a constant host with a numeric ID"),
    )
}

fn champion(object: &Value) -> Option<u32> {
    let id = object["championId"].as_f64()?;
    if id >= 1.0 && id < f64::from(MAX_CHAMPION_ID) && id.fract() == 0.0 {
        return Some(id as u32);
    }
    None
}

fn entries(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

pub fn select_resg_champion(
    phase: Option<&str>,
    session: &Value,
    champ_select: &Value,
    puuid: Option<&str>,
) -> Option<u32> {
    let game = &session["gameData"];
    if game["queue"]["gameMode"] != "KIWI" {
        return None;
    }
    match phase? {
        "ChampSelect" => {
            let cell = champ_select["localPlayerCellId"].as_i64()?;
            let player = entries(&champ_select["myTeam"])
                .find(|entry| entry["cellId"].as_i64() == Some(cell))?;
            // A bench swap updates the team before the old pick action.
            champion(player).or_else(|| {
                entries(&champ_select["actions"])
                    .flat_map(entries)
                    .find(|entry| {
                        entry["actorCellId"].as_i64() == Some(cell) && entry["type"] == "pick"
                    })
                    .and_then(champion)
            })
        }
        "GameStart" | "InProgress" | "Reconnect" => {
            let puuid = puuid.filter(|value| !value.is_empty())?;
            entries(&game["playerChampionSelections"])
                .find(|entry| entry["puuid"] == puuid)
                .and_then(champion)
                .or_else(|| {
                    entries(&game["teamOne"])
                        .chain(entries(&game["teamTwo"]))
                        .find(|entry| entry["puuid"] == puuid)
                        .and_then(champion)
                })
        }
        _ => None,
    }
}
