use maxgamestudio_league_core::profile_snapshot::snapshot_profile_database;
use rusqlite::{Connection, OpenFlags};
use std::fs;

#[test]
fn snapshots_uncheckpointed_wal_without_replacing_source_or_destination() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("工具箱数据");
    fs::create_dir(&root).unwrap();
    let source = root.join("LeagueAkari.db");
    let destination = root.join("snapshot.db");
    let writer = Connection::open(&source).unwrap();
    let journal: String = writer
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
    writer
        .execute_batch(
            "PRAGMA wal_autocheckpoint=0;
            CREATE TABLE profile (setting TEXT);
            INSERT INTO profile VALUES ('大乱斗');",
        )
        .unwrap();
    let original = fs::read(&source).unwrap();
    assert!(fs::metadata(root.join("LeagueAkari.db-wal")).unwrap().len() > 32);

    snapshot_profile_database(&source, &destination).unwrap();
    let saved =
        Connection::open_with_flags(&destination, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let setting: String = saved
        .query_row("SELECT setting FROM profile", [], |row| row.get(0))
        .unwrap();
    assert_eq!(setting, "大乱斗");
    assert_eq!(fs::read(&source).unwrap(), original);
    drop(saved);

    let committed = fs::read(&destination).unwrap();
    writer
        .execute("UPDATE profile SET setting='旧值已变'", [])
        .unwrap();
    assert!(snapshot_profile_database(&source, &destination).is_err());
    assert_eq!(fs::read(&destination).unwrap(), committed);
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".league-db-")
    }));
}
