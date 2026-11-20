use super::*;

fn fixture_file(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn restores_a_split_profile_and_reuses_a_verified_retry() {
    let scratch = tempfile::tempdir().unwrap();
    let legacy_root = scratch.path().join("legacy");
    let layout = (
        legacy_root.join("data"),
        scratch.path().join("old-webview"),
        scratch.path().join("old-league"),
    );
    let destination = scratch.path().join("新数据");
    fixture_file(
        &layout.0,
        "cs2-insight.config.json",
        b"{\"locale\":\"zh-CN\"}",
    );
    fixture_file(&layout.0, "cache/map.bin", b"existing cache");
    fixture_file(&layout.1, "User Data/Preferences", b"browser preferences");
    fixture_file(&layout.2, "LeagueAkari.db", b"legacy profile bytes");
    fixture_file(&legacy_root, ".window-state.json", b"window state");

    let before = fs::read(layout.0.join("cs2-insight.config.json")).unwrap();
    for _ in 0..2 {
        migrate_checked(&legacy_root, Some(&layout), &destination, || Ok(())).unwrap();
        assert_eq!(
            fs::read(destination.join("data/cs2-insight.config.json")).unwrap(),
            before
        );
        assert_eq!(
            fs::read(destination.join("data/cache/map.bin")).unwrap(),
            b"existing cache"
        );
        assert_eq!(
            fs::read(destination.join("webview/User Data/Preferences")).unwrap(),
            b"browser preferences"
        );
        assert_eq!(
            fs::read(destination.join("league-runtime/LeagueAkari.db")).unwrap(),
            b"legacy profile bytes"
        );
        assert_eq!(
            fs::read(destination.join("window-state.json")).unwrap(),
            b"window state"
        );
    }
    assert_eq!(
        fs::read(layout.0.join("cs2-insight.config.json")).unwrap(),
        before
    );
    fs::write(
        layout.0.join("cs2-insight.config.json"),
        b"changed after migration",
    )
    .unwrap();
    assert!(migrate_checked(&legacy_root, Some(&layout), &destination, || Ok(())).is_err());
    assert_eq!(
        fs::read(destination.join("data/cs2-insight.config.json")).unwrap(),
        before
    );
}

#[test]
fn changing_source_during_migration_keeps_original_data_and_target_empty() {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("current");
    let destination = scratch.path().join("selected");
    fixture_file(&source, "data/cs2-insight.db", b"before");
    fs::create_dir(&destination).unwrap();
    let calls = std::cell::Cell::new(0);
    let result = migrate_checked(&source, None, &destination, || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            fs::write(source.join("data/cs2-insight.db"), b"changed")?;
        }
        Ok(())
    });

    assert!(result.is_err());
    assert_eq!(
        fs::read(source.join("data/cs2-insight.db")).unwrap(),
        b"changed"
    );
    assert!(fs::read_dir(destination).unwrap().next().is_none());
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 2);
}

#[test]
fn refuses_an_occupied_destination_and_nested_storage() {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("current");
    let destination = scratch.path().join("selected");
    fixture_file(&source, "data/input.txt", b"source");
    fixture_file(&destination, "keep.txt", b"unknown file");
    assert!(migrate_checked(&source, None, &destination, || Ok(())).is_err());
    assert!(migrate_checked(&source, None, &source.join("nested"), || Ok(())).is_err());
    assert_eq!(
        fs::read(destination.join("keep.txt")).unwrap(),
        b"unknown file"
    );
    assert_eq!(fs::read(source.join("data/input.txt")).unwrap(), b"source");
}
