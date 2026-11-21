use crate::{
    demo_library::{DemoListKind, DemoListQuery, DemoPage},
    wire_json::integer_from_text,
};
use percent_encoding::percent_decode_str;
use serde_json::{json, Value};
use std::{borrow::Cow, collections::HashMap};

const INTEGER_TYPE: &str = "int_parsing";
const INTEGER_MESSAGE: &str =
    "Input should be a valid integer, unable to parse string as an integer";

pub(super) enum DemoRoute {
    List(DemoListKind),
    Discovered,
    Stream,
    InspectPath,
    OpenLocal,
    RadarMap,
    UtilityMask,
    Detail,
    PlayerStats,
    Players,
    Remark,
}

impl DemoRoute {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        let path = percent_decode_str(path).decode_utf8_lossy();
        match path.as_ref() {
            "/api/demos" => Some(Self::List(DemoListKind::Full)),
            "/api/demos/compact" => Some(Self::List(DemoListKind::Compact)),
            "/api/demos/ids" => Some(Self::List(DemoListKind::Ids)),
            "/api/demos/discovered" => Some(Self::Discovered),
            "/api/demos/stream" => Some(Self::Stream),
            "/api/demos/watch-path/inspect" => Some(Self::InspectPath),
            "/api/demo/open-local" => Some(Self::OpenLocal),
            _ => {
                for (prefix, route) in [
                    ("/api/demo/radar-map/", Self::RadarMap),
                    ("/api/demo/utility-mask/", Self::UtilityMask),
                ] {
                    if path
                        .strip_prefix(prefix)
                        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
                    {
                        return Some(route);
                    }
                }
                let rest = path.strip_prefix("/api/demos/")?;
                if matches!(
                    rest,
                    "" | "scan"
                        | "sync-match-times"
                        | "batch-resolve-players"
                        | "batch-summary"
                        | "batch-ingest"
                ) {
                    return None;
                }
                match rest.split_once('/') {
                    Some((id, "player-stats")) if !id.is_empty() => Some(Self::PlayerStats),
                    Some((id, "players")) if !id.is_empty() => Some(Self::Players),
                    Some((id, "remark")) if !id.is_empty() => Some(Self::Remark),
                    Some(_) => None,
                    None => Some(Self::Detail),
                }
            }
        }
    }
}

pub(super) fn parse_id(path: &str) -> Result<i64, Vec<Value>> {
    let path = percent_decode_str(path).decode_utf8_lossy();
    let id = path
        .strip_prefix("/api/demos/")
        .expect("demo item route")
        .split('/')
        .next()
        .expect("demo ID");
    parse_path_integer(id, "demo_id")
}

pub(super) fn parse_path_integer(id: &str, name: &str) -> Result<i64, Vec<Value>> {
    integer_from_text(id)
        .and_then(|id| id.as_i64())
        .ok_or_else(|| vec![issue("path", name, id, INTEGER_TYPE, INTEGER_MESSAGE, None)])
}

pub(super) fn parse_list(query: &str, kind: DemoListKind) -> Result<DemoListQuery, Vec<Value>> {
    let default_limit = if matches!(kind, DemoListKind::Ids) {
        1000
    } else {
        200
    };
    let mut params = Query::new(query);
    let page = params.page(default_limit);
    let map_names = csv(
        params.text("map_names", 4000).as_deref(),
        params.text("map_name", 200).as_deref(),
        |_| true,
    );
    let statuses = csv(
        params.text("statuses", 256).as_deref(),
        params.text("status", 64).as_deref(),
        |status| matches!(status, "loaded" | "parsing" | "done" | "error"),
    );
    let min_kills = params.integer("min_kills", 0, None);
    let max_deaths = params.integer("max_deaths", 0, None);
    let min_assists = params.integer("min_assists", 0, None);
    let min_kd = params.nonnegative_float("min_kd");
    let player_query = params.trimmed("player_query", 200);
    let steam_query = params.trimmed("steam_query", 64);
    let rounds_min = params.integer("rounds_min", 0, None);
    let rounds_max = params.integer("rounds_max", 0, None);
    let duration_min = params.nonnegative_float("duration_min");
    let duration_max = params.nonnegative_float("duration_max");
    let date_from = params.trimmed("date_from", 32);
    let date_to = params.trimmed("date_to", 32);
    let (sort_key, sort_asc) = if matches!(kind, DemoListKind::Ids) {
        (String::new(), false)
    } else {
        let sort_key = params
            .text("sort_key", 32)
            .unwrap_or_else(|| "library".to_owned());
        (sort_key, params.sort_asc())
    };
    if !params.errors.is_empty() {
        return Err(params.errors);
    }
    Ok(DemoListQuery {
        kind,
        page,
        map_names,
        statuses,
        min_kills,
        max_deaths,
        min_assists,
        min_kd,
        player_query,
        steam_query,
        rounds_min,
        rounds_max,
        duration_min,
        duration_max,
        date_from,
        date_to,
        sort_key,
        sort_asc,
    })
}

pub(super) fn parse_discovered(query: &str) -> Result<DemoPage, Vec<Value>> {
    let mut params = Query::new(query);
    let page = params.page(200);
    if params.errors.is_empty() {
        Ok(page)
    } else {
        Err(params.errors)
    }
}

pub(super) fn parse_offset_page(
    query: &str,
    default_limit: i64,
    max_limit: i64,
) -> Result<(i64, i64), Vec<Value>> {
    let mut query = Query::new(query);
    let limit = query
        .integer("limit", 1, Some(max_limit))
        .unwrap_or(default_limit);
    let offset = query.integer("offset", 0, None).unwrap_or(0);
    if query.errors.is_empty() {
        Ok((limit, offset))
    } else {
        Err(query.errors)
    }
}

pub(super) fn parse_project_page(
    query: &str,
    default_limit: i64,
    max_limit: i64,
) -> Result<(Option<i64>, i64, i64), Vec<Value>> {
    let mut query = Query::new(query);
    let project = query.integer("project_id", i64::MIN, None);
    let limit = query
        .integer("limit", 1, Some(max_limit))
        .unwrap_or(default_limit);
    let offset = query.integer("offset", 0, None).unwrap_or_default();
    if query.errors.is_empty() {
        Ok((project, limit, offset))
    } else {
        Err(query.errors)
    }
}

pub(super) fn parse_waveform(
    query: &str,
) -> Result<crate::recording_waveform::WaveformSlice, Vec<Value>> {
    let mut query = Query::new(query);
    let buckets = query.integer("buckets", 8, Some(512)).unwrap_or(72) as usize;
    let start = query.nonnegative_float("start_sec").unwrap_or(0.0);
    let end = query.nonnegative_float("end_sec");
    if query.errors.is_empty() {
        Ok(crate::recording_waveform::WaveformSlice {
            buckets,
            start,
            end,
        })
    } else {
        Err(query.errors)
    }
}
struct Query<'a> {
    values: HashMap<Cow<'a, str>, Cow<'a, str>>,
    errors: Vec<Value>,
}

impl<'a> Query<'a> {
    fn new(query: &'a str) -> Self {
        Self {
            // Like Starlette, the last value wins if a key is sent more than once.
            values: form_urlencoded::parse(query.as_bytes()).collect(),
            errors: Vec::new(),
        }
    }

    fn page(&mut self, default_limit: i64) -> DemoPage {
        DemoPage {
            limit: self
                .integer("limit", 1, Some(1000))
                .unwrap_or(default_limit),
            offset: self.integer("offset", 0, None).unwrap_or_default(),
            q: self.trimmed("q", 200),
        }
    }

    fn text(&mut self, key: &str, max_length: usize) -> Option<String> {
        let input = self.values.get(key)?.as_ref();
        if input.chars().count() > max_length {
            self.errors.push(issue(
                "query",
                key,
                input,
                "string_too_long",
                &format!("String should have at most {max_length} characters"),
                Some(json!({"max_length": max_length})),
            ));
            None
        } else {
            Some(input.to_owned())
        }
    }

    fn trimmed(&mut self, key: &str, max_length: usize) -> Option<String> {
        let input = self.text(key, max_length)?;
        let trimmed = input.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    fn integer(&mut self, key: &str, min: i64, max: Option<i64>) -> Option<i64> {
        let input = self.values.get(key)?.as_ref();
        let Some(number) = integer_from_text(input) else {
            self.errors.push(issue(
                "query",
                key,
                input,
                INTEGER_TYPE,
                INTEGER_MESSAGE,
                None,
            ));
            return None;
        };
        if let Some(max) = max.filter(|&max| number.as_i64().is_none_or(|value| value > max)) {
            self.errors.push(issue(
                "query",
                key,
                input,
                "less_than_equal",
                &format!("Input should be less than or equal to {max}"),
                Some(json!({"le": max})),
            ));
            return None;
        }
        let Some(value) = number.as_i64() else {
            self.errors.push(issue(
                "query",
                key,
                input,
                INTEGER_TYPE,
                INTEGER_MESSAGE,
                None,
            ));
            return None;
        };
        if value < min {
            self.errors.push(issue(
                "query",
                key,
                input,
                "greater_than_equal",
                &format!("Input should be greater than or equal to {min}"),
                Some(json!({"ge": min})),
            ));
            return None;
        }
        Some(value)
    }

    fn nonnegative_float(&mut self, key: &str) -> Option<f64> {
        let input = self.values.get(key)?.as_ref();
        let Ok(number) = input.trim().replace('_', "").parse::<f64>() else {
            self.errors.push(issue(
                "query",
                key,
                input,
                "float_parsing",
                "Input should be a valid number, unable to parse string as a number",
                None,
            ));
            return None;
        };
        if number.is_nan() || number < 0.0 {
            self.errors.push(issue(
                "query",
                key,
                input,
                "greater_than_equal",
                "Input should be greater than or equal to 0",
                Some(json!({"ge": 0.0})),
            ));
            return None;
        }
        Some(number)
    }

    fn sort_asc(&mut self) -> bool {
        const PATTERN: &str = "^(asc|desc)$";
        match self.values.get("sort_dir").map(Cow::as_ref) {
            Some("asc") => true,
            None | Some("desc") => false,
            Some(input) => {
                self.errors.push(issue(
                    "query",
                    "sort_dir",
                    input,
                    "string_pattern_mismatch",
                    "String should match pattern '^(asc|desc)$'",
                    Some(json!({"pattern": PATTERN})),
                ));
                false
            }
        }
    }
}

fn csv(list: Option<&str>, single: Option<&str>, allowed: impl Fn(&str) -> bool) -> Vec<String> {
    let mut items = list
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty() && allowed(item))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if items.is_empty() {
        if let Some(name) = single
            .map(str::trim)
            .filter(|name| !name.is_empty() && allowed(name))
        {
            items.push(name.to_owned());
        }
    }
    items
}

fn issue(
    location: &str,
    field: &str,
    input: &str,
    kind: &str,
    message: &str,
    context: Option<Value>,
) -> Value {
    let mut value = json!({"type": kind, "loc": [location, field], "msg": message, "input": input});
    if let Some(context) = context {
        value["ctx"] = context;
    }
    value
}
