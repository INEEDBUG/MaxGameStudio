use super::request::{
    strip, EventInfo, NormalizedRequest, Perspective, RequestType, SourceType, TargetPlayer,
};
use super::{event_segment, pov_interleave, RecordingSegment};

pub(super) fn plan(req: &NormalizedRequest) -> Vec<RecordingSegment> {
    let mut segments = match req.request_type {
        RequestType::Highlight => highlight(req),
        RequestType::TimelineKill => timeline_kill(req),
        RequestType::Fail | RequestType::TimelineDeath => death(req),
        _ => unreachable!("event clip planner received a non-clip request"),
    };
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.segment_index = index;
    }
    segments
}

fn highlight(req: &NormalizedRequest) -> Vec<RecordingSegment> {
    let opts = &req.options;
    let killer = timing(req, opts.highlight_pre_sec, opts.highlight_post_sec);
    let victim = victim_timing(req, opts.highlight_pre_sec);
    let mut sorted: Vec<_> = req.events.iter().collect();
    sorted.sort_by_key(|event| event.tick);
    if opts.enable_victim_pov && opts.interleave_pov_pairs {
        return pov_interleave::kill_then_victim(req, &sorted, killer, victim);
    }

    let threshold = (opts.kill_jump_cut_threshold_sec * req.demo.tick_rate) as i64;
    let mut segments: Vec<_> = sorted
        .chunk_by(|left, right| {
            i128::from(right.tick) - i128::from(left.tick) <= i128::from(threshold)
        })
        .map(|group| {
            let first = group[0];
            let last = group[group.len() - 1];
            event_segment(
                req,
                SourceType::Kill,
                Perspective::Killer,
                first.round,
                &req.target_player,
                (
                    first.tick.saturating_sub(killer.0),
                    last.tick.saturating_add(killer.1),
                ),
                group.iter().map(|event| event.tick).collect(),
            )
        })
        .collect();
    if opts.enable_victim_pov {
        // Grouped highlights can still follow the victim by name when the ID is absent.
        segments.extend(
            sorted
                .into_iter()
                .map(|event| victim_segment(req, event, victim)),
        );
    }
    segments
}

fn timeline_kill(req: &NormalizedRequest) -> Vec<RecordingSegment> {
    let opts = &req.options;
    let event = &req.events[0];
    let killer = timing(req, opts.timeline_kill_pre_sec, opts.timeline_kill_post_sec);
    let mut segments = vec![single_event_segment(
        req,
        event,
        SourceType::Kill,
        Perspective::Killer,
        &req.target_player,
        killer,
    )];
    if opts.enable_victim_pov {
        let mut victim = victim_segment(req, event, victim_timing(req, opts.timeline_kill_pre_sec));
        if victim.target_steamid64.is_empty() {
            victim.disabled = true;
            victim.disabled_reason = Some("missing_victim_steamid64".to_owned());
        }
        segments.push(victim);
    }
    segments
}

fn death(req: &NormalizedRequest) -> Vec<RecordingSegment> {
    let opts = &req.options;
    let event = &req.events[0];
    let timeline = req.request_type == RequestType::TimelineDeath;
    let killer = timing(req, opts.fail_killer_pre_sec, opts.fail_killer_post_sec);
    let victim = if timeline {
        killer
    } else {
        timing(req, opts.death_pre_sec, opts.death_post_sec)
    };
    let mut segments = vec![single_event_segment(
        req,
        event,
        SourceType::Death,
        Perspective::Victim,
        &req.target_player,
        victim,
    )];
    if opts.enable_fail_killer_pov {
        let mut killer = single_event_segment(
            req,
            event,
            SourceType::Death,
            Perspective::Killer,
            &event.killer,
            killer,
        );
        if timeline {
            killer.target_steamid64 = strip(&killer.target_steamid64).to_owned();
        }
        if killer.target_steamid64.is_empty() {
            killer.disabled = true;
            killer.disabled_reason = Some("missing_killer_steamid64".to_owned());
            if !timeline {
                killer.target_spec_slot = None;
            }
        }
        segments.push(killer);
    }
    segments
}

fn victim_segment(
    req: &NormalizedRequest,
    event: &EventInfo,
    ticks: (i64, i64),
) -> RecordingSegment {
    let mut segment = single_event_segment(
        req,
        event,
        SourceType::Kill,
        Perspective::Victim,
        &event.victim,
        ticks,
    );
    segment.target_steamid64 = strip(&segment.target_steamid64).to_owned();
    segment
}

fn single_event_segment(
    req: &NormalizedRequest,
    event: &EventInfo,
    source_type: SourceType,
    perspective: Perspective,
    target: &TargetPlayer,
    (pre, post): (i64, i64),
) -> RecordingSegment {
    event_segment(
        req,
        source_type,
        perspective,
        event.round,
        target,
        (
            event.tick.saturating_sub(pre),
            event.tick.saturating_add(post),
        ),
        vec![event.tick],
    )
}

fn victim_timing(req: &NormalizedRequest, default_pre: f64) -> (i64, i64) {
    timing(
        req,
        req.options.victim_pov_pre_sec.unwrap_or(default_pre),
        req.options.victim_pov_post_sec,
    )
}

fn timing(req: &NormalizedRequest, pre_seconds: f64, post_seconds: f64) -> (i64, i64) {
    let rate = req.demo.tick_rate;
    ((pre_seconds * rate) as i64, (post_seconds * rate) as i64)
}
