//! The standalone toolbox's existing SQLite profile schema and JSON settings.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use std::{error::Error, io, path::Path, time::Duration};

pub const PROFILE_DB_VERSION: i64 = 15;
pub type ProfileDatabaseError = Box<dyn Error + Send + Sync>;

pub struct LeagueProfileDatabase {
    connection: Connection,
    version: i64,
}

impl LeagueProfileDatabase {
    pub fn open(profile: &Path) -> Result<Self, ProfileDatabaseError> {
        let mut connection = Connection::open(profile.join("LeagueAkari.db"))?;
        connection.busy_timeout(Duration::from_secs(4))?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut version = schema_version(&transaction)?;
        if version == 0 {
            initialize_version_ten(&transaction)?;
            version = 10;
        }
        if version < PROFILE_DB_VERSION {
            transaction.execute_batch(
                "ALTER TABLE EncounteredGames ADD COLUMN queueType VARCHAR NOT NULL DEFAULT '';
                 CREATE INDEX encountered_games_queue_type ON EncounteredGames (queueType);
                 UPDATE Metadata SET value = json('15') WHERE key = 'version';",
            )?;
            version = PROFILE_DB_VERSION;
        }
        transaction.commit()?;
        Ok(Self {
            connection,
            version,
        })
    }

    pub fn version(&self) -> i64 {
        self.version
    }

    pub fn setting(&self, key: &str) -> Result<Option<Value>, ProfileDatabaseError> {
        // SQLite gives JSON numeric affinity, so CAST handles both numeric and text values.
        let serialized: Option<String> = self
            .connection
            .query_row(
                "SELECT CAST(value AS TEXT) FROM Settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(serialized
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?)
    }

    pub fn set_setting(&self, key: &str, value: &Value) -> Result<(), ProfileDatabaseError> {
        let serialized = serde_json::to_string(value)?;
        self.connection.execute(
            "INSERT INTO Settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, serialized],
        )?;
        Ok(())
    }
}

fn schema_version(transaction: &Transaction<'_>) -> Result<i64, ProfileDatabaseError> {
    let has_metadata: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'Metadata')",
        [],
        |row| row.get(0),
    )?;
    if has_metadata {
        let version: Option<i64> = transaction
            .query_row(
                "SELECT value FROM Metadata WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(version) = version.filter(|version| *version >= 10) {
            return Ok(version);
        }
    } else {
        let empty: bool = transaction.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM sqlite_schema WHERE name NOT LIKE 'sqlite\\_%' ESCAPE '\\')",
            [],
            |row| row.get(0),
        )?;
        if empty {
            return Ok(0);
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidData, "无法识别已有工具箱数据库版本。").into())
}

fn initialize_version_ten(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "CREATE TABLE Metadata (
             key VARCHAR NOT NULL PRIMARY KEY,
             value JSON NOT NULL
         );
         CREATE TABLE Settings (
             key VARCHAR NOT NULL PRIMARY KEY,
             value JSON NOT NULL
         );
         CREATE TABLE SavedPlayers (
             puuid VARCHAR NOT NULL,
             selfPuuid VARCHAR NOT NULL,
             region VARCHAR NOT NULL,
             rsoPlatformId VARCHAR NOT NULL,
             tag VARCHAR,
             updateAt DATETIME NOT NULL,
             lastMetAt DATETIME,
             PRIMARY KEY (puuid, selfPuuid, region, rsoPlatformId)
         );
         CREATE INDEX saved_players_update_at_index ON SavedPlayers (updateAt);
         CREATE INDEX saved_players_last_met_at_index ON SavedPlayers (lastMetAt);
         CREATE TABLE EncounteredGames (
             id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
             gameId INTEGER NOT NULL,
             puuid VARCHAR NOT NULL,
             selfPuuid VARCHAR NOT NULL,
             region VARCHAR NOT NULL,
             rsoPlatformId VARCHAR NOT NULL,
             updateAt DATETIME NOT NULL
         );
         CREATE INDEX encountered_games_game_id_index ON EncounteredGames (gameId);
         CREATE INDEX encountered_games_puuid_index ON EncounteredGames (puuid);
         CREATE INDEX encountered_games_self_puuid_index ON EncounteredGames (selfPuuid);
         CREATE INDEX encountered_games_region_index ON EncounteredGames (region);
         CREATE INDEX encountered_games_rso_platform_id_index ON EncounteredGames (rsoPlatformId);
         CREATE INDEX encountered_games_update_at_index ON EncounteredGames (updateAt);
         INSERT INTO Metadata (key, value) VALUES ('version', json('10'));",
    )
}
