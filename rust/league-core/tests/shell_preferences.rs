use maxgamestudio_league_core::{
    profile_database::LeagueProfileDatabase,
    shell_preferences::{ShellPreference, APP_COMMON, LEAGUE_CLIENT, MAIN_WINDOW},
};
use serde_json::json;

#[test]
fn shell_reads_beta_settings_and_writes_the_same_rows() {
    let profile = tempfile::tempdir().unwrap();
    let db = LeagueProfileDatabase::open(profile.path()).unwrap();
    let locale = ShellPreference::from_ipc(APP_COMMON, "locale").unwrap();
    let theme = ShellPreference::from_ipc(APP_COMMON, "theme").unwrap();
    let action = ShellPreference::from_ipc(MAIN_WINDOW, "closeAction").unwrap();
    let auto_connect = ShellPreference::from_ipc(LEAGUE_CLIENT, "autoConnect").unwrap();

    db.set_setting("app-common-main/locale", &json!("zh-CN"))
        .unwrap();
    db.set_setting("app-common-main/theme", &json!("graphite"))
        .unwrap();
    db.set_setting(
        "window-manager-main/main-window/closeAction",
        &json!("quit"),
    )
    .unwrap();

    assert_eq!(locale.read(&db, "en").unwrap(), "zh-CN");
    assert_eq!(theme.read(&db, "en").unwrap(), "graphite");
    assert_eq!(action.read(&db, "en").unwrap(), "quit");
    db.set_setting("league-client-main/autoConnect", &json!(false))
        .unwrap();
    assert_eq!(auto_connect.read(&db, "en").unwrap(), false);

    locale.write(&db, &json!("en")).unwrap();
    action.write(&db, &json!("minimize-to-tray")).unwrap();
    assert!(action.write(&db, &json!("return-to-main")).is_err());
    assert!(theme.write(&db, &json!("unknown")).is_err());
    drop(db);

    let db = LeagueProfileDatabase::open(profile.path()).unwrap();
    assert_eq!(
        db.setting("app-common-main/locale").unwrap(),
        Some(json!("en"))
    );
    assert_eq!(theme.read(&db, "en").unwrap(), "graphite");
    assert_eq!(action.read(&db, "en").unwrap(), "minimize-to-tray");
    assert!(ShellPreference::from_ipc(APP_COMMON, "streamerMode").is_none());
}

#[test]
fn shell_uses_system_defaults_and_keeps_incompatible_saved_values() {
    let profile = tempfile::tempdir().unwrap();
    let db = LeagueProfileDatabase::open(profile.path()).unwrap();
    assert_eq!(ShellPreference::Locale.read(&db, "zh-CN").unwrap(), "zh-CN");
    assert_eq!(ShellPreference::Locale.read(&db, "en").unwrap(), "en");
    assert_eq!(ShellPreference::Theme.read(&db, "en").unwrap(), "dark");
    assert_eq!(ShellPreference::CloseAction.read(&db, "en").unwrap(), "ask");
    assert_eq!(
        ShellPreference::ClientAutoConnect.read(&db, "en").unwrap(),
        true
    );

    db.set_setting("app-common-main/theme", &json!("future-theme"))
        .unwrap();
    assert_eq!(ShellPreference::Theme.read(&db, "en").unwrap(), "dark");
    assert_eq!(
        db.setting("app-common-main/theme").unwrap(),
        Some(json!("future-theme")),
    );
}
