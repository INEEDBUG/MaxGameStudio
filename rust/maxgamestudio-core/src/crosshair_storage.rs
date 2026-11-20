use crate::crosshair::{encode_crosshair, CrosshairError};
use serde_json::{json, Value};
use std::{fs, io, path::Path};

const PROFILE_NAME: &str = "valorant-crosshair-profiles.json";

#[derive(Debug)]
pub enum ProfileSaveError {
    Validation(CrosshairError),
    Storage(io::Error),
}

pub fn load_profiles(data_root: &Path) -> Result<Value, CrosshairError> {
    let stored = fs::read(data_root.join(PROFILE_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let profiles = stored
        .filter(|stored| stored["profiles"].is_object())
        .map(|mut stored| stored["profiles"].take())
        .unwrap_or_else(|| json!({}));
    if profiles
        .as_object()
        .is_some_and(|object| !object.is_empty())
    {
        let code = encode_crosshair(&profiles)?.code;
        return Ok(json!({ "profiles": profiles, "code": code }));
    }
    Ok(json!({ "profiles": profiles }))
}

pub fn save_profiles(data_root: &Path, profiles: Value) -> Result<Value, ProfileSaveError> {
    let encoded = encode_crosshair(&profiles).map_err(ProfileSaveError::Validation)?;
    let mut payload = json!({
        "profiles": profiles,
        "code": encoded.code,
        "format": encoded.format,
    });
    publish(data_root, &payload).map_err(ProfileSaveError::Storage)?;
    payload["saved"] = json!(true);
    Ok(payload)
}

fn publish(data_root: &Path, payload: &Value) -> io::Result<()> {
    fs::create_dir_all(data_root)?;
    let mut next = tempfile::NamedTempFile::new_in(data_root)?;
    serde_json::to_writer_pretty(next.as_file_mut(), payload)?;
    next.as_file().sync_all()?;
    next.persist(data_root.join(PROFILE_NAME))
        .map_err(|error| error.error)?;
    Ok(())
}
