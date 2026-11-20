use super::*;

const PNG_PATH: &str = "/lol-game-data/assets/v1/champion-icons/22.png";
const JPEG_PATH: &str = "/lol-game-data/assets/v1/profile-icons/29.jpg";
const WEBP_PATH: &str = "/lol-game-data/assets/v1/perk-images/Styles/Example.webp";

// Synthetic 3×2 bitmaps encoded with cached Pillow. No game artwork in the fixtures.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1,
    0x4d, 0x00, 0x00, 0x00, 0x15, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xd0, 0xa8, 0x60,
    0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x62, 0x80, 0x01, 0x00, 0x0b, 0x04, 0x00, 0xa4, 0xd8, 0x2a,
    0xe7, 0x7c, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08,
    0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0a, 0x0c, 0x14, 0x0d, 0x0c, 0x0b, 0x0b, 0x0c, 0x19, 0x12,
    0x13, 0x0f, 0x14, 0x1d, 0x1a, 0x1f, 0x1e, 0x1d, 0x1a, 0x1c, 0x1c, 0x20, 0x24, 0x2e, 0x27, 0x20,
    0x22, 0x2c, 0x23, 0x1c, 0x1c, 0x28, 0x37, 0x29, 0x2c, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1f, 0x27,
    0x39, 0x3d, 0x38, 0x32, 0x3c, 0x2e, 0x33, 0x34, 0x32, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x02,
    0x00, 0x03, 0x01, 0x01, 0x11, 0x00, 0xff, 0xc4, 0x00, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0xff, 0xc4, 0x00, 0x14,
    0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0x68, 0xff, 0xd9,
];

const WEBP_LOSSY: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x34, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x20,
    0x28, 0x00, 0x00, 0x00, 0x50, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x03, 0x00, 0x02, 0x00, 0x01, 0x40,
    0x26, 0x25, 0xa0, 0x00, 0x04, 0x00, 0x00, 0x00, 0xfe, 0xf0, 0xc4, 0x0b, 0xff, 0xff, 0x0a, 0x6b,
    0xff, 0xf0, 0xa6, 0xbf, 0xff, 0x0a, 0x6b, 0xfe, 0x00, 0x00, 0x00, 0x00,
];

const WEBP_LOSSLESS: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x1e, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x4c,
    0x11, 0x00, 0x00, 0x00, 0x2f, 0x02, 0x40, 0x00, 0x10, 0x07, 0x50, 0xbc, 0x52, 0x14, 0xa5, 0x0a,
    0x81, 0x88, 0xe8, 0x7f, 0x00, 0x00,
];

const WEBP_ALPHA: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x58, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x58,
    0x0a, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x01, 0x00, 0x00, 0x41, 0x4c,
    0x50, 0x48, 0x07, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x0a, 0x00, 0x56, 0x50,
    0x38, 0x20, 0x2a, 0x00, 0x00, 0x00, 0x90, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x03, 0x00, 0x02, 0x00,
    0x01, 0x40, 0x26, 0x25, 0xa0, 0x02, 0x74, 0xba, 0x00, 0x03, 0x98, 0x00, 0xfe, 0xf1, 0x4d, 0xaf,
    0xc6, 0xf8, 0x82, 0xf6, 0xaf, 0x1f, 0xfc, 0xd8, 0x1f, 0xb7, 0x9f, 0x6f, 0x3c, 0x2c, 0x00, 0x00,
];

#[test]
fn permits_only_known_bitmap_families_and_canonicalizes_percent_escapes() {
    for (raw, expected) in [
        (PNG_PATH, PNG_PATH),
        ("/lol-game-data/assets/v1/champion-icons/-1.png", "/lol-game-data/assets/v1/champion-icons/-1.png"),
        (JPEG_PATH, JPEG_PATH),
        ("/lol-game-data/assets/v1/perk-images/Styles/Resolve/p8400_s0_k0.jpg",
         "/lol-game-data/assets/v1/perk-images/Styles/Resolve/p8400_s0_k0.jpg"),
        ("/lol-game-data/assets/ASSETS/Items/Icons2D/A%20B%28Icon%29.PNG",
         "/lol-game-data/assets/ASSETS/Items/Icons2D/A B(Icon).PNG"),
        ("/lol-game-data/assets/DATA/Spells/Icons2D/Summoner_boost.png",
         "/lol-game-data/assets/DATA/Spells/Icons2D/Summoner_boost.png"),
        ("/lol-game-data/assets/ASSETS/UX/Jade/S3Icons/SameSized/S3_Summoner_Boost.project_jade.png",
         "/lol-game-data/assets/ASSETS/UX/Jade/S3Icons/SameSized/S3_Summoner_Boost.project_jade.png"),
        ("/lol-game-data/assets/content/src/LeagueClient/GameModeAssets/ARAM/img/game-select-icon-hover.png",
         "/lol-game-data/assets/content/src/LeagueClient/GameModeAssets/ARAM/img/game-select-icon-hover.png"),
        (WEBP_PATH, WEBP_PATH),
    ] {
        let path = GameArtPath::parse(raw).unwrap_or_else(|| panic!("Unexpected rejected asset: {raw}"));
        assert_eq!(path.path(), expected);
        assert_eq!(GameArtPath::parse(path.path()).unwrap().path(), expected);
    }
}

#[test]
fn rejects_traversal_other_origins_queries_unknown_formats_and_other_asset_families() {
    for path in [
        "https://127.0.0.1:31111/lol-game-data/assets/v1/champion-icons/22.png",
        "//127.0.0.1/lol-game-data/assets/v1/champion-icons/22.png",
        "akari://league-client/lol-game-data/assets/v1/champion-icons/22.png",
        "lol-game-data/assets/v1/champion-icons/22.png",
        "/lol-game-data/assets/v1/champion-icons/22.png?type=gif",
        "/lol-game-data/assets/v1/champion-icons/22.png#anchor",
        "/lol-game-data/assets/v1/champion-icons/22.png%3fheader",
        "/lol-game-data/assets/v1/champion-icons/022.png",
        "/lol-game-data/assets/v1/champion-icons/-3.png",
        "/lol-game-data/assets/v1/champion-icons/4294967296.png",
        "/lol-game-data/assets/v1/perk-images/a/../a.png",
        "/lol-game-data/assets/v1/perk-images/%2E%2E/a.png",
        "/lol-game-data/assets/v1/perk-images/..%20/a.png",
        "/lol-game-data/assets/v1/perk-images/a./a.png",
        "/lol-game-data/assets/v1/perk-images/a%2fa.png",
        "/lol-game-data/assets/v1/perk-images/a%5ca.png",
        "/lol-game-data/assets/v1/perk-images/a%252f.png",
        "/lol-game-data/assets/v1/perk-images/a%00.png",
        "/lol-game-data/assets/v1/perk-images/a%.png",
        "/lol-game-data/assets/v1/perk-images/a%zz.png",
        "/lol-game-data/assets/v1/perk-images/a//a.png",
        "/lol-game-data/assets/v1/perk-images/a.png\n",
        r"/lol-game-data/assets/v1/perk-images/a\..\a.png",
        "/lol-game-data/assets/v1/perk-images/resolve.svg",
        "/lol-game-data/assets/v1/perk-images/resolve.gif",
        "/lol-game-data/assets/v1/items.json",
        "/lol-game-data/assets/v1/champion-splashes/22/22000.jpg",
        "/lol-summoner/v1/current-summoner",
    ] {
        assert!(GameArtPath::parse(path).is_none(), "Accepted {path:?}");
    }
    let oversized = format!(
        "/lol-game-data/assets/v1/perk-images/{}.png",
        "p".repeat(1024)
    );
    assert!(GameArtPath::parse(&oversized).is_none());
}

#[test]
fn request_uses_the_verified_loopback_client_with_no_renderer_headers_or_query() {
    let target = LcuTarget::new(31111, "fixture-only").unwrap();
    let asset = GameArtPath::parse("/lol-game-data/assets/ASSETS/Items/Icons2D/A%20B.png").unwrap();
    let request = LcuTransport::new()
        .unwrap()
        .game_art_request(&target, &asset)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(
        request.url().as_str(),
        "https://127.0.0.1:31111/lol-game-data/assets/ASSETS/Items/Icons2D/A%20B.png"
    );
    assert_eq!(request.headers()[reqwest::header::ACCEPT], "image/png");
    assert_eq!(request.headers().len(), 2);
    assert!(request.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
}

#[test]
fn returns_only_the_matched_bitmap_bytes_and_canonical_mime() {
    for (path, mime, bytes) in [
        (PNG_PATH, Some("Image/PNG; optional=1"), PNG),
        (JPEG_PATH, Some("application/octet-stream"), JPEG),
        (WEBP_PATH, Some("image/webp"), WEBP_LOSSY),
        (WEBP_PATH, None, WEBP_LOSSLESS),
        (WEBP_PATH, Some("image/webp"), WEBP_ALPHA),
    ] {
        let path = GameArtPath::parse(path).unwrap();
        let art = path.finish(mime, bytes.to_vec()).unwrap();
        assert_eq!(art.body, bytes);
        assert_eq!(art.content_type, path.format.content_type());
        assert_eq!(path.format.dimensions(bytes), Some((3, 2)));
    }
    let png = GameArtPath::parse(PNG_PATH).unwrap();
    assert_eq!(
        png.finish(Some("text/html"), PNG.to_vec()),
        Err(LcuError::InvalidPayload)
    );
    assert_eq!(
        png.finish(Some("image/png"), JPEG.to_vec()),
        Err(LcuError::InvalidPayload)
    );
    assert_eq!(
        png.finish(None, b"<svg/>".to_vec()),
        Err(LcuError::InvalidPayload)
    );
}

#[test]
fn fails_closed_on_truncated_animated_or_oversized_bitmaps() {
    let png = GameArtPath::parse(PNG_PATH).unwrap();
    let jpeg = GameArtPath::parse(JPEG_PATH).unwrap();
    let webp = GameArtPath::parse(WEBP_PATH).unwrap();
    for (path, payload) in [(&png, PNG), (&jpeg, JPEG), (&webp, WEBP_ALPHA)] {
        assert_eq!(
            path.finish(None, payload[..payload.len() - 1].to_vec()),
            Err(LcuError::InvalidPayload)
        );
    }
    let mut huge_png = PNG.to_vec();
    huge_png[16..20].copy_from_slice(&2049_u32.to_be_bytes());
    assert_eq!(png.finish(None, huge_png), Err(LcuError::InvalidPayload));
    let mut animated_png = PNG.to_vec();
    animated_png[37..41].copy_from_slice(b"acTL");
    assert_eq!(
        png.finish(None, animated_png),
        Err(LcuError::InvalidPayload)
    );
    let mut animated_webp = WEBP_ALPHA.to_vec();
    animated_webp[20] |= 2;
    assert_eq!(
        webp.finish(None, animated_webp),
        Err(LcuError::InvalidPayload)
    );

    assert_eq!(
        art_buffer(MAX_ART_BYTES as u64 + 1),
        Err(LcuError::ResponseTooLarge)
    );
    let mut body = art_buffer(0).unwrap();
    append_chunk(&mut body, &vec![0; MAX_ART_BYTES - 1], MAX_ART_BYTES).unwrap();
    assert_eq!(
        append_chunk(&mut body, &[1, 2], MAX_ART_BYTES),
        Err(LcuError::ResponseTooLarge)
    );
    assert_eq!(body.len(), MAX_ART_BYTES - 1);
}
