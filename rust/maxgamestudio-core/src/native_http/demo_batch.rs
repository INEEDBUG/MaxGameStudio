use super::{
    demo_body::{field_issue, model},
    demo_read_error,
    demo_roster::{load_players, RosterError},
    json_reply, model_body, profile_io_error, profile_work, validation_reply, HttpResponse,
    ProfileState,
};
use crate::{
    case_fold::case_fold,
    demo_library::{BatchRosterSource, DemoLibrary, RosterCache},
    wire_json::integer_from_text,
};
use hyper::{body::Incoming, Request, StatusCode};
use serde_json::{json, Map, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::AtomicBool, Arc},
};

enum Names {
    None,
    Saved,
    Manual(Vec<String>),
}

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
    summary: bool,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let input = match model_body(request, "Demo batch request is too large").await {
        Ok(input) => input,
        Err(error) => return *error,
    };
    let (ids, mode) = match parse(&input, summary) {
        Ok(batch) => batch,
        Err(errors) => return validation_reply(errors),
    };
    let names = match mode {
        None => None,
        Some(Names::None) => return empty_matches(ids),
        Some(Names::Manual(names)) => Some(names),
        Some(Names::Saved) => {
            let names =
                match profile_work(profile, |profile| profile.config.expected_demo_players()).await
                {
                    Ok(Ok(names)) => names,
                    Ok(Err(error)) => return profile_io_error(&error),
                    Err(error) => return profile_io_error(&error),
                };
            let mut seen = HashSet::new();
            let names: Vec<_> = names
                .iter()
                .map(|name| name.trim())
                .filter(|name| !name.is_empty() && seen.insert(*name))
                .take(50)
                .map(str::to_owned)
                .collect();
            if names.is_empty() {
                return empty_matches(ids);
            }
            Some(names)
        }
    };
    let lookup = ids.clone();
    let mut library = match profile_work(profile, move |profile| {
        DemoLibrary::new(profile.config.database_path()).batch_rosters(&lookup, summary)
    })
    .await
    {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => return demo_read_error(&error),
        Err(error) => return profile_io_error(&error),
    };
    let mut seen = HashSet::new();
    let repeat: HashSet<_> = ids.iter().copied().filter(|id| !seen.insert(*id)).collect();
    let mut repeated = HashMap::<i64, Result<Value, Value>>::new();
    let mut items = Vec::new();
    let mut resolved = Map::new();
    let mut failed = Vec::new();
    for id in ids {
        let result = if let Some(previous) = repeated.get(&id) {
            previous.clone()
        } else {
            let result = load_item(
                profile,
                id,
                library.remove(&id),
                names.as_deref(),
                extended_request,
            )
            .await;
            if repeat.contains(&id) {
                repeated.insert(id, result.clone());
            }
            result
        };
        match result {
            Ok(item) if summary => items.push(item),
            Ok(names) => {
                resolved.insert(id.to_string(), names);
            }
            Err(error) => {
                if !summary {
                    resolved.insert(id.to_string(), json!([]));
                }
                failed.push(error);
            }
        }
    }
    let response = if summary {
        json!({"items":items,"failed":failed})
    } else {
        json!({"resolved":resolved,"failed":failed})
    };
    json_reply(StatusCode::OK, response)
}

fn empty_matches(ids: Vec<i64>) -> HttpResponse {
    let resolved = ids
        .into_iter()
        .map(|id| (id.to_string(), json!([])))
        .collect::<Map<_, _>>();
    json_reply(StatusCode::OK, json!({"resolved":resolved,"failed":[]}))
}

fn parse(input: &Value, summary: bool) -> Result<(Vec<i64>, Option<Names>), Vec<Value>> {
    let body = model(input)?;
    let (id_field, maximum) = if summary {
        ("ids", 100)
    } else {
        ("demo_ids", 200)
    };
    let array = match body.get(id_field) {
        Some(Value::Array(ids)) => ids,
        Some(value) => {
            return Err(vec![field_issue(
                id_field,
                value,
                "list_type",
                "Input should be a valid list",
                None,
            )])
        }
        None => {
            return Err(vec![field_issue(
                id_field,
                input,
                "missing",
                "Field required",
                None,
            )])
        }
    };
    if !(1..=maximum).contains(&array.len()) {
        return Err(vec![field_issue(
            id_field,
            &body[id_field],
            "value_error",
            &format!("List should contain 1 to {maximum} IDs"),
            None,
        )]);
    }
    let ids = array
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let value = match id {
                Value::String(id) => integer_from_text(id).and_then(|id| id.as_i64()),
                Value::Bool(id) => Some(i64::from(*id)),
                _ => id.as_i64().or_else(|| {
                    let number = id.as_f64()?;
                    let integer = number as i64;
                    (number == integer as f64 && number < i64::MAX as f64).then_some(integer)
                }),
            };
            value.ok_or_else(|| {
                vec![json!({"type":"int_parsing","loc":["body",id_field,index],
            "msg":"Input should be a valid integer","input":id})]
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if summary {
        return Ok((ids, None));
    }
    let manual = match body.get("manual_lines") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(lines)) => lines
            .iter()
            .map(|line| {
                line.as_str()
                    .map(str::trim)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        vec![field_issue(
                            "manual_lines",
                            line,
                            "string_type",
                            "Input should be a valid string",
                            None,
                        )]
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(value) => {
            return Err(vec![field_issue(
                "manual_lines",
                value,
                "list_type",
                "Input should be a valid list",
                None,
            )])
        }
    };
    let mode = match body.get("mode").map_or(Some("none"), Value::as_str) {
        Some("none") => Names::None,
        Some("manual") => Names::Manual(manual),
        Some("config_expected") => Names::Saved,
        _ => {
            return Err(vec![field_issue(
                "mode",
                &body["mode"],
                "literal_error",
                "Input should be 'config_expected', 'manual' or 'none'",
                None,
            )])
        }
    };
    Ok((ids, Some(mode)))
}

async fn load_item(
    profile: &ProfileState,
    id: i64,
    item: Option<BatchRosterSource>,
    names: Option<&[String]>,
    extended_request: &AtomicBool,
) -> Result<Value, Value> {
    let Some(BatchRosterSource { mut row, roster }) = item else {
        return Err(json!({"filename":id.to_string(),"id":id,"code":"DEMO_FILE_NOT_FOUND"}));
    };
    let failure = |code| {
        let label = [&row["display_name"], &row["filename"]]
            .into_iter()
            .find_map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
            });
        json!({"filename":label.map(str::to_owned).unwrap_or_else(|| id.to_string()),"id":id,"code":code})
    };
    if names.is_none() && !row["result_error"].is_null() {
        return Err(failure("DEMO_INSPECTION_FAILED"));
    }
    if matches!(roster.cache, RosterCache::Missing) && !roster.path.is_file() {
        return Err(failure("DEMO_FILE_NOT_FOUND"));
    }
    let players = load_players(profile, &roster.path, roster.cache, extended_request)
        .await
        .map_err(|error| failure(failure_code(error)))?;
    if let Some(names) = names {
        return Ok(json!(match_names(names, &players)));
    }
    row["players"] = json!(players);
    row["match_meta"] = roster.match_meta;
    Ok(row)
}

fn failure_code(error: RosterError) -> &'static str {
    match error {
        RosterError::Timeout => "DEMO_INSPECTION_TIMEOUT",
        RosterError::Saved(text) => {
            let text = text.to_lowercase();
            if ["not found", "no such file", "找不到", "不存在"]
                .iter()
                .any(|marker| text.contains(marker))
            {
                "DEMO_FILE_NOT_FOUND"
            } else if ["timeout", "timed out", "超时"]
                .iter()
                .any(|marker| text.contains(marker))
            {
                "DEMO_INSPECTION_TIMEOUT"
            } else {
                "DEMO_INSPECTION_FAILED"
            }
        }
        _ => "DEMO_INSPECTION_FAILED",
    }
}

fn match_names<'a>(wanted: &[String], players: &'a [Value]) -> Vec<&'a str> {
    let aliases: Vec<_> = players
        .iter()
        .filter_map(|row| row["name"].as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| (name, name_key(name), name.to_lowercase()))
        .collect();
    let mut seen = HashSet::new();
    wanted
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .filter_map(|request| {
            let key = name_key(request);
            let lower = request.to_lowercase();
            let found = aliases
                .iter()
                .find(|(_, key_name, lower_name)| *key_name == key || *lower_name == lower)
                .or_else(|| {
                    if lower.chars().count() < 3 {
                        return None;
                    }
                    aliases
                        .iter()
                        .find(|(_, _, alias)| alias.contains(&lower) || lower.contains(alias))
                })?;
            seen.insert(found.1.as_str()).then_some(found.0)
        })
        .collect()
}

fn name_key(name: &str) -> String {
    let compact: String = name
        .chars()
        .filter(|character| {
            !character.is_whitespace() && !('\u{1c}'..='\u{1f}').contains(character)
        })
        .collect();
    case_fold(&compact)
}
