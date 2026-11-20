//! Same-shard Riot ID and PUUID lookup inside the verified League client.
//! This is a child module of `lcu`, so its fixed reads use the same trust boundary.

use super::{
    append_chunk, decode_body, request_error, response_has_body, LcuError, LcuTarget, LcuTransport,
};
use reqwest::{RequestBuilder, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt,
    time::{Duration, Instant},
};

#[path = "player_history.rs"]
mod player_history;

use player_history::{MatchHistoryResponse, RecentAramHistory, HISTORY_PAGE_SIZE};

const HISTORY_BODY_LIMIT: usize = 1024 * 1024;
const HISTORY_READ_TIMEOUT: Duration = Duration::from_millis(1200);
// Reserve time for the final signed-in account check in the desktop's lookup window.
const OPTIONAL_LOOKUP_DEADLINE: Duration = Duration::from_secs(3);

const TENCENT_PLATFORMS: &[&str] = &[
    "HN1", "HN10", "TJ100", "TJ101", "NJ100", "GZ100", "CQ100", "BGP2", "PBE", "PREPBE",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlayerSearchShard {
    pub(crate) region: String,
    pub(crate) rso_platform: String,
    pub(crate) sgp_server_id: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlayerSearchResult {
    puuid: String,
    game_name: String,
    tag_line: String,
    profile_icon_id: u32,
    sgp_server_id: String,
    privacy: String,
    summoner_level: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    recent_aram: Option<RecentAramHistory>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerSearchError {
    InvalidQuery,
    NotSignedIn,
    ShardUnavailable,
    SessionChanged,
    OtherShard,
    InvalidResult,
    Read(LcuError),
}

impl fmt::Display for PlayerSearchError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::InvalidQuery => "请输入完整 Riot ID（玩家名#标签）或本地 PUUID。",
            Self::NotSignedIn => "请登录英雄联盟客户端后查询。",
            Self::ShardUnavailable => "当前客户端的区服暂不可用。",
            Self::SessionChanged => "当前登录的玩家已变化，请重新查询。",
            Self::OtherShard => "当前只支持查询已登录的区服。",
            Self::InvalidResult => "客户端返回的玩家资料无法确认。",
            Self::Read(error) => return write!(output, "无法读取客户端玩家资料。{error}"),
        })
    }
}

impl std::error::Error for PlayerSearchError {}

impl From<LcuError> for PlayerSearchError {
    fn from(error: LcuError) -> Self {
        Self::Read(error)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatIdentity {
    puuid: String,
    platform_id: String,
}

#[derive(Deserialize)]
struct AliasResponse {
    puuid: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummonerResponse {
    puuid: String,
    game_name: String,
    tag_line: String,
    display_name: Option<String>,
    profile_icon_id: u32,
    privacy: String,
    summoner_level: u32,
    #[serde(default)]
    account_id: Value,
}

#[derive(PartialEq, Eq)]
struct Puuid(String);

impl Puuid {
    fn parse(value: &str) -> Option<Self> {
        if value.len() != 36
            || value == "00000000-0000-0000-0000-000000000000"
            || !value.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
        {
            return None;
        }
        Some(Self(value.to_ascii_lowercase()))
    }
}

enum LookupQuery<'a> {
    Alias {
        game_name: &'a str,
        tag_line: &'a str,
    },
    Puuid(Puuid),
}

impl<'a> LookupQuery<'a> {
    fn parse(input: &'a str) -> Result<Self, PlayerSearchError> {
        let input = input.trim();
        if let Some(puuid) = Puuid::parse(input) {
            return Ok(Self::Puuid(puuid));
        }
        if input.len() > 256 || input.chars().any(char::is_control) {
            return Err(PlayerSearchError::InvalidQuery);
        }
        let (game_name, tag_line) = input
            .split_once('#')
            .ok_or(PlayerSearchError::InvalidQuery)?;
        let (game_name, tag_line) = (game_name.trim(), tag_line.trim());
        if game_name.is_empty() || tag_line.is_empty() || tag_line.contains('#') {
            return Err(PlayerSearchError::InvalidQuery);
        }
        Ok(Self::Alias {
            game_name,
            tag_line,
        })
    }
}

fn shard_from_chat(
    chat: Option<ChatIdentity>,
    own_puuid: &Puuid,
) -> Result<PlayerSearchShard, PlayerSearchError> {
    let chat = chat.ok_or(PlayerSearchError::ShardUnavailable)?;
    if Puuid::parse(&chat.puuid).as_ref() != Some(own_puuid) {
        return Err(PlayerSearchError::SessionChanged);
    }
    shard_from_platform(&chat.platform_id)
}

fn shard_from_platform(platform_id: &str) -> Result<PlayerSearchShard, PlayerSearchError> {
    let platform = platform_id.trim().to_ascii_uppercase();
    let tencent = platform.strip_prefix("TENCENT_").or_else(|| {
        // Bare PBE is Riot's test environment. Tencent PBE needs the Tencent prefix.
        (platform != "PBE").then_some(platform.as_str())
    });
    if let Some(tencent) = tencent.filter(|id| TENCENT_PLATFORMS.contains(id)) {
        return Ok(PlayerSearchShard {
            region: "TENCENT".into(),
            rso_platform: tencent.into(),
            sgp_server_id: format!("TENCENT_{tencent}"),
        });
    }

    // Names from the pinned Akari built-in server list and its `getSgpServerId` aliases.
    let sgp_server_id = match platform.as_str() {
        "NA" | "NA1" => "NA1",
        "BR" | "BR1" => "BR1",
        "TR" | "TR1" => "TR1",
        "LAN" | "LA1" => "LA1",
        "LAS" | "LA2" => "LA2",
        "OCE" | "OC1" => "OC1",
        "EUW1" | "EUW" => "EUW",
        "JP1" | "JP" => "JP",
        "PBE1" | "PBE" => "PBE",
        "KR" | "RU" | "SG2" | "PH2" | "TW2" | "TH2" | "VN2" => &platform,
        _ => return Err(PlayerSearchError::ShardUnavailable),
    };
    Ok(PlayerSearchShard {
        region: platform.clone(),
        rso_platform: platform.clone(),
        sgp_server_id: sgp_server_id.into(),
    })
}

fn require_requested_shard(
    current: &PlayerSearchShard,
    requested: Option<&str>,
) -> Result<(), PlayerSearchError> {
    if requested.is_some_and(|requested| requested != current.sgp_server_id) {
        return Err(PlayerSearchError::OtherShard);
    }
    Ok(())
}

fn project_summoner(
    summoner: SummonerResponse,
    expected: &Puuid,
    shard: &PlayerSearchShard,
) -> Result<PlayerSearchResult, PlayerSearchError> {
    if Puuid::parse(&summoner.puuid).as_ref() != Some(expected) {
        return Err(PlayerSearchError::InvalidResult);
    }
    let game_name = if summoner.game_name.trim().is_empty() {
        summoner
            .display_name
            .filter(|name| !name.trim().is_empty())
            .ok_or(PlayerSearchError::InvalidResult)?
    } else {
        summoner.game_name
    };
    Ok(PlayerSearchResult {
        puuid: summoner.puuid,
        game_name,
        tag_line: summoner.tag_line,
        profile_icon_id: summoner.profile_icon_id,
        sgp_server_id: shard.sgp_server_id.clone(),
        privacy: summoner.privacy,
        summoner_level: summoner.summoner_level,
        recent_aram: None,
    })
}

fn may_request_history(own: &Puuid, target: &Puuid, privacy: &str) -> bool {
    own == target || privacy == "PUBLIC"
}

enum PlayerRead<'a> {
    ChatSelf,
    Alias {
        game_name: &'a str,
        tag_line: &'a str,
    },
    Summoner(&'a Puuid),
    RecentMatches {
        player: &'a Puuid,
        timeout: Duration,
    },
}

impl LcuTransport {
    /// Read only the current account's platform and verify it against the signed-in sample.
    pub(crate) async fn read_player_search_shard(
        &self,
        target: &LcuTarget<'_>,
        own_puuid: &str,
    ) -> Result<PlayerSearchShard, PlayerSearchError> {
        let own = Puuid::parse(own_puuid).ok_or(PlayerSearchError::NotSignedIn)?;
        let chat = self
            .read_player_endpoint(target, PlayerRead::ChatSelf)
            .await?;
        shard_from_chat(chat, &own)
    }

    /// The caller must discard results if the retained client instance changes while awaiting a read.
    pub(crate) async fn lookup_player(
        &self,
        target: &LcuTarget<'_>,
        own_puuid: &str,
        requested_shard: Option<&str>,
        input: &str,
    ) -> Result<Option<PlayerSearchResult>, PlayerSearchError> {
        let query = LookupQuery::parse(input)?;
        let own = Puuid::parse(own_puuid).ok_or(PlayerSearchError::NotSignedIn)?;
        let history_deadline = Instant::now() + OPTIONAL_LOOKUP_DEADLINE;
        let shard = self.read_player_search_shard(target, own_puuid).await?;
        require_requested_shard(&shard, requested_shard)?;
        let puuid = match query {
            LookupQuery::Puuid(puuid) => Some(puuid),
            LookupQuery::Alias {
                game_name,
                tag_line,
            } => {
                let alias: Option<AliasResponse> = self
                    .read_player_endpoint(
                        target,
                        PlayerRead::Alias {
                            game_name,
                            tag_line,
                        },
                    )
                    .await?;
                alias
                    .map(|alias| Puuid::parse(&alias.puuid).ok_or(PlayerSearchError::InvalidResult))
                    .transpose()?
            }
        };
        let mut result = None;
        if let Some(puuid) = puuid {
            let summoner: Option<SummonerResponse> = self
                .read_player_endpoint(target, PlayerRead::Summoner(&puuid))
                .await?;
            if let Some(summoner) = summoner {
                let account = summoner.account_id.as_u64().filter(|&id| id > 0);
                let mut verified = project_summoner(summoner, &puuid, &shard)?;
                let timeout = history_deadline
                    .saturating_duration_since(Instant::now())
                    .min(HISTORY_READ_TIMEOUT);
                let mut history = RecentAramHistory::Unavailable;
                if !timeout.is_zero() && may_request_history(&own, &puuid, &verified.privacy) {
                    let reply: Result<Option<MatchHistoryResponse>, _> = self
                        .read_player_endpoint(
                            target,
                            PlayerRead::RecentMatches {
                                player: &puuid,
                                timeout,
                            },
                        )
                        .await;
                    if let Ok(Some(reply)) = reply {
                        history = reply.project(&puuid, &shard, account);
                    }
                }
                verified.recent_aram = Some(history);
                result = Some(verified);
            }
        }
        // Revalidate the signed-in account and its shard after the optional history read.
        if self.read_player_search_shard(target, own_puuid).await? != shard {
            return Err(PlayerSearchError::SessionChanged);
        }
        Ok(result)
    }

    fn player_request(&self, target: &LcuTarget<'_>, endpoint: PlayerRead<'_>) -> RequestBuilder {
        let mut url = Url::parse(&format!("https://127.0.0.1:{}", target.port))
            .expect("the local origin contains a validated numeric port");
        match endpoint {
            PlayerRead::ChatSelf => url.set_path("/lol-chat/v1/me"),
            PlayerRead::Alias {
                game_name,
                tag_line,
            } => {
                url.set_path("/lol-summoner/v1/alias/lookup");
                url.query_pairs_mut()
                    .append_pair("gameName", game_name)
                    .append_pair("tagLine", tag_line);
            }
            PlayerRead::Summoner(puuid) => {
                url.set_path(&format!("/lol-summoner/v2/summoners/puuid/{}", puuid.0));
            }
            PlayerRead::RecentMatches { player, .. } => {
                url.set_path(&format!(
                    "/lol-match-history/v1/products/lol/{}/matches",
                    player.0
                ));
                url.query_pairs_mut()
                    .append_pair("begIndex", "0")
                    .append_pair("endIndex", &(HISTORY_PAGE_SIZE - 1).to_string());
            }
        }
        let request = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .basic_auth("riot", Some(target.token));
        match endpoint {
            PlayerRead::RecentMatches { timeout, .. } => request.timeout(timeout),
            _ => request,
        }
    }

    async fn read_player_endpoint<T: DeserializeOwned>(
        &self,
        target: &LcuTarget<'_>,
        endpoint: PlayerRead<'_>,
    ) -> Result<Option<T>, LcuError> {
        let limit = match endpoint {
            PlayerRead::Alias { .. } => 4 * 1024,
            PlayerRead::ChatSelf | PlayerRead::Summoner(_) => 16 * 1024,
            PlayerRead::RecentMatches { .. } => HISTORY_BODY_LIMIT,
        };
        let mut response = self
            .player_request(target, endpoint)
            .send()
            .await
            .map_err(request_error)?;
        if !response_has_body(response.status())? {
            return Ok(None);
        }
        let content_length = response.content_length().unwrap_or_default();
        if content_length > limit as u64 {
            return Err(LcuError::ResponseTooLarge);
        }
        let mut body = Vec::with_capacity(content_length as usize);
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            append_chunk(&mut body, &chunk, limit)?;
        }
        decode_body(&body)
    }
}

#[cfg(test)]
#[path = "player_search_tests.rs"]
mod tests;
