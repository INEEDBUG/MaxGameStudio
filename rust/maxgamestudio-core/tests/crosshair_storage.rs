use maxgamestudio_core::crosshair_storage::{load_profiles, save_profiles};
use serde_json::{json, Value};
use std::fs;

#[test]
fn opens_existing_desktop_profiles_and_computes_the_code() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("valorant-crosshair-profiles.json");
    fs::write(
        &profile,
        r#"{"profiles":{"P":{"color":"red"}},"code":"stale"}"#,
    )
    .unwrap();
    let before = fs::read(&profile).unwrap();
    let saved = load_profiles(directory.path()).unwrap();
    assert_eq!(saved["profiles"]["P"]["color"], "red");
    assert_ne!(saved["code"], "stale");
    assert_eq!(fs::read(profile).unwrap(), before);
}

#[test]
fn saves_the_original_format_and_keeps_it_after_invalid_input() {
    let directory = tempfile::tempdir().unwrap();
    let first = json!({"P":{"color":"cyan"}});
    let result = save_profiles(directory.path(), first.clone()).unwrap();
    assert_eq!(result["saved"], true);
    assert_eq!(result["format"], "valorant-native-v0");
    let stored = directory.path().join("valorant-crosshair-profiles.json");
    let contents = fs::read(&stored).unwrap();
    let payload: Value = serde_json::from_slice(&contents).unwrap();
    assert_eq!(payload["profiles"], first);
    assert_eq!(payload["code"], result["code"]);
    assert_eq!(payload["saved"], Value::Null);
    assert!(save_profiles(directory.path(), json!(false)).is_err());
    assert_eq!(fs::read(&stored).unwrap(), contents);
    assert_eq!(directory.path().read_dir().unwrap().count(), 1);
}

#[test]
fn missing_or_broken_profiles_load_as_empty_without_rewriting_the_file() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        load_profiles(directory.path()).unwrap(),
        json!({"profiles": {}})
    );
    let stored = directory.path().join("valorant-crosshair-profiles.json");
    assert!(!stored.exists());
    fs::write(&stored, b"{").unwrap();
    assert_eq!(
        load_profiles(directory.path()).unwrap(),
        json!({"profiles": {}})
    );
    assert_eq!(fs::read(stored).unwrap(), b"{");
}
