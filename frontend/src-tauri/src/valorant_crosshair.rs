use maxgamestudio_core::crosshair::{self, CrosshairError, DecodedCrosshair, EncodedCrosshair};
use serde_json::{json, Value};

fn validation_error(error: CrosshairError) -> Value {
    let detail = error.to_string();
    json!({
        "message": detail,
        "response": { "status": 422, "data": { "detail": detail } }
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
