//! Recording request models and normalization shared by the recording planners.

use serde::{de::Error, Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RequestType {
    Highlight,
    Fail,
    TimelineKill,
    TimelineDeath,
    KillCompilation,
    DeathCompilation,
    RoundCompilation,
    TimelineRound,
}

impl RequestType {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Fail => "fail",
            Self::TimelineKill => "timeline_kill",
            Self::TimelineDeath => "timeline_death",
            Self::KillCompilation => "kill_compilation",
            Self::DeathCompilation => "death_compilation",
            Self::RoundCompilation => "round_compilation",
            Self::TimelineRound => "timeline_round",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SourceType {
    Kill,
    Death,
    Round,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Perspective {
    Killer,
    Victim,
    Main,
    Round,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EventType {
    Kill,
    Death,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct DemoContext {
    pub(super) demo_path: String,
    pub(super) demo_filename: String,
    pub(super) map_name: String,
    #[serde(deserialize_with = "de_float")]
    pub(super) tick_rate: f64,
    #[serde(deserialize_with = "de_int")]
    pub(super) first_tick: i64,
    #[serde(deserialize_with = "de_int")]
    pub(super) demo_end_tick: i64,
    #[serde(deserialize_with = "de_int")]
    pub(super) final_round: i64,
    #[serde(deserialize_with = "de_int")]
    pub(super) final_round_start_tick: i64,
    #[serde(deserialize_with = "de_int")]
    pub(super) final_round_end_tick: i64,
    #[serde(default)]
    pub(super) server_name: String,
    #[serde(default)]
    pub(super) all_players: Vec<Value>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct TargetPlayer {
    pub(super) name: String,
    pub(super) steamid64: String,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) spec_slot: Option<i64>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct EventInfo {
    pub(super) event_type: EventType,
    #[serde(deserialize_with = "de_int")]
    pub(super) tick: i64,
    #[serde(deserialize_with = "de_int")]
    pub(super) round: i64,
    pub(super) killer: TargetPlayer,
    pub(super) victim: TargetPlayer,
    pub(super) target_player: TargetPlayer,
    pub(super) perspective: Perspective,
    #[serde(default)]
    pub(super) weapon: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub(super) headshot: bool,
    #[serde(default)]
    pub(super) tags: Vec<String>,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) shots_to_kill: Option<i64>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct RoundInfo {
    #[serde(deserialize_with = "de_int")]
    pub(super) round: i64,
    #[serde(deserialize_with = "de_int")]
    pub(super) round_start_tick: i64,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) round_end_tick: Option<i64>,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) freeze_start_tick: Option<i64>,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) freeze_end_tick: Option<i64>,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) next_round_start_tick: Option<i64>,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) next_round_freeze_start_tick: Option<i64>,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) next_round_freeze_end_tick: Option<i64>,
    #[serde(default, deserialize_with = "de_option_int")]
    pub(super) target_death_tick: Option<i64>,
    #[serde(default = "yes", deserialize_with = "de_bool")]
    pub(super) round_end_tick_reliable: bool,
}

fn yes() -> bool {
    true
}

// Keep each duration's default and mandatory nonnegative check together.
macro_rules! recording_options {
    (
        checked { $($duration:ident = $default:expr),+ $(,)? }
        other { $($(#[$attribute:meta])* $option:ident: $kind:ty = $value:expr),+ $(,)? }
    ) => {
        #[derive(Clone, Deserialize, Serialize)]
        #[serde(default)]
        pub(super) struct RecordingOptions {
            $(
                #[serde(deserialize_with = "de_float")]
                pub(super) $duration: f64,
            )+
            $(
                $(#[$attribute])*
                pub(super) $option: $kind,
            )+
        }

        impl Default for RecordingOptions {
            fn default() -> Self {
                Self {
                    $($duration: $default,)+
                    $($option: $value,)+
                }
            }
        }

        impl RecordingOptions {
            fn check_durations(&self) -> Result<(), String> {
                $(
                    if !self.$duration.is_finite() || self.$duration < 0.0 {
                        return Err(concat!("options.", stringify!($duration), " must be >= 0").to_owned());
                    }
                )+
                Ok(())
            }
        }
    };
}

recording_options! {
    checked {
        highlight_pre_sec = 3.0,
        highlight_post_sec = 2.0,
        kill_jump_cut_threshold_sec = 12.0,
        timeline_kill_pre_sec = 3.0,
        timeline_kill_post_sec = 2.0,
        death_pre_sec = 3.0,
        death_post_sec = 2.0,
        kill_compilation_pre_sec = 2.0,
        kill_compilation_post_sec = 1.5,
        kill_compilation_jump_cut_threshold_sec = 10.0,
        death_compilation_pre_sec = 2.0,
        death_compilation_post_sec = 1.5,
        death_compilation_merge_gap_sec = 2.0,
        round_freeze_preroll_sec = 3.0,
        round_death_post_sec = 2.0,
        demo_end_guard_sec = 1.5,
        victim_pov_post_sec = 1.5,
        fail_killer_pre_sec = 3.0,
        fail_killer_post_sec = 2.0,
    }
    other {
        #[serde(deserialize_with = "de_bool")]
        enable_victim_pov: bool = false,
        #[serde(deserialize_with = "de_option_float")]
        victim_pov_pre_sec: Option<f64> = None,
        #[serde(deserialize_with = "de_bool")]
        interleave_pov_pairs: bool = false,
        #[serde(deserialize_with = "de_bool")]
        enable_fail_killer_pov: bool = false,
        #[serde(deserialize_with = "de_float")]
        final_round_extra_post_sec: f64 = 2.0,
        #[serde(deserialize_with = "de_option_bool")]
        obs_transition_enabled: Option<bool> = None,
        obs_transition_name: Option<String> = None,
        #[serde(deserialize_with = "de_option_int")]
        obs_transition_duration_ms: Option<i64> = None,
        #[serde(deserialize_with = "de_option_bool")]
        kb_overlay_enabled: Option<bool> = None,
        #[serde(deserialize_with = "de_option_int")]
        kb_overlay_tick_offset: Option<i64> = None,
        kb_overlay_position: Option<String> = None,
        #[serde(deserialize_with = "de_option_bool")]
        kill_fx_enabled: Option<bool> = None,
        #[serde(deserialize_with = "de_option_int")]
        kill_fx_tick_offset: Option<i64> = None,
        #[serde(deserialize_with = "de_bool")]
        use_ai_director: bool = false,
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct SourceRef {
    pub(super) original_clip_id: Option<String>,
    pub(super) context_tags: Vec<String>,
    pub(super) timeline_event_id: Option<String>,
    pub(super) queue_item_id: Option<String>,
    pub(super) group_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct RecordingRequestDto {
    pub(super) request_id: String,
    pub(super) request_type: RequestType,
    pub(super) source_type: SourceType,
    pub(super) demo: DemoContext,
    pub(super) target_player: TargetPlayer,
    #[serde(default)]
    pub(super) events: Vec<EventInfo>,
    #[serde(default)]
    pub(super) rounds: Vec<RoundInfo>,
    #[serde(default)]
    pub(super) options: RecordingOptions,
    #[serde(default)]
    pub(super) source_ref: SourceRef,
}

#[derive(Clone, Serialize)]
pub(super) struct NormalizedRequest {
    pub(super) request_id: String,
    pub(super) request_type: RequestType,
    pub(super) source_type: SourceType,
    pub(super) demo: DemoContext,
    pub(super) target_player: TargetPlayer,
    pub(super) events: Vec<EventInfo>,
    pub(super) rounds: Vec<RoundInfo>,
    pub(super) options: RecordingOptions,
    pub(super) source_ref: SourceRef,
    pub(super) warnings: Vec<String>,
}

pub(super) fn decode(value: Value) -> Result<RecordingRequestDto, String> {
    // Serde's raw enum error can contain submitted values.
    serde_json::from_value(value).map_err(|_| "recording request fields are invalid".to_owned())
}

pub(super) fn normalize(
    mut dto: RecordingRequestDto,
    actual_demo_end_tick: Option<i64>,
) -> Result<NormalizedRequest, String> {
    let demo = &mut dto.demo;
    if let Some(actual) = actual_demo_end_tick {
        if actual <= demo.first_tick {
            return Err("demo EOF must be greater than first_tick".to_owned());
        }
        demo.demo_end_tick = actual;
    }
    if !demo.tick_rate.is_finite() || demo.tick_rate <= 0.0 {
        return Err("demo.tick_rate must be > 0".to_owned());
    }
    if demo.first_tick < 0 {
        return Err("demo.first_tick must be >= 0".to_owned());
    }
    if demo.demo_end_tick <= demo.first_tick {
        return Err("demo.demo_end_tick must be > demo.first_tick".to_owned());
    }
    match dto.request_type {
        RequestType::RoundCompilation | RequestType::TimelineRound => {
            if dto.rounds.is_empty() {
                return Err(format!(
                    "request_type {} requires non-empty rounds",
                    dto.request_type.as_str()
                ));
            }
        }
        _ if dto.events.is_empty() => {
            return Err(format!(
                "request_type {} requires non-empty events",
                dto.request_type.as_str()
            ));
        }
        _ => {}
    }
    if dto.target_player.steamid64.is_empty() {
        return Err("target_player.steamid64 must be non-empty".to_owned());
    }
    dto.options.check_durations()?;

    let mut warnings = Vec::new();
    for round in &mut dto.rounds {
        normalize_round(round, demo.tick_rate, &mut warnings)?;
    }
    if dto
        .options
        .victim_pov_pre_sec
        .is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0)
    {
        return Err("options.victim_pov_pre_sec must be >= 0 if set".to_owned());
    }
    if dto.options.enable_victim_pov
        && !matches!(
            dto.request_type,
            RequestType::Highlight | RequestType::KillCompilation | RequestType::TimelineKill,
        )
    {
        warnings.push(
            "enable_victim_pov only applies to highlight, kill_compilation, and timeline_kill"
                .to_owned(),
        );
    }
    let mut filled = 0;
    for event in &mut dto.events {
        if enrich_victim(&mut event.victim, &demo.all_players) {
            filled += 1;
        }
    }
    if filled > 0 {
        warnings.push(format!(
            "roster: filled victim steamid/spec_slot for {filled}/{} kill events",
            dto.events.len(),
        ));
    }
    Ok(NormalizedRequest {
        request_id: dto.request_id,
        request_type: dto.request_type,
        source_type: dto.source_type,
        demo: dto.demo,
        target_player: dto.target_player,
        events: dto.events,
        rounds: dto.rounds,
        options: dto.options,
        source_ref: dto.source_ref,
        warnings,
    })
}

fn normalize_round(
    round: &mut RoundInfo,
    tick_rate: f64,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let number = round.round;
    if round.freeze_end_tick.is_none() {
        warnings.push(format!(
            "round {number}: freeze_end_tick missing, will use fallback"
        ));
    }
    if let (Some(freeze_start), None, Some(start)) = (
        round.next_round_freeze_start_tick,
        round.next_round_freeze_end_tick,
        round.next_round_start_tick,
    ) {
        if freeze_start > start {
            round.next_round_freeze_end_tick = Some(freeze_start);
            round.next_round_freeze_start_tick = None;
            warnings.push(format!(
                "round {number}: round_metadata_next_freeze_start_rewritten_to_freeze_end"
            ));
        }
    }
    round.round_end_tick_reliable = true;
    if let (Some(end), None) = (round.round_end_tick, round.target_death_tick) {
        let five_seconds = seconds_to_ticks(5.0, tick_rate)?;
        if round.next_round_freeze_end_tick.is_some_and(|freeze_end| {
            i128::from(end) == i128::from(freeze_end) - i128::from(five_seconds)
        }) {
            round.round_end_tick_reliable = false;
            warnings.push(format!(
                "round {number}: round_end_tick_unreliable_derived_from_next_freeze"
            ));
        } else if round.next_round_start_tick.is_some_and(|start| end > start) {
            round.round_end_tick_reliable = false;
            warnings.push(format!(
                "round {number}: round_end_tick_after_next_round_start"
            ));
        }
    }
    Ok(())
}

pub(super) fn seconds_to_ticks(seconds: f64, tick_rate: f64) -> Result<i64, String> {
    truncated_i64(seconds * tick_rate)
        .ok_or_else(|| "recording tick calculation is out of range".to_owned())
}

pub(super) fn lookup_roster_player<'a>(
    roster: &'a [Value],
    name: &str,
    steamid64: &str,
) -> Option<&'a Map<String, Value>> {
    let sid = strip(steamid64);
    if !sid.is_empty() {
        if let Some(row) = roster
            .iter()
            .filter_map(Value::as_object)
            .find(|row| roster_text(row.get("steamid64")) == sid)
        {
            return Some(row);
        }
    }
    let name = strip(name);
    if name.is_empty() {
        return None;
    }
    let folded = name.to_lowercase();
    roster.iter().filter_map(Value::as_object).find(|row| {
        let candidate = roster_text(row.get("name"));
        candidate == name || candidate.to_lowercase() == folded
    })
}

fn enrich_victim(victim: &mut TargetPlayer, roster: &[Value]) -> bool {
    let Some(row) = lookup_roster_player(roster, &victim.name, &victim.steamid64) else {
        return false;
    };
    let mut changed = false;
    if strip(&victim.steamid64).is_empty() {
        let known = roster_text(row.get("steamid64"));
        if !known.is_empty() {
            victim.steamid64 = known.into_owned();
            changed = true;
        }
    }
    if victim.spec_slot.is_none() {
        if let Some(slot) = row.get("spec_slot").and_then(roster_int) {
            victim.spec_slot = Some(slot);
            changed = true;
        }
    }
    if strip(&victim.name).is_empty() {
        let known = roster_text(row.get("name"));
        if !known.is_empty() {
            victim.name = known.into_owned();
            changed = true;
        }
    }
    changed
}

fn roster_text(value: Option<&Value>) -> Cow<'_, str> {
    match value {
        Some(Value::String(value)) => strip(value).into(),
        Some(Value::Number(value)) if value.as_f64() != Some(0.0) => value.to_string().into(),
        Some(Value::Bool(true)) => "True".into(),
        _ => "".into(),
    }
}

fn roster_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Some(integer)
            } else if value.is_f64() {
                truncated_i64(value.as_f64()?)
            } else {
                None
            }
        }
        Value::String(value) => numeric_text(value)?.parse().ok(),
        _ => None,
    }
}

fn strip(value: &str) -> &str {
    value.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch))
}

fn numeric_text(value: &str) -> Option<Cow<'_, str>> {
    let value = strip(value);
    if !value.contains('_') {
        return Some(value.into());
    }
    let bytes = value.as_bytes();
    if bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'_'
            && !(index > 0
                && bytes[index - 1].is_ascii_digit()
                && bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
    }) {
        return None;
    }
    Some(value.replace('_', "").into())
}

fn truncated_i64(value: f64) -> Option<i64> {
    let lower = i64::MIN as f64;
    (lower..-lower)
        .contains(&value)
        .then_some(value.trunc() as i64)
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(value) if value.is_f64() => {
            let value = value.as_f64()?;
            if value.fract() != 0.0 {
                return None;
            }
            truncated_i64(value)
        }
        Value::String(value) => {
            let value = numeric_text(value)?;
            match value.split_once('.') {
                Some((whole, fraction))
                    if !fraction.is_empty() && fraction.bytes().all(|ch| ch == b'0') =>
                {
                    whole.parse().ok()
                }
                Some(_) => None,
                None => value.parse().ok(),
            }
        }
        value => roster_int(value),
    }
}

fn float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        Value::Number(value) => value.as_f64(),
        Value::String(value) => numeric_text(value)?.parse().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(value) if value.as_f64() == Some(0.0) => Some(false),
        Value::Number(value) if value.as_f64() == Some(1.0) => Some(true),
        Value::String(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

macro_rules! coerce_field {
    ($required:ident, $optional:ident, $kind:ty, $parse:ident, $message:literal) => {
        fn $required<'de, D: Deserializer<'de>>(input: D) -> Result<$kind, D::Error> {
            $parse(&Value::deserialize(input)?).ok_or_else(|| D::Error::custom($message))
        }
        fn $optional<'de, D: Deserializer<'de>>(input: D) -> Result<Option<$kind>, D::Error> {
            Option::<Value>::deserialize(input)?
                .map(|value| $parse(&value).ok_or_else(|| D::Error::custom($message)))
                .transpose()
        }
    };
}

coerce_field!(de_int, de_option_int, i64, integer, "expected an integer");
coerce_field!(
    de_float,
    de_option_float,
    f64,
    float,
    "expected a finite number"
);
coerce_field!(de_bool, de_option_bool, bool, boolean, "expected a boolean");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request_body() -> Value {
        json!({
            "request_id":"synthetic",
            "request_type":"highlight",
            "source_type":"kill",
            "demo":{
                "demo_path":"C:/private/fixture.dem","demo_filename":"test.dem","map_name":"de_mirage",
                "tick_rate":"64.5","first_tick":"160","demo_end_tick":21000,"final_round":20,
                "final_round_start_tick":20000,"final_round_end_tick":21000
            },
            "target_player":{"name":"Fixture","steamid64":"76561198012345678"},
            "events":[{
                "event_type":"kill","tick":"1000.0","round":5,
                "killer":{"name":"Fixture","steamid64":"76561198012345678"},
                "victim":{"name":"  eNEmY ","steamid64":""},
                "target_player":{"name":"Fixture","steamid64":"76561198012345678"},
                "perspective":"killer"
            }]
        })
    }

    #[test]
    fn released_defaults_and_json_scalar_coercions_are_preserved() {
        let normal = normalize(decode(request_body()).unwrap(), None).unwrap();
        assert_eq!(normal.demo.tick_rate, 64.5);
        assert_eq!(normal.demo.first_tick, 160);
        assert_eq!(normal.events[0].tick, 1000);
        assert_eq!(normal.events[0].perspective, Perspective::Killer);
        assert!(normal.events[0].tags.is_empty());
        assert!(!normal.events[0].headshot);
        assert!(normal.rounds.is_empty());
        assert!(normal.warnings.is_empty());
        assert!(normal.source_ref.context_tags.is_empty());
        assert!(normal.demo.all_players.is_empty());
        assert_eq!(
            serde_json::to_value(normal.options).unwrap(),
            json!({
                "highlight_pre_sec":3.0,"highlight_post_sec":2.0,"kill_jump_cut_threshold_sec":12.0,
                "timeline_kill_pre_sec":3.0,"timeline_kill_post_sec":2.0,"death_pre_sec":3.0,"death_post_sec":2.0,
                "kill_compilation_pre_sec":2.0,"kill_compilation_post_sec":1.5,"kill_compilation_jump_cut_threshold_sec":10.0,
                "death_compilation_pre_sec":2.0,"death_compilation_post_sec":1.5,"death_compilation_merge_gap_sec":2.0,
                "round_freeze_preroll_sec":3.0,"round_death_post_sec":2.0,
                "enable_victim_pov":false,"victim_pov_pre_sec":null,"victim_pov_post_sec":1.5,
                "interleave_pov_pairs":false,"enable_fail_killer_pov":false,"fail_killer_pre_sec":3.0,
                "fail_killer_post_sec":2.0,"demo_end_guard_sec":1.5,"final_round_extra_post_sec":2.0,
                "obs_transition_enabled":null,"obs_transition_name":null,"obs_transition_duration_ms":null,
                "kb_overlay_enabled":null,"kb_overlay_tick_offset":null,"kb_overlay_position":null,
                "kill_fx_enabled":null,"kill_fx_tick_offset":null,"use_ai_director":false
            })
        );
    }

    #[test]
    fn verified_eof_overrides_even_a_stale_boundary_and_checks_first_tick() {
        let mut raw = request_body();
        raw["demo"]["demo_end_tick"] = json!(100);
        let dto = decode(raw).unwrap();
        assert_eq!(
            normalize(dto.clone(), None).err().unwrap(),
            "demo.demo_end_tick must be > demo.first_tick"
        );
        assert_eq!(
            normalize(dto.clone(), Some(160)).err().unwrap(),
            "demo EOF must be greater than first_tick"
        );
        // The existing public PBDEMS2 fixture ends at 57985.
        let normal = normalize(dto, Some(57985)).unwrap();
        assert_eq!(normal.demo.demo_end_tick, 57985);
        assert_eq!(normal.demo.final_round_end_tick, 21000);
        assert!(normal.warnings.is_empty());
    }

    #[test]
    fn stale_freeze_ticks_and_victim_details_match_the_released_normalizer() {
        let mut raw = request_body();
        raw["request_type"] = json!("timeline_round");
        raw["source_type"] = json!("round");
        raw["options"] = json!({"enable_victim_pov":"yes","final_round_extra_post_sec":-1});
        raw["demo"]["all_players"] = json!([
            null, {"name":"Enemy","steamid64":76561198087654321_u64,"spec_slot":7.9},
        ]);
        raw["rounds"] = json!([
            {"round":5,"round_start_tick":1000,"round_end_tick":1278,
             "next_round_start_tick":1500,"next_round_freeze_start_tick":1600},
            {"round":6,"round_start_tick":3500,"freeze_end_tick":3600,"round_end_tick":4000,
             "next_round_start_tick":3900},
            {"round":7,"round_start_tick":4500,"freeze_end_tick":4600,"round_end_tick":5000,
             "next_round_start_tick":4900,"target_death_tick":0,"round_end_tick_reliable":false},
        ]);
        let normal = normalize(decode(raw).unwrap(), None).unwrap();
        assert_eq!(seconds_to_ticks(5.0, 64.5).unwrap(), 322);
        assert_eq!(normal.rounds[0].round_end_tick, Some(1278));
        assert_eq!(normal.rounds[0].next_round_freeze_end_tick, Some(1600));
        assert_eq!(normal.rounds[0].next_round_freeze_start_tick, None);
        assert_eq!(
            normal
                .rounds
                .iter()
                .map(|r| r.round_end_tick_reliable)
                .collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(normal.options.final_round_extra_post_sec, -1.0);
        assert_eq!(normal.events[0].victim.steamid64, "76561198087654321");
        assert_eq!(normal.events[0].victim.spec_slot, Some(7));
        assert_eq!(normal.events[0].victim.name, "  eNEmY ");
        assert_eq!(normal.events[0].killer.spec_slot, None);
        assert_eq!(
            normal.warnings,
            [
                "round 5: freeze_end_tick missing, will use fallback",
                "round 5: round_metadata_next_freeze_start_rewritten_to_freeze_end",
                "round 5: round_end_tick_unreliable_derived_from_next_freeze",
                "round 6: round_end_tick_after_next_round_start",
                "enable_victim_pov only applies to highlight, kill_compilation, and timeline_kill",
                "roster: filled victim steamid/spec_slot for 1/1 kill events",
            ]
        );
    }

    #[test]
    fn roster_prefers_existing_steam_ids_and_only_fills_missing_victim_fields() {
        let roster = [
            json!({"name":"Enemy","steamid64":"different","spec_slot":3}),
            json!({"name":"Other","steamid64":"known","spec_slot":" 8 "}),
        ];
        let mut victim = TargetPlayer {
            name: "Enemy".to_owned(),
            steamid64: " known ".to_owned(),
            spec_slot: None,
        };
        assert!(enrich_victim(&mut victim, &roster));
        assert_eq!(victim.steamid64, " known ");
        assert_eq!(victim.name, "Enemy");
        assert_eq!(victim.spec_slot, Some(8));
        assert!(!enrich_victim(&mut victim, &roster));
    }

    #[test]
    fn malformed_requests_fail_with_sanitized_errors() {
        let mut raw = request_body();
        raw["request_type"] = json!("C:/private/fixture.dem");
        assert_eq!(
            decode(raw).err().unwrap(),
            "recording request fields are invalid"
        );
        let mut raw = request_body();
        raw["events"][0]["tick"] = json!(1000.5);
        assert!(decode(raw).is_err());
        let mut raw = request_body();
        raw["events"] = json!([]);
        assert_eq!(
            normalize(decode(raw).unwrap(), None).err().unwrap(),
            "request_type highlight requires non-empty events"
        );
        let mut raw = request_body();
        raw["request_type"] = json!("round_compilation");
        assert_eq!(
            normalize(decode(raw).unwrap(), None).err().unwrap(),
            "request_type round_compilation requires non-empty rounds"
        );
        let mut raw = request_body();
        raw["options"] = json!({"demo_end_guard_sec":-1.0});
        assert_eq!(
            normalize(decode(raw).unwrap(), None).err().unwrap(),
            "options.demo_end_guard_sec must be >= 0"
        );
        assert!(seconds_to_ticks(5.0, f64::MAX).is_err());
    }
}
