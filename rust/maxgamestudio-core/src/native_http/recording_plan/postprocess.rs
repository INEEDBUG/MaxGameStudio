use super::{request::strip, NormalizedRequest, Perspective, RecordingSegment};
use serde_json::Value;
use std::fmt::Write;

pub(super) fn finish(
    req: &NormalizedRequest,
    segments: Vec<RecordingSegment>,
) -> (Vec<RecordingSegment>, Vec<RecordingSegment>, Vec<String>) {
    let mut active = Vec::new();
    let mut disabled = Vec::new();
    let mut warnings = Vec::new();
    let slot_offset = platform_slot_offset(&req.demo.demo_filename, &req.demo.server_name);
    let guard = ((req.options.demo_end_guard_sec * req.demo.tick_rate) as i64).max(1);
    let safe_end = req.demo.demo_end_tick.saturating_sub(guard);
    for mut segment in segments {
        segment.start_tick = segment.start_tick.max(req.demo.first_tick);
        segment.end_tick = segment.end_tick.min(req.demo.demo_end_tick);
        segment.safe_end_tick = Some(safe_end);
        if let Some(&anchor) = segment
            .anchor_ticks
            .iter()
            .max()
            .filter(|&&tick| tick >= safe_end)
        {
            warnings.push(format!(
                "segment {}: anchor_too_close_to_demo_end (safe_end={safe_end}, max_anchor={anchor})",
                segment.segment_index,
            ));
            disable(&mut segment, "anchor_too_close_to_demo_end");
        } else if segment.end_tick.min(safe_end) <= segment.start_tick {
            disable(&mut segment, "too_close_to_demo_end");
        } else {
            segment.end_tick = segment.end_tick.min(safe_end);
        }
        let (team, enemy) = voice_masks(
            &req.demo.all_players,
            &segment.target_steamid64,
            slot_offset,
        );
        segment.voice_listen_mask = Some(team);
        segment.voice_listen_mask_enemy = Some(enemy);

        let name = strip(&segment.target_player_name);
        if !segment.disabled && segment.perspective == Perspective::Victim && !name.is_empty() {
            let label = name_repr(&segment.target_player_name);
            if strip(&segment.target_steamid64).is_empty() {
                warnings.push(format!(
                    "segment {}: victim segment missing steamid64 for {label} — will spectate by name without verification",
                    segment.segment_index,
                ));
            }
            if segment.target_spec_slot.is_none() {
                warnings.push(format!(
                    "segment {}: victim {label} missing spec_slot — will spectate by name",
                    segment.segment_index,
                ));
            }
        }
        if segment.disabled {
            disabled.push(segment);
        } else {
            segment.segment_index = active.len();
            active.push(segment);
        }
    }
    (active, disabled, warnings)
}

fn name_repr(name: &str) -> String {
    let quote = if name.contains('\'') && !name.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut text = String::with_capacity(name.len() + 2);
    text.push(quote);
    for ch in name.chars() {
        match ch {
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            '\\' => text.push_str("\\\\"),
            ch if ch == quote => {
                text.push('\\');
                text.push(ch);
            }
            ch if ch.is_control() => {
                write!(text, "\\x{:02x}", u32::from(ch)).expect("string write")
            }
            ch => text.push(ch),
        }
    }
    text.push(quote);
    text
}

fn disable(segment: &mut RecordingSegment, reason: &str) {
    segment.disabled = true;
    segment.disabled_reason = Some(reason.to_owned());
}

fn platform_slot_offset(filename: &str, server: &str) -> i64 {
    let server = server.to_lowercase();
    let platforms = [
        ("faceit", 0),
        ("5e", 1),
        ("完美世界", 1),
        ("wanmei", 1),
        ("valve", 0),
        ("esl", 0),
        ("esea", 0),
        ("blast", 0),
        ("pgl", 0),
        ("starladder", 0),
        ("flashpoint", 0),
        ("challengermode", 0),
    ];
    if let Some(offset) = platforms
        .iter()
        .find_map(|(name, offset)| server.contains(name).then_some(*offset))
    {
        return offset;
    }
    let name = filename.to_lowercase();
    let digit_prefix = |raw: &str, separator: &str| {
        raw.split_once(separator).is_some_and(|(digits, _)| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    };
    if name
        .strip_prefix('g')
        .is_some_and(|name| digit_prefix(name, "-"))
    {
        1
    } else if digit_prefix(&name, "_team") || name.contains("faceit") {
        0
    } else {
        i64::from(name.contains("5e") || name.contains("perfectworld") || name.contains("pvp"))
    }
}

fn voice_masks(roster: &[Value], expected_steam_id: &str, offset: i64) -> (u64, u64) {
    let steam_id = |value: &Value| match value {
        Value::String(value) => value.trim().to_owned(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    };
    let expected_steam_id = expected_steam_id.trim();
    if expected_steam_id.is_empty() {
        return (0, 0);
    }
    let mut found_t = false;
    let mut found_ct = false;
    for player in roster
        .iter()
        .filter(|player| steam_id(&player["steamid64"]) == expected_steam_id)
    {
        match integer(&player["team_num"]) {
            Some(2) => found_t = true,
            Some(3) => found_ct = true,
            _ => {}
        }
    }
    let team = match (found_t, found_ct) {
        (true, false) => 2,
        (false, true) => 3,
        _ => return (0, 0),
    };
    let mut masks = [0, 0];
    for player in roster
        .iter()
        .filter(|player| !steam_id(&player["steamid64"]).is_empty())
    {
        let Some(player_team @ (2 | 3)) = integer(&player["team_num"]) else {
            continue;
        };
        let slot = integer(&player["spec_slot"])
            .and_then(|slot| slot.checked_add(offset))
            .filter(|slot| (1..=64).contains(slot));
        if let Some(slot) = slot {
            masks[usize::from(player_team != team)] |= 1_u64 << (slot - 1);
        }
    }
    (masks[0], masks[1])
}

fn integer(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .map(|value| value as i64)
        })
        .or_else(|| value.as_str()?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn voice_masks_use_the_current_pov_team_and_preserve_slot_64() {
        let roster = vec![
            json!({"steamid64":76561198000000001_u64, "spec_slot":63,"team_num":"2"}),
            json!({"steamid64":"76561198000000002", "spec_slot":1,"team_num":3}),
            json!({"steamid64":"", "spec_slot":2,"team_num":2}),
        ];
        let offset = platform_slot_offset("g1234-test.dem", "");
        assert_eq!(
            voice_masks(&roster, "76561198000000001", offset),
            (1 << 63, 2)
        );
        assert_eq!(
            voice_masks(&roster, "76561198000000002", offset),
            (2, 1 << 63)
        );
        let conflicting = [
            roster[0].clone(),
            json!({"steamid64":"76561198000000001","team_num":3}),
        ];
        assert_eq!(
            voice_masks(&conflicting, "76561198000000001", offset),
            (0, 0)
        );
        assert_eq!(platform_slot_offset("g1234-test.dem", "Faceit"), 0);
    }
}
