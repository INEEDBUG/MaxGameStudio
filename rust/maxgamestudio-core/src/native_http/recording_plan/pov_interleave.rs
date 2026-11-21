//! Plan one pair of perspectives per event in the supplied event order.

use super::{
    event_segment,
    request::{strip, EventInfo, NormalizedRequest, Perspective, SourceType, TargetPlayer},
    RecordingSegment,
};

const INTERLEAVE_TAIL_SEC: f64 = 0.35;

pub(super) fn kill_then_victim(
    req: &NormalizedRequest,
    sorted: &[&EventInfo],
    killer: (i64, i64),
    victim: (i64, i64),
) -> Vec<RecordingSegment> {
    let killer = (killer.0, short_tail(req, killer.1));
    let mut segments = Vec::with_capacity(sorted.len() * 2);
    for event in sorted {
        let index = segments.len();
        let first = pair_segment(
            req,
            event,
            SourceType::Kill,
            Perspective::Killer,
            &req.target_player,
            killer,
            index,
        );
        let mut second = pair_segment(
            req,
            event,
            SourceType::Kill,
            Perspective::Victim,
            &event.victim,
            victim,
            index + 1,
        );
        second.target_steamid64 = strip(&second.target_steamid64).to_owned();
        segments.extend([first, second]);
    }
    segments
}

pub(super) fn victim_then_killer(
    req: &NormalizedRequest,
    sorted: &[&EventInfo],
    victim: (i64, i64),
    killer: (i64, i64),
) -> Vec<RecordingSegment> {
    let victim = (victim.0, short_tail(req, victim.1));
    let mut segments = Vec::with_capacity(sorted.len() * 2);
    for event in sorted {
        let index = segments.len();
        let first = pair_segment(
            req,
            event,
            SourceType::Death,
            Perspective::Victim,
            &req.target_player,
            victim,
            index,
        );
        let mut second = pair_segment(
            req,
            event,
            SourceType::Death,
            Perspective::Killer,
            &event.killer,
            killer,
            index + 1,
        );
        second.target_steamid64 = strip(&second.target_steamid64).to_owned();
        if second.target_steamid64.is_empty() {
            second.disabled = true;
            second.disabled_reason = Some("missing_killer_steamid64".to_owned());
        }
        segments.extend([first, second]);
    }
    segments
}

fn short_tail(req: &NormalizedRequest, requested_post: i64) -> i64 {
    // The death camera starts near the event when the other POV follows.
    requested_post.min(((INTERLEAVE_TAIL_SEC * req.demo.tick_rate) as i64).max(1))
}

fn pair_segment(
    req: &NormalizedRequest,
    event: &EventInfo,
    source_type: SourceType,
    perspective: Perspective,
    target: &TargetPlayer,
    timing: (i64, i64),
    index: usize,
) -> RecordingSegment {
    let (pre, post) = timing;
    let bounds = (
        event.tick.saturating_sub(pre),
        event.tick.saturating_add(post),
    );
    let mut segment = event_segment(
        req,
        source_type,
        perspective,
        event.round,
        target,
        bounds,
        vec![event.tick],
    );
    segment.segment_index = index;
    segment
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_http::recording_plan::request::{decode, normalize};
    use serde_json::{json, Value};

    fn main_player() -> Value {
        json!({"name":"Main Player","steamid64":" main-id ","spec_slot":7})
    }

    fn event(kind: &str, tick: i64, round: i64, killer: &str, victim: &str) -> Value {
        json!({
            "event_type":kind, "tick":tick, "round":round,
            "killer":{"name":"Enemy Killer","steamid64":killer,"spec_slot":3},
            "victim":{"name":"Enemy Victim","steamid64":victim,"spec_slot":4},
            "target_player": main_player(),
            "perspective": if kind == "kill" { "killer" } else { "victim" },
        })
    }

    fn fixture(kind: &str, tick_rate: f64, events: Vec<Value>) -> NormalizedRequest {
        normalize(decode(json!({
            "request_id":"synthetic", "request_type":format!("{kind}_compilation"), "source_type":kind,
            "target_player":main_player(), "events":events,
            "demo":{
                "demo_path":"synthetic.dem", "demo_filename":"synthetic.dem", "map_name":"de_fixture",
                "tick_rate":tick_rate, "first_tick":50, "demo_end_tick":1000, "final_round":6,
                "final_round_start_tick":900, "final_round_end_tick":970,
            },
        })).unwrap(), None).unwrap()
    }

    // Rows from the released planner helpers, before shared EOF and voice processing.
    fn oracle_rows(segments: &[RecordingSegment]) -> Value {
        let rows = segments
            .iter()
            .map(|segment| {
                assert!(segment.metadata.is_empty());
                assert!(segment.safe_end_tick.is_none());
                assert!(segment.voice_listen_mask.is_none());
                assert!(segment.voice_listen_mask_enemy.is_none());
                json!([
                    segment.segment_index,
                    segment.source_type,
                    segment.perspective,
                    [segment.start_tick, segment.end_tick, segment.safe_seek_tick],
                    segment.anchor_ticks,
                    segment.round,
                    segment.is_final_round,
                    [
                        segment.target_player_name,
                        segment.target_steamid64,
                        segment.target_spec_slot
                    ],
                    segment.disabled,
                    segment.disabled_reason,
                ])
            })
            .collect::<Vec<_>>();
        Value::Array(rows)
    }

    #[test]
    fn kill_pairs_match_the_released_order_clamp_and_missing_victim_behavior() {
        let req = fixture(
            "kill",
            64.5,
            vec![
                event("kill", 995, 5, "unused", " \u{1c} "),
                event("kill", 64, 6, "unused", " victim-id "),
            ],
        );
        let sorted: Vec<_> = req.events.iter().collect();
        let got = kill_then_victim(&req, &sorted, (60, 500), (40, 80));
        assert_eq!(
            oracle_rows(&got),
            json!([
                [
                    0,
                    "kill",
                    "killer",
                    [935, 1000, 613],
                    [995],
                    5,
                    false,
                    ["Main Player", " main-id ", 7],
                    false,
                    null
                ],
                [
                    1,
                    "kill",
                    "victim",
                    [955, 1000, 633],
                    [995],
                    5,
                    false,
                    ["Enemy Victim", "", 4],
                    false,
                    null
                ],
                [
                    2,
                    "kill",
                    "killer",
                    [50, 86, 50],
                    [64],
                    6,
                    true,
                    ["Main Player", " main-id ", 7],
                    false,
                    null
                ],
                [
                    3,
                    "kill",
                    "victim",
                    [50, 144, 50],
                    [64],
                    6,
                    true,
                    ["Enemy Victim", "victim-id", 4],
                    false,
                    null
                ],
            ])
        );
    }

    #[test]
    fn death_pairs_keep_missing_killers_and_out_of_range_windows_for_postprocessing() {
        let req = fixture(
            "death",
            64.5,
            vec![
                event("death", 42, 6, " \u{1f} ", "unused"),
                event("death", 1100, 6, " killer-id ", "unused"),
            ],
        );
        let sorted: Vec<_> = req.events.iter().collect();
        let got = victim_then_killer(&req, &sorted, (20, 50), (100, 40));
        assert_eq!(
            oracle_rows(&got),
            json!([
                [
                    0,
                    "death",
                    "victim",
                    [50, 64, 50],
                    [42],
                    6,
                    true,
                    ["Main Player", " main-id ", 7],
                    false,
                    null
                ],
                [
                    1,
                    "death",
                    "killer",
                    [50, 82, 50],
                    [42],
                    6,
                    true,
                    ["Enemy Killer", "", 3],
                    true,
                    "missing_killer_steamid64"
                ],
                [
                    2,
                    "death",
                    "victim",
                    [1080, 1000, 758],
                    [1100],
                    6,
                    true,
                    ["Main Player", " main-id ", 7],
                    false,
                    null
                ],
                [
                    3,
                    "death",
                    "killer",
                    [1000, 1000, 678],
                    [1100],
                    6,
                    true,
                    ["Enemy Killer", "killer-id", 3],
                    false,
                    null
                ],
            ])
        );
    }

    #[test]
    fn short_tail_keeps_the_released_one_tick_floor_and_zero_post_is_respected() {
        let req = fixture("kill", 2.0, vec![event("kill", 550, 6, "unused", "v")]);
        let sorted: Vec<_> = req.events.iter().collect();
        let got = kill_then_victim(&req, &sorted, (0, 20), (0, 0));
        assert_eq!(
            oracle_rows(&got),
            json!([
                [
                    0,
                    "kill",
                    "killer",
                    [550, 551, 540],
                    [550],
                    6,
                    true,
                    ["Main Player", " main-id ", 7],
                    false,
                    null
                ],
                [
                    1,
                    "kill",
                    "victim",
                    [550, 550, 540],
                    [550],
                    6,
                    true,
                    ["Enemy Victim", "v", 4],
                    false,
                    null
                ],
            ])
        );
        assert_eq!(
            kill_then_victim(&req, &sorted, (0, 0), (0, 0))[0].end_tick,
            550
        );
        assert!(kill_then_victim(&req, &[], (0, 20), (0, 0)).is_empty());
    }
}
