use maxgamestudio_core::gsi_ready::GsiReady;
use serde_json::json;
use std::time::Duration;

#[test]
fn game_payloads_mark_ready_but_menus_and_provider_only_data_do_not() {
    let state = GsiReady::default();
    assert!(!state.notify(json!({"provider":{"name":"Counter-Strike 2"}}), 1.0));
    assert!(!state.notify(
        json!({"map":{"name":"de_mirage"},"player":{"activity":" loading "}}),
        2.0
    ));
    assert!(state.notify(json!({"map":{"round":0}}), 3.0));
    assert!(state.wait_ready(Duration::ZERO));
    assert!(!state.notify(json!({"player":{"activity":"menu"}}), 4.0));
    let status = state.status();
    assert!(status.ready);
    assert_eq!(status.last_ready_at, 3.0);
    assert_eq!(status.last_payload_at, 4.0);
    assert_eq!(status.last_payload["player"]["activity"], "menu");
}

#[test]
fn reset_clears_a_previous_game_and_waiters_observe_fresh_payloads() {
    let state = GsiReady::default();
    assert!(state.notify(json!({"round":{"phase":"live"}}), 1.0));
    state.reset();
    assert!(!state.wait_ready(Duration::ZERO));
    assert_eq!(state.status().last_payload, json!({}));
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| state.wait_payload_after(1.0, Duration::from_secs(2)));
        assert!(state.notify(json!({"phase_countdowns":{"phase":"freezetime"}}), 2.0));
        assert_eq!(waiter.join().unwrap().last_payload_at, 2.0);
    });
}
