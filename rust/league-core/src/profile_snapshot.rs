use rusqlite::{Connection, OpenFlags};
use std::error::Error;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::time::Duration;

/// Commit a verified SQLite snapshot to a new file in the caller's staging directory.
pub fn snapshot_profile_database(
    source: &Path,
    destination: &Path,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Missing snapshot directory"))?;
    let stage = tempfile::Builder::new()
        .prefix(".league-db-")
        .tempdir_in(parent)?;
    let snapshot = tempfile::NamedTempFile::new_in(stage.path())?;
    let snapshot_path = snapshot.path().to_str().ok_or_else(|| {
        io::Error::new(ErrorKind::InvalidInput, "Snapshot path is not valid text")
    })?;

    let original = open_read_only(source)?;
    original.execute("VACUUM INTO ?1", [snapshot_path])?;
    original.close().map_err(|(_, error)| error)?;

    let saved = open_read_only(snapshot.path())?;
    let integrity: String = saved.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "Profile database failed quick_check",
        )
        .into());
    }
    saved.close().map_err(|(_, error)| error)?;

    snapshot.as_file().sync_all()?;
    snapshot.persist_noclobber(destination)?;
    Ok(())
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let database = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    database.busy_timeout(Duration::from_secs(4))?;
    Ok(database)
}
