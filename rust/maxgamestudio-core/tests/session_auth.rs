use maxgamestudio_core::session_auth::{strip_session_query, SessionAuth, WebSocketAuthorization};

const TOKEN: &str = "synthetic-desktop-session";

#[test]
fn protects_http_with_the_existing_header_and_media_query() {
    let auth = SessionAuth::new(TOKEN);
    let query = b"start=1&_session=synthetic-desktop-session";
    assert!(auth.authorize_http(None, query));
    assert!(auth.authorize_http(Some(TOKEN), b"start=1"));
    assert!(auth.authorize_http(Some(""), query));
    assert!(!auth.authorize_http(Some(" "), query));
    assert!(!auth.authorize_http(Some("wrong"), query));
    assert!(auth.authorize_http(None, b"_session=wrong&_session=synthetic-desktop-session"));
    assert!(!auth.authorize_http(None, b"_session=synthetic-desktop-session&_session=wrong"));
    assert!(!auth.authorize_http(Some("无效"), b""));
}

#[test]
fn strips_all_media_tokens_before_routing_or_logging() {
    assert_eq!(
        strip_session_query(b"t=1&_session=fixture&%5Fsession=fixture&x=foo%2Fbar&empty=").as_ref(),
        b"t=1&x=foo%2Fbar&empty=",
    );
    let original = b"range=a%2Fb&name=one%20two";
    assert_eq!(strip_session_query(original).as_ref(), original);
}

#[test]
fn requires_both_the_websocket_marker_and_the_correct_session() {
    let auth = SessionAuth::new(TOKEN);
    assert_eq!(
        auth.authorize_websocket(None),
        WebSocketAuthorization::Reject
    );
    assert_eq!(
        auth.authorize_websocket(Some(TOKEN)),
        WebSocketAuthorization::Reject,
    );
    assert_eq!(
        auth.authorize_websocket(Some("cs2-insight, bogus")),
        WebSocketAuthorization::Reject,
    );
    assert_eq!(
        auth.authorize_websocket(Some(" cs2-insight , synthetic-desktop-session ")),
        WebSocketAuthorization::AcceptWithProtocol("cs2-insight"),
    );
    assert_eq!(
        auth.overlay_fragment(),
        "#session=synthetic-desktop-session"
    );
}

#[test]
fn keeps_manual_local_browser_mode_and_its_plain_websocket() {
    let auth = SessionAuth::new("");
    assert!(auth.authorize_http(None, b""));
    assert_eq!(
        auth.authorize_websocket(None),
        WebSocketAuthorization::Accept
    );
    assert_eq!(auth.overlay_fragment(), "");
}
