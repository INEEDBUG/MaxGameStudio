use super::*;
use serde_json::json;
use std::{
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, TcpListener},
    thread,
    time::Instant,
};

const BETA_CURRENT: &str = "1.0.0-beta.9";
const BETA_OFFER: &str = "1.0.0-beta.10";
const BETA_ASSET: &str = "https://github.com/INEEDBUG/MaxGameStudio/releases/download/league-v1.0.0-beta.10/MaxGameStudioLeague_1.0.0-beta.10_x64-setup.exe";
const INSTALLER: &[u8] = b"League installer fixture\n";
const INSTALLER_SHA256: &str = "89152d6b1a92540c461ddb9c3b5092aa4fdeaec31c38c61d834b5e6f1fb62a88";

// Deterministic fixture-only keys and a prehashed signature in Tauri's outer base64 format.
const FIXTURE_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IGZpeHR1cmUtb25seQpSV1J0WjNNdGRHVnpkT3BLYkdQaW5GSUt2dlZRZXhNdXhmbVZSM2F1dnI1N2trSWU2bWtVUnRJcw==";
const OTHER_KEY_WITH_SAME_ID: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IGZpeHR1cmUtb25seQpSV1J0WjNNdGRHVnpkQk9ZOWl4dEdrVjhVYnBxUzE4OXZTOXAvS2t5RmlHTnlKbCtRV3ZSZlpQSw==";
const FIXTURE_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSdFozTXRkR1Z6ZEZHRG5Qcy9MMWpUdkZMNXVERkM4R2pmL0lMclFuTjRoSTE2MmFpYVQzbXI5bHg2SEQ5Zkx2WEdTUCsrSUhhWVJGZjZDbG1RbUNXaXRYaHdFVTlqNmdVPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDowCWZpbGU6TWF4R2FtZVN0dWRpb0xlYWd1ZV8xLjAuMC1iZXRhLjEwX3g2NC1zZXR1cC5leGUKT1hlaDN1UXNIemUzSzV5Tjd3N0crWE00TFRsTkRBemdoa2VXT21JT0lFQ2g3c1YrajBFeUxFdXMyek90ZzVIRWUxUE8xWU9vWC9MSzJ1TkdWT0p5QlE9PQ==";

fn fixture_manifest() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "version": BETA_OFFER, "product": "maxgamestudio-league", "channel": "beta",
        "tag": "league-v1.0.0-beta.10", "url": BETA_ASSET,
        "sha256": INSTALLER_SHA256, "signature": FIXTURE_SIGNATURE,
    }))
    .unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn native_request_is_a_credential_free_get_to_the_installed_feed() {
    let client = build_client().unwrap();
    for installed in ["1.0.0-beta.9", "1.0.0"] {
        let feed = league_feed_identity(installed).unwrap();
        let request = manifest_request(&client, feed.url).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(request.url().as_str(), feed.url);
        assert_eq!(request.url().host_str(), Some("raw.githubusercontent.com"));
        assert!(request
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .is_none());
        assert!(request.headers().get(reqwest::header::COOKIE).is_none());
    }
}

#[test]
fn opener_requires_the_currently_verified_version() {
    let updater = LeagueUpdater::default();
    let scratch = tempfile::tempdir().unwrap();
    let download = |version| runtime().block_on(updater.download(version, scratch.path()));
    assert_eq!(download(BETA_OFFER), Err(CheckError::NoVerifiedUpdate));
    *updater.verified.try_lock().unwrap() =
        parse_manifest(&fixture_manifest(), BETA_CURRENT, FIXTURE_KEY).unwrap();
    for forbidden in [BETA_ASSET, BETA_CURRENT, "1.0.0"] {
        assert_eq!(download(forbidden), Err(CheckError::NoVerifiedUpdate));
    }
    let target = scratch
        .path()
        .join("MaxGameStudioLeague_1.0.0-beta.10_x64-setup.exe");
    std::fs::write(&target, b"user-owned file").unwrap();
    assert_eq!(download(BETA_OFFER), Err(CheckError::DestinationExists));
    assert_eq!(std::fs::read(&target).unwrap(), b"user-owned file");
    assert!(matches!(
        runtime().block_on(updater.check("invalid")),
        Err(CheckError::Manifest(UpdateError::Version))
    ));
    assert_eq!(download(BETA_OFFER), Err(CheckError::NoVerifiedUpdate));
}

fn serve_fixture<T>(response: Vec<u8>, receive: impl FnOnce(&str) -> T) -> T {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture did not receive a connection: {error}"),
            }
        };
        stream.set_read_timeout(Some(CONNECT_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(CONNECT_TIMEOUT)).unwrap();
        let mut request = [0; 4096];
        let mut used = 0;
        while !request[..used].ends_with(b"\r\n\r\n") {
            let count = stream.read(&mut request[used..]).unwrap();
            assert_ne!(count, 0, "incomplete fixture request");
            used += count;
        }
        assert!(request[..used].starts_with(b"GET /fixture HTTP/1.1\r\n"));
        let request = std::str::from_utf8(&request[..used])
            .unwrap()
            .to_ascii_lowercase();
        assert!(!request.contains("\r\nauthorization:"));
        assert!(!request.contains("\r\ncookie:"));
        // A rejected response can close the stream without consuming all bytes.
        let _ = stream.write_all(&response);
    });
    let result = receive(&url);
    worker.join().unwrap();
    result
}

fn receive_fixture(response: Vec<u8>) -> Result<Vec<u8>, CheckError> {
    serve_fixture(response, |url| {
        runtime().block_on(async {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let mock = Client::builder()
                .no_proxy()
                .http1_only()
                .redirect(Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap();
            let response = manifest_request(&mock, url)
                .send()
                .await
                .map_err(request_error)?;
            bounded_manifest(response).await
        })
    })
}

fn save_fixture(body: &[u8], offer: &DownloadOffer, path: &Path) -> Result<(), CheckError> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    save_fixture_response([header.as_bytes(), body].concat(), offer, path)
}

fn save_fixture_response(
    response: Vec<u8>,
    offer: &DownloadOffer,
    path: &Path,
) -> Result<(), CheckError> {
    serve_fixture(response, |url| {
        runtime().block_on(async {
            let client = Client::builder().no_proxy().build().unwrap();
            let response = release_request(&client, url).send().await.unwrap();
            save_download(response, offer, path).await
        })
    })
}

#[test]
fn retains_stale_unsigned_behavior_and_rejects_signed_beta_on_stable() {
    let mut payload: serde_json::Value = serde_json::from_slice(&fixture_manifest()).unwrap();
    assert!(matches!(
        parse_manifest(&fixture_manifest(), "1.0.0", FIXTURE_KEY),
        Err(CheckError::Manifest(UpdateError::Channel))
    ));
    payload.as_object_mut().unwrap().remove("signature");
    let unsigned = serde_json::to_vec(&payload).unwrap();
    assert!(parse_manifest(&unsigned, BETA_OFFER, FIXTURE_KEY)
        .unwrap()
        .is_none());
    assert!(matches!(
        parse_manifest(&unsigned, BETA_CURRENT, FIXTURE_KEY),
        Err(CheckError::Signature)
    ));

    // A well-formed stable manifest still cannot make a genuinely signed beta a stable installer.
    payload["version"] = json!("1.1.0");
    payload["channel"] = json!("stable");
    payload["tag"] = json!("league-v1.1.0");
    payload["url"] = json!(concat!(
        "https://github.com/INEEDBUG/MaxGameStudio/releases/download/league-v1.1.0/",
        "MaxGameStudioLeague_1.1.0_x64-setup.exe"
    ));
    payload["signature"] = json!(FIXTURE_SIGNATURE);
    assert!(matches!(
        parse_manifest(&serde_json::to_vec(&payload).unwrap(), "1.0.0", FIXTURE_KEY),
        Err(CheckError::Signature)
    ));

    let forged_comment = decode_tauri_text(FIXTURE_SIGNATURE).unwrap().replace(
        "file:MaxGameStudioLeague_1.0.0-beta.10_x64-setup.exe",
        "file:MaxGameStudioLeague_1.1.0_x64-setup.exe",
    );
    payload["signature"] = json!(STANDARD.encode(forged_comment));
    assert!(matches!(
        parse_manifest(&serde_json::to_vec(&payload).unwrap(), "1.0.0", FIXTURE_KEY),
        Err(CheckError::Signature)
    ));
}

#[test]
fn saves_verified_bytes_once_and_never_overwrites_an_existing_download() {
    let offer = parse_manifest(&fixture_manifest(), BETA_CURRENT, FIXTURE_KEY)
        .unwrap()
        .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let target = scratch.path().join(&offer.filename);
    assert_eq!(save_fixture(INSTALLER, &offer, &target), Ok(()));
    assert_eq!(std::fs::read(&target).unwrap(), INSTALLER);
    #[cfg(windows)]
    {
        let mut zone = target.as_os_str().to_owned();
        zone.push(":Zone.Identifier");
        assert_eq!(
            std::fs::read_to_string(zone).unwrap(),
            "[ZoneTransfer]\r\nZoneId=3\r\n"
        );
    }

    // Simulate a collision appearing after the preflight check but before publication.
    std::fs::write(&target, b"user-owned file").unwrap();
    assert_eq!(
        save_fixture(INSTALLER, &offer, &target),
        Err(CheckError::DestinationExists)
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"user-owned file");
    assert_eq!(scratch.path().read_dir().unwrap().count(), 1);
}

#[test]
fn rejects_wrong_key_tampered_bytes_and_wrong_checksum_without_a_saved_file() {
    let scratch = tempfile::tempdir().unwrap();
    let offer = |key| {
        parse_manifest(&fixture_manifest(), BETA_CURRENT, key)
            .unwrap()
            .unwrap()
    };
    assert!(matches!(
        parse_manifest(&fixture_manifest(), BETA_CURRENT, OTHER_KEY_WITH_SAME_ID),
        Err(CheckError::Signature)
    ));

    let mut genuine = offer(FIXTURE_KEY);
    let target = scratch.path().join(&genuine.filename);
    genuine.sha256 = Sha256::digest(b"modified").into();
    assert_eq!(
        save_fixture(b"modified", &genuine, &target),
        Err(CheckError::Signature)
    );
    assert_eq!(
        save_fixture(INSTALLER, &genuine, &target),
        Err(CheckError::Checksum)
    );
    assert_eq!(scratch.path().read_dir().unwrap().count(), 0);
}

#[test]
fn rejects_oversized_and_non_success_installer_responses_without_a_file() {
    let offer = parse_manifest(&fixture_manifest(), BETA_CURRENT, FIXTURE_KEY)
        .unwrap()
        .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let target = scratch.path().join(&offer.filename);
    let too_large = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        INSTALLER_LIMIT + 1
    );
    assert_eq!(
        save_fixture_response(too_large.into_bytes(), &offer, &target),
        Err(CheckError::InstallerTooLarge)
    );
    let missing = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
    assert_eq!(
        save_fixture_response(missing.to_vec(), &offer, &target),
        Err(CheckError::Status(404))
    );
    assert_eq!(scratch.path().read_dir().unwrap().count(), 0);
}

#[test]
fn download_redirect_accepts_one_credential_free_https_github_asset_hop() {
    let initial = reqwest::Url::parse(BETA_ASSET).unwrap();
    let allowed =
        reqwest::Url::parse("https://release-assets.githubusercontent.com/asset?fixture=1")
            .unwrap();
    assert!(release_redirect_allowed(
        std::slice::from_ref(&initial),
        &allowed
    ));
    assert!(!release_redirect_allowed(
        &[initial.clone(), allowed.clone()],
        &allowed
    ));
    for bad in [
        "http://release-assets.githubusercontent.com/asset",
        "https://release-assets.githubusercontent.com.evil.invalid/a",
        "https://release-assets.githubusercontent.com:8443/a",
        "https://user@release-assets.githubusercontent.com/a",
    ] {
        assert!(!release_redirect_allowed(
            std::slice::from_ref(&initial),
            &bad.parse().unwrap()
        ));
    }
}

#[test]
#[ignore = "requires the exact hosted League installer and its Tauri signature"]
fn signed_release_installer_matches_embedded_league_key() {
    let installer = PathBuf::from(std::env::var_os("MGS_LEAGUE_SIGNED_UPDATE_INSTALLER").unwrap());
    let version = std::env::var("MGS_LEAGUE_SIGNED_UPDATE_VERSION").unwrap();
    assert!(installer.is_absolute());
    let filename = format!("MaxGameStudioLeague_{version}_x64-setup.exe");
    assert_eq!(
        installer.file_name().unwrap(),
        std::ffi::OsStr::new(&filename)
    );
    let signature = std::fs::read_to_string(installer.with_extension("exe.sig")).unwrap();

    let mut file = std::fs::File::open(&installer).unwrap();
    assert!((1..=INSTALLER_LIMIT).contains(&file.metadata().unwrap().len()));
    let mut sha256 = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        sha256.update(&buffer[..count]);
    }
    let payload = serde_json::to_vec(&json!({
        "version": version, "product": "maxgamestudio-league", "channel": "beta",
        "tag": format!("league-v{version}"),
        "url": format!("https://github.com/INEEDBUG/MaxGameStudio/releases/download/league-v{version}/{filename}"),
        "sha256": hex::encode(sha256.finalize()), "signature": signature.trim(),
    }))
    .unwrap();
    let offer = parse_manifest(&payload, "0.0.0-beta.0", LEAGUE_PUBLIC_KEY)
        .unwrap()
        .unwrap();
    file.rewind().unwrap();
    assert_eq!(verify_installer(&offer, file), Ok(()));
    assert!(matches!(
        parse_manifest(&payload, "0.0.0", LEAGUE_PUBLIC_KEY),
        Err(CheckError::Manifest(UpdateError::Channel))
    ));
}

#[test]
fn response_bounds_reject_redirects_and_oversized_headers_and_bodies() {
    let body = fixture_manifest();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let valid = receive_fixture([head.as_bytes(), &body].concat()).unwrap();
    assert_eq!(
        parse_manifest(&valid, BETA_CURRENT, FIXTURE_KEY)
            .unwrap()
            .unwrap()
            .update
            .version,
        BETA_OFFER
    );

    let redirect = b"HTTP/1.1 307 Temporary Redirect\r\nLocation: https://example.invalid/\r\nContent-Length: 0\r\n\r\n";
    assert_eq!(
        receive_fixture(redirect.to_vec()),
        Err(CheckError::Status(307))
    );
    let large_header = format!(
        "HTTP/1.1 200 OK\r\nX-Fixture: {}\r\nContent-Length: 0\r\n\r\n",
        "a".repeat(HEADER_LIMIT)
    );
    assert_eq!(
        receive_fixture(large_header.into_bytes()),
        Err(CheckError::HeadersTooLarge)
    );
    let declared = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        BODY_LIMIT + 1
    );
    assert_eq!(
        receive_fixture(declared.into_bytes()),
        Err(CheckError::BodyTooLarge)
    );
    let streamed = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{BODY_LIMIT:x}\r\n{}\r\n1\r\na\r\n0\r\n\r\n", "a".repeat(BODY_LIMIT));
    assert_eq!(
        receive_fixture(streamed.into_bytes()),
        Err(CheckError::BodyTooLarge)
    );
}
