//! Recoverable removal of a managed LiteCut asset and its owned sidecars.

use super::{asset_root, load_row, stored_path, AssetMediaError};
use crate::{
    native_http::{
        demo_query::parse_path_integer, detail, json_reply, lite_cut::project_store,
        profile_work, validation_reply, HttpResponse, ProfileState,
    },
    profile_platform::{expand_user, path_text},
};
use hyper::StatusCode;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const SIDECARS: [&str; 6] = [
    "preview60.mp4",
    "preview.mp4",
    "preview-alpha-v3.webm",
    "preview-alpha-v2.webm",
    "preview.webm",
    "waveform-v1.json",
];

#[derive(Debug)]
enum Problem {
    Missing,
    Busy(&'static str),
    Storage(io::Error),
    Restore(PathBuf),
    Database,
}

impl Problem {
    fn response(self) -> HttpResponse {
        match self {
            Self::Missing => AssetMediaError::Missing.response(),
            Self::Busy(message) => detail(StatusCode::CONFLICT, message),
            Self::Storage(error) => {
                let reason = if cross_volume(&error) {
                    "Recovery could not stay on the asset volume".to_owned()
                } else {
                    error.to_string()
                };
                detail(
                    StatusCode::CONFLICT,
                    &format!("Asset files could not be moved to the recovery area: {reason}"),
                )
            }
            Self::Restore(directory) => json_reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"detail":{
                    "code":"LITECUT_ASSET_RESTORE_FAILED",
                    "recovery_directory":path_text(&directory),
                }}),
            ),
            Self::Database => detail(StatusCode::INTERNAL_SERVER_ERROR, "无法移除剪辑素材"),
        }
    }
}

impl From<io::Error> for Problem {
    fn from(error: io::Error) -> Self {
        Self::Storage(error)
    }
}

pub(super) async fn respond(profile: &Arc<ProfileState>, raw_id: &str) -> HttpResponse {
    let id = match parse_path_integer(raw_id, "asset_id") {
        Ok(id) => id,
        Err(errors) => return validation_reply(errors),
    };
    match delete(profile, id).await {
        Ok(directory) => json_reply(
            StatusCode::OK,
            json!({"deleted":true,"id":id,"recovery_directory":directory.as_deref().map(path_text)}),
        ),
        Err(problem) => problem.response(),
    }
}

async fn delete(profile: &Arc<ProfileState>, id: i64) -> Result<Option<PathBuf>, Problem> {
    let selected = profile_work(profile, move |profile| {
        let row = load_row(profile, id).map_err(|error| match error {
            AssetMediaError::Missing => Problem::Missing,
            _ => Problem::Database,
        })?;
        let raw = stored_path(&row).to_owned();
        let root = asset_root(profile).map_err(|_| Problem::Database)?;
        let source = managed_source(&root, &raw)?;
        Ok::<_, Problem>((raw, root, source))
    })
    .await
    .map_err(|_| Problem::Database)??;
    let (raw, root, source) = selected;
    let mut deleting = source.as_deref()
        .map(|path| profile.lite_cut_proxies.stop_for_deletion(profile, path))
        .transpose()
        .map_err(Problem::Busy)?;
    if let Some(deleting) = &mut deleting {
        deleting.wait().await.map_err(Problem::Busy)?;
    }

    // Waveform and export own this same permit throughout source use and
    // cache publication. Never cancel or rename files under another job.
    let media = Arc::clone(&profile.media_process)
        .try_acquire_owned()
        .map_err(|_| Problem::Busy("正在处理素材，请稍后重试"))?;
    profile_work(profile, move |profile| {
        // Once disk work starts, keep both guards until commit or restore
        // finishes, even if the HTTP client disconnects.
        let (_deleting, _media) = (deleting, media);
        let current_root = asset_root(profile).map_err(|_| Problem::Database)?;
        if current_root != root || managed_source(&root, &raw)? != source {
            return Err(Problem::Busy("素材或存储位置已变更，请重试"));
        }
        delete_owned_bundle(
            &profile.config.database_path(),
            profile.config.data_dir(),
            &root,
            id,
            &raw,
            source.as_deref(),
        )
    })
    .await
    .map_err(|_| Problem::Database)?
}

fn delete_owned_bundle(
    database: &Path,
    data_root: &Path,
    asset_root: &Path,
    id: i64,
    expected_path: &str,
    source: Option<&Path>,
) -> Result<Option<PathBuf>, Problem> {
    let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| Problem::Database)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(|_| Problem::Database)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Problem::Database)?;
    let current: String = tx.query_row(
        "SELECT file_path FROM lite_cut_assets WHERE id=?1", [id], |row| row.get(0),
    )
    .optional()
    .map_err(|_| Problem::Database)?
    .ok_or(Problem::Missing)?;
    if current != expected_path {
        return Err(Problem::Busy("素材已变更，请重试"));
    }

    let files = match source {
        Some(source) => owned_files(&tx, asset_root, source, id)?,
        None => Vec::new(),
    };
    let batch = if files.is_empty() {
        None
    } else {
        let stamp = project_store::timestamp(&tx).map_err(|_| Problem::Database)?;
        Some(quarantine(data_root, asset_root, &files, &stamp, move_noclobber)?)
    };
    let outcome = match tx.execute("DELETE FROM lite_cut_assets WHERE id=?1", [id]) {
        Ok(1) => tx.commit().map_err(|_| Problem::Database),
        Ok(_) => Err(Problem::Missing),
        Err(_) => Err(Problem::Database),
    };
    match outcome {
        Ok(()) => Ok(batch.map(|batch| batch.directory)),
        Err(problem) => {
            if let Some(batch) = batch {
                batch.restore(batch.files.len())?;
            }
            Err(problem)
        }
    }
}

// A removed original can still have legacy sidecars. Resolve its parent in
// that case, but never move a file outside the selected managed asset root.
fn managed_source(root: &Path, raw: &str) -> io::Result<Option<PathBuf>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let path = expand_user(raw)?;
    let resolved = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return Ok(None);
            };
            match parent.canonicalize() {
                Ok(parent) => parent.join(name),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            }
        }
        Err(error) => return Err(error),
    };
    Ok((resolved != root && resolved.starts_with(root)).then_some(resolved))
}

fn bundle_paths(source: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    SIDECARS.into_iter().map(|suffix| source.with_extension(suffix))
        .chain(std::iter::once_with(|| source.to_owned()))
}

fn path_key(path: &Path) -> String {
    let name = path_text(path);
    if cfg!(windows) { name.to_lowercase() } else { name }
}

fn owned_files(
    db: &Connection,
    root: &Path,
    source: &Path,
    id: i64,
) -> Result<Vec<PathBuf>, Problem> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for candidate in bundle_paths(source) {
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => {
                let canonical = candidate.canonicalize()?;
                if canonical.starts_with(root) && seen.insert(path_key(&canonical)) {
                    files.push(canonical);
                }
            }
            Ok(_) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    if files.is_empty() {
        return Ok(files);
    }

    // Old databases can contain separate asset rows for one source or
    // same-stem sources sharing a proxy. Preserve their file bundles.
    let mut query = db.prepare("SELECT file_path FROM lite_cut_assets WHERE id<>?1")
        .map_err(|_| Problem::Database)?;
    let mut rows = query.query([id]).map_err(|_| Problem::Database)?;
    while let Some(row) = rows.next().map_err(|_| Problem::Database)? {
        let raw: String = row.get(0).map_err(|_| Problem::Database)?;
        let Some(other) = managed_source(root, &raw)? else { continue };
        for path in bundle_paths(&other) {
            seen.remove(&path_key(&path));
        }
        if seen.is_empty() {
            break;
        }
    }
    files.retain(|path| seen.contains(&path_key(path)));
    Ok(files)
}

struct Entry {
    original: PathBuf,
    quarantined: PathBuf,
}

struct Batch {
    directory: PathBuf,
    files: Vec<Entry>,
}

impl Batch {
    fn new(root: &Path, paths: &[PathBuf], created_at: &str) -> io::Result<Self> {
        let namespace = recovery_namespace(root)?;
        let directory = tempfile::Builder::new().prefix("").rand_bytes(24)
            .tempdir_in(namespace)?.keep();
        let files_dir = directory.join("files");
        let mut batch = Self {
            directory,
            files: Vec::new(),
        };
        let prepared = (|| {
            fs::create_dir(&files_dir)?;
            for (index, original) in paths.iter().enumerate() {
                let filename = original.file_name().ok_or_else(|| io::Error::other("invalid asset filename"))?;
                let quarantined = files_dir.join(format!("{index:04}-{}", filename.to_string_lossy()));
                batch.files.push(Entry { original: original.clone(), quarantined });
            }
            let manifest = json!({
                "created_at": created_at,
                "namespace": "lite-cut",
                "files": batch.files.iter().map(|entry| json!({
                    "original": path_text(&entry.original),
                    "quarantined": path_text(&entry.quarantined),
                })).collect::<Vec<Value>>(),
            });
            let mut file = fs::File::options().write(true).create_new(true)
                .open(batch.directory.join("manifest.json"))?;
            serde_json::to_writer_pretty(&mut file, &manifest)?;
            file.flush()?;
            file.sync_all()
        })();
        if let Err(error) = prepared {
            batch.remove_empty();
            return Err(error);
        }
        Ok(batch)
    }

    fn restore(&self, moved: usize) -> Result<(), Problem> {
        let mut failed = false;
        for item in self.files[..moved].iter().rev() {
            if move_noclobber(&item.quarantined, &item.original).is_err() {
                failed = true;
            }
        }
        if failed {
            return Err(Problem::Restore(self.directory.clone()));
        }
        self.remove_empty();
        Ok(())
    }

    fn remove_empty(&self) {
        // A batch only owns these exact entries. A rollback never removes a
        // user-created file in the recovery area or recurses into it.
        let _ = fs::remove_file(self.directory.join("manifest.json"));
        let _ = fs::remove_dir(self.directory.join("files"));
        let _ = fs::remove_dir(&self.directory);
    }
}

fn recovery_namespace(root: &Path) -> io::Result<PathBuf> {
    let mut path = root.canonicalize()?;
    for segment in ["trash", "lite-cut"] {
        path.push(segment);
        match fs::create_dir(&path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        let ready = path.canonicalize()?;
        if path_key(&ready) != path_key(&path) || !ready.is_dir() {
            return Err(io::Error::other("recovery directory is redirected"));
        }
        path = ready;
    }
    Ok(path)
}

fn quarantine(
    data_root: &Path,
    asset_root: &Path,
    files: &[PathBuf],
    stamp: &str,
    move_file: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<Batch, Problem> {
    let mut roots = [data_root, asset_root].into_iter();
    while let Some(root) = roots.next() {
        let batch = Batch::new(root, files, stamp)?;
        let mut retry_on_asset_root = false;
        for (index, entry) in batch.files.iter().enumerate() {
            if let Err(error) = move_file(&entry.original, &entry.quarantined) {
                batch.restore(index)?;
                if index == 0 && cross_volume(&error) && roots.len() > 0 {
                    retry_on_asset_root = true;
                    break;
                }
                return Err(error.into());
            }
        }
        if !retry_on_asset_root {
            return Ok(batch);
        }
    }
    unreachable!("the asset-root attempt either succeeds or returns the error")
}

fn cross_volume(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices
        || (cfg!(windows) && error.raw_os_error() == Some(17))
}

#[cfg(windows)]
fn move_noclobber(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |path: &Path| path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let (from, to) = (wide(from), wide(to));
    // Flags 0 requests a same-volume rename that neither copies nor replaces.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn move_noclobber(from: &Path, to: &Path) -> io::Result<()> {
    // The portable fixture path still refuses cross-device moves and never
    // replaces an existing file. It links the same data instead of copying it.
    fs::hard_link(from, to)?;
    if let Err(error) = fs::remove_file(from) {
        let _ = fs::remove_file(to);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        data: PathBuf,
        assets: PathBuf,
        database: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let data = root.join("profile");
            let assets = root.join("assets");
            fs::create_dir(&data).unwrap();
            fs::create_dir(&assets).unwrap();
            let database = data.join("assets.sqlite3");
            Connection::open(&database)
                .unwrap()
                .execute_batch("CREATE TABLE lite_cut_assets (id INTEGER PRIMARY KEY, file_path TEXT NOT NULL);")
                .unwrap();
            Self { _temp: temp, data, assets, database }
        }

        fn row(&self, id: i64, source: &Path) {
            Connection::open(&self.database)
                .unwrap()
                .execute("INSERT INTO lite_cut_assets VALUES (?1, ?2)", rusqlite::params![id, path_text(source)])
                .unwrap();
        }

        fn delete(&self, id: i64, source: &Path) -> Result<Option<PathBuf>, Problem> {
            let raw = path_text(source);
            let source = managed_source(&self.assets, &raw).unwrap();
            delete_owned_bundle(&self.database, &self.data, &self.assets, id, &raw, source.as_deref())
        }

        fn count(&self) -> i64 {
            Connection::open(&self.database)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM lite_cut_assets", [], |row| row.get(0))
                .unwrap()
        }
    }

    #[test]
    fn deleting_asset_quarantines_owned_files_and_writes_legacy_recovery_manifest() {
        let fixture = Fixture::new();
        let source = fixture.assets.join("回放.mov");
        let cache = source.with_extension("preview-alpha-v3.webm");
        let waveform = source.with_extension("waveform-v1.json");
        let unrelated = fixture.assets.join("notes.txt");
        for (path, data) in [
            (&source, "original"), (&cache, "preview"), (&waveform, "waveform"), (&unrelated, "untouched"),
        ] {
            fs::write(path, data).unwrap();
        }
        fixture.row(5, &source);
        let directory = fixture.delete(5, &source).unwrap().unwrap();
        assert_eq!(fixture.count(), 0);
        assert_eq!(directory.parent().unwrap(), fixture.data.join("trash/lite-cut"));
        let manifest: Value = serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["namespace"], "lite-cut");
        assert!(manifest["created_at"].as_str().unwrap().ends_with("+00:00"));
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        for (index, (path, bytes)) in [(&cache, "preview"), (&waveform, "waveform"), (&source, "original")].into_iter().enumerate() {
            assert!(!path.exists());
            assert_eq!(files[index]["original"], path_text(path));
            let dest = PathBuf::from(files[index]["quarantined"].as_str().unwrap());
            assert_eq!(dest.file_name().unwrap().to_string_lossy(), format!("{index:04}-{}", path.file_name().unwrap().to_string_lossy()));
            assert_eq!(fs::read_to_string(dest).unwrap(), bytes);
        }
        assert_eq!(fs::read_to_string(&unrelated).unwrap(), "untouched");
        assert!(matches!(fixture.delete(5, &source), Err(Problem::Missing)));
    }

    #[test]
    fn duplicate_rows_and_same_stem_keep_other_source_and_shared_sidecars() {
        let fixture = Fixture::new();
        let original = fixture.assets.join("same.mkv");
        let other = fixture.assets.join("same.mov");
        let proxy = original.with_extension("preview60.mp4");
        for path in [&original, &other, &proxy] {
            fs::write(path, b"fixture").unwrap();
        }
        fixture.row(1, &original);
        fixture.row(2, &original);
        fixture.row(3, &other);
        assert!(fixture.delete(1, &original).unwrap().is_none());
        assert!(original.is_file());
        assert!(fixture.delete(2, &original).unwrap().is_some());
        assert!(!original.exists() && other.is_file() && proxy.is_file());
        assert_eq!(fixture.count(), 1);
    }

    #[test]
    fn database_abort_restores_every_file_and_retains_the_row() {
        let fixture = Fixture::new();
        let original = fixture.assets.join("clip.mov");
        let cache = original.with_extension("preview.mp4");
        fs::write(&original, b"clip").unwrap();
        fs::write(&cache, b"cache").unwrap();
        fixture.row(1, &original);
        Connection::open(&fixture.database).unwrap().execute_batch(
            "CREATE TRIGGER reject_delete BEFORE DELETE ON lite_cut_assets
             BEGIN SELECT RAISE(ABORT, 'fixture rollback'); END;",
        ).unwrap();
        assert!(matches!(fixture.delete(1, &original), Err(Problem::Database)));
        assert_eq!(fs::read(original).unwrap(), b"clip");
        assert_eq!(fs::read(cache).unwrap(), b"cache");
        assert_eq!(fixture.count(), 1);
        assert_eq!(fs::read_dir(fixture.data.join("trash/lite-cut")).unwrap().count(), 0);
    }

    #[test]
    fn out_of_root_row_does_not_move_a_file() {
        let fixture = Fixture::new();
        let unrelated = fixture.data.join("unrelated.mov");
        fs::write(&unrelated, b"other data").unwrap();
        fixture.row(1, &unrelated);
        assert!(fixture.delete(1, &unrelated).unwrap().is_none());
        assert_eq!(fs::read(unrelated).unwrap(), b"other data");
        assert_eq!(fixture.count(), 0);
        assert!(!fixture.data.join("trash").exists());
    }

    #[test]
    fn cross_volume_attempt_uses_the_managed_asset_root_without_copying() {
        let fixture = Fixture::new();
        let original = fixture.assets.join("selected-drive.mov");
        fs::write(&original, b"original data").unwrap();
        let batch = quarantine(
            &fixture.data,
            &fixture.assets,
            &[original.clone()],
            "2026-11-23T00:00:00+00:00",
            |from, to| {
                if to.starts_with(&fixture.data) {
                    Err(io::Error::from(io::ErrorKind::CrossesDevices))
                } else {
                    move_noclobber(from, to)
                }
            },
        ).unwrap();
        assert_eq!(batch.directory.parent().unwrap(), fixture.assets.join("trash/lite-cut"));
        assert_eq!(fs::read(&batch.files[0].quarantined).unwrap(), b"original data");
        assert!(!original.exists());
        assert_eq!(fs::read_dir(fixture.data.join("trash/lite-cut")).unwrap().count(), 0);
        batch.restore(1).unwrap();
        assert_eq!(fs::read(original).unwrap(), b"original data");
    }
}
