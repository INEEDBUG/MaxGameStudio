use super::{buffered_body, demo_query::DemoRoute, detail, HttpResponse};
use hyper::{
    body::Bytes,
    header::{CACHE_CONTROL, CONTENT_TYPE},
    Response, StatusCode, Uri,
};
use percent_encoding::percent_decode_str;

pub(super) fn image(route: DemoRoute, uri: &Uri) -> HttpResponse {
    let encoded = uri.path().rsplit('/').next().unwrap_or_default();
    let decoded = percent_decode_str(encoded).decode_utf8_lossy();
    let mut map = decoded.trim().to_lowercase();
    if map.is_empty()
        || map.chars().count() > 64
        || !map.chars().any(char::is_alphanumeric)
        || !map.chars().all(|c| c == '_' || c.is_alphanumeric())
    {
        return detail(StatusCode::BAD_REQUEST, "Invalid map name");
    }
    if !map.starts_with("de_") && !map.starts_with("cs_") && !map.starts_with("ar_") {
        map.insert_str(0, "de_");
    }
    let layer = form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
        .filter_map(|(key, value)| (key == "layer").then_some(value))
        .last()
        .unwrap_or_default();
    let floor = match layer.trim().to_ascii_lowercase().as_str() {
        "upper" => "",
        "" if layer.is_empty() => "",
        "lower" => "_lower",
        _ => return detail(StatusCode::BAD_REQUEST, "Invalid radar layer"),
    };
    let (suffix, missing) = if matches!(route, DemoRoute::UtilityMask) {
        ("_utility_mask", "No utility mask")
    } else {
        ("", "No bundled radar map")
    };
    let Some(data) = asset(&format!("{map}{floor}{suffix}.png")) else {
        return detail(StatusCode::NOT_FOUND, &format!("{missing} for {map}"));
    };
    Response::builder()
        .header(CONTENT_TYPE, "image/png")
        .header(CACHE_CONTROL, "public, max-age=86400")
        .body(buffered_body(Bytes::from_static(data)))
        .expect("bundled radar response")
}

// Use the repository's existing radar images directly from the mapped native
// executable, without writing a second copy into an installed user's data.
macro_rules! radar_assets {
    ($($file:literal),+ $(,)?) => {
        fn asset(name: &str) -> Option<&'static [u8]> {
            match name {
                $($file => Some(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"), "/../../backend/assets/bundled_radar_maps/", $file
                ))),)+
                _ => None,
            }
        }
    };
}

radar_assets!(
    "ar_baggage.png",
    "ar_baggage_utility_mask.png",
    "ar_baggage_lower.png",
    "ar_baggage_lower_utility_mask.png",
    "ar_shoots.png",
    "ar_shoots_utility_mask.png",
    "cs_italy.png",
    "cs_italy_utility_mask.png",
    "cs_office.png",
    "cs_office_utility_mask.png",
    "de_ancient.png",
    "de_ancient_utility_mask.png",
    "de_anubis.png",
    "de_anubis_utility_mask.png",
    "de_cache.png",
    "de_cache_utility_mask.png",
    "de_dust2.png",
    "de_dust2_utility_mask.png",
    "de_inferno.png",
    "de_inferno_utility_mask.png",
    "de_mirage.png",
    "de_mirage_utility_mask.png",
    "de_nuke.png",
    "de_nuke_utility_mask.png",
    "de_nuke_lower.png",
    "de_nuke_lower_utility_mask.png",
    "de_overpass.png",
    "de_overpass_utility_mask.png",
    "de_train.png",
    "de_train_utility_mask.png",
    "de_train_lower.png",
    "de_train_lower_utility_mask.png",
    "de_vertigo.png",
    "de_vertigo_utility_mask.png",
    "de_vertigo_lower.png",
    "de_vertigo_lower_utility_mask.png",
);
