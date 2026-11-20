#![cfg(windows)]

use maxgamestudio_league_core::profile_migration::{prepare_league_profile, ProfileMigrationError};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
use std::path::{Path, PathBuf};

struct Fixture {
    root: tempfile::TempDir,
    control: PathBuf,
    program: PathBuf,
    source: PathBuf,
    target: PathBuf,
}

fn locator_bytes(path: &Path) -> Vec<u8> {
    [0xfeff]
        .into_iter()
        .chain(path.as_os_str().encode_wide())
        .flat_map(u16::to_le_bytes)
        .collect()
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let control = root.path().join("control");
        let program = root.path().join("program");
        let source = root.path().join("原 League 数据");
        let target = root.path().join("备用 League 数据");
        for path in [&control, &program, &source] {
            fs::create_dir(path).unwrap();
        }
        let original = source.join("base-config.json");
        fs::write(&original, b"{\"locale\":\"zh-CN\"}").unwrap();
        fs::write(control.join("data-location.txt"), locator_bytes(&source)).unwrap();
        Self {
            root,
            control,
            program,
            source,
            target,
        }
    }

    fn select(&self, target: &Path) {
        fs::write(
            self.control.join("installer-data-location.txt"),
            locator_bytes(target),
        )
        .unwrap();
    }

    fn prepare(&self) -> Result<PathBuf, ProfileMigrationError> {
        prepare_league_profile(&self.control, &self.program, None)
    }

    fn assert_location(&self, active: &Path, pending: &Path) {
        assert_eq!(
            fs::read(self.control.join("data-location.txt")).unwrap(),
            locator_bytes(active)
        );
        assert_eq!(
            fs::read(self.control.join("installer-data-location.txt")).unwrap(),
            locator_bytes(pending)
        );
    }

    fn assert_no_staging(&self) {
        assert!(fs::read_dir(self.root.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".mgs-league-")));
    }
}

#[test]
fn unchanged_upgrade_reuses_a_case_variant_without_rewriting_locator() {
    let fixture = Fixture::new();
    let same_profile = fixture.root.path().join("原 lEAGUE 数据");
    fixture.select(&same_profile);
    assert_eq!(fixture.prepare().unwrap(), same_profile);
    fixture.assert_location(&fixture.source, &same_profile);
    assert!(!fixture
        .source
        .join(".maxgamestudio-league-profile")
        .exists());
    fixture.assert_no_staging();
}

#[test]
fn moves_only_supported_settings_and_takes_a_live_wal_snapshot() {
    let fixture = Fixture::new();
    fixture.select(&fixture.target);
    fs::create_dir(&fixture.target).unwrap();
    fs::create_dir(fixture.source.join("AkariConfig")).unwrap();
    fs::write(fixture.source.join("AkariConfig/runes.json"), b"[]").unwrap();
    fs::write(fixture.source.join("unrelated.txt"), b"keep").unwrap();
    let db = fixture.source.join("LeagueAkari.db");
    let wal = fixture.source.join("LeagueAkari.db-wal");
    let writer = Connection::open(&db).unwrap();
    writer
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
        CREATE TABLE profile (value TEXT); INSERT INTO profile VALUES ('大乱斗');",
        )
        .unwrap();
    let before = fs::read(&db).unwrap();
    let wal_before = fs::read(&wal).unwrap();
    assert!(wal_before.len() > 32);

    assert_eq!(fixture.prepare().unwrap(), fixture.target);
    fixture.assert_location(&fixture.target, &fixture.target);
    let snapshot = Connection::open_with_flags(
        fixture.target.join("LeagueAkari.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        snapshot
            .query_row::<String, _, _>("SELECT value FROM profile", [], |row| row.get(0))
            .unwrap(),
        "大乱斗"
    );
    assert_eq!(
        fs::read(fixture.target.join("base-config.json")).unwrap(),
        fs::read(fixture.source.join("base-config.json")).unwrap()
    );
    assert_eq!(
        fs::read(fixture.target.join("AkariConfig/runes.json")).unwrap(),
        b"[]"
    );
    assert!(!fixture.target.join("unrelated.txt").exists());
    assert!(!fixture.target.join("LeagueAkari.db-wal").exists());
    assert_eq!(fs::read(&db).unwrap(), before);
    assert_eq!(fs::read(&wal).unwrap(), wal_before);
    fixture.assert_no_staging();
}

#[test]
fn refuses_occupied_program_and_nested_selections_before_write() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.target).unwrap();
    let unknown = fixture.target.join("unknown.txt");
    fs::write(&unknown, b"retain").unwrap();
    // The profile marker is a specific file format, not just an existing name.
    fs::write(
        fixture.target.join(".maxgamestudio-league-profile"),
        b"foreign marker",
    )
    .unwrap();
    fixture.select(&fixture.target);
    assert!(matches!(
        fixture.prepare(),
        Err(ProfileMigrationError::Occupied)
    ));
    assert_eq!(fs::read(&unknown).unwrap(), b"retain");
    fixture.assert_location(&fixture.source, &fixture.target);

    let nested = fixture.source.join("child");
    fixture.select(&nested);
    assert!(matches!(
        fixture.prepare(),
        Err(ProfileMigrationError::Overlap)
    ));
    assert!(!nested.exists());
    fixture.assert_location(&fixture.source, &nested);

    let program_data = fixture.program.join("data");
    fixture.select(&program_data);
    assert!(matches!(
        fixture.prepare(),
        Err(ProfileMigrationError::ProgramOverlap)
    ));
    assert!(!program_data.exists());
    fixture.assert_no_staging();
}

#[test]
fn malformed_db_keeps_active_and_pending_locations_and_empty_target() {
    let fixture = Fixture::new();
    let bad = fixture.source.join("LeagueAkari.db");
    fs::write(&bad, b"malformed database").unwrap();
    fs::create_dir(&fixture.target).unwrap();
    fixture.select(&fixture.target);
    assert!(matches!(
        fixture.prepare(),
        Err(ProfileMigrationError::Database(_))
    ));
    fixture.assert_location(&fixture.source, &fixture.target);
    assert_eq!(fs::read(&bad).unwrap(), b"malformed database");
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
    fixture.assert_no_staging();
}

#[test]
fn marked_destination_recovers_after_atomic_locator_replace_fails() {
    let fixture = Fixture::new();
    fixture.select(&fixture.target);
    let protected = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(fixture.control.join("data-location.txt"))
        .unwrap();
    assert!(fixture.prepare().is_err());
    fixture.assert_location(&fixture.source, &fixture.target);
    assert_eq!(fs::read_dir(&fixture.control).unwrap().count(), 2);
    assert_eq!(
        fs::read(fixture.target.join(".maxgamestudio-league-profile")).unwrap(),
        b"1\n"
    );
    let target_config = fixture.target.join("base-config.json");
    fs::write(&target_config, b"{\"next\":true}").unwrap();
    drop(protected);
    assert_eq!(fixture.prepare().unwrap(), fixture.target);
    fixture.assert_location(&fixture.target, &fixture.target);
    assert_eq!(fs::read(&target_config).unwrap(), b"{\"next\":true}");
    fixture.assert_no_staging();
}

#[test]
fn first_start_can_import_a_resolved_embedded_profile() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.control.join("data-location.txt")).unwrap();
    let profile =
        prepare_league_profile(&fixture.control, &fixture.program, Some(&fixture.source)).unwrap();
    assert_eq!(profile, fixture.control.join("Profile"));
    assert_eq!(
        fs::read(profile.join("base-config.json")).unwrap(),
        fs::read(fixture.source.join("base-config.json")).unwrap()
    );
    assert_eq!(
        fs::read(fixture.control.join("data-location.txt")).unwrap(),
        locator_bytes(&profile)
    );
    assert!(!fixture.control.join("installer-data-location.txt").exists());
}
