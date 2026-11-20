//! Bounded static artwork reads from an already verified local League client.

use super::{append_chunk, request_error, response_has_body, LcuError, LcuTarget, LcuTransport};
use reqwest::{header, RequestBuilder, Url};

const ASSET_PREFIX: &str = "/lol-game-data/assets/";
const MAX_PATH_BYTES: usize = 1024;
const MAX_ART_BYTES: usize = 1024 * 1024;
const MAX_DIMENSION: u32 = 2048;

const ARAM_BADGE: &str =
    "content/src/LeagueClient/GameModeAssets/ARAM/img/game-select-icon-hover.png";
const BITMAP_FAMILIES: &[&str] = &[
    "v1/perk-images/",
    "ASSETS/Items/Icons2D/",
    "DATA/Spells/Icons2D/",
    "ASSETS/UX/Jade/S3Icons/SameSized/",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum BitmapFormat {
    Png,
    Jpeg,
    Webp,
}

impl BitmapFormat {
    fn of_filename(filename: &str) -> Option<Self> {
        let (_, extension) = filename.rsplit_once('.')?;
        if extension.eq_ignore_ascii_case("png") {
            Some(Self::Png)
        } else if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
            Some(Self::Jpeg)
        } else if extension.eq_ignore_ascii_case("webp") {
            Some(Self::Webp)
        } else {
            None
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    fn accepts_mime(self, claimed: Option<&str>) -> bool {
        claimed.is_none_or(|value| {
            let mime = value.split(';').next().unwrap_or_default().trim();
            mime.eq_ignore_ascii_case(self.content_type())
                || mime.eq_ignore_ascii_case("application/octet-stream")
        })
    }

    fn dimensions(self, body: &[u8]) -> Option<(u32, u32)> {
        match self {
            Self::Png => png_dimensions(body),
            Self::Jpeg => jpeg_dimensions(body),
            Self::Webp => webp_dimensions(body),
        }
    }
}

/// An origin-free path for an approved League bitmap. Construction validates
/// the normalized segments before any URL parser can discard dot segments.
pub(crate) struct GameArtPath {
    canonical: String,
    format: BitmapFormat,
}

impl GameArtPath {
    pub(crate) fn parse(path: &str) -> Option<Self> {
        if path.len() > MAX_PATH_BYTES {
            return None;
        }
        let relative = path
            .strip_prefix(ASSET_PREFIX)?
            .split('/')
            .map(canonical_segment)
            .collect::<Option<Vec<_>>>()?
            .join("/");
        let format = BitmapFormat::of_filename(relative.rsplit('/').next()?)?;
        if !supports_bitmap(&relative, format) {
            return None;
        }
        Some(Self {
            canonical: format!("{ASSET_PREFIX}{relative}"),
            format,
        })
    }

    pub(crate) fn path(&self) -> &str {
        &self.canonical
    }

    fn finish(&self, mime: Option<&str>, body: Vec<u8>) -> Result<GameArt, LcuError> {
        let valid = self.format.accepts_mime(mime)
            && self
                .format
                .dimensions(&body)
                .is_some_and(|(width, height)| {
                    (1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height)
                });
        if !valid {
            return Err(LcuError::InvalidPayload);
        }
        Ok(GameArt {
            body,
            content_type: self.format.content_type(),
        })
    }
}

fn canonical_segment(segment: &str) -> Option<String> {
    let mut canonical = String::with_capacity(segment.len());
    let mut raw = segment.bytes();
    while let Some(byte) = raw.next() {
        let decoded = if byte == b'%' {
            let high = char::from(raw.next()?).to_digit(16)?;
            let low = char::from(raw.next()?).to_digit(16)?;
            u8::try_from((high << 4) | low).ok()?
        } else {
            byte
        };
        if !(decoded.is_ascii_alphanumeric() || b"._- ()'".contains(&decoded)) {
            return None;
        }
        canonical.push(char::from(decoded));
    }
    // Windows can discard trailing periods or spaces while resolving paths.
    if canonical.is_empty()
        || canonical.len() > 255
        || canonical.starts_with(['.', ' '])
        || canonical.ends_with(['.', ' '])
    {
        return None;
    }
    Some(canonical)
}

fn supports_bitmap(relative: &str, format: BitmapFormat) -> bool {
    if let Some(file) = relative.strip_prefix("v1/champion-icons/") {
        return format == BitmapFormat::Png && numeric_icon(file, true);
    }
    if let Some(file) = relative.strip_prefix("v1/profile-icons/") {
        return format == BitmapFormat::Jpeg && numeric_icon(file, false);
    }
    relative == ARAM_BADGE
        || BITMAP_FAMILIES
            .iter()
            .any(|prefix| relative.starts_with(prefix))
}

fn numeric_icon(filename: &str, champion: bool) -> bool {
    filename.rsplit_once('.').is_some_and(|(stem, _)| {
        (champion && stem == "-1") || stem.parse::<u32>().is_ok_and(|id| id.to_string() == stem)
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GameArt {
    pub(crate) body: Vec<u8>,
    pub(crate) content_type: &'static str,
}

impl LcuTransport {
    fn game_art_request(
        &self,
        target: &LcuTarget<'_>,
        asset: &GameArtPath,
    ) -> Result<RequestBuilder, LcuError> {
        let mut url = Url::parse(&format!("https://127.0.0.1:{}", target.port))
            .map_err(|_| LcuError::InvalidTarget)?;
        url.set_path(asset.path());
        Ok(self
            .client
            .get(url)
            .header(header::ACCEPT, asset.format.content_type())
            .basic_auth("riot", Some(target.token)))
    }

    pub(crate) async fn read_game_art(
        &self,
        target: &LcuTarget<'_>,
        asset: &GameArtPath,
    ) -> Result<Option<GameArt>, LcuError> {
        let mut response = self
            .game_art_request(target, asset)?
            .send()
            .await
            .map_err(request_error)?;
        if !response_has_body(response.status())? {
            return Ok(None);
        }

        let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
        let mime = content_type
            .as_ref()
            .map(header::HeaderValue::to_str)
            .transpose()
            .map_err(|_| LcuError::InvalidPayload)?;
        let mut body = art_buffer(response.content_length().unwrap_or_default())?;
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            append_chunk(&mut body, &chunk, MAX_ART_BYTES)?;
        }
        asset.finish(mime, body).map(Some)
    }
}

fn art_buffer(declared_bytes: u64) -> Result<Vec<u8>, LcuError> {
    if declared_bytes > MAX_ART_BYTES as u64 {
        return Err(LcuError::ResponseTooLarge);
    }
    Ok(Vec::with_capacity(declared_bytes as usize))
}

// These are container and dimension checks. Decoding remains with WebView2.
fn png_dimensions(body: &[u8]) -> Option<(u32, u32)> {
    let header = body.strip_prefix(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR")?;
    let width = u32::from_be_bytes(header.get(..4)?.try_into().ok()?);
    let height = u32::from_be_bytes(header.get(4..8)?.try_into().ok()?);

    // Advance past the 13-byte IHDR payload and its four-byte CRC.
    let mut chunks = header.get(17..)?;
    let mut has_pixels = false;
    loop {
        let length = u32::from_be_bytes(chunks.get(..4)?.try_into().ok()?) as usize;
        let kind = chunks.get(4..8)?;
        chunks = chunks.get(length.checked_add(12)?..)?;
        match kind {
            b"IDAT" => has_pixels |= length > 0,
            b"IEND" => {
                return (has_pixels && length == 0 && chunks.is_empty()).then_some((width, height))
            }
            b"IHDR" | b"acTL" | b"fcTL" | b"fdAT" => return None,
            _ => {}
        }
    }
}

fn jpeg_dimensions(body: &[u8]) -> Option<(u32, u32)> {
    let mut markers = body
        .strip_prefix(&[0xff, 0xd8])?
        .strip_suffix(&[0xff, 0xd9])?;
    let mut dimensions = None;
    loop {
        let mut marker = markers.strip_prefix(&[0xff])?;
        while marker.first() == Some(&0xff) {
            marker = &marker[1..];
        }
        let (&kind, tail) = marker.split_first()?;
        let length = usize::from(u16::from_be_bytes(tail.get(..2)?.try_into().ok()?));
        let data = tail.get(2..length)?;
        markers = tail.get(length..)?;
        match kind {
            0xc0..=0xc2 if dimensions.is_none() => {
                let &[8, h0, h1, w0, w1, components, ..] = data else {
                    return None;
                };
                if !matches!(components, 1 | 3 | 4) || data.len() != 6 + 3 * usize::from(components)
                {
                    return None;
                }
                dimensions = Some((
                    u32::from(u16::from_be_bytes([w0, w1])),
                    u32::from(u16::from_be_bytes([h0, h1])),
                ));
            }
            0xda if data.len() >= 6 && !markers.is_empty() => return dimensions,
            0xc4 | 0xdb | 0xdd | 0xe0..=0xef | 0xfe => {}
            _ => return None,
        }
    }
}

fn webp_dimensions(body: &[u8]) -> Option<(u32, u32)> {
    let riff = body.strip_prefix(b"RIFF")?;
    let size = u32::from_le_bytes(riff.get(..4)?.try_into().ok()?) as usize;
    if size.checked_add(8)? != body.len() {
        return None;
    }
    let mut chunks = riff.get(4..)?.strip_prefix(b"WEBP")?;
    let mut first_chunk = true;
    let mut canvas = None;
    let mut dimensions = None;
    while !chunks.is_empty() {
        let kind = chunks.get(..4)?;
        let length = u32::from_le_bytes(chunks.get(4..8)?.try_into().ok()?) as usize;
        let end = length.checked_add(8)?;
        let data = chunks.get(8..end)?;
        match kind {
            b"VP8X" if first_chunk => {
                let &[flags, 0, 0, 0, w0, w1, w2, h0, h1, h2] = data else {
                    return None;
                };
                // The animation flag and reserved flag bits must be clear.
                if flags & 0xc3 != 0 {
                    return None;
                }
                canvas = Some((
                    u32::from_le_bytes([w0, w1, w2, 0]) + 1,
                    u32::from_le_bytes([h0, h1, h2, 0]) + 1,
                ));
            }
            b"VP8 " | b"VP8L" if dimensions.is_none() => {
                dimensions = Some(webp_frame_dimensions(kind, data)?);
            }
            b"ALPH" | b"ICCP" | b"EXIF" | b"XMP " if canvas.is_some() => {}
            _ => return None,
        }
        chunks = chunks.get(end.checked_add(length % 2)?..)?;
        first_chunk = false;
    }
    dimensions.filter(|&image| canvas.is_none_or(|value| value == image))
}

fn webp_frame_dimensions(kind: &[u8], data: &[u8]) -> Option<(u32, u32)> {
    match (kind, data) {
        (b"VP8 ", &[tag, _, _, 0x9d, 0x01, 0x2a, w0, w1, h0, h1, _, ..]) if tag & 1 == 0 => Some((
            u32::from(u16::from_le_bytes([w0, w1]) & 0x3fff),
            u32::from(u16::from_le_bytes([h0, h1]) & 0x3fff),
        )),
        (b"VP8L", &[0x2f, b0, b1, b2, b3, _, ..]) => {
            let bits = u32::from_le_bytes([b0, b1, b2, b3]);
            (bits >> 29 == 0).then_some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "game_art_tests.rs"]
mod tests;
