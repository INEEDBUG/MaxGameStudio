//! Read-only game data for the retained Vue store, using the verified local client.

use super::{LcuError, LcuTarget, LcuTransport, ReadEndpoint};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GameDataEndpoint {
    Champions,
    Perks,
    Perkstyles,
    SummonerSpells,
    Items,
    Queues,
}

impl GameDataEndpoint {
    const ALL: [Self; 6] = [
        Self::Champions,
        Self::Perks,
        Self::Perkstyles,
        Self::SummonerSpells,
        Self::Items,
        Self::Queues,
    ];

    pub(super) const fn path(self) -> &'static str {
        match self {
            Self::Champions => "/lol-game-data/assets/v1/champion-summary.json",
            Self::Perks => "/lol-game-data/assets/v1/perks.json",
            Self::Perkstyles => "/lol-game-data/assets/v1/perkstyles.json",
            Self::SummonerSpells => "/lol-game-data/assets/v1/summoner-spells.json",
            Self::Items => "/lol-game-data/assets/v1/items.json",
            Self::Queues => "/lol-game-data/assets/v1/queues.json",
        }
    }

    // Current unpacked client assets range from about 13 KiB for spells to
    // 664 KiB for items. Allow room for localization and patch growth.
    pub(super) const fn body_limit(self) -> usize {
        let kib = match self {
            Self::Champions => 256,
            Self::Perks => 512,
            Self::Perkstyles | Self::SummonerSpells => 128,
            Self::Items => 2048,
            Self::Queues => 1024,
        };
        kib * 1024
    }

    const fn property(self) -> &'static str {
        match self {
            Self::Champions => "champions",
            Self::Perks => "perks",
            Self::Perkstyles => "perkstyles",
            Self::SummonerSpells => "summonerSpells",
            Self::Items => "items",
            Self::Queues => "queues",
        }
    }

    const fn row_limit(self) -> usize {
        match self {
            Self::Champions => 512,
            Self::Perks => 1024,
            Self::Perkstyles => 32,
            Self::SummonerSpells => 128,
            Self::Items => 8192,
            Self::Queues => 4096,
        }
    }

    fn normalize(self, payload: Value) -> Result<Value, LcuError> {
        match (self, payload) {
            (Self::Perkstyles, payload) => {
                let styles: Perkstyles = from_value(payload)?;
                let styles_by_id = self.index_rows(styles.styles)?;
                Ok(json!({"schemaVersion": styles.schema_version, "styles": styles_by_id}))
            }
            (Self::Queues, Value::Object(queues)) => {
                self.normalize_legacy_queues(queues).map(Value::Object)
            }
            (_, Value::Array(rows)) => self.index_rows(rows).map(Value::Object),
            _ => Err(LcuError::InvalidPayload),
        }
    }

    fn index_rows(self, rows: Vec<Value>) -> Result<Map<String, Value>, LcuError> {
        if rows.len() > self.row_limit() {
            return Err(LcuError::InvalidPayload);
        }
        let mut indexed = Map::new();
        for row in rows {
            let id = record_id(&row["id"])?;
            if id < 0 && !(self == Self::Champions && id == -1) {
                return Err(LcuError::InvalidPayload);
            }
            let key = id.to_string();
            // The first zero queue describes the custom lobby in the original client.
            if self == Self::Queues && id == 0 && indexed.contains_key(&key) {
                continue;
            }
            indexed.insert(key, self.project_row(row, id)?);
        }
        Ok(indexed)
    }

    fn normalize_legacy_queues(
        self,
        queues: Map<String, Value>,
    ) -> Result<Map<String, Value>, LcuError> {
        if queues.len() > self.row_limit() {
            return Err(LcuError::InvalidPayload);
        }
        queues
            .into_iter()
            .map(|(key, row)| {
                let id = i64::from(
                    key.parse::<u32>()
                        .ok()
                        .filter(|id| id.to_string() == key)
                        .ok_or(LcuError::InvalidPayload)?,
                );
                if let Some(value) = row.get("id") {
                    if record_id(value)? != id {
                        return Err(LcuError::InvalidPayload);
                    }
                }
                Ok((key, self.project_row(row, id)?))
            })
            .collect()
    }

    fn project_row(self, row: Value, id: i64) -> Result<Value, LcuError> {
        let projected = match self {
            Self::Champions => project::<Champion>(row)?,
            Self::Perks => project::<Perk>(row)?,
            Self::Perkstyles => project::<Perkstyle>(row)?,
            Self::SummonerSpells => project::<SummonerSpell>(row)?,
            Self::Items => project::<Item>(row)?,
            Self::Queues => project::<Queue>(row)?,
        };
        let Value::Object(mut fields) = projected else {
            return Err(LcuError::InvalidPayload);
        };
        fields.retain(|name, value| match name.as_str() {
            "iconPath" | "squarePortraitPath" => value.as_str().is_some_and(local_asset_path),
            _ => !value.is_null(),
        });
        if let Some(assets) = fields.get_mut("assetMap").and_then(Value::as_object_mut) {
            assets.retain(|key, path| {
                !matches!(key.as_str(), "__proto__" | "constructor" | "prototype")
                    && path.as_str().is_some_and(local_asset_path)
            });
        }
        let tooltip = match self {
            Self::Items => fields.get_mut("description"),
            Self::Perks => fields.get_mut("longDesc"),
            _ => None,
        };
        if let Some(Value::String(markup)) = tooltip {
            *markup = inert_tooltip(markup);
        }
        if let Some(Value::Array(entries)) = fields.get_mut("endOfGameStatDescs") {
            for entry in entries {
                if let Value::String(markup) = entry {
                    *markup = inert_tooltip(markup);
                }
            }
        }
        fields.insert("id".into(), json!(id));
        Ok(Value::Object(fields))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GameDataReadError {
    Missing,
    Read(LcuError),
}

#[derive(Default)]
pub(crate) struct GameDataRead {
    pub(crate) fields: Map<String, Value>,
    pub(crate) unavailable: Vec<(GameDataEndpoint, GameDataReadError)>,
}

impl GameDataRead {
    fn record(
        &mut self,
        endpoint: GameDataEndpoint,
        value: Result<Option<Value>, LcuError>,
    ) -> Result<(), LcuError> {
        let failure = match value.and_then(|value| value.map(|v| endpoint.normalize(v)).transpose())
        {
            Ok(Some(fields)) => {
                self.fields.insert(endpoint.property().into(), fields);
                return Ok(());
            }
            Ok(None) => GameDataReadError::Missing,
            Err(error @ (LcuError::AuthenticationRejected | LcuError::InvalidTarget)) => {
                return Err(error);
            }
            Err(error) => GameDataReadError::Read(error),
        };
        self.unavailable.push((endpoint, failure));
        Ok(())
    }
}

impl LcuTransport {
    /// Read once per verified client instance, outside the gameflow polling loop.
    /// Publish the fields only if the initiating client and generation are still current.
    pub(crate) async fn read_game_data(
        &self,
        target: &LcuTarget<'_>,
    ) -> Result<GameDataRead, LcuError> {
        let mut read = GameDataRead::default();
        for endpoint in GameDataEndpoint::ALL {
            let value = self.read(target, ReadEndpoint::GameData(endpoint)).await;
            read.record(endpoint, value)?;
        }
        Ok(read)
    }
}

fn record_id(value: &Value) -> Result<i64, LcuError> {
    // Champion -1 is the dummy. Client assets also use u32::MAX for Smite variants.
    value
        .as_i64()
        .filter(|id| (-1..=i64::from(u32::MAX)).contains(id))
        .ok_or(LcuError::InvalidPayload)
}

fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, LcuError> {
    serde_json::from_value(value).map_err(|_| LcuError::InvalidPayload)
}

fn project<T: DeserializeOwned + Serialize>(value: Value) -> Result<Value, LcuError> {
    serde_json::to_value(from_value::<T>(value)?).map_err(|_| LcuError::InvalidPayload)
}

fn local_asset_path(path: &str) -> bool {
    path.len() <= 1024
        && path
            .strip_prefix("/lol-game-data/assets/")
            .is_some_and(|asset| {
                asset.split('/').all(|part| {
                    !matches!(part, "" | "." | "..")
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"._- ()".contains(&byte))
                })
            })
}

// Existing Vue tooltips use v-html. Keep the elements styled by the UI,
// remove attributes and other elements, and retain their text.
fn inert_tooltip(value: &str) -> String {
    let mut fragments = value.split('<');
    let mut output = String::with_capacity(value.len());
    output.push_str(fragments.next().unwrap_or_default());
    for fragment in fragments {
        let element = fragment.split_once('>').and_then(|(tag, text)| {
            let name = tag
                .strip_prefix('/')
                .unwrap_or(tag)
                .split(|character: char| character.is_ascii_whitespace() || character == '/')
                .next()?;
            (name.as_bytes().first()?.is_ascii_alphabetic()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
            .then_some((tag, name, text))
        });
        if let Some((tag, name, text)) = element {
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "b" | "i"
                    | "u"
                    | "em"
                    | "strong"
                    | "br"
                    | "hr"
                    | "ul"
                    | "ol"
                    | "li"
                    | "maintext"
                    | "stats"
                    | "attention"
                    | "active"
                    | "passive"
                    | "status"
                    | "truedamage"
                    | "physicaldamage"
                    | "magicdamage"
                    | "lifesteal"
                    | "buffedstat"
                    | "nerfedstat"
                    | "ornnbonus"
                    | "raritymythic"
                    | "raritylegendary"
                    | "scalemr"
                    | "scalemana"
                    | "scalead"
                    | "scaleap"
                    | "scalelevel"
                    | "scalehealth"
                    | "speed"
                    | "gold"
                    | "flavortext"
                    | "rules"
                    | "lol-uikit-tooltipped-keyword"
            ) {
                output.push('<');
                if tag.starts_with('/') {
                    output.push('/');
                }
                output.push_str(name);
                if tag.ends_with('/') {
                    output.push('/');
                }
                output.push('>');
            }
            output.push_str(text);
        } else {
            output.push_str("&lt;");
            output.push_str(fragment);
        }
    }
    output
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Champion {
    name: String,
    alias: Option<String>,
    square_portrait_path: Option<String>,
    roles: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Perk {
    name: String,
    icon_path: Option<String>,
    long_desc: Option<String>,
    end_of_game_stat_descs: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Perkstyles {
    schema_version: u32,
    styles: Vec<Value>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Perkstyle {
    name: String,
    tooltip: Option<String>,
    icon_path: Option<String>,
    asset_map: Option<BTreeMap<String, String>>,
    allowed_sub_styles: Option<Vec<u32>>,
    slots: Option<Vec<PerkSlot>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerkSlot {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    slot_label: Option<String>,
    perks: Vec<u32>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SummonerSpell {
    name: String,
    description: Option<String>,
    summoner_level: Option<u32>,
    cooldown: Option<f64>,
    game_modes: Vec<String>,
    icon_path: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    name: String,
    description: Option<String>,
    from: Vec<u32>,
    to: Vec<u32>,
    price: Option<u32>,
    price_total: Option<u32>,
    icon_path: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Queue {
    name: String,
    short_name: Option<String>,
    description: Option<String>,
    detailed_description: Option<String>,
    game_select_mode_group: Option<String>,
    game_select_category: Option<String>,
    game_select_priority: Option<i32>,
}

#[cfg(test)]
#[path = "game_data_tests.rs"]
mod tests;
