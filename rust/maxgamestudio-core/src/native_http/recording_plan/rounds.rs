use super::request::{NormalizedRequest, Perspective, RequestType, SourceType};
use super::RecordingSegment;
use serde_json::json;

const ESTIMATED_FREEZE_SECS: f64 = 15.0;
const NEXT_FREEZE_GUARD_SECS: f64 = 0.5;
const COMPILATION_ROUND_TAIL_SECS: f64 = 3.0;

pub(super) fn plan(req: &NormalizedRequest) -> (Vec<RecordingSegment>, Vec<String>) {
    let mut segments = Vec::with_capacity(req.rounds.len());
    let mut warnings = Vec::new();
    let to_ticks = |seconds: f64| (seconds * req.demo.tick_rate) as i64;
    let preroll = to_ticks(req.options.round_freeze_preroll_sec);
    let death_tail = to_ticks(req.options.round_death_post_sec);
    let round_tail = to_ticks(COMPILATION_ROUND_TAIL_SECS);
    let guard = to_ticks(NEXT_FREEZE_GUARD_SECS);
    let compilation = req.request_type == RequestType::RoundCompilation;

    for (segment_index, round) in req.rounds.iter().enumerate() {
        let freeze_end = round.freeze_end_tick.unwrap_or_else(|| {
            warnings.push(format!(
                "round {}: freeze_end_tick missing; used round_start_tick + 15s freeze as fallback for start_tick",
                round.round,
            ));
            round.round_start_tick.saturating_add(to_ticks(ESTIMATED_FREEZE_SECS))
        });
        let start_tick = freeze_end.saturating_sub(preroll).max(req.demo.first_tick);
        let round_end = round
            .round_end_tick
            .filter(|_| round.round_end_tick_reliable);
        let is_final_round = round.round == req.demo.final_round;

        let (mut end_tick, end_reason) = if let Some(death_tick) = round.target_death_tick {
            let mut end = death_tick.saturating_add(death_tail);
            let mut reason = "target_death_post";
            if let Some(round_end) = round_end {
                // In compilations a round-ending death retains its configured tail, up to 3 s.
                let cap = if compilation {
                    round_end.saturating_add(round_tail)
                } else {
                    round_end
                };
                if end > cap {
                    end = cap;
                    reason = if compilation {
                        "target_death_post_clamped_to_round_end_post"
                    } else {
                        "target_death_post_clamped_to_round_end"
                    };
                }
            }
            if let Some(next_start) = round.next_round_start_tick {
                let cap = if compilation {
                    next_start.saturating_sub(guard)
                } else {
                    next_start
                };
                if end > cap {
                    end = cap;
                    reason = "target_death_post_clamped_to_next_round_start";
                }
            }
            (end, reason)
        } else {
            // Stop before the next freeze phase so the recording latency cannot show its buy overlay.
            let (mut end, mut reason) = if let Some(round_end) = round_end {
                if compilation && !is_final_round {
                    (round_end.saturating_add(round_tail), "round_end_post")
                } else {
                    (round_end, "round_end")
                }
            } else if let Some(next_start) = round.next_round_start_tick {
                (
                    next_start.saturating_sub(guard),
                    "fallback_next_round_start",
                )
            } else if let Some(next_freeze) = round.next_round_freeze_start_tick {
                (
                    next_freeze.saturating_sub(guard),
                    "fallback_next_round_freeze_start",
                )
            } else {
                (req.demo.demo_end_tick, "fallback_demo_end")
            };
            if let Some(next_start) = round.next_round_start_tick {
                let cap = next_start.saturating_sub(guard);
                if end > cap {
                    end = cap;
                    reason = "round_end_clamped_to_next_round_start";
                }
            }
            (end, reason)
        };

        let final_extra = req.options.final_round_extra_post_sec;
        let end_reason = if is_final_round && round.target_death_tick.is_none() && final_extra > 0.0
        {
            end_tick = end_tick.saturating_add(to_ticks(final_extra));
            format!("{end_reason}+final_round_alive_extra")
        } else {
            end_reason.to_owned()
        };

        let metadata = [
            ("round_start_tick", json!(round.round_start_tick)),
            ("round_end_tick", json!(round.round_end_tick)),
            ("freeze_end_tick", json!(round.freeze_end_tick)),
            ("next_round_start_tick", json!(round.next_round_start_tick)),
            (
                "next_round_freeze_start_tick",
                json!(round.next_round_freeze_start_tick),
            ),
            (
                "next_round_freeze_end_tick",
                json!(round.next_round_freeze_end_tick),
            ),
            ("target_death_tick", json!(round.target_death_tick)),
            ("end_reason", json!(end_reason)),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();

        segments.push(RecordingSegment {
            segment_index,
            source_type: SourceType::Round,
            start_tick,
            end_tick: end_tick.min(req.demo.demo_end_tick),
            anchor_ticks: Vec::new(),
            round: Some(round.round),
            target_player_name: req.target_player.name.clone(),
            target_steamid64: req.target_player.steamid64.clone(),
            target_spec_slot: req.target_player.spec_slot,
            perspective: Perspective::Round,
            is_final_round,
            safe_seek_tick: start_tick,
            safe_end_tick: None,
            disabled: false,
            disabled_reason: None,
            metadata,
            voice_listen_mask: None,
            voice_listen_mask_enemy: None,
        });
    }

    (segments, warnings)
}
