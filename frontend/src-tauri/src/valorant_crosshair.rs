use maxgamestudio_core::crosshair::{self, CrosshairError, DecodedCrosshair, EncodedCrosshair};
use maxgamestudio_core::crosshair_storage::{self, ProfileSaveError};
use serde_json::{json, Value};
use tauri::AppHandle;

fn validation_error(error: CrosshairError) -> Value {
    let detail = error.to_string();
    json!({
        "message": detail,
        "response": { "status": 422, "data": { "detail": detail } }
    })
}

fn storage_error(error: impl ToString) -> Value {
    let detail = error.to_string();
    json!({
        "message": detail,
        "response": { "status": 500, "data": { "detail": detail } }
    })
}

#[tauri::command]
pub(crate) fn encode_valorant_crosshair(profiles: Value) -> Result<EncodedCrosshair, Value> {
    crosshair::encode_crosshair(&profiles).map_err(validation_error)
}

#[tauri::command]
pub(crate) fn decode_valorant_crosshair(code: String) -> Result<DecodedCrosshair, Value> {
    crosshair::decode_crosshair(&code).map_err(validation_error)
}

#[tauri::command]
pub(crate) async fn load_valorant_crosshair(app: AppHandle) -> Result<Value, Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = super::writable_data_root(&app).map_err(storage_error)?;
        crosshair_storage::load_profiles(&root).map_err(validation_error)
    })
    .await
    .map_err(storage_error)?
}

#[tauri::command]
pub(crate) async fn save_valorant_crosshair(
    app: AppHandle,
    profiles: Value,
) -> Result<Value, Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = super::writable_data_root(&app).map_err(storage_error)?;
        crosshair_storage::save_profiles(&root, profiles).map_err(|error| match error {
            ProfileSaveError::Validation(error) => validation_error(error),
            ProfileSaveError::Storage(error) => storage_error(error),
        })
    })
    .await
    .map_err(storage_error)?
}
