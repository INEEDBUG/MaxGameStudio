use maxgamestudio_league_core::update::{
    league_feed_identity, league_update_from_feed, LeagueFeedIdentity, LeagueUpdate, UpdateError,
};
use reqwest::{
    header::{ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL},
    redirect::Policy,
    Client, RequestBuilder, Response, StatusCode,
};
use serde::Serialize;
use std::{fmt, time::Duration};
use tokio::sync::{Mutex, OnceCell};

pub(super) const NAMESPACE: &str = "league-update-main";
const HEADER_LIMIT: usize = 16 * 1024;
const BODY_LIMIT: usize = 16 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CheckError {
    Configuration,
    Transport,
    TimedOut,
    Status(u16),
    HeadersTooLarge,
    BodyTooLarge,
    Manifest(UpdateError),
    Busy,
    NoVerifiedUpdate,
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration => f.write_str("无法初始化 League 更新检查。"),
            Self::Transport => f.write_str("无法连接 League 更新源。"),
            Self::TimedOut => f.write_str("League 更新检查超时。"),
            Self::Status(status) => write!(f, "League 更新源返回 HTTP {status}。"),
            Self::HeadersTooLarge | Self::BodyTooLarge => f.write_str("League 更新清单响应过大。"),
            Self::Manifest(error) => write!(f, "League 更新清单验证失败。{error}"),
            Self::Busy => f.write_str("League 更新检查正在进行。"),
            Self::NoVerifiedUpdate => f.write_str("请重新检查 League 更新后下载。"),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateCheck<'a> {
    current_version: &'a str,
    feed: LeagueFeedIdentity,
    available_version: Option<String>,
}

#[derive(Default)]
pub(super) struct LeagueUpdater {
    client: OnceCell<Client>,
    verified: Mutex<Option<LeagueUpdate>>,
}

impl LeagueUpdater {
    pub(super) async fn check<'a>(&self, current: &'a str) -> Result<UpdateCheck<'a>, CheckError> {
        // One request owns the candidate until its response is validated. A failed refresh
        // clears the old candidate so a stale button can never select it.
        let mut verified = self.verified.try_lock().map_err(|_| CheckError::Busy)?;
        *verified = None;
        let feed = league_feed_identity(current).map_err(CheckError::Manifest)?;
        let client = self
            .client
            .get_or_try_init(|| async { build_client() })
            .await?;
        let response = manifest_request(client, feed.url)
            .send()
            .await
            .map_err(request_error)?;
        let manifest = bounded_manifest(response).await?;
        let offer = parse_manifest(&manifest, current)?;
        let available_version = offer.as_ref().map(|update| update.version.clone());
        *verified = offer;
        Ok(UpdateCheck {
            current_version: current,
            feed,
            available_version,
        })
    }

    pub(super) fn download_url(&self, version: &str) -> Result<String, CheckError> {
        self.verified
            .try_lock()
            .map_err(|_| CheckError::Busy)?
            .as_ref()
            .filter(|offer| offer.version == version)
            .map(|offer| offer.url.clone())
            .ok_or(CheckError::NoVerifiedUpdate)
    }
}

fn build_client() -> Result<Client, CheckError> {
    // The pinned reqwest feature uses the platform certificate verifier and expects an
    // installed Rustls crypto provider. The separate LCU client supplies its own trust roots.
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::builder()
        .tls_backend_rustls()
        .https_only(true)
        .http1_only()
        .no_proxy()
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .pool_max_idle_per_host(0)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| CheckError::Configuration)
}

fn manifest_request(client: &Client, url: &str) -> RequestBuilder {
    client
        .get(url)
        .header(ACCEPT, "application/json")
        .header(ACCEPT_ENCODING, "identity")
        .header(CACHE_CONTROL, "no-cache")
}

async fn bounded_manifest(mut response: Response) -> Result<Vec<u8>, CheckError> {
    if response.status() != StatusCode::OK {
        return Err(CheckError::Status(response.status().as_u16()));
    }
    // Hyper bounds the HTTP/1 parser. Apply a smaller application limit before reading the body.
    let header_bytes: usize = response
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + 4)
        .sum();
    if header_bytes > HEADER_LIMIT {
        return Err(CheckError::HeadersTooLarge);
    }
    let length = response.content_length().unwrap_or_default();
    if length > BODY_LIMIT as u64 {
        return Err(CheckError::BodyTooLarge);
    }
    let mut body = Vec::with_capacity(length as usize);
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if chunk.len() > BODY_LIMIT - body.len() {
            return Err(CheckError::BodyTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_manifest(body: &[u8], current: &str) -> Result<Option<LeagueUpdate>, CheckError> {
    let payload =
        serde_json::from_slice(body).map_err(|_| CheckError::Manifest(UpdateError::Manifest))?;
    league_update_from_feed(&payload, current).map_err(CheckError::Manifest)
}

fn request_error(error: reqwest::Error) -> CheckError {
    if error.is_timeout() {
        CheckError::TimedOut
    } else {
        CheckError::Transport
    }
}

#[cfg(test)]
#[path = "updater_tests.rs"]
mod tests;
