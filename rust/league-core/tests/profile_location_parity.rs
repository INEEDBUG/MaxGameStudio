use maxgamestudio_league_core::profile_location::ProfileLocation;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize)]
struct LocatorCase {
    name: String,
    bytes: Option<Vec<u8>>,
    path: Option<PathBuf>,
}

#[test]
fn replay_released_utf16_locators_without_writing_a_profile() {
    let cases: Vec<LocatorCase> =
        serde_json::from_str(include_str!("fixtures/profile-location.json")).unwrap();
    for case in cases {
        let fixture = tempfile::tempdir().unwrap();
        let control = fixture.path().join("MaxGameStudioLeague");
        fs::create_dir(&control).unwrap();
        let active = control.join("data-location.txt");
        if let Some(bytes) = &case.bytes {
            fs::write(&active, bytes).unwrap();
        }
        let locations = ProfileLocation::read(&control).unwrap();
        assert_eq!(locations.active, case.path, "{}", case.name);
        assert_eq!(
            locations.selected,
            case.path.unwrap_or_else(|| control.join("Profile")),
            "{}",
            case.name
        );
        match case.bytes {
            Some(bytes) => assert_eq!(fs::read(active).unwrap(), bytes),
            None => assert!(!active.exists()),
        }
        assert!(!control.join("Profile").exists());
    }
}

#[test]
fn installer_selection_takes_precedence_without_rewriting_active_location() {
    let fixture = tempfile::tempdir().unwrap();
    let active = fixture.path().join("data-location.txt");
    let pending = fixture.path().join("installer-data-location.txt");
    let encode_location = |path: &str| {
        path.encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>()
    };
    let original = encode_location("\u{feff}X:\\Old Profile");
    let selection = encode_location("\u{feff}Y:\\New Profile\t");
    fs::write(&active, &original).unwrap();
    fs::write(&pending, &selection).unwrap();

    let locations = ProfileLocation::read(fixture.path()).unwrap();
    assert_eq!(locations.active, Some(PathBuf::from("X:\\Old Profile")));
    assert_eq!(locations.selected, PathBuf::from("Y:\\New Profile"));
    assert_eq!(fs::read(active).unwrap(), original);
    assert_eq!(fs::read(pending).unwrap(), selection);
}

#[test]
fn unreadable_selection_is_not_treated_as_absent() {
    let fixture = tempfile::tempdir().unwrap();
    fs::create_dir(fixture.path().join("installer-data-location.txt")).unwrap();
    assert!(ProfileLocation::read(fixture.path()).is_err());
}
