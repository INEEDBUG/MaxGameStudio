use base64::{engine::general_purpose::STANDARD, Engine};
use maxgamestudio_league_core::update::{
    league_feed_identity, league_update_from_feed, LeagueFeedIdentity, LeagueUpdate, UpdateError,
};
use minisign_verify::{PublicKey, Signature};
use reqwest::{
    header::{ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL},
    redirect::Policy,
    Client, ClientBuilder, RequestBuilder, Response, StatusCode, Url,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::{Mutex, OnceCell};

pub(super) const NAMESPACE: &str = "league-update-main";
const LEAGUE_PUBLIC_KEY: &str = include_str!("../league-updater.pub");
const HEADER_LIMIT: usize = 16 * 1024;
const BODY_LIMIT: usize = 16 * 1024;
const INSTALLER_LIMIT: u64 = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CheckError {
    Configuration,
    Transport,
    TimedOut,
    Status(u16),
    HeadersTooLarge,
    BodyTooLarge,
    InstallerTooLarge,
    Manifest(UpdateError),
    Signature,
    Checksum,
    Storage,
    DestinationExists,
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
            Self::InstallerTooLarge => f.write_str("League 安装包超过下载大小限制。"),
            Self::Manifest(error) => write!(f, "League 更新清单验证失败。{error}"),
            Self::Signature => f.write_str("League 安装包签名缺失或验证失败。"),
            Self::Checksum => f.write_str("League 安装包校验和不匹配。"),
            Self::Storage => f.write_str("无法保存 League 安装包到下载目录。"),
            Self::DestinationExists => f.write_str("下载目录已有同名文件，请移开原文件后重试。"),
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
    verified: Mutex<Option<DownloadOffer>>,
}

struct DownloadOffer {
    update: LeagueUpdate,
    filename: String,
    public_key: PublicKey,
    signature: Signature,
    sha256: [u8; 32],
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
        let offer = parse_manifest(&manifest, current, LEAGUE_PUBLIC_KEY)?;
        let available_version = offer.as_ref().map(|offer| offer.update.version.clone());
        *verified = offer;
        Ok(UpdateCheck {
            current_version: current,
            feed,
            available_version,
        })
    }

    pub(super) async fn download(
        &self,
        version: &str,
        directory: &Path,
    ) -> Result<PathBuf, CheckError> {
        // Keep this offer locked for the download. A refresh or another click cannot select a
        // different package until the current action finishes.
        let verified = self.verified.try_lock().map_err(|_| CheckError::Busy)?;
        let offer = verified
            .as_ref()
            .filter(|offer| offer.update.version == version)
            .ok_or(CheckError::NoVerifiedUpdate)?;
        let destination = directory.join(&offer.filename);
        match destination.symlink_metadata() {
            Ok(_) => return Err(CheckError::DestinationExists),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(CheckError::Storage),
        }
        let client = client_builder()
            .redirect(Policy::custom(|attempt| {
                if release_redirect_allowed(attempt.previous(), attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .timeout(DOWNLOAD_TIMEOUT)
            .build()
            .map_err(|_| CheckError::Configuration)?;
        let response = release_request(&client, &offer.update.url)
            .send()
            .await
            .map_err(request_error)?;
        save_download(response, offer, &destination).await?;
        Ok(destination)
    }
}

fn client_builder() -> ClientBuilder {
    // The pinned reqwest feature uses the platform certificate verifier and expects an
    // installed Rustls crypto provider. The separate LCU client supplies its own trust roots.
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::builder()
        .tls_backend_rustls()
        .https_only(true)
        .http1_only()
        .no_proxy()
        .retry(reqwest::retry::never())
        .pool_max_idle_per_host(0)
        .connect_timeout(CONNECT_TIMEOUT)
}

fn build_client() -> Result<Client, CheckError> {
    client_builder()
        .redirect(Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| CheckError::Configuration)
}

fn release_redirect_allowed(previous: &[Url], next: &Url) -> bool {
    previous.len() == 1
        && previous[0].host_str() == Some("github.com")
        && next.scheme() == "https"
        && next.host_str() == Some("release-assets.githubusercontent.com")
        && next.port().is_none()
        && next.username().is_empty()
        && next.password().is_none()
}

fn release_request(client: &Client, url: &str) -> RequestBuilder {
    client
        .get(url)
        .header(ACCEPT, "application/octet-stream")
        .header(ACCEPT_ENCODING, "identity")
}

async fn save_download(
    mut response: Response,
    offer: &DownloadOffer,
    destination: &Path,
) -> Result<(), CheckError> {
    if response.status() != StatusCode::OK {
        return Err(CheckError::Status(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > INSTALLER_LIMIT)
    {
        return Err(CheckError::InstallerTooLarge);
    }

    let directory = destination.parent().ok_or(CheckError::Storage)?;
    let mut file = tempfile::Builder::new()
        .prefix(".league-update-")
        .suffix(".part")
        .tempfile_in(directory)
        .map_err(|_| CheckError::Storage)?;
    let mut received = 0;
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        received += chunk.len() as u64;
        if received > INSTALLER_LIMIT {
            return Err(CheckError::InstallerTooLarge);
        }
        file.write_all(&chunk).map_err(|_| CheckError::Storage)?;
    }
    file.rewind().map_err(|_| CheckError::Storage)?;
    verify_installer(offer, file.as_file_mut())?;
    file.as_file().sync_all().map_err(|_| CheckError::Storage)?;

    #[cfg(windows)]
    {
        // Preserve Internet origin metadata normally set by a browser download. Moving the
        // verified file within the same directory also retains its alternate data stream.
        let mut zone = file.path().as_os_str().to_owned();
        zone.push(":Zone.Identifier");
        std::fs::write(zone, "[ZoneTransfer]\r\nZoneId=3\r\n").map_err(|_| CheckError::Storage)?;
    }

    file.persist_noclobber(destination).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            CheckError::DestinationExists
        } else {
            CheckError::Storage
        }
    })?;
    Ok(())
}

fn verify_installer(offer: &DownloadOffer, mut file: impl Read) -> Result<(), CheckError> {
    let mut verifier = offer
        .public_key
        .verify_stream(&offer.signature)
        .map_err(|_| CheckError::Signature)?;
    let mut sha256 = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| CheckError::Storage)?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count]);
        sha256.update(&buffer[..count]);
    }
    let digest: [u8; 32] = sha256.finalize().into();
    if digest != offer.sha256 {
        return Err(CheckError::Checksum);
    }
    // Finalizing checks the installer bytes and authenticates the comment matched at check time.
    verifier.finalize().map_err(|_| CheckError::Signature)
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

fn parse_manifest(
    body: &[u8],
    current: &str,
    key: &str,
) -> Result<Option<DownloadOffer>, CheckError> {
    let payload =
        serde_json::from_slice(body).map_err(|_| CheckError::Manifest(UpdateError::Manifest))?;
    let Some(update) = league_update_from_feed(&payload, current).map_err(CheckError::Manifest)?
    else {
        return Ok(None);
    };
    let key = decode_tauri_text(key.trim()).ok_or(CheckError::Configuration)?;
    let public_key = PublicKey::decode(&key).map_err(|_| CheckError::Configuration)?;
    let signature = payload["signature"]
        .as_str()
        .and_then(decode_tauri_text)
        .ok_or(CheckError::Signature)?;
    let signature = Signature::decode(&signature).map_err(|_| CheckError::Signature)?;
    // This rejects legacy or incompatible records early. Verifying the artifact and trusted
    // comment still requires the installer bytes.
    public_key
        .verify_stream(&signature)
        .map_err(|_| CheckError::Signature)?;
    let filename = format!("MaxGameStudioLeague_{}_x64-setup.exe", update.version);
    // Tauri v2.11.4 includes the basename in the signed comment. Reject mismatched package names
    // before offering a download, including a Beta signature relabeled as a stable release.
    let expected = format!("file:{filename}");
    let mut fields = signature
        .trusted_comment()
        .split('\t')
        .filter(|field| field.starts_with("file:"));
    if fields.next() != Some(expected.as_str()) || fields.next().is_some() {
        return Err(CheckError::Signature);
    }
    let mut sha256 = [0; 32];
    hex::decode_to_slice(payload["sha256"].as_str().unwrap_or_default(), &mut sha256)
        .map_err(|_| CheckError::Manifest(UpdateError::Manifest))?;
    Ok(Some(DownloadOffer {
        update,
        filename,
        public_key,
        signature,
        sha256,
    }))
}

fn decode_tauri_text(value: &str) -> Option<String> {
    String::from_utf8(STANDARD.decode(value).ok()?).ok()
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
