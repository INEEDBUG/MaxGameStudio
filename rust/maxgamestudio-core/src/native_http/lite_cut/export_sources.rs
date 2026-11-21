use super::project_schema::{list, normalize};
use super::{
    detail, invalid_project, json_reply, model, model_body, profile_disk, validation_reply,
    HttpResponse, ProfileState,
};
use crate::{native_http::demo_body::field_issue, profile_platform::expand_user, recorded_clips};
use hyper::{body::Incoming, Request, StatusCode};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "m4v", "webm", "mkv", "avi"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "m4a", "aac", "ogg", "flac"];

pub(super) async fn validate(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
) -> HttpResponse {
    let input = match model_body(request, "LiteCut project is too large").await {
        Ok(input) => input,
        Err(error) => return *error,
    };
    let object = match model(&input) {
        Ok(object) => object,
        Err(errors) => return validation_reply(errors),
    };
    let body = match object.get("body") {
        Some(body) if body.is_object() => body.clone(),
        Some(body) => {
            return validation_reply(vec![field_issue(
                "body",
                body,
                "dict_type",
                "Input should be a valid dictionary",
                None,
            )])
        }
        None => {
            return validation_reply(vec![field_issue(
                "body",
                &input,
                "missing",
                "Field required",
                None,
            )])
        }
    };
    let project = match normalize(Some(body)) {
        Ok(project) => project,
        Err(message) => return invalid_project(&message),
    };
    profile_disk(profile, move |profile| {
        let sources = Sources::from_project(&project);
        match sources.missing(&profile.config.database_path()) {
            Ok(items) => json_reply(StatusCode::OK, json!({"items":items})),
            Err(_) => detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "无法读取剪辑引用的录制文件",
            ),
        }
    })
    .await
}

#[derive(Default)]
struct Sources<'a> {
    paths: Vec<(&'static str, &'a str)>,
    recordings: BTreeSet<i64>,
}

impl<'a> Sources<'a> {
    fn from_project(project: &'a Value) -> Self {
        let mut sources = Self::default();
        let tracks = list(&project["tracks"]);
        let base = base_track(tracks);
        if let Some(base) = base {
            let base = &tracks[base];
            let mut clips: Vec<_> = list(&base["clips"])
                .iter()
                .filter(|clip| main_clip(clip))
                .collect();
            sort_timeline(&mut clips);
            for clip in clips {
                if clip["source_type"] == "file" {
                    sources.add("video", &clip["file_path"]);
                } else if let Some(id) = clip["source_id"].as_i64() {
                    sources.recordings.insert(id);
                }
            }
        }
        for track in tracks[..base.unwrap_or(tracks.len())]
            .iter()
            .rev()
            .filter(|track| visible_video(track))
        {
            let mut clips: Vec<_> = list(&track["clips"]).iter().collect();
            sort_timeline(&mut clips);
            for clip in clips {
                if clip["source_type"] == "file" {
                    sources.add("overlay", &clip["file_path"]);
                } else if let Some(id) = clip["source_id"].as_i64() {
                    sources.recordings.insert(id);
                    sources.add("overlay", &clip["file_path"]);
                }
            }
        }
        let hidden: HashSet<_> = list(&project["overlay_tracks"])
            .iter()
            .filter(|track| track["hidden"] == true)
            .filter_map(|track| track["id"].as_str())
            .collect();
        for item in list(&project["overlays"]) {
            let track = text(&item["meta"]["overlay_track_id"]);
            if hidden.contains(if track.is_empty() { "ot1" } else { track }) {
                continue;
            }
            if item["type"] == "text" {
                sources.add("font", &item["text"]["font_file"]);
            } else {
                sources.add("overlay", &item["asset_path"]);
            }
        }
        let solo = tracks
            .iter()
            .any(|track| track["type"] == "audio" && track["solo"] == true);
        let mut audio: Vec<_> = tracks
            .iter()
            .filter(|track| {
                track["type"] == "audio"
                    && track["hidden"] == false
                    && track["muted"] == false
                    && (!solo || track["solo"] == true)
            })
            .flat_map(|track| list(&track["clips"]))
            .filter(|clip| {
                clip["source_type"] == "file"
                    && (has_extension(clip, AUDIO_EXTENSIONS) || clip["meta"]["kind"] == "audio")
            })
            .collect();
        sort_timeline(&mut audio);
        for clip in audio {
            sources.add("audio", &clip["file_path"]);
        }
        sources.add("bgm", &project["audio"]["bgm"]["path"]);
        sources
    }

    fn add(&mut self, kind: &'static str, field: &'a Value) {
        let path = text(field);
        if !path.is_empty() {
            self.paths.push((kind, path));
        }
    }

    fn missing(self, database: &Path) -> Result<Vec<Value>, recorded_clips::RecordedClipsError> {
        let mut missing = Vec::new();
        let mut seen = HashSet::new();
        for (kind, raw) in self.paths {
            let path = media_path(raw);
            if !path.is_file() && seen.insert(path.clone()) {
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(raw);
                missing.push(json!({"kind":kind,"name":name,"path":raw}));
            }
        }
        let recordings = recorded_clips::output_paths(database, &self.recordings)?;
        for id in self.recordings {
            let raw = recordings
                .get(&id)
                .map(|path| path.trim())
                .unwrap_or_default();
            let path = media_path(raw);
            if raw.is_empty() || !path.is_file() {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("Insight recording #{id}"));
                missing.push(json!({"kind":"recording","name":name,"path":raw,"source_id":id}));
            }
        }
        Ok(missing)
    }
}

pub(super) fn base_track(tracks: &[Value]) -> Option<usize> {
    tracks
        .iter()
        .rposition(|track| visible_video(track) && list(&track["clips"]).iter().any(main_clip))
}

fn visible_video(track: &Value) -> bool {
    track["type"] == "video" && track["hidden"] == false
}

pub(super) fn main_clip(clip: &Value) -> bool {
    if clip["source_type"] == "file" {
        has_extension(clip, VIDEO_EXTENSIONS)
    } else {
        !clip["source_id"].is_null()
    }
}

fn has_extension(clip: &Value, supported: &[&str]) -> bool {
    Path::new(text(&clip["file_path"]))
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            supported
                .iter()
                .any(|value| extension.eq_ignore_ascii_case(value))
        })
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default().trim()
}

pub(super) fn media_path(raw: &str) -> PathBuf {
    expand_user(raw).unwrap_or_else(|_| PathBuf::from(raw))
}

pub(super) fn sort_timeline(clips: &mut [&Value]) {
    let time = |clip: &Value| {
        clip["timeline_start"]
            .as_f64()
            .expect("normalized timeline start")
    };
    clips.sort_by(|first, second| time(first).total_cmp(&time(second)));
}
