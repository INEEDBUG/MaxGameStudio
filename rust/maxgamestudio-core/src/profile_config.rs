//! Retain the installed profile's JSON fields, including extensions unknown to this build.

mod schema;

use crate::profile_platform::{
    detect_cs2_path, effective_locale, expand_user, ffmpeg_on_path, path_text, resolved_path,
};
use schema::{normalize_config, validate_patch, Object, DEFAULTS};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::{Builder, NamedTempFile};

const CONFIG_NAME: &str = "cs2-insight.config.json";
const EXAMPLE_NAME: &str = "cs2-insight.config.example.json";
const LAUNCH_ARGS: &str = "cs2_extra_launch_args";
const LAUNCH_CONFIGURED: &str = "cs2_extra_launch_args_user_configured";
const CONSOLE_LINES: &str = "record_inject_console_lines";
const CONSOLE_CONFIGURED: &str = "record_inject_console_lines_user_configured";
const NESTED_MODELS: &[&str] = &["obs", "llm", "experimental", "spec_player_verify"];

pub(crate) struct ProfileConfig {
    data_dir: PathBuf,
    path: PathBuf,
    example_path: PathBuf,
    legacy_path: Option<PathBuf>,
}

struct LoadedConfig {
    raw: Object,
    known: Object,
    write_required: bool,
    invalid_file: Option<Vec<u8>>,
}

pub(crate) enum ConfigUpdateError {
    Validation(Vec<Value>),
    Storage(io::Error),
}

impl ProfileConfig {
    pub(crate) fn new(
        data_dir: PathBuf,
        config_path: Option<PathBuf>,
        bundle_data_dir: Option<PathBuf>,
        legacy_path: Option<PathBuf>,
    ) -> io::Result<Self> {
        let data_dir = profile_path(data_dir)?;
        let path = profile_path(config_path.unwrap_or_else(|| data_dir.join(CONFIG_NAME)))?;
        let example_path = match bundle_data_dir {
            Some(bundle) => profile_path(bundle)?.join(EXAMPLE_NAME),
            None => data_dir.join(EXAMPLE_NAME),
        };
        let legacy_path = legacy_path.map(profile_path).transpose()?;
        Ok(Self {
            data_dir,
            path,
            example_path,
            legacy_path,
        })
    }

    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub(crate) fn bundle_data_dir(&self) -> &Path {
        self.example_path.parent().expect("bundled example parent")
    }

    pub(crate) fn database_path(&self) -> PathBuf {
        self.path.with_file_name("cs2-insight.db")
    }

    pub(crate) fn expected_demo_players(&self) -> io::Result<Vec<String>> {
        let config = self.load()?;
        Ok(config.known["expected_parse_players"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect())
    }

    pub(crate) fn lite_cut_assets_dir(&self) -> io::Result<PathBuf> {
        let stored = self.load()?;
        let configured = stored.known["lite_cut_assets_dir"]
            .as_str()
            .expect("normalized asset directory")
            .trim();
        if configured.is_empty() {
            Ok(self.data_dir.join("lite_cut_assets"))
        } else {
            crate::profile_platform::expand_user(configured)
        }
    }

    pub(crate) fn lite_cut_proxy_resolution(&self) -> io::Result<u64> {
        let stored = self.load()?;
        let edge = stored.known["lite_cut_proxy_resolution"].as_i64().unwrap_or(720);
        Ok(if edge == 0 { 720 } else { edge.clamp(360, 2160) as u64 })
    }

    pub(crate) fn obs_settings(&self) -> io::Result<crate::obs_wire::ObsSettings> {
        self.obs_profile().map(|(settings, _)| settings)
    }

    pub(crate) fn obs_profile(&self) -> io::Result<(crate::obs_wire::ObsSettings, String)> {
        let stored = self.load()?;
        let (settings, program) = obs_fields(&stored.known["obs"])?;
        Ok((settings, program.to_owned()))
    }

    pub(crate) fn mark_obs_verified(
        &self,
        checked: &crate::obs_wire::ObsSettings,
        checked_program: &str,
    ) -> io::Result<()> {
        let mut stored = self.load()?;
        let obs = stored
            .known
            .get_mut("obs")
            .expect("normalized OBS configuration");
        let (current, program) = obs_fields(obs)?;
        if &current == checked && program == checked_program && obs["obs_config_verified"] != true {
            // PUT /config uses the legacy whole-submodel replacement contract.
            // Update just this flag so a successful test cannot reset connection settings.
            obs["obs_config_verified"] = json!(true);
            stored.write_required = true;
            self.publish(stored)?;
        }
        Ok(())
    }

    pub(crate) fn view(&self) -> io::Result<Value> {
        let mut data = self.read(true)?;
        mask(&mut data["llm"]["api_key"], 0, 4, "****");
        mask(&mut data["obs"]["password"], 5, 4, "****");
        mask(&mut data["steam_api_key"], 4, 4, "****");
        mask(&mut data["steam_game_auth_code"], 4, 4, "****");
        mask(
            &mut data["steam_known_share_code"],
            0,
            5,
            "CSGO-****-****-****-****-",
        );
        data.as_object_mut()
            .unwrap()
            .remove("steam_match_share_codes");
        data["effective_locale"] = json!(effective_locale(data["locale"].as_str().unwrap()));
        Ok(data)
    }

    pub(crate) fn quick_check(&self) -> io::Result<Value> {
        let data = self.read(true)?;
        let cs2 = data["cs2_path"].as_str().unwrap();
        let ffmpeg = data["ffmpeg_path"].as_str().unwrap();
        let key = data["llm"]["api_key"].as_str().unwrap().trim();
        let key_saved = !key.is_empty() && key != "****";
        let local_llm = data["llm"]["base_url"].as_str().is_some_and(local_llm_url);

        Ok(json!({
            "obs_configured": data["obs"]["obs_config_verified"],
            "cs2_path_ok": !cs2.is_empty() && Path::new(cs2).is_file(),
            "ffmpeg_ok": if ffmpeg.is_empty() { ffmpeg_on_path() } else { Path::new(ffmpeg).is_file() },
            "ai_key_ok": key_saved || local_llm,
            "cs2_path": cs2,
            "ffmpeg_path": ffmpeg,
        }))
    }

    pub(crate) fn ffmpeg_check(&self) -> io::Result<Value> {
        let data = self.read(false)?;
        let raw = data["ffmpeg_path"].as_str().unwrap().trim();
        if raw.is_empty() {
            return Ok(json!({"ok": false, "reason": "not_configured", "ffmpeg_path": ""}));
        }
        if !Path::new(raw).is_file() {
            return Ok(json!({"ok": false, "reason": "path_not_found", "ffmpeg_path": raw}));
        }
        let result = match expand_user(raw).and_then(|path| resolved_path(&path)) {
            Ok(path) => json!({"ok": true, "ffmpeg_path": path_text(&path)}),
            Err(_) => json!({"ok": false, "reason": "not_usable", "ffmpeg_path": raw}),
        };
        Ok(result)
    }

    pub(crate) fn ffmpeg_executable(&self) -> io::Result<PathBuf> {
        let stored = self.load()?;
        let configured = stored
            .known
            .get("ffmpeg_path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let path = if configured.is_empty() {
            let bundled = self
                .data_dir
                .parent()
                .map(|root| root.join("third_party/ffmpeg/ffmpeg.exe"));
            bundled
                .filter(|path| path.is_file())
                .or_else(crate::profile_platform::ffmpeg_from_path)
        } else {
            Some(expand_user(configured)?)
        };
        let path = path.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "未找到 FFmpeg"))?;
        #[cfg(windows)]
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "请选择 FFmpeg 的 .exe 文件",
            ));
        }
        path.canonicalize()
    }

    pub(crate) fn update(&self, input: Value) -> Result<(), ConfigUpdateError> {
        let patch = validate_patch(input).map_err(ConfigUpdateError::Validation)?;
        let mut stored = self.load().map_err(ConfigUpdateError::Storage)?;
        let mut known = Value::Object(stored.known.clone());
        apply_patch(&mut known, &patch).map_err(ConfigUpdateError::Storage)?;
        normalize_recording_defaults(&mut known);
        let next = std::mem::take(known.as_object_mut().unwrap());
        stored.write_required |= next != stored.known;
        stored.known = next;
        self.publish(stored).map_err(ConfigUpdateError::Storage)?;
        Ok(())
    }

    fn read(&self, ensure_cs2: bool) -> io::Result<Value> {
        let mut stored = self.load()?;
        if ensure_cs2 && stored.known["cs2_path"] == "" {
            if let Some(path) = detect_cs2_path() {
                stored
                    .known
                    .insert("cs2_path".to_owned(), json!(path_text(&path)));
                stored.write_required = true;
            }
        }
        self.publish(stored)
    }

    fn load(&self) -> io::Result<LoadedConfig> {
        let (raw, mut write_required, migrate_legacy, invalid_file) =
            match read_optional(&self.path)? {
                Some(bytes) => match parse_file(&bytes) {
                    Ok((raw, trailing)) => (raw, trailing, true, None),
                    Err(_) => (self.example_seed()?, true, false, Some(bytes)),
                },
                None => (self.seed()?, true, false, None),
            };

        let fields = normalize_config(&raw).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid installed configuration fields",
            )
        })?;
        let mut known = Value::Object(fields);
        if migrate_legacy {
            // Only apply the missing-marker inference to primary files already written by an old build.
            write_required |= migrate_recording_defaults(&raw, &mut known)?;
        }
        write_required |= normalize_recording_defaults(&mut known);
        Ok(LoadedConfig {
            raw,
            write_required,
            invalid_file,
            known: std::mem::take(known.as_object_mut().unwrap()),
        })
    }

    fn seed(&self) -> io::Result<Object> {
        if let Some(legacy) = self.legacy_path.as_deref() {
            if let Some(bytes) = read_optional(legacy)? {
                return parse_file(&bytes).map(|(raw, _)| raw);
            }
        }
        self.example_seed()
    }

    fn example_seed(&self) -> io::Result<Object> {
        read_optional(&self.example_path)?
            .map(|bytes| parse_file(&bytes).map(|(raw, _)| raw))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn publish(&self, mut stored: LoadedConfig) -> io::Result<Value> {
        if stored.write_required {
            let parent = self.path.parent().expect("absolute configuration path");
            fs::create_dir_all(parent)?;
            if let Some(bytes) = stored.invalid_file {
                // Keep the original bytes on disk before replacing the live file.
                let stem = self.path.file_stem().unwrap_or_default().to_string_lossy();
                let mut backup = Builder::new()
                    .prefix(&format!("{stem}.invalid-"))
                    .suffix(".json")
                    .tempfile_in(parent)?;
                backup.write_all(&bytes)?;
                backup.as_file().sync_all()?;
                backup.keep().map_err(|error| error.error)?;
            }
            for (key, known) in &stored.known {
                if NESTED_MODELS.contains(&key.as_str()) {
                    if let Some(unknown) = stored.raw.get_mut(key).and_then(Value::as_object_mut) {
                        unknown.extend(known.as_object().unwrap().clone());
                        continue;
                    }
                }
                stored.raw.insert(key.clone(), known.clone());
            }
            let mut next = NamedTempFile::new_in(parent)?;
            serde_json::to_writer_pretty(next.as_file_mut(), &stored.raw)?;
            next.as_file().sync_all()?;
            next.persist(&self.path).map_err(|error| error.error)?;
        }
        Ok(Value::Object(stored.known))
    }
}

fn obs_fields(obs: &Value) -> io::Result<(crate::obs_wire::ObsSettings, &str)> {
    let program = obs["obs_path"]
        .as_str()
        .expect("normalized OBS executable")
        .trim();
    let settings = serde::Deserialize::deserialize(obs)?;
    Ok((settings, program))
}

fn profile_path(path: PathBuf) -> io::Result<PathBuf> {
    if let Some(home_path) = path.to_str().filter(|text| text.starts_with('~')) {
        return std::path::absolute(expand_user(home_path)?);
    }
    std::path::absolute(path)
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn parse_file(bytes: &[u8]) -> io::Result<(Object, bool)> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .trim();
    if text.is_empty() {
        return Ok((Object::new(), false));
    }
    let mut parser = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let value = parser.next().expect("nonempty configuration text")?;
    let trailing = !text[parser.byte_offset()..].trim().is_empty();
    let object = match value {
        Value::Object(object) => object,
        _ => Object::new(),
    };
    Ok((object, trailing))
}

fn migrate_recording_defaults(raw: &Object, known: &mut Value) -> io::Result<bool> {
    let mut migrated = !raw.contains_key(LAUNCH_CONFIGURED);
    if !raw.contains_key(CONSOLE_CONFIGURED) {
        known[CONSOLE_CONFIGURED] =
            json!(!known[CONSOLE_LINES].as_str().unwrap().trim().is_empty());
        migrated = true;
    }
    if !raw
        .get("overlay_offsets_independent")
        .is_some_and(truthy_flag)
    {
        let parse_offset = |value: &Value| {
            value
                .as_i64()
                .map(i128::from)
                .or_else(|| value.as_u64().map(i128::from))
                .unwrap_or_default()
        };
        let kb = parse_offset(&known["kb_overlay_tick_offset"]);
        let fx = raw
            .get("kill_fx_tick_offset")
            .map(|_| parse_offset(&known["kill_fx_tick_offset"]))
            .unwrap_or_default();
        let combined = kb + fx;
        known["kill_fx_tick_offset"] = i64::try_from(combined)
            .map(|value| json!(value))
            .or_else(|_| u64::try_from(combined).map(|value| json!(value)))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "overlay offset overflow"))?;
        known["overlay_offsets_independent"] = json!(true);
        migrated = true;
    }
    Ok(migrated)
}

fn truthy_flag(flag: &Value) -> bool {
    match flag {
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        _ => false,
    }
}

fn normalize_recording_defaults(known: &mut Value) -> bool {
    let mut changed = false;
    if known[LAUNCH_CONFIGURED] == false {
        let current = known[LAUNCH_ARGS].as_str().unwrap();
        let args = current.trim();
        let next = if args.is_empty() {
            "-fullscreen".to_owned()
        } else if args
            .split_whitespace()
            .any(|arg| arg.eq_ignore_ascii_case("-fullscreen"))
        {
            args.to_owned()
        } else {
            format!("{args}\n-fullscreen")
        };
        if next != current {
            known[LAUNCH_ARGS] = json!(next);
            changed = true;
        }
    }
    if known[CONSOLE_CONFIGURED] == false && known[CONSOLE_LINES] != DEFAULTS[CONSOLE_LINES] {
        known[CONSOLE_LINES] = DEFAULTS[CONSOLE_LINES].clone();
        changed = true;
    }
    changed
}

fn apply_patch(known: &mut Value, patch: &Object) -> io::Result<()> {
    for (name, value) in patch {
        let text = || value.as_str().expect("validated text");
        match name.as_str() {
            "obs" => {
                known["obs"]["host"] = value["host"].clone();
                known["obs"]["port"] = value["port"].clone();
                known["obs"]["obs_path"] = json!(value["obs_path"].as_str().unwrap().trim());
                let password = value["password"].as_str().unwrap().trim();
                if !password.is_empty() && !password.starts_with("****") {
                    known["obs"]["password"] = json!(password);
                }
            }
            "llm" => {
                let key = value["api_key"].as_str().unwrap();
                if !key.is_empty() && !key.starts_with("****") {
                    known["llm"] = value.clone();
                } else {
                    known["llm"]["provider"] = value["provider"].clone();
                    known["llm"]["model"] = value["model"].clone();
                    if !value["base_url"].is_null() {
                        known["llm"]["base_url"] = value["base_url"].clone();
                    }
                }
            }
            "experimental" => {
                if let Some(enabled) = value.get("pov_enabled") {
                    known["experimental"]["pov_enabled"] = enabled.clone();
                }
            }
            "demo_watch_paths" => {
                let paths = value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap())
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| expand_user(value).map(|path| path_text(&path)))
                    .collect::<io::Result<Vec<_>>>()?;
                known[name] = json!(paths);
            }
            "expected_parse_players" => {
                let mut seen = HashSet::new();
                let players = value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|player| player.as_str().unwrap().trim())
                    .filter(|player| !player.is_empty() && seen.insert(*player))
                    .take(50)
                    .collect::<Vec<_>>();
                known[name] = json!(players);
            }
            "locale" => {
                if matches!(
                    text(),
                    "auto" | "zh" | "zh-HK" | "zh-TW" | "en" | "ms-MY" | "ru-RU"
                ) {
                    known[name] = value.clone();
                }
            }
            "match_mode" => {
                if matches!(text(), "premier" | "competitive") {
                    known[name] = value.clone();
                }
            }
            "match_count" => {
                if matches!(value.as_i64(), Some(20 | 50 | 100)) {
                    known[name] = value.clone();
                }
            }
            "kb_overlay_position" => {
                if matches!(text(), "bottom_center" | "minimap_below" | "weapon_right") {
                    known[name] = value.clone();
                }
            }
            "ffmpeg_path" | "last_update_check_at" => known[name] = json!(text().trim()),
            "montage_encoder" => {
                let lower = text().trim().to_lowercase();
                known[name] = json!(if lower.is_empty() { "auto" } else { &lower });
            }
            "obs_transition_name" => {
                let trimmed = text().trim();
                known[name] = json!(if trimmed.is_empty() { "Fade" } else { trimmed });
            }
            "obs_transition_duration_ms" => {
                known[name] = if value.as_i64().is_some_and(|value| value < 0) {
                    json!(0)
                } else {
                    value.clone()
                };
            }
            "update_check_frequency" => {
                let frequency = text().trim().to_lowercase();
                if matches!(frequency.as_str(), "weekly" | "monthly" | "never") {
                    known[name] = json!(frequency);
                }
            }
            "steam_api_key" | "steam_game_auth_code" => {
                if !text().is_empty() && !text().starts_with("****") {
                    known[name] = json!(text().trim());
                }
            }
            "steam_id64" => {
                if !text().is_empty() {
                    known[name] = json!(text().trim());
                }
            }
            "steam_known_share_code" => {
                if !text().is_empty() && !text().starts_with("CSGO-****") {
                    let next = text().trim();
                    if known[name] != next {
                        known["steam_match_share_codes"] = json!([]);
                    }
                    known[name] = json!(next);
                }
            }
            LAUNCH_ARGS | CONSOLE_LINES => {
                if known[name] != *value {
                    let marker = if name == LAUNCH_ARGS {
                        LAUNCH_CONFIGURED
                    } else {
                        CONSOLE_CONFIGURED
                    };
                    known[marker] = patch.get(marker).cloned().unwrap_or(json!(true));
                }
                known[name] = value.clone();
            }
            _ => known[name] = value.clone(),
        }
    }
    if let Some(action) = patch.get("close_action") {
        known["close_to_tray"] = json!(action != "exit");
    }
    Ok(())
}

fn mask(value: &mut Value, minimum: usize, suffix_len: usize, prefix: &str) {
    let text = value.as_str().unwrap();
    if text.is_empty() {
        return;
    }
    let suffix = if text.chars().count() < minimum {
        ""
    } else {
        let offset = text
            .char_indices()
            .rev()
            .nth(suffix_len - 1)
            .map_or(0, |(index, _)| index);
        &text[offset..]
    };
    *value = json!(format!("{prefix}{suffix}"));
}

fn local_llm_url(url: &str) -> bool {
    let url = if url.contains("://") {
        url.to_owned()
    } else {
        format!("http://{url}")
    };
    url.parse::<hyper::Uri>()
        .ok()
        .and_then(|url| url.host().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
                || host.ends_with(".localhost")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_an_explicit_legacy_source_then_keeps_both_seeds_read_only() {
        let scratch = tempfile::tempdir().unwrap();
        let data = scratch.path().join("data");
        let bundle = scratch.path().join("bundle");
        fs::create_dir(&bundle).unwrap();
        let example = bundle.join(EXAMPLE_NAME);
        let example_bytes = br#"{"locale":"ru-RU"}"#;
        fs::write(&example, example_bytes).unwrap();
        let legacy = scratch.path().join("backend-config.json");
        let legacy_bytes = br#"{"locale":"ms-MY","future":{"value":3}} trailing data"#;
        fs::write(&legacy, legacy_bytes).unwrap();
        let config =
            ProfileConfig::new(data.clone(), None, Some(bundle), Some(legacy.clone())).unwrap();

        assert_eq!(config.read(false).unwrap()["locale"], "ms-MY");
        assert_eq!(fs::read(&example).unwrap(), example_bytes);
        assert_eq!(fs::read(&legacy).unwrap(), legacy_bytes);
        let destination = data.join(CONFIG_NAME);
        let persisted: Value = serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
        assert_eq!(persisted["future"], json!({"value": 3}));

        let invalid = b"\xff{ synthetically interrupted JSON";
        fs::write(&destination, invalid).unwrap();
        assert_eq!(config.read(false).unwrap()["locale"], "ru-RU");
        let saved = fs::read(&destination).unwrap();
        assert_eq!(config.read(false).unwrap()["locale"], "ru-RU");
        assert_eq!(fs::read(destination).unwrap(), saved);
        let invalid_files = fs::read_dir(&data)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("cs2-insight.config.invalid-")
            })
            .collect::<Vec<_>>();
        assert_eq!(invalid_files.len(), 1);
        assert_eq!(fs::read(invalid_files[0].path()).unwrap(), invalid);
        assert_eq!(fs::read(example).unwrap(), example_bytes);
        assert_eq!(fs::read(legacy).unwrap(), legacy_bytes);
    }

    #[test]
    fn preserves_a_config_with_invalid_known_fields_and_repairs_only_valid_trailing_content() {
        let scratch = tempfile::tempdir().unwrap();
        let config = ProfileConfig::new(scratch.path().to_owned(), None, None, None).unwrap();
        let path = scratch.path().join(CONFIG_NAME);
        let bad_fields = br#"{"obs":{"port":"invalid"},"future":true}"#;
        fs::write(&path, bad_fields).unwrap();
        assert_eq!(
            config.read(false).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&path).unwrap(), bad_fields);
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);

        fs::write(
            &path,
            br#"{"kb_overlay_tick_offset":9,"kill_fx_tick_offset":-4,"future":true} {}"#,
        )
        .unwrap();
        assert_eq!(config.read(false).unwrap()["kill_fx_tick_offset"], 5);
        let repaired = fs::read(&path).unwrap();
        let persisted: Value = serde_json::from_slice(&repaired).unwrap();
        assert_eq!(persisted["future"], true);
        assert_eq!(persisted["overlay_offsets_independent"], true);
        assert_eq!(config.read(false).unwrap()["kill_fx_tick_offset"], 5);
        assert_eq!(fs::read(&path).unwrap(), repaired);
    }

    #[test]
    fn local_llm_hosts_include_loopback_and_subdomains() {
        assert!(local_llm_url("localhost:9412/api"));
        assert!(local_llm_url("http://[::1]:9412/v1"));
        assert!(local_llm_url("https://setup.localhost/api"));
        assert!(!local_llm_url("https://localhost.example.invalid"));
    }
}
