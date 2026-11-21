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
