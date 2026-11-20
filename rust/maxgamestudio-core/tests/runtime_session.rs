use maxgamestudio_core::runtime_session::{OperationOwner, RuntimeSession};
use serde_json::json;

fn owner(id: &str, operation: &str) -> OperationOwner {
    OperationOwner {
        id: id.into(),
        operation: operation.into(),
        started_at: "2026-11-20T18:44:00+00:00".into(),
    }
}

#[test]
fn reports_the_current_operation_and_rejects_overlapping_game_or_obs_work() {
    let session = RuntimeSession::default();
    let recording = owner("one", "/api/recording/execute");
    let playback = owner("two", "/api/demo/play");
    let held = session.try_begin(recording.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(session.state()).unwrap(),
        json!({"busy": true, "owner": recording}),
    );
    let error = std::thread::scope(|thread| {
        thread
            .spawn(|| session.try_begin(playback.clone()).unwrap_err())
            .join()
            .unwrap()
    });
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({
            "code": "RUNTIME_SESSION_BUSY",
            "message": "Another CS2/OBS operation is already running.",
            "owner": recording,
        }),
    );

    drop(held);
    assert_eq!(
        serde_json::to_value(session.state()).unwrap(),
        json!({"busy": false, "owner": null}),
    );
    assert!(session.try_begin(playback).is_ok());
}

#[test]
fn unwinding_an_operation_releases_its_claim() {
    let session = RuntimeSession::default();
    let result = std::panic::catch_unwind(|| {
        let _claim = session
            .try_begin(owner("one", "/api/recording/execute"))
            .unwrap();
        panic!("fixture operation failed");
    });
    assert!(result.is_err());
    assert!(!session.state().busy);
}
