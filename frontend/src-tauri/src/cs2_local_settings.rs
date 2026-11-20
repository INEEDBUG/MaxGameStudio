//! Read-only import of Steam's per-account CS2 settings.

use crate::steam_accounts::{
    display_steam_path, settings_registry,
    vdf::{flat_settings, local_login_users, LocalLoginUser, STEAM_ID64_BASE},
};
use serde_json::{json, Value};
use std::{
    cmp::Reverse,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[tauri::command]
pub(crate) async fn get_local_cs2_settings() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        discover(
            |name| std::env::var_os(name),
            settings_registry(),
            std::env::home_dir(),
        )
    })
    .await
    .map_err(|_| "steam_read_failed".to_owned())?
}

fn discover(
    environment: impl Fn(&str) -> Option<OsString>,
    (registry_root, registry_user): (Option<PathBuf>, Option<OsString>),
    home: Option<PathBuf>,
) -> Result<Value, String> {
    let configured_root = nonempty_text(environment("CS2_INSIGHT_STEAM_PATH")).map(PathBuf::from);
    let program_files = environment("PROGRAMFILES(X86)")
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join("Steam"));
    let home_steam = home.map(|path| path.join(".steam").join("steam"));
    let root = [configured_root, registry_root, program_files, home_steam]
        .into_iter()
        .flatten()
        .find_map(|candidate| {
            if candidate.join("userdata").is_dir() {
                candidate.canonicalize().ok()
            } else {
                None
            }
        });
    let Some(root) = root else {
        return Ok(
            json!({"found": false, "steam_root": null, "active_account_id": null, "accounts": []}),
        );
    };

    let login_users = local_login_users(&read_text(&root.join("config").join("loginusers.vdf")));
    let auto_login_user = nonempty_text(environment("CS2_INSIGHT_STEAM_AUTO_LOGIN_USER"))
        .or_else(|| nonempty_text(registry_user))
        .unwrap_or_default()
        .to_lowercase();
    let mut accounts = Vec::new();
    for entry in fs::read_dir(root.join("userdata")).map_err(|_| "steam_read_failed".to_owned())? {
        let path = entry.map_err(|_| "steam_read_failed".to_owned())?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !path.is_dir() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(account_id) = name.parse::<u32>() else {
            continue;
        };
        if let Some(account) = read_account(
            &path,
            name,
            account_id,
            login_users.get(&account_id),
            &auto_login_user,
        ) {
            accounts.push(account);
        }
    }
    accounts.sort_by_key(|account| Reverse(account.priority));
    let accounts: Vec<_> = accounts.into_iter().map(|account| account.value).collect();
    Ok(json!({
        "found": !accounts.is_empty(),
        "steam_root": display_steam_path(&root),
        "active_account_id": accounts.first().and_then(|account| account.get("account_id")),
        "accounts": accounts,
        "limitations": ["dpi_not_in_cs2_cfg", "gpu_scaling_not_in_cs2_cfg"],
    }))
}

fn nonempty_text(value: Option<OsString>) -> Option<String> {
    let value = value?.to_string_lossy().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn read_text(path: &Path) -> String {
    fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn display_aspect(mode: Option<&str>, width: Option<i64>, height: Option<i64>) -> &'static str {
    match mode {
        Some("0") => return "4:3",
        Some("1") => return "16:10",
        Some("2") => return "16:9",
        _ => {}
    }
    let (Some(width), Some(height)) = (width, height) else {
        return "other";
    };
    if width == 0 || height == 0 {
        return "other";
    }
    let ratio = width as f64 / height as f64;
    let closest = [
        ("16:9", 16.0 / 9.0),
        ("16:10", 16.0 / 10.0),
        ("4:3", 4.0 / 3.0),
        ("5:4", 5.0 / 4.0),
    ]
    .into_iter()
    .map(|(label, shape)| (label, (ratio - shape).abs()))
    .min_by(|(_, left), (_, right)| left.total_cmp(right));
    closest
        .filter(|(_, distance)| *distance <= 0.04)
        .map_or("other", |(label, _)| label)
}

type UtcTimestamp = (i64, u32);

struct LocalAccount {
    priority: (bool, bool, i64, Option<UtcTimestamp>),
    value: Value,
}

fn read_account(
    path: &Path,
    name: &str,
    account_id: u32,
    metadata: Option<&LocalLoginUser>,
    auto_login_user: &str,
) -> Option<LocalAccount> {
    let cfg = path.join("730").join("local").join("cfg");
    let video_path = cfg.join("cs2_video.txt");
    let convars_path = cfg.join("cs2_user_convars_0_slot0.vcfg");
    let video_info = fs::metadata(&video_path)
        .ok()
        .filter(|entry| entry.is_file());
    let convars_info = fs::metadata(&convars_path)
        .ok()
        .filter(|entry| entry.is_file());
    if video_info.is_none() && convars_info.is_none() {
        return None;
    }
    let modified = [video_info, convars_info]
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.modified().ok())
        .max()
        .and_then(modified_timestamp);
    let video = flat_settings(&read_text(&video_path));
    let convars = flat_settings(&read_text(&convars_path));
    let video_value = |key| video.get(key).map(String::as_str);
    let convar_number = |key| {
        convars
            .get(key)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|number| number.is_finite())
    };
    let width =
        video_value("setting.defaultres").and_then(|value| value.trim().parse::<i64>().ok());
    let height =
        video_value("setting.defaultresheight").and_then(|value| value.trim().parse::<i64>().ok());
    let aspect = display_aspect(video_value("setting.aspectratiomode"), width, height);
    let window_mode = if video_value("setting.fullscreen") == Some("1") {
        "fullscreen"
    } else if video_value("setting.nowindowborder") == Some("1") {
        "borderless"
    } else {
        "windowed"
    };
    let account_name = metadata.map_or("", |user| user.account_name.as_str());
    let persona_name = metadata.map_or("", |user| user.persona_name.as_str());
    let most_recent = metadata.is_some_and(|user| user.most_recent);
    let is_current = !auto_login_user.is_empty() && account_name.to_lowercase() == auto_login_user;
    let last_login = metadata.map_or(0, |user| user.login_timestamp);
    let steam_id64 = (STEAM_ID64_BASE + u64::from(account_id)).to_string();
    Some(LocalAccount {
        priority: (is_current, most_recent, last_login, modified),
        value: json!({
            "account_id": name,
            "steam_id64": steam_id64,
            "account_name": account_name,
            "persona_name": persona_name,
            "most_recent": most_recent,
            "is_current": is_current,
            "remember_password": metadata.is_some_and(|user| user.remember_password),
            "last_login_at": (last_login != 0).then(|| utc_iso((last_login, 0))).flatten(),
            "updated_at": modified.and_then(utc_iso),
            "settings": {
                "game_width": width,
                "game_height": height,
                "display_aspect": aspect,
                "window_mode": window_mode,
                "current_sensitivity": convar_number("sensitivity"),
                "m_yaw": convar_number("m_yaw"),
                "zoom_sensitivity_ratio": convar_number("zoom_sensitivity_ratio"),
                "dpi": null,
                "scaling_mode": null,
            },
        }),
    })
}

fn modified_timestamp(time: SystemTime) -> Option<UtcTimestamp> {
    // The original reads st_mtime as floating seconds and rounds to microseconds.
    let seconds = time.duration_since(UNIX_EPOCH).map_or_else(
        |err| -err.duration().as_secs_f64(),
        |elapsed| elapsed.as_secs_f64(),
    );
    if seconds == 0.0 {
        return None;
    }
    let floor = seconds.floor();
    let micros = ((seconds - floor) * 1_000_000.0).round_ties_even() as u32;
    Some((
        floor as i64 + i64::from(micros / 1_000_000),
        micros % 1_000_000,
    ))
}

fn utc_iso((seconds, micros): UtcTimestamp) -> Option<String> {
    const SECONDS_PER_DAY: i64 = 86_400;
    const DAYS_TO_UNIX_EPOCH: i64 = 719_162;
    const DAYS_PER_GREGORIAN_CYCLE: i64 = 146_097;
    let mut days = seconds.div_euclid(SECONDS_PER_DAY) + DAYS_TO_UNIX_EPOCH;
    if !(0..3_652_059).contains(&days) {
        return None;
    }
    let mut year = 1 + 400 * (days / DAYS_PER_GREGORIAN_CYCLE);
    days %= DAYS_PER_GREGORIAN_CYCLE;
    let leap = |year| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    loop {
        let year_days = 365 + i64::from(leap(year));
        if days < year_days {
            break;
        }
        days -= year_days;
        year += 1;
    }
    let mut month = 1;
    for month_days in [
        31,
        28 + i64::from(leap(year)),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ] {
        if days < month_days {
            break;
        }
        days -= month_days;
        month += 1;
    }
    let day = days + 1;
    let clock = seconds.rem_euclid(SECONDS_PER_DAY);
    let (hour, minute, second) = (clock / 3600, clock / 60 % 60, clock % 60);
    let base = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    Some(if micros == 0 {
        format!("{base}+00:00")
    } else {
        format!("{base}.{micros:06}+00:00")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, File, FileTimes},
        io::Write,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, UNIX_EPOCH},
    };

    static FIXTURE_COUNT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let temporary = std::env::temp_dir().canonicalize().unwrap();
            let path = temporary.join(format!(
                "mgs-cs2-settings-{}-{}",
                std::process::id(),
                FIXTURE_COUNT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.0.canonicalize().ok().as_ref() == Some(&self.0) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    fn write(path: &Path, contents: &str, stamp: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = File::create(path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file.set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(stamp)))
            .unwrap();
    }

    fn cfg(root: &Path, user: &str) -> PathBuf {
        root.join("userdata").join(user).join("730/local/cfg")
    }

    fn convars(root: &Path, user: &str, stamp: u64) {
        write(
            &cfg(root, user).join("cs2_user_convars_0_slot0.vcfg"),
            "\"sensitivity\" \"1.00\"\n",
            stamp,
        );
    }

    #[test]
    fn imports_unicode_account_and_all_settings_read_only() {
        let fixture = Fixture::new();
        let root = fixture.0.join("Steam 测试");
        let config = cfg(&root, "123456");
        let video = config.join("cs2_video.txt");
        let convars = config.join("cs2_user_convars_0_slot0.vcfg");
        let login = root.join("config/loginusers.vdf");
        write(
            &video,
            "\"video.cfg\"\n{\n\"setting.defaultres\" \"1024\"\n\"setting.defaultresheight\" \"1080\"\n\"setting.aspectratiomode\" \"0\"\n\"setting.fullscreen\" \"1\"\n}\n",
            1_700_000_000,
        );
        write(
            &convars,
            "\"sensitivity\" \"0.35\"\n\"m_yaw\" \"0.022\"\n\"zoom_sensitivity_ratio\" \"1\"\n",
            1_700_000_100,
        );
        write(
            &login,
            "\u{feff}\"users\"\n{\n\"76561197960389184\"\n{\n\"AccountName\" \"CaseUser\"\n\"PersonaName\" \"青色玩家\"\n\"MostRecent\" \"1\"\n\"RememberPassword\" \"1\"\n\"Timestamp\" \"10\"\n}\n}\n",
            1_700_000_200,
        );
        let paths = [&video, &convars, &login];
        let snapshot = || {
            paths.map(|path| {
                (
                    fs::read(path).unwrap(),
                    fs::metadata(path).unwrap().modified().unwrap(),
                )
            })
        };
        let before = snapshot();
        let result = discover(|_| None, (Some(root), Some("caseuser".into())), None).unwrap();
        assert_eq!(result["found"], true);
        assert_eq!(result["active_account_id"], "123456");
        assert_eq!(
            result["limitations"],
            json!(["dpi_not_in_cs2_cfg", "gpu_scaling_not_in_cs2_cfg"])
        );
        assert_eq!(
            result["accounts"][0],
            json!({
                "account_id": "123456", "steam_id64": "76561197960389184",
                "account_name": "CaseUser", "persona_name": "青色玩家", "most_recent": true,
                "is_current": true, "remember_password": true,
                "last_login_at": "1970-01-01T00:00:10+00:00",
                "updated_at": "2023-11-14T22:15:00+00:00",
                "settings": {
                    "game_width": 1024, "game_height": 1080, "display_aspect": "4:3",
                    "window_mode": "fullscreen", "current_sensitivity": 0.35, "m_yaw": 0.022,
                    "zoom_sensitivity_ratio": 1.0, "dpi": null, "scaling_mode": null,
                }
            })
        );
        assert_eq!(snapshot(), before);
    }

    #[test]
    fn most_recent_user_without_name_wins_over_newer_cfg() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        convars(root, "111", 100);
        convars(root, "222", 200);
        write(
            &root.join("config/loginusers.vdf"),
            "\"users\"\n{\n\"76561197960265839\"\n{\n\"MostRecent\" \"1\"\n}\n\"76561197960265950\"\n{\n\"AutoLogin\" \"1\"\n}\n}\n",
            100,
        );

        let result = discover(|_| None, (Some(root.clone()), None), None).unwrap();
        assert_eq!(result["active_account_id"], "111");
        assert_eq!(result["accounts"][0]["account_name"], "");
        assert_eq!(result["accounts"][0]["most_recent"], true);
        assert_eq!(result["accounts"][1]["most_recent"], false);
    }

    #[test]
    fn honors_path_and_login_overrides_and_invalid_path_fallback() {
        let fixture = Fixture::new();
        let registry = fixture.0.join("registry");
        let configured = fixture.0.join("configured");
        for root in [&registry, &configured] {
            convars(root, "111", 100);
            convars(root, "222", 200);
            write(
                &root.join("config/loginusers.vdf"),
                "\"users\"\n{\n\"76561197960265839\"\n{\n\"AccountName\" \"old\"\n\"MostRecent\" \"1\"\n}\n\"76561197960265950\"\n{\n\"AccountName\" \"current\"\n\"MostRecent\" \"0\"\n}\n}\n",
                100,
            );
        }
        let detected = discover(
            |key| {
                (key == "CS2_INSIGHT_STEAM_PATH")
                    .then(|| fixture.0.join("missing").into_os_string())
            },
            (Some(registry.clone()), Some("current".into())),
            None,
        )
        .unwrap();
        assert_eq!(detected["active_account_id"], "222");
        assert_eq!(detected["accounts"][0]["is_current"], true);
        assert!(detected["steam_root"]
            .as_str()
            .unwrap()
            .ends_with("registry"));

        let detected = discover(
            |key| match key {
                "CS2_INSIGHT_STEAM_PATH" => Some(format!("  {}  ", configured.display()).into()),
                "CS2_INSIGHT_STEAM_AUTO_LOGIN_USER" => Some("  OLD  ".into()),
                _ => None,
            },
            (Some(registry), Some("current".into())),
            None,
        )
        .unwrap();
        assert_eq!(detected["active_account_id"], "111");
        assert_eq!(detected["accounts"][0]["is_current"], true);
        assert!(detected["steam_root"]
            .as_str()
            .unwrap()
            .ends_with("configured"));
    }

    #[test]
    fn falls_back_to_latest_login_then_latest_settings_file() {
        let fixture = Fixture::new();
        convars(&fixture.0, "111", 100);
        convars(&fixture.0, "222", 200);
        convars(&fixture.0, "333", 300);
        write(
            &fixture.0.join("config/loginusers.vdf"),
            "\"users\"\n{\n\"76561197960265839\"\n{\n\"Timestamp\" \"30\"\n}\n\"76561197960265950\"\n{\n\"Timestamp\" \"20\"\n}\n\"76561197960266061\"\n{\n\"Timestamp\" \"20\"\n}\n}\n",
            100,
        );
        let result = discover(|_| None, (Some(fixture.0.clone()), None), None).unwrap();
        let ids: Vec<_> = result["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|account| account["account_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["111", "333", "222"]);
    }

    #[test]
    fn uninstalled_shape_and_empty_userdata_are_distinct() {
        let uninstalled = discover(|_| None, (None, None), None).unwrap();
        assert_eq!(
            uninstalled,
            json!({"found": false, "steam_root": null, "active_account_id": null, "accounts": []})
        );
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("userdata")).unwrap();
        let found_root = discover(|_| None, (Some(fixture.0.clone()), None), None).unwrap();
        assert_eq!(found_root["found"], false);
        assert!(found_root["steam_root"].is_string());
        assert_eq!(found_root["active_account_id"], Value::Null);
        assert_eq!(found_root["accounts"], json!([]));
        assert_eq!(found_root["limitations"].as_array().unwrap().len(), 2);
    }
}
