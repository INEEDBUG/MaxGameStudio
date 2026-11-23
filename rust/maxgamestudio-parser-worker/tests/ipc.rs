use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    process::{Command, Output, Stdio},
};

fn call(input: &[u8]) -> Output {
    let mut worker = Command::new(env!("CARGO_BIN_EXE_maxgamestudio-parser-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    worker.stdin.take().unwrap().write_all(input).unwrap();
    worker.wait_with_output().unwrap()
}

fn request(action: Value) -> Output {
    call(&serde_json::to_vec(&action).unwrap())
}

fn fixture_checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn result(output: &Output) -> Value {
    assert_eq!(output.stdout.split(|&byte| byte == b'\n').count(), 2);
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["protocol"], 1);
    assert_eq!(
        reply["parser_commit"],
        "db3ba5f7a1b02b8cb27d0063d52649883a98469c"
    );
    reply
}

fn fixture_header() -> Vec<u8> {
    let mut header = Vec::new();
    for (wire_key, text) in [(0x0a, "PBDEMS2"), (0x2a, "de_fixture"), (0x32, "csgo")] {
        header.extend([wire_key, text.len() as u8]);
        header.extend_from_slice(text.as_bytes());
    }
    let mut demo = b"PBDEMS2\0".to_vec();
    demo.extend_from_slice(&((header.len() + 4) as u32).to_le_bytes());
    demo.extend_from_slice(&[0; 4]);
    demo.extend([1, 0, header.len() as u8]); // DEM_FileHeader, tick zero, one protobuf frame.
    demo.extend(header);
    demo.extend([0, 0, 0]); // DEM_Stop, tick zero, empty payload.
    demo
}

#[test]
fn exposes_its_pin_and_parses_a_real_source2_header() {
    let probe = request(json!({"protocol":1,"action":"probe"}));
    assert!(probe.status.success());
    assert!(probe.stderr.is_empty());
    assert_eq!(
        result(&probe)["result"],
        json!({
            "actions": ["probe", "inspect_header", "write_replay_parquet", "extract_replay_effect_rows", "inspect_demo", "inspect_roster", "inspect_round_windows", "inspect_match_timeline", "inspect_inferno_tracks", "inspect_effect_tracks", "inspect_projectile_trajectories"], "upstream_tag": "v0.41.4"
        })
    );

    let scratch = tempfile::tempdir().unwrap();
    let sample = scratch.path().join("合成 fixture.dem");
    let data = fixture_header();
    fs::write(&sample, &data).unwrap();
    let inspected = request(json!({"protocol":1,"action":"inspect_header","path":sample}));
    assert!(
        inspected.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&inspected.stdout),
        String::from_utf8_lossy(&inspected.stderr)
    );
    let header = &result(&inspected)["result"]["header"];
    assert_eq!(header["map_name"], "de_fixture");
    assert_eq!(header["demo_file_stamp"], "PBDEMS2");
    assert_eq!(header["game_directory"], "csgo");
    assert_eq!(fs::read(sample).unwrap(), data);
}

#[test]
fn inspection_preserves_a_real_header_and_unknown_match_fields_without_a_roster() {
    let scratch = tempfile::tempdir().unwrap();
    let sample = scratch.path().join("合成 fixture.dem");
    let source = fixture_header();
    fs::write(&sample, &source).unwrap();
    let inspected = request(json!({"protocol":1,"action":"inspect_demo","path":sample}));
    assert!(
        inspected.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&inspected.stdout),
        String::from_utf8_lossy(&inspected.stderr)
    );
    assert_eq!(
        result(&inspected)["result"],
        json!({
            "players": [],
            "match_meta": {
                "map_name":"de_fixture", "server_name":"", "target_player":"",
                "target_player_user_id":null, "target_steam_id":null,
                "total_rounds":null, "target_kills":null, "target_deaths":null,
                "team_a_score":null, "team_b_score":null, "team_a_name":null, "team_b_name":null,
                "match_date":"", "duration_mins":null, "demo_end_tick":null,
                "inspection_level":"header_signon",
            }
        })
    );
    assert_eq!(fs::read(sample).unwrap(), source);
}

#[test]
fn requested_match_summary_reports_no_rounds_for_a_header_only_demo() {
    let scratch = tempfile::tempdir().unwrap();
    let sample = scratch.path().join("header-only.dem");
    let source = fixture_header();
    fs::write(&sample, &source).unwrap();
    let inspected = request(json!({
        "protocol": 1, "action": "inspect_round_windows", "path": sample,
        "tick_rate": 64.0, "include_match_summary": true,
    }));
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stdout)
    );
    assert_eq!(
        result(&inspected)["result"],
        json!({
            "match_start_tick": 0, "demo_end_tick": 0, "rounds": [],
            "match_meta": {
                "map_name": "de_fixture", "server_name": "", "target_player": "",
                "target_player_user_id": null, "target_steam_id": null,
                "total_rounds": 0, "target_kills": 0, "target_deaths": 0,
                "team_a_score": 0, "team_b_score": 0, "team_a_name": "Team A",
                "team_b_name": "Team B", "match_date": "", "duration_mins": 0, "demo_end_tick": 0,
            },
        })
    );
    assert_eq!(fs::read(sample).unwrap(), source);
}

#[test]
#[ignore = "requires the pinned public demo via MGS_ANALYSIS_EVENTS_FIXTURE"]
fn opt_in_match_facts_keep_the_legacy_round_scores_and_default_timeline() {
    let sample = std::path::PathBuf::from(
        std::env::var_os("MGS_ANALYSIS_EVENTS_FIXTURE").expect("public demo fixture path"),
    );
    let mut payload = json!({
        "protocol":1, "action":"inspect_match_timeline", "path":sample, "tick_rate":64,
    });
    let default = request(payload.clone());
    assert!(default.status.success());
    let default_bytes = default
        .stdout
        .strip_suffix(b"\r\n")
        .or_else(|| default.stdout.strip_suffix(b"\n"))
        .unwrap();
    assert_eq!(fixture_checksum(default_bytes), 0xed28e20cc8b12a6a);
    let default = result(&default)["result"].clone();
    assert!(default.get("match_facts").is_none());

    payload["include_match_facts"] = json!(true);
    let opt_in = request(payload.clone());
    assert!(opt_in.status.success());
    let mut opt_in = result(&opt_in)["result"].clone();
    let facts = opt_in
        .as_object_mut()
        .unwrap()
        .remove("match_facts")
        .unwrap();
    let summary = &facts["match_meta"];
    assert_eq!(summary["map_name"], "de_mirage");
    assert_eq!(summary["total_rounds"], 10);
    assert_eq!(
        (
            summary["team_a_score"].as_i64(),
            summary["team_b_score"].as_i64()
        ),
        (Some(8), Some(2))
    );
    assert!(summary.get("target_player").is_none());
    assert_eq!(facts["team_assignment_source"], "round_side_groups");
    assert_eq!(
        (
            facts["team_a_group"].as_i64(),
            facts["team_b_group"].as_i64()
        ),
        (Some(2), Some(3))
    );
    assert_eq!(
        facts["phase_meta"],
        json!({"halftime_round":null,"regulation_end_round":null})
    );

    let players = facts["players"].as_array().unwrap();
    assert_eq!(players.len(), 10);
    for (group, key) in [(2, "a"), (3, "b")] {
        assert_eq!(
            players
                .iter()
                .filter(|player| player["team_key"] == key && player["final_group"] == group)
                .count(),
            5
        );
    }
    assert!(players.iter().all(|player| {
        player["name"].as_str().is_some_and(|name| !name.is_empty())
            && player["steam_id64"]
                .as_str()
                .is_some_and(|id| id.starts_with("7656"))
    }));

    let stats = facts["player_stats"].as_array().unwrap();
    assert_eq!(stats.len(), 10);
    let averages = [4472, 2561, 3422, 2428, 4206, 4194, 2094, 4167, 2033, 2606];
    for ((stat, player), average) in stats.iter().zip(players).zip(averages) {
        assert_eq!(stat["steam_id64"], player["steam_id64"]);
        assert_eq!(stat["team_key"], player["team_key"]);
        assert_eq!(stat["average_equipment_value"], average);
        for field in ["rating_pro_2", "rating_pro_3"] {
            assert!(
                stat.get(field).is_none(),
                "unavailable stat {field} must stay absent"
            );
        }
    }
    for (field, count) in [
        ("kills", 64),
        ("deaths", 64),
        ("assists", 18),
        ("first_kills", 8),
        ("first_deaths", 8),
        ("trade_kills", 9),
        ("trade_deaths", 9),
        ("clutch_attempts", 9),
        ("clutch_wins", 2),
        ("utility_damage", 558),
    ] {
        let actual: i64 = stats.iter().map(|row| row[field].as_i64().unwrap()).sum();
        assert_eq!(actual, count, "legacy {field} total");
    }
    let adr_tenths: i64 = stats
        .iter()
        .map(|row| (row["adr"].as_f64().unwrap() * 10.0).round() as i64)
        .sum();
    assert_eq!(adr_tenths, 7746);
    let ranked = facts["ranked_player_ids"].as_array().unwrap();
    assert_eq!(ranked.len(), stats.len());
    let rank_ids: std::collections::HashSet<_> =
        ranked.iter().map(|id| id.as_str().unwrap()).collect();
    assert_eq!(
        rank_ids,
        stats
            .iter()
            .map(|row| row["steam_id64"].as_str().unwrap())
            .collect()
    );

    let mut scores = [0, 0];
    let mut legacy_annotations = Vec::new();
    let rows = opt_in["rounds"].as_array_mut().unwrap();
    assert_eq!(rows.len(), 9);
    let winners = ["b", "a", "a", "b", "a", "a", "a", "a", "a"];
    // Round-start equipment and player buy votes from the pinned legacy wheel.
    let economy = [
        (2400, 3650, "pistol", "pistol"),
        (14850, 3700, "full", "eco"),
        (17200, 26700, "full", "full"),
        (20800, 2650, "full", "eco"),
        (22100, 12450, "full", "full"),
        (23750, 25850, "full", "full"),
        (27150, 12400, "full", "force"),
        (27950, 4100, "full", "eco"),
        (27950, 14000, "full", "full"),
    ];
    for ((round, winner), (equipment_a, equipment_b, buy_a, buy_b)) in
        rows.iter_mut().zip(winners).zip(economy)
    {
        let row = round.as_object_mut().unwrap();
        assert_eq!(row.remove("winner_team_key"), Some(json!(winner)));
        assert_eq!(row.remove("team_a_side"), Some(json!("T")));
        assert_eq!(row.remove("team_b_side"), Some(json!("CT")));
        assert_eq!(row.remove("team_a_score_before"), Some(json!(scores[0])));
        assert_eq!(row.remove("team_b_score_before"), Some(json!(scores[1])));
        scores[usize::from(winner == "b")] += 1;
        assert_eq!(row.remove("team_a_score_after"), Some(json!(scores[0])));
        assert_eq!(row.remove("team_b_score_after"), Some(json!(scores[1])));
        assert_eq!(
            row.remove("team_a_equipment_value"),
            Some(json!(equipment_a))
        );
        assert_eq!(
            row.remove("team_b_equipment_value"),
            Some(json!(equipment_b))
        );
        assert_eq!(row.remove("team_a_economy"), Some(json!(buy_a)));
        assert_eq!(row.remove("team_b_economy"), Some(json!(buy_b)));
        let mut annotations: serde_json::Map<String, Value> = [
            "site",
            "headline",
            "tags",
            "duration_seconds",
            "special_events",
        ]
        .into_iter()
        .map(|field| (field.to_owned(), row.remove(field).unwrap()))
        .collect();
        // The released Python clutch list uses a set for player iteration.
        annotations
            .get_mut("special_events")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .sort_by_cached_key(Value::to_string);
        legacy_annotations.push(json!([row["round_number"], annotations]));
    }
    assert_eq!(scores, [7, 2]);
    // Canonical annotation rows recorded from the pinned legacy wheel.
    assert_eq!(
        fixture_checksum(&serde_json::to_vec(&legacy_annotations).unwrap()),
        0x4da3581a08cf87c6
    );
    assert!(
        opt_in == default,
        "opt-in fields must preserve the existing timeline"
    );

    let names: Vec<_> = players.iter().map(|player| player["name"].as_str().unwrap()).collect();
    payload["round_compilations"] = json!({"players":names,"rounds":[2,4,5]});
    let response = request(payload);
    assert!(response.status.success());
    let parsed = result(&response);
    let clips = &parsed["result"]["match_facts"]["round_compilations"];
    // Each clip is the released wheel's serialized result, with only its random ID omitted.
    for (kind, segments, digest) in [
        ("all_deaths", 64, 0x5ba3407f0eb2cb86),
        ("all_kills", 64, 0x42075bd89ecb24c9),
        ("freeze_to_death", 25, 0xf67cb212d57012a2),
    ] {
        let clips: Vec<_> = names
            .iter()
            .map(|name| {
                let mut clip = clips[*name]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|clip| clip["compilation_kind"] == kind)
                    .unwrap()
                    .clone();
                clip.as_object_mut().unwrap().remove("clip_id");
                clip
            })
            .collect();
        let count: usize = clips.iter().map(|clip| clip["source_ticks"].as_array().unwrap().len()).sum();
        assert_eq!(count, segments, "{kind} source segments");
        assert_eq!(fixture_checksum(&serde_json::to_vec(&clips).unwrap()), digest, "{kind} legacy parity");
    }
    let mut highlights = Vec::new();
    let mut single_highlights = Vec::new();
    for name in names {
        for clip in clips[name].as_array().unwrap() {
            if clip["category"] == "highlight" {
                let mut clip = clip.clone();
                clip.as_object_mut().unwrap().remove("clip_id");
                match clip["kill_count"].as_u64() {
                    Some(1) => single_highlights.push(clip),
                    Some(2..) => highlights.push(clip),
                    _ => panic!("unexpected kill count in public Demo highlight: {clip}"),
                }
            }
        }
    }
    assert_eq!(highlights.len(), 17);
    assert_eq!(
        highlights.iter().map(|clip| clip["kill_count"].as_u64().unwrap()).sum::<u64>(),
        35
    );
    assert_eq!(
        fixture_checksum(&serde_json::to_vec(&highlights).unwrap()),
        0xd39d7a3e29e405a1,
        "legacy ordinary multi-kill highlight parity"
    );
    assert_eq!(single_highlights.len(), 4);
    assert_eq!(
        fixture_checksum(&serde_json::to_vec(&single_highlights).unwrap()),
        0x87a6f668d685286c,
        "legacy single-kill highlight parity"
    );
}

#[test]
fn replay_writer_preserves_final_and_existing_staged_files() {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("sample.dem");
    let final_path = scratch.path().join("sample.parquet");
    let staged_path = scratch.path().join("sample.parquet.partial");
    fs::write(&source, fixture_header()).unwrap();
    fs::write(&final_path, b"retained cache").unwrap();
    fs::write(&staged_path, b"other worker output").unwrap();
    let call_output = |path: &std::path::Path| {
        request(json!({
            "protocol": 1, "action": "write_replay_parquet", "path": source,
            "output_path": path, "tick_rate": 64.0, "fps": 32.0,
            "rounds": [{"round_number":1,"start_tick":10,"end_tick":20}]
        }))
    };

    let final_write = call_output(&final_path);
    assert_eq!(final_write.status.code(), Some(2));
    assert_eq!(
        result(&final_write)["error"]["detail"],
        "expected an absolute .parquet.partial output path"
    );
    let collision = call_output(&staged_path);
    assert_eq!(collision.status.code(), Some(6));
    assert_eq!(
        result(&collision)["error"]["detail"],
        "staged replay output already exists"
    );
    assert_eq!(fs::read(final_path).unwrap(), b"retained cache");
    assert_eq!(fs::read(staged_path).unwrap(), b"other worker output");
}

#[test]
fn replay_writer_errors_leave_no_cache_artifact() {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("invalid.dem");
    let output_path = scratch.path().join("sample.parquet.partial");
    fs::write(&source, b"PBDEMS2").unwrap();
    let invalid = request(json!({
        "protocol": 1, "action": "write_replay_parquet", "path": source,
        "output_path": output_path, "tick_rate": 64.0, "fps": 32.0,
        "rounds": [{"round_number":1,"start_tick":10,"end_tick":20}]
    }));
    assert_eq!(invalid.status.code(), Some(5));
    assert_eq!(
        result(&invalid)["error"],
        json!({"code":"replay_parse_failed","stage":"parse_replay","detail":"invalid Source 2 demo framing"})
    );
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);
    assert_eq!(fs::read(source).unwrap(), b"PBDEMS2");
}

#[test]
fn replay_effect_extraction_validates_bounds_and_protects_staged_files() {
    let scratch = tempfile::tempdir().unwrap();
    let source = scratch.path().join("invalid.dem");
    let output = scratch.path().join("raw.effects.jsonl.zst.partial");
    let call = |end_tick: i32| {
        request(json!({
            "protocol": 1, "action": "extract_replay_effect_rows", "path": source,
            "output_path": output, "start_tick":30,"end_tick":end_tick,
        }))
    };
    fs::write(&source, b"PBDEMS2").unwrap();
    let invalid = call(40_030);
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(
        result(&invalid)["error"]["detail"],
        "expected an inclusive replay effect window of at most 40000 ticks"
    );
    let unreadable = call(40);
    assert_eq!(unreadable.status.code(), Some(5));
    assert_eq!(
        result(&unreadable)["error"]["detail"],
        "invalid Source 2 demo framing"
    );
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);

    fs::write(&output, b"other worker output").unwrap();
    let collision = call(40);
    assert_eq!(collision.status.code(), Some(6));
    assert_eq!(
        result(&collision)["error"]["detail"],
        "staged replay output already exists"
    );
    assert_eq!(fs::read(output).unwrap(), b"other worker output");
}

#[test]
fn invalid_or_missing_inputs_have_collectible_failures() {
    let scratch = tempfile::tempdir().unwrap();
    let demo = scratch.path().join("sample.dem");
    let missing = request(json!({"protocol":1,"action":"inspect_header","path":demo}));
    assert_eq!(missing.status.code(), Some(3));
    assert_eq!(
        result(&missing)["error"],
        json!({
            "code": "unreadable_demo", "stage": "open_demo", "detail": "could not open demo"
        })
    );
    assert!(!missing.stderr.is_empty());

    for (data, detail) in [
        (&b"PBDEMS2"[..], "truncated demo framing"),
        (&b"HL2DEMO\0\0\0\0\0\0\0\0\0"[..], "Source 1 demo"),
        (
            &b"PBDEMS2\0\0\0\0\0\0\0\0\0\x01\x00\x40"[..],
            "malformed or truncated demo header",
        ),
    ] {
        fs::write(&demo, data).unwrap();
        let invalid = request(json!({"protocol":1,"action":"inspect_header","path":demo}));
        assert_eq!(invalid.status.code(), Some(4));
        let error = result(&invalid)["error"].clone();
        assert_eq!(
            error,
            json!({"code":"invalid_demo_header","stage":"parse_header","detail":detail})
        );
        assert!(!invalid.stderr.is_empty());
        assert_eq!(fs::read(&demo).unwrap(), data);
    }
}

#[test]
fn rejects_unknown_protocols_and_bounds_the_request() {
    let wrong = request(json!({"protocol":2,"action":"probe"}));
    assert_eq!(wrong.status.code(), Some(2));
    assert_eq!(
        result(&wrong)["error"]["detail"],
        "unsupported parser worker protocol"
    );

    let oversized = call(&vec![b' '; 64 * 1024 + 1]);
    assert_eq!(oversized.status.code(), Some(2));
    assert_eq!(
        result(&oversized)["error"]["detail"],
        "request exceeds 64 KiB"
    );

    let bad = request(json!({"protocol":1,"action":"fixture-protected-input"}));
    assert_eq!(bad.status.code(), Some(2));
    assert_eq!(result(&bad)["error"]["code"], "invalid_request");
    for output in [bad.stdout, bad.stderr] {
        assert!(!String::from_utf8_lossy(&output).contains("fixture-protected-input"));
    }
}
