use maxgamestudio_league_core::profile_database::{LeagueProfileDatabase, PROFILE_DB_VERSION};
use rusqlite::Connection;
use serde_json::json;
use std::error::Error;

#[test]
fn creates_a_profile_that_the_old_toolbox_can_read() -> Result<(), Box<dyn Error + Send + Sync>> {
    let fixture = tempfile::tempdir()?;
    let db = LeagueProfileDatabase::open(fixture.path())?;
    assert_eq!(db.version(), PROFILE_DB_VERSION);
    assert_eq!(db.setting("app-common-main/locale")?, None);
    db.set_setting("app-common-main/locale", &json!("zh-CN"))?;
    db.set_setting("opgg-window-main/autoShow", &json!(false))?;
    db.set_setting("aram/count", &json!(3))?;
    assert_eq!(db.setting("aram/count")?, Some(json!(3)));
    db.set_setting("aram/count", &json!(null))?;
    assert_eq!(db.setting("aram/count")?, Some(json!(null)));

    let legacy = Connection::open(fixture.path().join("LeagueAkari.db"))?;
    let locale: String = legacy.query_row(
        "SELECT value FROM Settings WHERE key = 'app-common-main/locale'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(locale, "\"zh-CN\"");
    let count: i64 = legacy.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name IN (
         'Metadata', 'Settings', 'SavedPlayers', 'EncounteredGames',
         'encountered_games_queue_type')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count, 5);
    Ok(())
}

#[test]
fn upgrades_version_ten_without_overwriting_legacy_rows() -> Result<(), Box<dyn Error + Send + Sync>>
{
    let fixture = tempfile::tempdir()?;
    let legacy = Connection::open(fixture.path().join("LeagueAkari.db"))?;
    legacy.execute_batch(
        "CREATE TABLE Metadata (key VARCHAR NOT NULL PRIMARY KEY, value JSON NOT NULL);
         CREATE TABLE Settings (key VARCHAR NOT NULL PRIMARY KEY, value JSON NOT NULL);
         CREATE TABLE EncounteredGames (id INTEGER PRIMARY KEY, gameId INTEGER NOT NULL);
         INSERT INTO Metadata VALUES ('version', json('10'));
         INSERT INTO Settings VALUES ('app-common-main/locale', json('\"zh-CN\"'));
         INSERT INTO EncounteredGames VALUES (27, 132);",
    )?;
    let upgraded = LeagueProfileDatabase::open(fixture.path())?;
    assert_eq!(upgraded.version(), 15);
    assert_eq!(
        upgraded.setting("app-common-main/locale")?,
        Some(json!("zh-CN")),
    );
    let game: (i64, String) = legacy.query_row(
        "SELECT gameId, queueType FROM EncounteredGames WHERE id = 27",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(game, (132, String::new()));
    assert_eq!(LeagueProfileDatabase::open(fixture.path())?.version(), 15);
    Ok(())
}

#[test]
fn fails_safely_for_unknown_or_incomplete_schemas() -> Result<(), Box<dyn Error + Send + Sync>> {
    let fixture = tempfile::tempdir()?;
    let legacy = Connection::open(fixture.path().join("LeagueAkari.db"))?;
    legacy.execute_batch(
        "CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('keep');",
    )?;
    assert!(LeagueProfileDatabase::open(fixture.path()).is_err());
    let saved: String = legacy.query_row("SELECT value FROM unrelated", [], |row| row.get(0))?;
    assert_eq!(saved, "keep");

    legacy.execute_batch(
        "CREATE TABLE Metadata (key VARCHAR NOT NULL PRIMARY KEY, value JSON NOT NULL);
         INSERT INTO Metadata VALUES ('version', json('10'));",
    )?;
    assert!(LeagueProfileDatabase::open(fixture.path()).is_err());
    let version: i64 = legacy.query_row(
        "SELECT value FROM Metadata WHERE key = 'version'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 10);
    Ok(())
}
