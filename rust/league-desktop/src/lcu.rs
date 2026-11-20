//! Narrow HTTPS reads from an already verified local League client.

use reqwest::{redirect::Policy, Client, RequestBuilder, StatusCode};
use rustls::pki_types::{pem::PemObject, CertificateDer};
use rustls::{client::Resumption, crypto::ring, ClientConfig, RootCertStore};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
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

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GameflowGameData {
    pub(crate) game_id: Option<u64>,
    pub(crate) is_custom_game: Option<bool>,
    pub(crate) queue: Option<GameflowQueue>,
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
}

impl ReadEndpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Phase => "/lol-gameflow/v1/gameflow-phase",
            Self::CurrentSummoner => "/lol-summoner/v1/current-summoner",
            Self::GameflowSession => "/lol-gameflow/v1/session",
        }
    }

    fn body_limit(self) -> usize {
        match self {
            Self::Phase => 1024,
            Self::CurrentSummoner => 16 * 1024,
            Self::GameflowSession => 256 * 1024,
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
}
