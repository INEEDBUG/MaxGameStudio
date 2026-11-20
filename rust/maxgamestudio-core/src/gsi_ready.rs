//! Latest CS2 Game State Integration snapshot and recording readiness.

use serde::Serialize;
use serde_json::{json, Value};
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

#[derive(Clone, Serialize)]
pub struct GsiStatus {
    pub ready: bool,
    pub last_payload_at: f64,
    pub last_ready_at: f64,
    pub last_payload: Value,
}

impl Default for GsiStatus {
    fn default() -> Self {
        Self {
            ready: false,
            last_payload_at: 0.0,
            last_ready_at: 0.0,
            last_payload: json!({}),
        }
    }
}

#[derive(Default)]
pub struct GsiReady {
    status: Mutex<GsiStatus>,
    changed: Condvar,
}

impl GsiReady {
    pub fn reset(&self) {
        *self.status.lock().expect("GSI state") = GsiStatus::default();
        self.changed.notify_all();
    }

    /// The observer supplies the timestamp from the runtime's monotonic clock.
    pub fn notify(&self, payload: Value, at: f64) -> bool {
        let ready = payload_has_demo_world(&payload);
        {
            let mut status = self.status.lock().expect("GSI state");
            status.last_payload = if payload.is_object() {
                payload
            } else {
                json!({})
            };
            status.last_payload_at = at;
            if ready {
                status.ready = true;
                status.last_ready_at = at;
            }
        }
        self.changed.notify_all();
        ready
    }

    pub fn status(&self) -> GsiStatus {
        self.status.lock().expect("GSI state").clone()
    }

    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let (status, _) = self
            .changed
            .wait_timeout_while(self.status.lock().expect("GSI state"), timeout, |status| {
                !status.ready
            })
            .expect("GSI state");
        status.ready
    }

    pub fn wait_payload_after(&self, since: f64, timeout: Duration) -> GsiStatus {
        let (status, _) = self
            .changed
            .wait_timeout_while(self.status.lock().expect("GSI state"), timeout, |status| {
                status.last_payload_at <= since
                    || status
                        .last_payload
                        .as_object()
                        .is_none_or(|payload| payload.is_empty())
            })
            .expect("GSI state");
        status.clone()
    }
}

fn meaningful_text(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn payload_has_demo_world(payload: &Value) -> bool {
    if payload["player"]["activity"]
        .as_str()
        .is_some_and(|activity| {
            let activity = activity.trim();
            activity.eq_ignore_ascii_case("menu") || activity.eq_ignore_ascii_case("loading")
        })
    {
        return false;
    }
    let map = &payload["map"];
    ["name", "mode", "phase"]
        .iter()
        .any(|key| meaningful_text(&map[*key]))
        || !map["round"].is_null()
        || meaningful_text(&payload["round"]["phase"])
        || meaningful_text(&payload["phase_countdowns"]["phase"])
        || payload["allplayers"]
            .as_object()
            .is_some_and(|players| !players.is_empty())
}
