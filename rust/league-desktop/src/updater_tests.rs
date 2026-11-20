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

fn fixture_manifest() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "version": BETA_OFFER, "product": "maxgamestudio-league", "channel": "beta",
        "tag": "league-v1.0.0-beta.10", "url": BETA_ASSET, "sha256": "bad0".repeat(16),
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
    assert_eq!(
        updater.download_url(BETA_OFFER),
        Err(CheckError::NoVerifiedUpdate)
    );
    *updater.verified.try_lock().unwrap() =
        parse_manifest(&fixture_manifest(), BETA_CURRENT).unwrap();
    assert_eq!(updater.download_url(BETA_OFFER).unwrap(), BETA_ASSET);
    for forbidden in [BETA_ASSET, BETA_CURRENT, "1.0.0"] {
        assert_eq!(
            updater.download_url(forbidden),
            Err(CheckError::NoVerifiedUpdate)
        );
    }
    assert!(matches!(
        runtime().block_on(updater.check("invalid")),
        Err(CheckError::Manifest(UpdateError::Version))
    ));
    assert_eq!(
        updater.download_url(BETA_OFFER),
        Err(CheckError::NoVerifiedUpdate)
    );
}

fn receive_fixture(response: Vec<u8>) -> Result<Vec<u8>, CheckError> {
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
        // A rejected response can close the stream without consuming all bytes.
        let _ = stream.write_all(&response);
    });
    let result = runtime().block_on(async {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mock = Client::builder()
            .no_proxy()
            .http1_only()
            .redirect(Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap();
        let response = manifest_request(&mock, &url)
            .send()
            .await
            .map_err(request_error)?;
        bounded_manifest(response).await
    });
    worker.join().unwrap();
    result
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
        parse_manifest(&valid, BETA_CURRENT)
            .unwrap()
            .unwrap()
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
