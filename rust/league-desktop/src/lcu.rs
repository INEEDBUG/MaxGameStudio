//! Narrow HTTPS reads from an already verified local League client.

use crate::aram_automation::{AramOptions, ScheduledAction, SelectionLists};
use maxgamestudio_league_core::{auto_select::AutoSelectAction, resg::select_resg_champion};
use reqwest::{redirect::Policy, Client, RequestBuilder, StatusCode};
use rustls::pki_types::{pem::PemObject, CertificateDer};
use rustls::{client::Resumption, crypto::ring, ClientConfig, RootCertStore};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fmt, num::NonZeroU16, sync::Arc, time::Duration};

// Byte-for-byte match to the CA bundled by League Akari and Riot's League Director.
// https://github.com/RiotGames/leaguedirector/blob/main/resources/riotgames.pem
// SHA-256 of the DER: CA8C9D325B4CDC464C6C94A585C85E91EC23D40BA5BF3AE2822B951A4A504EA3
const RIOT_ROOT_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIEIDCCAwgCCQDJC+QAdVx4UDANBgkqhkiG9w0BAQUFADCB0TELMAkGA1UEBhMC
VVMxEzARBgNVBAgTCkNhbGlmb3JuaWExFTATBgNVBAcTDFNhbnRhIE1vbmljYTET
MBEGA1UEChMKUmlvdCBHYW1lczEdMBsGA1UECxMUTG9MIEdhbWUgRW5naW5lZXJp
bmcxMzAxBgNVBAMTKkxvTCBHYW1lIEVuZ2luZWVyaW5nIENlcnRpZmljYXRlIEF1
dGhvcml0eTEtMCsGCSqGSIb3DQEJARYeZ2FtZXRlY2hub2xvZ2llc0ByaW90Z2Ft
ZXMuY29tMB4XDTEzMTIwNDAwNDgzOVoXDTQzMTEyNzAwNDgzOVowgdExCzAJBgNV
BAYTAlVTMRMwEQYDVQQIEwpDYWxpZm9ybmlhMRUwEwYDVQQHEwxTYW50YSBNb25p
Y2ExEzARBgNVBAoTClJpb3QgR2FtZXMxHTAbBgNVBAsTFExvTCBHYW1lIEVuZ2lu
ZWVyaW5nMTMwMQYDVQQDEypMb0wgR2FtZSBFbmdpbmVlcmluZyBDZXJ0aWZpY2F0
ZSBBdXRob3JpdHkxLTArBgkqhkiG9w0BCQEWHmdhbWV0ZWNobm9sb2dpZXNAcmlv
dGdhbWVzLmNvbTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAKoJemF/
6PNG3GRJGbjzImTdOo1OJRDI7noRwJgDqkaJFkwv0X8aPUGbZSUzUO23cQcCgpYj
21ygzKu5dtCN2EcQVVpNtyPuM2V4eEGr1woodzALtufL3Nlyh6g5jKKuDIfeUBHv
JNyQf2h3Uha16lnrXmz9o9wsX/jf+jUAljBJqsMeACOpXfuZy+YKUCxSPOZaYTLC
y+0GQfiT431pJHBQlrXAUwzOmaJPQ7M6mLfsnpHibSkxUfMfHROaYCZ/sbWKl3lr
ZA9DbwaKKfS1Iw0ucAeDudyuqb4JntGU/W0aboKA0c3YB02mxAM4oDnqseuKV/CX
8SQAiaXnYotuNXMCAwEAATANBgkqhkiG9w0BAQUFAAOCAQEAf3KPmddqEqqC8iLs
lcd0euC4F5+USp9YsrZ3WuOzHqVxTtX3hR1scdlDXNvrsebQZUqwGdZGMS16ln3k
WObw7BbhU89tDNCN7Lt/IjT4MGRYRE+TmRc5EeIXxHkQ78bQqbmAI3GsW+7kJsoO
q3DdeE+M+BUJrhWorsAQCgUyZO166SAtKXKLIcxa+ddC49NvMQPJyzm3V+2b1roP
SvD2WV8gRYUnGmy/N0+u6ANq5EsbhZ548zZc+BI4upsWChTLyxt2RxR7+uGlS1+5
EcGfKZ+g024k/J32XP4hdho7WYAS2xMiV83CfLR/MNi8oSMaVQTdKD8cpgiWJk3L
XWehWA==
-----END CERTIFICATE-----";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);

#[path = "player_search.rs"]
pub(crate) mod player_search;

#[path = "game_data.rs"]
pub(crate) mod game_data;

/// Borrow the port and token that trusted desktop discovery verified for the same client generation.
pub(crate) struct LcuTarget<'a> {
    port: NonZeroU16,
    token: &'a str,
}

impl<'a> LcuTarget<'a> {
    pub(crate) fn new(port: u16, token: &'a str) -> Result<Self, LcuError> {
        let port = NonZeroU16::new(port).ok_or(LcuError::InvalidTarget)?;
        if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(LcuError::InvalidTarget);
        }

        Ok(Self { port, token })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LcuError {
    InvalidTarget,
    ClientConfiguration,
    Timeout,
    Transport,
    AuthenticationRejected,
    UnexpectedStatus(u16),
    ResponseTooLarge,
    InvalidPayload,
}

impl fmt::Display for LcuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidTarget => "Invalid local League client endpoint or token",
            Self::ClientConfiguration => "Could not initialize League client HTTPS",
            Self::Timeout => "League client read timed out",
            Self::Transport => "League client HTTPS connection or read failed",
            Self::AuthenticationRejected => "League client rejected authentication",
            Self::UnexpectedStatus(status) => {
                return write!(formatter, "League client returned HTTP {status}")
            }
            Self::ResponseTooLarge => "League client response exceeded the endpoint size limit",
            Self::InvalidPayload => "Invalid League client response",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for LcuError {}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentSummoner {
    pub(crate) puuid: String,
    pub(crate) game_name: Option<String>,
    pub(crate) tag_line: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) summoner_id: Option<i64>,
    pub(crate) profile_icon_id: Option<i32>,
    pub(crate) summoner_level: Option<i32>,
    pub(crate) xp_since_last_level: Option<i32>,
    pub(crate) xp_until_next_level: Option<i32>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GameflowSession {
    pub(crate) phase: String,
    pub(crate) game_data: Option<GameflowGameData>,
}

impl GameflowSession {
    pub(crate) fn is_aram(&self) -> bool {
        matches!(self.game_mode(), Some("ARAM" | "KIWI"))
    }

    pub(crate) fn is_hextech_aram(&self) -> bool {
        self.game_mode() == Some("KIWI")
    }

    fn game_mode(&self) -> Option<&str> {
        self.game_data
            .as_ref()?
            .queue
            .as_ref()?
            .game_mode
            .as_deref()
    }

    pub(crate) fn resg_champion(&self, selection: &Value, puuid: Option<&str>) -> Option<u32> {
        let game = self.game_data.as_ref()?;
        let private_sample = json!({"gameData": {
            "queue": {"gameMode": self.game_mode()},
            "playerChampionSelections": game.player_champion_selections,
            "teamOne": game.team_one,
            "teamTwo": game.team_two,
        }});
        select_resg_champion(Some(&self.phase), &private_sample, selection, puuid)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GameflowGameData {
    pub(crate) game_id: Option<u64>,
    pub(crate) is_custom_game: Option<bool>,
    pub(crate) queue: Option<GameflowQueue>,
    // Keep rosters inside the native sampler. The renderer only needs the local chart target.
    #[serde(default, skip_serializing)]
    player_champion_selections: Value,
    #[serde(default, skip_serializing)]
    team_one: Value,
    #[serde(default, skip_serializing)]
    team_two: Value,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GameflowQueue {
    pub(crate) id: Option<i32>,
    pub(crate) game_mode: Option<String>,
    #[serde(rename = "type")]
    pub(crate) queue_type: Option<String>,
}

#[derive(Clone, Copy)]
enum ReadEndpoint {
    Phase,
    CurrentSummoner,
    GameflowSession,
    ChampSelect,
    ChampionGrid,
    Pickable,
    Bannable,
    Subset,
    GameData(game_data::GameDataEndpoint),
}

impl ReadEndpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Phase => "/lol-gameflow/v1/gameflow-phase",
            Self::CurrentSummoner => "/lol-summoner/v1/current-summoner",
            Self::GameflowSession => "/lol-gameflow/v1/session",
            Self::ChampSelect => "/lol-champ-select/v1/session",
            Self::ChampionGrid => "/lol-champ-select/v1/all-grid-champions",
            Self::Pickable => "/lol-champ-select/v1/pickable-champion-ids",
            Self::Bannable => "/lol-champ-select/v1/bannable-champion-ids",
            Self::Subset => "/lol-lobby-team-builder/champ-select/v1/subset-champion-list",
            Self::GameData(endpoint) => endpoint.path(),
        }
    }

    fn body_limit(self) -> usize {
        match self {
            Self::Phase => 1024,
            Self::CurrentSummoner => 16 * 1024,
            Self::GameflowSession | Self::ChampSelect => 256 * 1024,
            Self::ChampionGrid => 512 * 1024,
            Self::Pickable | Self::Bannable | Self::Subset => 16 * 1024,
            Self::GameData(endpoint) => endpoint.body_limit(),
        }
    }
}

pub(crate) struct LcuTransport {
    client: Client,
}

impl LcuTransport {
    pub(crate) fn new() -> Result<Self, LcuError> {
        let root = CertificateDer::from_pem_slice(RIOT_ROOT_CA_PEM)
            .map_err(|_| LcuError::ClientConfiguration)?;
        let mut roots = RootCertStore::empty();
        roots.add(root).map_err(|_| LcuError::ClientConfiguration)?;
        let mut tls = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| LcuError::ClientConfiguration)?
            .with_root_certificates(roots)
            .with_no_client_auth();
        // Each local read authenticates its current peer across client restarts.
        tls.resumption = Resumption::disabled();

        let client = Client::builder()
            .tls_backend_preconfigured(tls)
            .https_only(true)
            .http1_only()
            .no_proxy()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .pool_max_idle_per_host(0)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| LcuError::ClientConfiguration)?;

        Ok(Self { client })
    }

    pub(crate) async fn read_phase(
        &self,
        target: &LcuTarget<'_>,
    ) -> Result<Option<String>, LcuError> {
        self.read(target, ReadEndpoint::Phase).await
    }

    pub(crate) async fn read_current_summoner(
        &self,
        target: &LcuTarget<'_>,
    ) -> Result<Option<CurrentSummoner>, LcuError> {
        self.read(target, ReadEndpoint::CurrentSummoner).await
    }

    pub(crate) async fn read_gameflow_session(
        &self,
        target: &LcuTarget<'_>,
    ) -> Result<Option<GameflowSession>, LcuError> {
        self.read(target, ReadEndpoint::GameflowSession).await
    }

    pub(crate) async fn read_champ_select(
        &self,
        target: &LcuTarget<'_>,
    ) -> Result<Value, LcuError> {
        Ok(self
            .read(target, ReadEndpoint::ChampSelect)
            .await?
            .unwrap_or(Value::Null))
    }

    pub(crate) async fn read_selection_lists(
        &self,
        target: &LcuTarget<'_>,
        options: &AramOptions,
        has_subset: bool,
    ) -> Result<SelectionLists, LcuError> {
        let mut lists = SelectionLists {
            grid: self
                .read(target, ReadEndpoint::ChampionGrid)
                .await?
                .unwrap_or(Value::Null),
            ..Default::default()
        };
        if options.want_pick() {
            lists.pickable = self
                .read(target, ReadEndpoint::Pickable)
                .await?
                .unwrap_or(Value::Null);
            if has_subset {
                lists.subset = self
                    .read(target, ReadEndpoint::Subset)
                    .await?
                    .unwrap_or(Value::Null);
            }
        }
        if options.want_ban() {
            lists.bannable = self
                .read(target, ReadEndpoint::Bannable)
                .await?
                .unwrap_or(Value::Null);
        }
        Ok(lists)
    }

    pub(crate) async fn apply_selection(
        &self,
        target: &LcuTarget<'_>,
        step: ScheduledAction,
    ) -> Result<(), LcuError> {
        let address = format!("https://127.0.0.1:{}", target.port);
        let champion = step.champion;
        let patch = |id: i64, body: Value| {
            self.client
                .patch(format!(
                    "{address}/lol-champ-select/v1/session/actions/{id}"
                ))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
        };
        let request = match step.action {
            AutoSelectAction::Intent { action_id } => {
                patch(action_id, json!({"championId": champion}))
            }
            AutoSelectAction::Pick {
                action_id,
                completed,
            } => patch(
                action_id,
                json!({"championId": champion, "type": "pick", "completed": completed}),
            ),
            AutoSelectAction::Ban {
                action_id,
                completed,
            } => patch(
                action_id,
                json!({"championId": champion, "type": "ban", "completed": completed}),
            ),
            AutoSelectAction::BenchSwap => self.client.post(format!(
                "{address}/lol-champ-select/v1/session/bench/swap/{champion}"
            )),
        };
        let response = request
            .basic_auth("riot", Some(target.token))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .map_err(request_error)?;
        if !response.status().is_success() {
            return Err(LcuError::UnexpectedStatus(response.status().as_u16()));
        }
        Ok(())
    }

    fn request(&self, target: &LcuTarget<'_>, read: ReadEndpoint) -> RequestBuilder {
        self.client
            .get(format!("https://127.0.0.1:{}{}", target.port, read.path()))
            .header(reqwest::header::ACCEPT, "application/json")
            .basic_auth("riot", Some(target.token))
    }

    async fn read<T: DeserializeOwned>(
        &self,
        target: &LcuTarget<'_>,
        read: ReadEndpoint,
    ) -> Result<Option<T>, LcuError> {
        let mut response = self
            .request(target, read)
            .send()
            .await
            .map_err(request_error)?;
        if !response_has_body(response.status())? {
            return Ok(None);
        }
        let limit = read.body_limit();
        let declared_length = response.content_length().unwrap_or_default();
        if declared_length > limit as u64 {
            return Err(LcuError::ResponseTooLarge);
        }
        let mut body = Vec::with_capacity(declared_length as usize);
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            append_chunk(&mut body, &chunk, limit)?;
        }
        decode_body(&body)
    }
}

fn request_error(error: reqwest::Error) -> LcuError {
    if error.is_timeout() {
        LcuError::Timeout
    } else {
        LcuError::Transport
    }
}

fn response_has_body(status: StatusCode) -> Result<bool, LcuError> {
    match status {
        StatusCode::OK => Ok(true),
        StatusCode::NO_CONTENT | StatusCode::NOT_FOUND => Ok(false),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(LcuError::AuthenticationRejected),
        _ => Err(LcuError::UnexpectedStatus(status.as_u16())),
    }
}

fn append_chunk(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), LcuError> {
    if chunk.len() > limit - body.len() {
        return Err(LcuError::ResponseTooLarge);
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<Option<T>, LcuError> {
    serde_json::from_slice(body).map_err(|_| LcuError::InvalidPayload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{ErrorKind, Read, Write},
        net::{Ipv4Addr, TcpListener},
        thread,
        time::Instant,
    };

    #[test]
    fn target_requires_a_valid_port_and_sensible_authentication_text() {
        assert!(LcuTarget::new(0, "fixture-only").is_err());
        assert!(LcuTarget::new(31111, "").is_err());
        assert!(LcuTarget::new(31111, "fixture\nonly").is_err());
        assert!(LcuTarget::new(31111, "fixture-only").is_ok());
    }

    #[test]
    fn requests_are_allowlisted_loopback_gets_with_sensitive_authentication() {
        let transport = LcuTransport::new().unwrap();
        let target = LcuTarget::new(31111, "fixture-only").unwrap();

        for (read, path) in [
            (ReadEndpoint::Phase, "/lol-gameflow/v1/gameflow-phase"),
            (
                ReadEndpoint::CurrentSummoner,
                "/lol-summoner/v1/current-summoner",
            ),
            (ReadEndpoint::GameflowSession, "/lol-gameflow/v1/session"),
            (ReadEndpoint::ChampSelect, "/lol-champ-select/v1/session"),
        ] {
            let request = transport.request(&target, read).build().unwrap();
            assert_eq!(request.method(), reqwest::Method::GET);
            assert_eq!(request.url().scheme(), "https");
            assert_eq!(request.url().host_str(), Some("127.0.0.1"));
            assert_eq!(request.url().port(), Some(31111));
            assert_eq!(request.url().path(), path);
            assert!(request.url().username().is_empty());
            assert!(request.url().password().is_none());
            assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
        }
    }

    #[test]
    fn refuses_a_plaintext_service_on_the_verified_transport_path() {
        let transport = LcuTransport::new().unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let target = LcuTarget::new(listener.local_addr().unwrap().port(), "fixture-only").unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + REQUEST_TIMEOUT;
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Synthetic listener did not accept a connection: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(CONNECT_TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(CONNECT_TIMEOUT)).unwrap();
            let mut first_record = [0; 5];
            stream.read_exact(&mut first_record).unwrap();
            assert_eq!(
                &first_record[..2],
                &[22, 3],
                "TLS handshake must precede HTTP"
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
        });

        let result = tauri::async_runtime::block_on(transport.read_phase(&target));
        server.join().unwrap();
        assert_eq!(result, Err(LcuError::Transport));
    }

    #[test]
    fn missing_data_is_distinct_from_authentication_failure_and_redirect() {
        assert_eq!(response_has_body(StatusCode::NO_CONTENT), Ok(false));
        assert_eq!(response_has_body(StatusCode::NOT_FOUND), Ok(false));
        assert_eq!(response_has_body(StatusCode::OK), Ok(true));
        assert_eq!(
            response_has_body(StatusCode::UNAUTHORIZED),
            Err(LcuError::AuthenticationRejected)
        );
        assert_eq!(
            response_has_body(StatusCode::FORBIDDEN),
            Err(LcuError::AuthenticationRejected)
        );
        assert_eq!(
            response_has_body(StatusCode::TEMPORARY_REDIRECT),
            Err(LcuError::UnexpectedStatus(307))
        );
    }

    #[test]
    fn body_size_checks_total_across_chunks_before_appending() {
        let mut body = Vec::new();
        append_chunk(&mut body, b"ab", 4).unwrap();
        append_chunk(&mut body, b"cd", 4).unwrap();
        assert_eq!(
            append_chunk(&mut body, b"e", 4),
            Err(LcuError::ResponseTooLarge)
        );
        assert_eq!(&body, b"abcd");
    }

    #[test]
    fn phase_and_old_summoner_name_shapes_are_preserved() {
        assert_eq!(
            decode_body::<String>(br#""ChampSelect""#)
                .unwrap()
                .as_deref(),
            Some("ChampSelect")
        );
        assert_eq!(decode_body::<String>(b"null").unwrap(), None);
        assert_eq!(decode_body::<String>(b"{}"), Err(LcuError::InvalidPayload));

        let player = decode_body::<CurrentSummoner>(
            br#"{"puuid":"fixture-player","displayName":"Fixture","summonerId":41,"profileIconId":7,
                 "xpSinceLastLevel":200,"xpUntilNextLevel":1000}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(player.display_name.as_deref(), Some("Fixture"));
        assert!(player.game_name.is_none());
        assert!(player.tag_line.is_none());
        assert_eq!(player.summoner_id, Some(41));
        assert_eq!(player.xp_since_last_level, Some(200));
        assert_eq!(player.xp_until_next_level, Some(1000));
    }

    #[test]
    fn session_keeps_aram_queue_and_discards_private_game_data() {
        let session = decode_body::<GameflowSession>(
            br#"{
                "phase":"ChampSelect",
                "gameData":{
                    "gameId":9001123,
                    "isCustomGame":false,
                    "password":"synthetic",
                    "spectatorKey":"synthetic",
                    "queue":{"id":2400,"gameMode":"KIWI","type":"ARAM_UNRANKED_5x5"},
                    "teamOne":[{"puuid":"fixture-opponent"}]
                },
                "gameClient":{"serverIp":"192.0.2.1"}
            }"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::to_value(session).unwrap(),
            serde_json::json!({
                "phase": "ChampSelect",
                "gameData": {
                    "gameId": 9001123,
                    "isCustomGame": false,
                    "queue": { "id": 2400, "gameMode": "KIWI", "type": "ARAM_UNRANKED_5x5" }
                }
            })
        );
    }

    #[test]
    fn resg_uses_the_local_champion_without_exposing_the_roster() {
        let mut source = serde_json::json!({
            "phase": "ChampSelect",
            "gameData": {
                "gameId": 19,
                "queue": { "gameMode": "KIWI" },
                "teamOne": [{ "puuid": "other-fixture", "championId": 21 }],
                "playerChampionSelections": [{ "puuid": "self-fixture", "championId": 22 }]
            }
        });
        let select = serde_json::json!({
            "localPlayerCellId": 4,
            "myTeam": [{ "cellId": 4, "championId": 23 }],
            "actions": [[{ "actorCellId": 4, "type": "pick", "championId": 24 }]]
        });
        let session: GameflowSession = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(session.resg_champion(&select, None), Some(23));
        let renderer = serde_json::to_value(session).unwrap();
        assert!(renderer["gameData"].get("teamOne").is_none());
        assert!(renderer["gameData"]
            .get("playerChampionSelections")
            .is_none());

        source["phase"] = serde_json::json!("InProgress");
        let session: GameflowSession = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(
            session.resg_champion(&select, Some("self-fixture")),
            Some(22)
        );
        assert_eq!(
            session.resg_champion(&select, Some("missing-fixture")),
            None
        );

        source["gameData"]["queue"]["gameMode"] = serde_json::json!("ARAM");
        let session: GameflowSession = serde_json::from_value(source).unwrap();
        assert_eq!(session.resg_champion(&select, Some("self-fixture")), None);
    }
}
