//! LiteCut managed storage accounting and proxy-resolution settings.

use super::{assets::needs_proxy, model, model_body, read_table};
use crate::{
    native_http::{
        demo_body::field_issue, detail, json_reply, profile_disk, validation_reply, HttpResponse,
        ProfileState,
    },
    profile_platform::{expand_user, path_text, resolved_path},
    wire_json::integer_from_text,
};
use hyper::{body::Incoming, Request, StatusCode};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_CACHE_ROWS: i64 = 1000;
const MAX_SCAN_ENTRIES: usize = 100_000;
const MAX_SCAN_DEPTH: usize = 64;
const SCAN_DURATION: Duration = Duration::from_secs(5);
const SCAN_LIMIT: &str = "素材目录扫描达到上限，请检查素材目录范围后重试";
const READ_FAILED: &str = "无法读取剪辑素材存储信息";

pub(super) async fn storage(profile: &Arc<ProfileState>) -> HttpResponse {
    profile_disk(profile, |profile| {
        let result = (|| {
            let root = managed_root(&profile.config.lite_cut_assets_dir()?)?;
            let default = resolved_path(&profile.config.data_dir().join("lite_cut_assets"))?;
            let size = storage_size(&root, MAX_SCAN_ENTRIES, Instant::now() + SCAN_DURATION)?;
            Ok::<_, io::Error>(json!({
                "path": path_text(&root),
                "default_path": path_text(&default),
                "custom": key(&root) != key(&default),
                "size_bytes": size,
            }))
        })();
        read_response(result)
    })
    .await
}

pub(super) async fn proxy_cache(profile: &Arc<ProfileState>) -> HttpResponse {
    profile_disk(profile, |profile| {
        let result = (|| {
            let root = managed_root(&profile.config.lite_cut_assets_dir()?)?;
            let resolution = profile.config.lite_cut_proxy_resolution()?;
            let rows = read_table(&profile.config.database_path(), "lite_cut_assets", |db| {
                let mut query = db.prepare(
                    "SELECT file_path FROM lite_cut_assets ORDER BY created_at DESC, id DESC LIMIT ?1",
                )?;
                let paths = query.query_map([MAX_CACHE_ROWS], |row| row.get::<_, String>(0))?;
                paths.collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|error| io::Error::other(error.to_string()))?
            .unwrap_or_default();
            let totals = cache_totals(&root, &rows, Instant::now() + SCAN_DURATION)?;
            Ok(json!({
                "proxy_bytes": totals.bytes,
                "proxy_files": totals.files,
                "ready_assets": totals.ready,
                "asset_count": rows.len(),
                "proxy_required_assets": totals.required,
                // A filename alone can't establish that an unmatched file is disposable.
                "orphan_bytes": 0,
                "orphan_files": 0,
                "resolution": resolution,
            }))
        })();
        read_response(result)
    })
    .await
}

fn read_response(result: io::Result<Value>) -> HttpResponse {
    match result {
        Ok(value) => json_reply(StatusCode::OK, value),
        Err(error) if error.kind() == io::ErrorKind::TimedOut => {
            detail(StatusCode::SERVICE_UNAVAILABLE, SCAN_LIMIT)
        }
        Err(_) => detail(StatusCode::INTERNAL_SERVER_ERROR, READ_FAILED),
    }
}

pub(super) async fn patch_proxy_settings(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
) -> HttpResponse {
    let body = match model_body(request, "LiteCut proxy settings are too large").await {
        Ok(body) => body,
        Err(error) => return *error,
    };
    let resolution = match settings_resolution(&body) {
        Ok(resolution) => resolution,
        Err(issues) => return validation_reply(issues),
    };
    profile_disk(profile, move |profile| {
        match profile.config.set_lite_cut_proxy_resolution(resolution) {
            Ok(()) => json_reply(StatusCode::OK, json!({"resolution": resolution})),
            Err(_) => detail(StatusCode::INTERNAL_SERVER_ERROR, "无法保存素材代理设置"),
        }
    })
    .await
}

fn settings_resolution(body: &Value) -> Result<u64, Vec<Value>> {
    let object = model(body)?;
    let input = object.get("resolution").ok_or_else(|| {
        vec![field_issue("resolution", body, "missing", "Field required", None)]
    })?;
    let issue = |kind, message, context| vec![field_issue("resolution", input, kind, message, context)];
    let number = match input {
        Value::Bool(value) => f64::from(u8::from(*value)),
        Value::String(value) => integer_from_text(value)
            .and_then(|number| number.as_f64())
            .ok_or_else(|| issue(
                "int_parsing",
                "Input should be a valid integer, unable to parse string as an integer",
                None,
            ))?,
        Value::Number(number) => {
            let value = number.as_f64().unwrap_or(f64::NAN);
            if value.fract() != 0.0 {
                return Err(issue(
                    "int_from_float",
                    "Input should be a valid integer, got a number with a fractional part",
                    None,
                ));
            }
            value
        }
        _ => return Err(issue("int_type", "Input should be a valid integer", None)),
    };
    if number < 360.0 {
        return Err(issue(
            "greater_than_equal",
            "Input should be greater than or equal to 360",
            Some(json!({"ge":360})),
        ));
    }
    if number > 2160.0 {
        return Err(issue(
            "less_than_equal",
            "Input should be less than or equal to 2160",
            Some(json!({"le":2160})),
        ));
    }
    // Match Python's ties-to-even round(resolution / 2) * 2.
    Ok(((number / 2.0).round_ties_even() * 2.0) as u64)
}

fn managed_root(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if redirect(&metadata) || !metadata.is_dir() => {
            Err(io::Error::other("invalid managed asset directory"))
        }
        Ok(_) => path.canonicalize(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => resolved_path(path),
        Err(error) => Err(error),
    }
}

fn redirect(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn check_budget(deadline: Instant, exhausted: bool) -> io::Result<()> {
    if exhausted || Instant::now() >= deadline {
        return Err(io::Error::new(io::ErrorKind::TimedOut, SCAN_LIMIT));
    }
    Ok(())
}

fn storage_size(root: &Path, max_entries: usize, deadline: Instant) -> io::Result<u64> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut directories = vec![entries];
    let mut visited = 0;
    let mut bytes = 0_u64;
    while let Some(entries) = directories.last_mut() {
        let Some(entry) = entries.next().transpose()? else {
            directories.pop();
            continue;
        };
        check_budget(deadline, visited == max_entries)?;
        visited += 1;
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if redirect(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            check_budget(deadline, directories.len() >= MAX_SCAN_DEPTH)?;
            // Revalidate a directory that might have been replaced during the scan.
            match path.canonicalize() {
                Ok(canonical) if key(&canonical) == key(&path) => (),
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
            match fs::read_dir(&path) {
                Ok(children) => directories.push(children),
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => return Err(error),
            }
        } else if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    check_budget(deadline, false)?;
    Ok(bytes)
}

fn key(path: &Path) -> String {
    let value = path_text(path);
    if cfg!(windows) { value.to_lowercase() } else { value }
}

fn managed_relative(root: &Path, raw: &str) -> Option<PathBuf> {
    if raw.trim().is_empty() {
        return None;
    }
    let path = expand_user(raw).ok()?;
    let path = PathBuf::from(path_text(&path));
    let root = PathBuf::from(path_text(root));
    let mut parts = path.components();
    for prefix in root.components() {
        if key(Path::new(prefix.as_os_str())) != key(Path::new(parts.next()?.as_os_str())) {
            return None;
        }
    }
    let relative = parts.as_path();
    if relative.as_os_str().is_empty()
        || relative.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    Some(relative.to_owned())
}

fn regular_source(root: &Path, relative: &Path, deadline: Instant) -> io::Result<Option<PathBuf>> {
    let mut target = root.to_owned();
    if relative.components().count() > MAX_SCAN_DEPTH {
        return Ok(None);
    }
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        check_budget(deadline, false)?;
        target.push(part.as_os_str());
        let Ok(metadata) = fs::symlink_metadata(&target) else { return Ok(None) };
        if redirect(&metadata) {
            return Ok(None);
        }
        if parts.peek().is_some() {
            if !metadata.is_dir() {
                return Ok(None);
            }
        } else if metadata.is_file() {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CacheTotals {
    bytes: u64,
    files: usize,
    ready: usize,
    required: usize,
}

fn cache_totals(root: &Path, rows: &[String], deadline: Instant) -> io::Result<CacheTotals> {
    let sources: Vec<_> = rows.iter().filter_map(|raw| managed_relative(root, raw)).collect();
    // A source registered under a sidecar-like name still belongs to the project.
    let originals: HashSet<_> = sources.iter().map(|path| key(&root.join(path))).collect();
    let mut accounted = HashSet::new();
    let mut totals = CacheTotals::default();
    for path in sources.into_iter().filter(|path| needs_proxy(path)) {
        check_budget(deadline, false)?;
        totals.required += 1;
        let Some(source) = regular_source(root, &path, deadline)? else { continue };
        let mut ready = false;
        for kind in ["preview60.mp4", "preview-alpha-v3.webm"] {
            let candidate = source.with_extension(kind);
            let candidate_key = key(&candidate);
            if originals.contains(&candidate_key) {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&candidate) else { continue };
            if !metadata.is_file() || redirect(&metadata) || metadata.len() == 0 {
                continue;
            }
            ready = true;
            if accounted.insert(candidate_key) {
                totals.bytes = totals.bytes.saturating_add(metadata.len());
                totals.files += 1;
            }
        }
        totals.ready += usize::from(ready);
    }
    check_budget(deadline, false)?;
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_legacy_proxy_resolution_and_rounds_even_with_python_ties() {
        for (input, expected) in [
            (json!(540), 540),
            (json!(721), 720),
            (json!(723), 724),
            (json!("1080"), 1080),
            (json!(2160.0), 2160),
        ] {
            assert_eq!(settings_resolution(&json!({"resolution":input})).unwrap(), expected);
        }
        for (input, expected) in [
            (json!(359), "greater_than_equal"),
            (json!(2161), "less_than_equal"),
            (json!(720.5), "int_from_float"),
            (json!("auto"), "int_parsing"),
            (json!(null), "int_type"),
        ] {
            let issues = settings_resolution(&json!({"resolution":input})).unwrap_err();
            assert_eq!(issues[0]["type"], expected);
            assert_eq!(issues[0]["loc"], json!(["body","resolution"]));
        }
        assert_eq!(settings_resolution(&json!({})).unwrap_err()[0]["type"], "missing");
    }

    #[test]
    fn dedicated_setting_preserves_other_profile_fields_and_generic_put_scope() {
        use crate::profile_config::ProfileConfig;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.json");
        let original = json!({
            "lite_cut_proxy_resolution": 720,
            "lite_cut_assets_dir": path_text(directory.path()),
            "future_extension": {"value":3},
        });
        fs::write(&path, original.to_string()).unwrap();
        let config = ProfileConfig::new(directory.path().to_owned(), Some(path.clone()), None, None).unwrap();
        config.set_lite_cut_proxy_resolution(1080).unwrap();
        assert_eq!(config.lite_cut_proxy_resolution().unwrap(), 1080);
        let persisted: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["lite_cut_assets_dir"], original["lite_cut_assets_dir"]);
        assert_eq!(persisted["future_extension"], original["future_extension"]);
        assert!(config.update(json!({"lite_cut_proxy_resolution":540})).is_ok());
        assert_eq!(config.lite_cut_proxy_resolution().unwrap(), 1080);
    }

    #[test]
    fn storage_counts_regular_files_and_reports_budget_limit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let sub = root.join("素材");
        fs::create_dir(&sub).unwrap();
        fs::write(root.join("clip.mov"), b"abc").unwrap();
        fs::write(sub.join("clip.preview60.mp4"), b"12345").unwrap();
        assert_eq!(storage_size(&root, 8, Instant::now() + SCAN_DURATION).unwrap(), 8);
        assert_eq!(
            storage_size(&root, 2, Instant::now() + SCAN_DURATION).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        let missing = managed_root(&root.join("missing")).unwrap();
        assert_eq!(storage_size(&missing, 8, Instant::now() + SCAN_DURATION).unwrap(), 0);
        assert!(!missing.exists());
    }

    #[test]
    fn cache_counts_only_registered_sources_and_deduplicates_owned_proxy_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().canonicalize().unwrap();
        let root = fixture.join("assets");
        fs::create_dir(&root).unwrap();
        let original = root.join("原片.mkv");
        let proxy = original.with_extension("preview60.mp4");
        let registered_sidecar = original.with_extension("preview-alpha-v3.webm");
        let unknown = root.join("unrelated.preview60.mp4");
        let external = fixture.join("external.avi");
        let external_proxy = external.with_extension("preview60.mp4");
        let direct = root.join("music.wav");
        let direct_preview = direct.with_extension("preview60.mp4");
        for (path, content) in [
            (&original, b"source".as_slice()),
            (&proxy, b"counted"),
            (&registered_sidecar, b"also an original"),
            (&unknown, b"unknown"),
            (&external, b"external"),
            (&external_proxy, b"outside"),
            (&direct, b"music"),
            (&direct_preview, b"never a proxy"),
        ] {
            fs::write(path, content).unwrap();
        }
        let missing = root.join("missing.mov");
        let rows = [&original, &original, &registered_sidecar, &external, &direct, &missing]
            .into_iter().map(|path| path_text(path)).collect::<Vec<_>>();
        assert_eq!(
            cache_totals(&root, &rows, Instant::now() + SCAN_DURATION).unwrap(),
            CacheTotals { bytes: 7, files: 1, ready: 2, required: 3 }
        );
        assert_eq!(fs::read_to_string(unknown).unwrap(), "unknown");
        assert_eq!(fs::read_to_string(registered_sidecar).unwrap(), "also an original");
    }

    #[test]
    #[cfg_attr(windows, ignore = "requires permission to create a directory symlink")]
    fn storage_and_cache_never_walk_a_redirected_child() {
        let temp = tempfile::tempdir().unwrap();
        let temp = temp.path().canonicalize().unwrap();
        let root = temp.join("owned");
        let unrelated = temp.join("other");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("external.mov"), b"external").unwrap();
        fs::write(unrelated.join("external.preview60.mp4"), b"do not count").unwrap();
        let alias = root.join("alias");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&unrelated, &alias).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&unrelated, &alias).unwrap();
        assert_eq!(storage_size(&root, 5, Instant::now() + SCAN_DURATION).unwrap(), 0);
        let rows = [path_text(&alias.join("external.mov"))];
        assert_eq!(
            cache_totals(&root, &rows, Instant::now() + SCAN_DURATION).unwrap(),
            CacheTotals { required: 1, ..CacheTotals::default() }
        );
    }
}
