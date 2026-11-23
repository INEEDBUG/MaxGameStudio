//! Stage a single browser FormData file before publishing its LiteCut asset row.

use super::{
    asset_json, asset_root, asset_select, decorate_proxy, metadata::image_dimensions, read_table,
    TABLE,
};
use crate::{
    native_http::{
        detail, json_reply,
        lite_cut::{
            ffmpeg::{find_ffprobe, inspect, video_has_alpha},
            project_store,
        },
        profile_work, validation_reply, HttpResponse, ProfileState,
    },
    profile_platform::{expand_user, path_text},
    wire_json::integer_from_text,
};
use futures_util::{Stream, StreamExt};
use http_body_util::BodyExt;
use hyper::{
    body::{Bytes, Incoming},
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    Request, StatusCode,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use s3s_multipart::{Boundary, Error as MultipartError, FinalPartDataStream, Multipart};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tempfile::{NamedTempFile, TempPath};
use tokio::{io::AsyncWriteExt, time::timeout};

const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const MAX_PART_HEADERS: usize = 16 * 1024;
const MAX_BODY_BYTES: u64 = MAX_FILE_BYTES + 2 * MAX_PART_HEADERS as u64;
const UPLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const STAGING_PREFIX: &str = ".litecut-upload-";

#[derive(Debug)]
enum Problem {
    Bad(&'static str),
    Unsupported(String),
    MissingFile,
    Validation(Vec<Value>),
    ProjectMissing,
    Save,
    Timeout,
}

impl Problem {
    fn response(self) -> HttpResponse {
        match self {
            Self::Bad(message) => detail(StatusCode::BAD_REQUEST, message),
            Self::Unsupported(ext) => detail(
                StatusCode::BAD_REQUEST,
                &format!("unsupported file type: {ext}"),
            ),
            Self::MissingFile => validation_reply(vec![json!({
                "type":"missing","loc":["body","file"],"msg":"Field required","input":null,
            })]),
            Self::Validation(issues) => validation_reply(issues),
            Self::ProjectMissing => json_reply(
                StatusCode::NOT_FOUND,
                json!({"detail":{"code":"LITECUT_PROJECT_NOT_FOUND"}}),
            ),
            Self::Save => json_reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"detail":{"code":"LITECUT_ASSET_SAVE_FAILED"}}),
            ),
            Self::Timeout => detail(StatusCode::REQUEST_TIMEOUT, "asset upload timed out"),
        }
    }
}

#[derive(Debug)]
struct BodyLimit;

impl std::fmt::Display for BodyLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("asset upload exceeds the request limit")
    }
}

impl std::error::Error for BodyLimit {}

fn multipart_problem(error: MultipartError) -> Problem {
    if matches!(&error, MultipartError::StreamReadFailed(error) if error.is::<BodyLimit>()) {
        Problem::Bad("file too large (max 20GB)")
    } else {
        Problem::Bad("invalid multipart upload")
    }
}

#[derive(Clone, Copy)]
struct Query {
    project: Option<i64>,
    duration: Option<f64>,
}

impl Query {
    fn parse(raw: &str) -> Result<Self, Problem> {
        let fields: HashMap<_, _> = form_urlencoded::parse(raw.as_bytes()).collect();
        let mut issues = Vec::new();
        let project = fields.get("project_id").and_then(|raw| {
            let parsed = integer_from_text(raw).and_then(|number| number.as_i64());
            if parsed.is_none() {
                issues.push(json!({
                    "type":"int_parsing","loc":["query","project_id"],
                    "msg":"Input should be a valid integer, unable to parse string as an integer","input":raw,
                }));
            }
            parsed
        });
        let duration = fields.get("client_duration_sec").and_then(|raw| {
            match raw.trim().replace('_', "").parse::<f64>() {
                Ok(value) if value.is_finite() && value >= 0.0 => Some(value),
                Ok(value) if value.is_finite() => {
                    issues.push(json!({
                        "type":"greater_than_equal","loc":["query","client_duration_sec"],
                        "msg":"Input should be greater than or equal to 0","input":raw,"ctx":{"ge":0.0},
                    }));
                    None
                }
                _ => {
                    issues.push(json!({
                        "type":"float_parsing","loc":["query","client_duration_sec"],
                        "msg":"Input should be a valid number, unable to parse string as a number","input":raw,
                    }));
                    None
                }
            }
        });
        if issues.is_empty() {
            Ok(Self { project, duration })
        } else {
            Err(Problem::Validation(issues))
        }
    }
}

struct Asset {
    name: String,
    stem: String,
    ext: String,
    kind: &'static str,
    mime: Option<String>,
}

impl Asset {
    fn from_part(name: &str, mime: Option<String>) -> Result<Self, Problem> {
        let name = name.rsplit(['/', '\\']).next().unwrap_or_default();
        if name.is_empty() || name == "." || name.contains("..") || name.chars().any(char::is_control) {
            return Err(Problem::Bad("invalid filename"));
        }
        let filename = Path::new(name);
        let ext = filename
            .extension()
            .and_then(|suffix| suffix.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let audio_mime = mime
            .as_deref()
            .and_then(|value| value.get(..6))
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("audio/"));
        let kind = match ext.as_str() {
            "webm" if audio_mime => "audio",
            "webm" => "webm",
            "mp4" | "mov" | "m4v" | "mkv" | "avi" | "gif" => "video",
            "mp3" | "wav" | "m4a" | "aac" | "ogg" | "flac" => "audio",
            "png" | "jpg" | "jpeg" | "webp" => "image",
            "woff" | "woff2" | "ttf" | "otf" => "font",
            "" => return Err(Problem::Unsupported("(none)".to_owned())),
            _ => return Err(Problem::Unsupported(format!(".{ext}"))),
        };
        let stem = replace_runs(
            filename.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default(),
            |ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')),
        );
        Ok(Self {
            name: name.to_owned(),
            stem: if stem.is_empty() { "asset".to_owned() } else { stem.chars().take(80).collect() },
            ext,
            kind,
            mime,
        })
    }

    fn destination(&self, temp: &Path) -> Result<PathBuf, Problem> {
        let random = temp.file_stem()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(STAGING_PREFIX))
            .ok_or(Problem::Save)?;
        Ok(temp.with_file_name(format!("{}_{random}.{}", self.stem, self.ext)))
    }
}

fn replace_runs(value: &str, disallowed: impl Fn(char) -> bool) -> String {
    let mut result = String::new();
    let mut replaced = false;
    for ch in value.chars() {
        if disallowed(ch) {
            if !replaced {
                result.push('_');
            }
            replaced = true;
        } else {
            replaced = false;
            result.push(ch);
        }
    }
    result
}

fn boundary(request: &Request<Incoming>) -> Result<Boundary, Problem> {
    if request.headers().get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|bytes| bytes > MAX_BODY_BYTES)
    {
        return Err(Problem::Bad("file too large (max 20GB)"));
    }
    let mime: mime::Mime = request.headers().get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or(Problem::Bad("invalid multipart upload"))?;
    if mime.essence_str() != "multipart/form-data" {
        return Err(Problem::Bad("invalid multipart upload"));
    }
    let boundary = mime.get_param("boundary").ok_or(Problem::Bad("missing multipart boundary"))?;
    Boundary::new(boundary.as_str().as_bytes()).map_err(multipart_problem)
}

type BodyChunk = Result<Bytes, MultipartError>;

async fn file_part<S>(
    body: S,
    boundary: &Boundary,
) -> Result<(Asset, FinalPartDataStream<S>), Problem>
where
    S: Stream<Item = BodyChunk> + Send + Unpin,
{
    let mut form = Multipart::new(body, boundary, MAX_PART_HEADERS);
    let mut part = form.next_part().await.map_err(multipart_problem)?.ok_or(Problem::MissingFile)?;
    let mut filename = None;
    let mut mime = None;
    while let Some(header) = part.next_header().await.map_err(multipart_problem)? {
        if header.name.eq_ignore_ascii_case("content-disposition") {
            if filename.is_some() {
                return Err(Problem::Bad("invalid multipart upload"));
            }
            let disposition = s3s_multipart::parse_content_disposition(header.value)
                .filter(|part| part.name == Some(b"file"))
                .ok_or(Problem::MissingFile)?;
            filename = Some(
                disposition.file_name.and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .ok_or(Problem::Bad("invalid filename"))?.to_owned(),
            );
        } else if header.name.eq_ignore_ascii_case("content-type") {
            if mime.is_some() {
                return Err(Problem::Bad("invalid multipart upload"));
            }
            mime = std::str::from_utf8(header.value).ok()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
        }
    }
    let asset = Asset::from_part(&filename.ok_or(Problem::MissingFile)?, mime)?;
    // The editor sends query metadata and exactly one file part. A partial
    // request, extra part, or malformed closing delimiter never publishes it.
    let stream = part.take_data_stream().map_err(multipart_problem)?.into_final();
    Ok((asset, stream))
}

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let query = match Query::parse(request.uri().query().unwrap_or_default()) {
        Ok(query) => query,
        Err(problem) => return problem.response(),
    };
    let boundary = match boundary(&request) {
        Ok(boundary) => boundary,
        Err(problem) => return problem.response(),
    };
    extended_request.store(true, Ordering::Relaxed);
    match upload(profile, request.into_body(), boundary, query).await {
        Ok(mut row) => {
            super::queue_uploaded_proxy(profile, &mut row).await;
            json_reply(StatusCode::OK, row)
        }
        Err(problem) => problem.response(),
    }
}

async fn upload(
    profile: &Arc<ProfileState>,
    body: Incoming,
    boundary: Boundary,
    query: Query,
) -> Result<Value, Problem> {
    let mut received = 0;
    let body = body.into_data_stream().map(move |part| {
        let chunk = part.map_err(MultipartError::stream_read_failed)?;
        received += chunk.len() as u64;
        if received > MAX_BODY_BYTES {
            return Err(MultipartError::stream_read_failed(BodyLimit));
        }
        Ok(chunk)
    });
    let (asset, stream) = timeout(UPLOAD_IDLE_TIMEOUT, file_part(body, &boundary))
        .await
        .map_err(|_| Problem::Timeout)??;
    let prepared = profile_work(profile, move |profile| prepare(profile, query.project))
        .await.map_err(|_| Problem::Save)??;
    let (file, temp) = prepared.temp.into_parts();
    receive_file(stream, tokio::fs::File::from_std(file), MAX_FILE_BYTES).await?;

    let info = asset_info(profile, &temp, asset.kind, query.duration, prepared.ffprobe.as_deref()).await;
    profile_work(profile, move |profile| {
        let root = asset_root(profile).map_err(|_| Problem::Save)?;
        if root != prepared.root {
            return Err(Problem::Save);
        }
        publish(&profile.config.database_path(), &root, query.project, asset, temp, info)
    })
    .await.map_err(|_| Problem::Save)?
}

async fn receive_file(
    mut stream: impl Stream<Item = BodyChunk> + Unpin,
    mut out: tokio::fs::File,
    max: u64,
) -> Result<(), Problem> {
    let mut written = 0;
    while let Some(chunk) = timeout(UPLOAD_IDLE_TIMEOUT, stream.next())
        .await
        .map_err(|_| Problem::Timeout)?
    {
        let chunk = chunk.map_err(multipart_problem)?;
        written += chunk.len() as u64;
        if written > max {
            return Err(Problem::Bad("file too large (max 20GB)"));
        }
        out.write_all(&chunk).await.map_err(|_| Problem::Save)?;
    }
    if written == 0 {
        return Err(Problem::Bad("empty file"));
    }
    out.flush().await.map_err(|_| Problem::Save)?;
    out.sync_all().await.map_err(|_| Problem::Save)
}

struct Prepared {
    root: PathBuf,
    temp: NamedTempFile,
    ffprobe: Option<PathBuf>,
}

fn prepare(profile: &ProfileState, project: Option<i64>) -> Result<Prepared, Problem> {
    let database = profile.config.database_path();
    let name = project.map(|id| {
        read_table(&database, "lite_cut_projects", |db| project_name(db, id))
            .map_err(|_| Problem::Save)?.flatten().ok_or(Problem::ProjectMissing)
    }).transpose()?;
    let root = asset_root(profile).map_err(|_| Problem::Save)?;
    fs::create_dir_all(&root).map_err(|_| Problem::Save)?;
    let root = root.canonicalize().map_err(|_| Problem::Save)?;
    let directory = match project.zip(name) {
        Some((id, name)) => project_directory(&database, &root, id, &name)?,
        None => root.clone(),
    };
    let temp = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .suffix(".part")
        .rand_bytes(10)
        .tempfile_in(directory)
        .map_err(|_| Problem::Save)?;
    let ffprobe = profile.config.ffmpeg_executable().ok().and_then(|ffmpeg| find_ffprobe(&ffmpeg));
    Ok(Prepared { root, temp, ffprobe })
}

fn project_name(db: &Connection, id: i64) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT name FROM lite_cut_projects WHERE id=?1", [id], |row| row.get(0)).optional()
}

fn project_directory(database: &Path, root: &Path, id: i64, name: &str) -> Result<PathBuf, Problem> {
    let existing = read_table(database, TABLE, |db| {
        let mut query = db.prepare(
            "SELECT file_path FROM lite_cut_assets WHERE project_id=?1 ORDER BY created_at DESC, id DESC",
        )?;
        let mut rows = query.query([id])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(0)?;
            if let Some(parent) = expand_user(&raw).ok()
                .and_then(|path| path.parent()?.canonicalize().ok())
                .filter(|parent| parent != root && parent.starts_with(root) && parent.is_dir())
            {
                return Ok(Some(parent));
            }
        }
        Ok(None)
    })
    .map_err(|_| Problem::Save)?.flatten();
    if let Some(existing) = existing {
        return Ok(existing);
    }

    // Existing folders retain their original name after project renames.
    let prefix = format!("{id}_");
    let existing = fs::read_dir(root).map_err(|_| Problem::Save)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|entry| entry.path().canonicalize().ok())
        .filter(|path| path != root && path.starts_with(root) && path.is_dir())
        .min();
    if let Some(path) = existing {
        return Ok(path);
    }
    let name = replace_runs(name.trim(), |ch| ch < '\u{20}' || "<>:\"/\\|?*".contains(ch));
    let name = name.trim_end_matches([' ', '.']).chars().take(120).collect::<String>();
    let name = if name.is_empty() {
        "未命名工程".to_owned()
    } else if windows_device_name(&name) {
        format!("_{name}")
    } else {
        name
    };
    let path = root.join(format!("{prefix}{name}"));
    fs::create_dir_all(&path).map_err(|_| Problem::Save)?;
    path.canonicalize().ok()
        .filter(|path| path != root && path.starts_with(root) && path.is_dir())
        .ok_or(Problem::Save)
}

fn windows_device_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    matches!(name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (name.len() == 4
            && (name.starts_with("COM") || name.starts_with("LPT"))
            && matches!(name.as_bytes()[3], b'1'..=b'9'))
}

#[derive(Default)]
struct AssetInfo {
    duration: Option<f64>,
    width: Option<u64>,
    height: Option<u64>,
    alpha: bool,
}

async fn asset_info(
    profile: &Arc<ProfileState>,
    path: &Path,
    kind: &str,
    client_duration: Option<f64>,
    ffprobe: Option<&Path>,
) -> AssetInfo {
    let mut result = AssetInfo {
        duration: client_duration.filter(|value| *value > 0.0),
        ..AssetInfo::default()
    };
    if kind == "image" {
        let path = path.to_owned();
        if let Ok(Ok(Some((width, height)))) =
            profile_work(profile, move |_| image_dimensions(&path)).await
        {
            result.width = Some(u64::from(width));
            result.height = Some(u64::from(height));
        }
    }
    if kind == "font" {
        return result;
    }
    if let Some(ffprobe) = ffprobe {
        let _permit = profile.media_process.acquire().await.expect("profile media semaphore");
        if let Ok(probe) = inspect(ffprobe, path, None).await {
            apply_probe(&mut result, &probe, kind);
        }
    }
    result
}

fn apply_probe(result: &mut AssetInfo, info: &Value, kind: &str) {
    if let Some(duration) = info["format"]["duration"].as_str().and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        result.duration = Some(duration);
    }
    if let Some(video) = info["streams"].as_array().and_then(|streams| {
        streams.iter().find(|stream| stream["codec_type"] == "video")
    }) {
        if kind != "audio" {
            result.width = video["width"].as_u64().filter(|value| *value > 0).or(result.width);
            result.height = video["height"].as_u64().filter(|value| *value > 0).or(result.height);
        }
        result.alpha = video_has_alpha(video);
    }
}

fn publish(
    database: &Path,
    root: &Path,
    project: Option<i64>,
    asset: Asset,
    temp: TempPath,
    info: AssetInfo,
) -> Result<Value, Problem> {
    let staged: &Path = temp.as_ref();
    let directory = staged.parent().ok_or(Problem::Save)?;
    if !directory.starts_with(root) || directory.canonicalize().map_err(|_| Problem::Save)? != directory {
        return Err(Problem::Save);
    }
    let path = asset.destination(staged)?;
    if let Some(parent) = database.parent() {
        fs::create_dir_all(parent).map_err(|_| Problem::Save)?;
    }
    let mut db = Connection::open(database).map_err(|_| Problem::Save)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(|_| Problem::Save)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_| Problem::Save)?;
    ensure_assets(&tx).map_err(|_| Problem::Save)?;
    if let Some(id) = project {
        if project_name(&tx, id).map_err(|_| Problem::Save)?.is_none() {
            return Err(Problem::ProjectMissing);
        }
    }
    let now = project_store::timestamp(&tx).map_err(|_| Problem::Save)?;
    tx.execute(
        "INSERT INTO lite_cut_assets
         (project_id, name, kind, mime_type, file_path, duration_sec, width, height, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            project, asset.name, asset.kind, asset.mime, path_text(&path),
            info.duration, info.width, info.height, now,
        ],
    ).map_err(|_| Problem::Save)?;
    let select = asset_select(&tx).map_err(|_| Problem::Save)?;
    let mut row = tx.query_row(
        &format!("{select} WHERE id=?1"), [tx.last_insert_rowid()], asset_json,
    ).map_err(|_| Problem::Save)?;

    // Publish only the completed file. If SQLite cannot commit, remove exactly
    // the name created by this request; other files, even duplicates, are untouched.
    temp.persist_noclobber(&path).map_err(|_| Problem::Save)?;
    if tx.commit().is_err() {
        if let Err(error) = fs::remove_file(&path) {
            eprintln!("Could not remove a failed LiteCut upload: {:?}", error.kind());
        }
        return Err(Problem::Save);
    }
    decorate_proxy(&mut row, root);
    if info.alpha {
        row["has_alpha"] = json!(true);
    }
    Ok(row)
}

fn ensure_assets(db: &Connection) -> rusqlite::Result<()> {
    project_store::ensure_projects(db)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS lite_cut_assets(
            id INTEGER PRIMARY KEY AUTOINCREMENT, project_id INTEGER,
            name TEXT NOT NULL, kind TEXT NOT NULL, mime_type TEXT,
            file_path TEXT NOT NULL, duration_sec REAL, width INTEGER, height INTEGER,
            created_at TEXT NOT NULL, FOREIGN KEY(project_id) REFERENCES lite_cut_projects(id)
         );
         CREATE INDEX IF NOT EXISTS idx_lite_cut_assets_project ON lite_cut_assets(project_id);",
    )?;
    let (width, height): (bool, bool) = db.query_row(
        "SELECT coalesce(max(lower(name)='width'),0), coalesce(max(lower(name)='height'),0)
         FROM pragma_table_info('lite_cut_assets')",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !width {
        db.execute("ALTER TABLE lite_cut_assets ADD COLUMN width INTEGER", [])?;
    }
    if !height {
        db.execute("ALTER TABLE lite_cut_assets ADD COLUMN height INTEGER", [])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    fn multipart_body(delimiter: &str) -> Vec<BodyChunk> {
        format!(
            "--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"录制.WEBM\"\r\n\
             Content-Type: audio/webm\r\n\r\nabc\r\n--b{delimiter}"
        )
        .as_bytes()
        .chunks(5)
        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
        .collect()
    }

    #[test]
    fn parses_fragmented_browser_upload_and_rejects_truncation_and_an_extra_part() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let boundary = Boundary::new(b"b").unwrap();
            for delimiter in [
                "--\r\n",
                "--",
                "\r\nContent-Disposition: form-data; name=\"extra\"\r\n\r\nx\r\n--b--\r\n",
            ] {
                let (asset, mut body) = file_part(stream::iter(multipart_body(delimiter)), &boundary)
                    .await
                    .unwrap();
                assert_eq!((asset.name.as_str(), asset.kind, asset.stem.as_str()), ("录制.WEBM", "audio", "_"));
                let mut written = Vec::new();
                let mut failed = false;
                while let Some(chunk) = body.next().await {
                    match chunk {
                        Ok(chunk) => written.extend_from_slice(&chunk),
                        Err(_) => failed = true,
                    }
                }
                if delimiter == "--\r\n" {
                    assert!(!failed);
                    assert_eq!(written, b"abc");
                } else {
                    assert!(failed, "accepted invalid closing delimiter");
                }
            }
            let (_, body) = file_part(stream::iter(multipart_body("--\r\n")), &boundary)
                .await
                .unwrap();
            let output = tempfile::tempfile().unwrap();
            assert!(matches!(
                receive_file(body, tokio::fs::File::from_std(output), 2).await,
                Err(Problem::Bad("file too large (max 20GB)"))
            ));
        });
    }

    fn stage(directory: &Path, data: &[u8]) -> TempPath {
        let mut temp = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .suffix(".part")
            .tempfile_in(directory)
            .unwrap();
        std::io::Write::write_all(&mut temp, data).unwrap();
        temp.into_temp_path()
    }

    #[test]
    fn uploads_keep_legacy_project_directory_and_duplicate_names_without_overwriting() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().canonicalize().unwrap();
        let legacy = root.join("项目旧名");
        fs::create_dir(&legacy).unwrap();
        let db_path = root.join("fixture.sqlite3");
        let db = Connection::open(&db_path).unwrap();
        project_store::ensure_projects(&db).unwrap();
        db.execute_batch(
            "INSERT INTO lite_cut_projects(id,name,body_json,created_at,updated_at)
                 VALUES(3,'新项目名','{}','old','old');
             CREATE TABLE lite_cut_assets(
                 id INTEGER PRIMARY KEY, project_id INTEGER, name TEXT NOT NULL, kind TEXT NOT NULL,
                 mime_type TEXT, file_path TEXT NOT NULL, duration_sec REAL, created_at TEXT NOT NULL
             );",
        ).unwrap();
        let existing = legacy.join("old.mp4");
        fs::write(&existing, b"legacy").unwrap();
        db.execute(
            "INSERT INTO lite_cut_assets(project_id,name,kind,file_path,created_at)
             VALUES(3,'旧','video',?1,'old')",
            [path_text(&existing)],
        ).unwrap();

        let selected = project_directory(&db_path, &root, 3, "新项目名").unwrap();
        assert_eq!(selected, legacy);
        let upload = |content| {
            publish(
                &db_path, &root, Some(3),
                Asset::from_part("剪辑.MP4", Some("video/mp4".to_owned())).unwrap(),
                stage(&selected, content),
                AssetInfo { duration: Some(2.5), width: Some(96), height: Some(64), alpha: false },
            ).unwrap()
        };
        let first = upload(b"first");
        let second = upload(b"second");
        assert_eq!(first["name"], "剪辑.MP4");
        assert_eq!(first["duration_sec"], 2.5);
        assert_eq!(first["width"], 96);
        assert_eq!(first["preview_proxy_status"], "not_needed");
        assert_ne!(first["file_path"], second["file_path"]);
        assert_eq!(fs::read(first["file_path"].as_str().unwrap()).unwrap(), b"first");
        assert_eq!(fs::read(second["file_path"].as_str().unwrap()).unwrap(), b"second");
        assert_eq!(fs::read(existing).unwrap(), b"legacy");

        let asset = Asset::from_part("剪辑.MP4", None).unwrap();
        let staged = stage(&selected, b"must not replace");
        let path = asset.destination(&staged).unwrap();
        fs::write(&path, b"already there").unwrap();
        assert!(matches!(
            publish(&db_path, &root, Some(3), asset, staged, AssetInfo::default()),
            Err(Problem::Save)
        ));
        assert_eq!(fs::read(path).unwrap(), b"already there");
        assert_eq!(db.query_row("SELECT count(*) FROM lite_cut_assets", [], |row| row.get::<_, u64>(0)).unwrap(), 3);
        assert!(!fs::read_dir(legacy).unwrap().any(|entry| {
            entry.unwrap().file_name().to_string_lossy().starts_with(STAGING_PREFIX)
        }));
    }
}
