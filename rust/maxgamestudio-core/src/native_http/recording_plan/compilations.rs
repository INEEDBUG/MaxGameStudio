use super::{
    event_segment, pov_interleave,
    request::{strip, EventInfo, NormalizedRequest, Perspective, RequestType, SourceType},
    RecordingSegment,
};

pub(super) fn plan(req: &NormalizedRequest) -> Vec<RecordingSegment> {
    let mut events = req.events.iter().collect::<Vec<_>>();
    events.sort_by_key(|event| (event.round, event.tick));
    match req.request_type {
        RequestType::KillCompilation => kills(req, &events),
        RequestType::DeathCompilation => deaths(req, &events),
        _ => unreachable!("compilation type from the recording plan dispatcher"),
    }
}

fn kills(req: &NormalizedRequest, events: &[&EventInfo]) -> Vec<RecordingSegment> {
    let options = &req.options;
    let ticks = |seconds| (seconds * req.demo.tick_rate) as i64;
    let killer = (
        ticks(options.kill_compilation_pre_sec),
        ticks(options.kill_compilation_post_sec),
    );
    let victim = (
        ticks(
            options
                .victim_pov_pre_sec
                .unwrap_or(options.kill_compilation_pre_sec),
        ),
        ticks(options.victim_pov_post_sec),
    );
    if options.enable_victim_pov && options.interleave_pov_pairs {
        return pov_interleave::kill_then_victim(req, events, killer, victim);
    }
    let limit = ticks(options.kill_compilation_jump_cut_threshold_sec);
    let groups = events.chunk_by(|previous, next| {
        previous.round == next.round
            && previous.target_player.steamid64 == next.target_player.steamid64
            && previous.perspective == next.perspective
            && next.tick.saturating_sub(previous.tick) <= limit
    });
    let mut segments = group_segments(req, SourceType::Kill, Perspective::Killer, killer, groups);
    if options.enable_victim_pov {
        for event in events {
            let bounds = (
                event.tick.saturating_sub(victim.0),
                event.tick.saturating_add(victim.1),
            );
            let mut segment = event_segment(
                req,
                SourceType::Kill,
                Perspective::Victim,
                event.round,
                &event.victim,
                bounds,
                vec![event.tick],
            );
            segment.segment_index = segments.len();
            segment.target_steamid64 = strip(&event.victim.steamid64).to_owned();
            segments.push(segment);
        }
    }
    segments
}

fn deaths(req: &NormalizedRequest, events: &[&EventInfo]) -> Vec<RecordingSegment> {
    let options = &req.options;
    let ticks = |seconds| (seconds * req.demo.tick_rate) as i64;
    let victim = (
        ticks(options.death_compilation_pre_sec),
        ticks(options.death_compilation_post_sec),
    );
    let killer = (
        ticks(options.fail_killer_pre_sec),
        ticks(options.fail_killer_post_sec),
    );
    if options.enable_fail_killer_pov && options.interleave_pov_pairs {
        return pov_interleave::victim_then_killer(req, events, victim, killer);
    }
    let merge_gap = ticks(options.death_compilation_merge_gap_sec);
    let groups = events.chunk_by(|previous, next| {
        previous.round == next.round
            && next
                .tick
                .saturating_sub(victim.0)
                .saturating_sub(previous.tick.saturating_add(victim.1))
                <= merge_gap
    });
    let mut segments = group_segments(req, SourceType::Death, Perspective::Victim, victim, groups);
    if options.enable_fail_killer_pov {
        for event in events {
            let bounds = (
                event.tick.saturating_sub(killer.0),
                event.tick.saturating_add(killer.1),
            );
            let mut segment = event_segment(
                req,
                SourceType::Death,
                Perspective::Killer,
                event.round,
                &event.killer,
                bounds,
                vec![event.tick],
            );
            segment.segment_index = segments.len();
            segment.target_steamid64 = strip(&event.killer.steamid64).to_owned();
            if segment.target_steamid64.is_empty() {
                segment.disabled = true;
                segment.disabled_reason = Some("missing_killer_steamid64".to_owned());
            }
            segments.push(segment);
        }
    }
    segments
}

fn group_segments<'a>(
    req: &NormalizedRequest,
    source_type: SourceType,
    perspective: Perspective,
    (pre, post): (i64, i64),
    groups: impl Iterator<Item = &'a [&'a EventInfo]>,
) -> Vec<RecordingSegment> {
    groups
        .enumerate()
        .map(|(index, group)| {
            let first = group[0];
            let last = group.last().expect("nonempty compilation group");
            let bounds = (
                first.tick.saturating_sub(pre),
                last.tick.saturating_add(post),
            );
            let mut segment = event_segment(
                req,
                source_type,
                perspective,
                first.round,
                &req.target_player,
                bounds,
                group.iter().map(|event| event.tick).collect(),
            );
            segment.segment_index = index;
            segment
        })
        .collect()
}
