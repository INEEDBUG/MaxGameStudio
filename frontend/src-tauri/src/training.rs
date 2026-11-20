use maxgamestudio_core::{
    training_db::{TrainingDb, TrainingDbError},
    training_input::{self, InputLabRequest, SensitivityRecommendationRequest},
};
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub(crate) struct TrainingStore(Mutex<bool>);

fn failure(status: u16, detail: impl ToString) -> Value {
    let detail = detail.to_string();
    json!({
        "message": detail,
        "response": { "status": status, "data": { "detail": detail } }
    })
}

fn storage_error(error: impl ToString) -> Value {
    failure(500, format!("无法访问训练记录。{}", error.to_string()))
}

async fn use_store(
    app: AppHandle,
    operation: impl FnOnce(&TrainingDb) -> Result<Value, Value> + Send + 'static,
) -> Result<Value, Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = super::writable_data_root(&app)
            .map_err(storage_error)?
            .join("cs2-insight.db");
        let database = TrainingDb::new(path);
        let store = app.state::<TrainingStore>();
        {
            let mut initialized = store
                .0
                .lock()
                .map_err(|_| storage_error("训练记录初始化中断，请重新打开软件。"))?;
            if !*initialized {
                database.init_tables().map_err(storage_error)?;
                *initialized = true;
            }
        }
        operation(&database)
    })
    .await
    .map_err(storage_error)?
}

async fn list_history(
    app: AppHandle,
    limit: i64,
    list: fn(&TrainingDb, i64) -> Result<Vec<Value>, TrainingDbError>,
) -> Result<Value, Value> {
    if !(1..=100).contains(&limit) {
        return Err(failure(422, "查询条数必须在 1 到 100 之间。"));
    }
    use_store(app, move |database| {
        list(database, limit)
            .map(|items| json!({ "items": items }))
            .map_err(storage_error)
    })
    .await
}

#[tauri::command]
pub(crate) async fn save_sensitivity_recommendation(
    app: AppHandle,
    request: SensitivityRecommendationRequest,
) -> Result<Value, Value> {
    let result = training_input::recommend_sensitivity(&request)
        .map_err(|error| failure(422, error.message))?;
    use_store(app, move |database| {
        database
            .save_sensitivity_session(&request, &result)
            .map_err(storage_error)
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_sensitivity_history(app: AppHandle, limit: i64) -> Result<Value, Value> {
    list_history(app, limit, TrainingDb::list_sensitivity_sessions).await
}

#[tauri::command]
pub(crate) async fn save_input_analysis(
    app: AppHandle,
    request: InputLabRequest,
) -> Result<Value, Value> {
    let result = training_input::analyze_input_session(&request)
        .map_err(|error| failure(422, error.message))?;
    use_store(app, move |database| {
        database
            .save_input_session(&request, &result)
            .map_err(storage_error)
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_input_history(app: AppHandle, limit: i64) -> Result<Value, Value> {
    list_history(app, limit, TrainingDb::list_input_sessions).await
}
