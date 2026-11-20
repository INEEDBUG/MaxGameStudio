use super::{
    demo_body::{field_issue, model},
    detail, json_reply, model_body, profile_io_error, validation_reply, HttpResponse, ProfileState,
};
use crate::profile_platform::path_text;
use hyper::{body::Incoming, Request, StatusCode};
use serde_json::{json, Value};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) async fn open(
    profile: &ProfileState,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let value = match model_body(request, "Demo path list is too large").await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let paths = match parse_paths(value) {
        Ok(paths) => paths,
        Err(errors) => return validation_reply(errors),
    };
    let Some(parser) = profile.parser_worker.as_ref() else {
        return detail(StatusCode::SERVICE_UNAVAILABLE, "未找到原生 Demo 解析组件");
    };
    let Ok(_permit) = profile.demo_parser.try_acquire() else {
        return detail(
            StatusCode::SERVICE_UNAVAILABLE,
            "正在处理其他 Demo，请稍后重试",
        );
    };
    extended_request.store(true, Ordering::Relaxed);
    let (prepared, mut failed) = match tokio::task::spawn_blocking(move || {
        let mut valid = Vec::new();
        let mut failed = Vec::new();
        for path in paths {
            match prepare_path(path) {
                Ok(item) => valid.push(item),
                Err(error) => failed.push(error),
            }
        }
        (valid, failed)
    })
    .await
    {
        Ok(paths) => paths,
        Err(error) => return profile_io_error(&error),
    };
    let mut uploads = Vec::new();
    for (path, filename) in prepared {
        match parser.inspect_demo(&path).await {
            Ok(mut result)
                if result["players"].as_array().is_some_and(|x| !x.is_empty())
                    && result["match_meta"].is_object() =>
            {
                uploads.push(json!({
                    "filename": filename, "path": path_text(&path), "uploaded_path": null,
                    "source_scope": "original", "compatibility": null,
                    "players": result["players"].take(), "match_meta": result["match_meta"].take(),
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                failed.push(failure(&filename, "DEMO_INSPECTION_TIMEOUT"));
            }
            _ => failed.push(failure(&filename, "DEMO_INSPECTION_FAILED")),
        }
    }
    json_reply(
        StatusCode::OK,
        json!({"uploads": uploads, "failed": failed}),
    )
}

fn parse_paths(input: Value) -> Result<Vec<String>, Vec<Value>> {
    let object = model(&input)?;
    let raw = object.get("paths").ok_or_else(|| {
        vec![field_issue(
            "paths",
            &input,
            "missing",
            "Field required",
            None,
        )]
    })?;
    let items = raw.as_array().ok_or_else(|| {
        vec![field_issue(
            "paths",
            raw,
            "list_type",
            "Input should be a valid list",
            None,
        )]
    })?;
    if items.len() > 100 || items.is_empty() {
        let (kind, message, context) = if items.is_empty() {
            (
                "too_short",
                "List should have at least 1 item after validation, not 0".to_owned(),
                json!({"field_type":"List","min_length":1,"actual_length":0}),
            )
        } else {
            (
                "too_long",
                format!(
                    "List should have at most 100 items after validation, not {}",
                    items.len()
                ),
                json!({"field_type":"List","max_length":100,"actual_length":items.len()}),
            )
        };
        return Err(vec![field_issue(
            "paths",
            raw,
            kind,
            &message,
            Some(context),
        )]);
    }
    let (mut paths, mut errors) = (Vec::with_capacity(items.len()), Vec::new());
    for (index, value) in items.iter().enumerate() {
        if let Some(path) = value.as_str() {
            paths.push(path.to_owned());
        } else {
            errors.push(json!({"type":"string_type","loc":["body","paths",index],"msg":"Input should be a valid string","input":value}));
        }
    }
    if errors.is_empty() {
        Ok(paths)
    } else {
        Err(errors)
    }
}

fn prepare_path(raw: String) -> Result<(PathBuf, String), Value> {
    let path = Path::new(&raw);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&raw);
    let path = fs::canonicalize(path).map_err(|error| {
        let code = if error.kind() == io::ErrorKind::NotFound {
            "DEMO_FILE_NOT_FOUND"
        } else {
            "DEMO_PREPARE_FAILED"
        };
        failure(name, code)
    })?;
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dem"))
    {
        return Err(failure(name, "DEMO_PREPARE_FAILED"));
    }
    let filename = path
        .file_name()
        .expect("regular Demo file")
        .to_string_lossy()
        .into_owned();
    Ok((path, filename))
}

fn failure(filename: &str, code: &str) -> Value {
    json!({"filename": if filename.is_empty() {"Demo"} else { filename }, "code":code})
}
