use crate::obs_wire::{ObsRpc, ObsSettings, OUTPUTS_SUBSCRIPTION};
use serde_json::Value;
use std::{io, path::PathBuf, time::Duration};
use tokio::time::{sleep_until, timeout, timeout_at, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const STATE_TIMEOUT: Duration = Duration::from_secs(3);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(80);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ownership {
    Unowned,
    Starting { observed_start: bool },
    Owned,
    Stopping,
    Revoked,
}

impl Ownership {
    fn observe(&mut self, event: &Value) -> io::Result<()> {
        let kind = event["eventType"].as_str().ok_or_else(invalid_event)?;
        if kind != "RecordStateChanged" {
            return Ok(());
        }
        let data = &event["eventData"];
        if event["eventIntent"].as_u64() != Some(OUTPUTS_SUBSCRIPTION) {
            return Err(invalid_event());
        }
        let active = data["outputActive"].as_bool().ok_or_else(invalid_event)?;
        match data["outputState"].as_str() {
            Some("OBS_WEBSOCKET_OUTPUT_STARTED") if active => match self {
                Self::Starting {
                    observed_start: false,
                } => {
                    *self = Self::Starting {
                        observed_start: true,
                    };
                }
                Self::Starting {
                    observed_start: true,
                }
                | Self::Owned
                | Self::Stopping => {
                    *self = Self::Revoked;
                }
                _ => {}
            },
            Some("OBS_WEBSOCKET_OUTPUT_STARTING") => {
                if matches!(
                    self,
                    Self::Starting {
                        observed_start: true
                    } | Self::Owned
                        | Self::Stopping
                ) {
                    *self = Self::Revoked;
                }
            }
            Some("OBS_WEBSOCKET_OUTPUT_STOPPING" | "OBS_WEBSOCKET_OUTPUT_STOPPED") => {
                if matches!(self, Self::Starting { .. } | Self::Owned) {
                    *self = Self::Revoked;
                }
            }
            Some("OBS_WEBSOCKET_OUTPUT_PAUSED" | "OBS_WEBSOCKET_OUTPUT_RESUMED") => {}
            Some(
                "OBS_WEBSOCKET_OUTPUT_UNKNOWN"
                | "OBS_WEBSOCKET_OUTPUT_RECONNECTING"
                | "OBS_WEBSOCKET_OUTPUT_RECONNECTED",
            ) => {
                if !matches!(self, Self::Unowned) {
                    *self = Self::Revoked;
                }
            }
            _ => return Err(invalid_event()),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordState {
    Idle,
    Recording,
    Paused,
}

/// Controls a recording this connection started and confirmed. The caller
/// must hold the application's recording lease. OBS v5 has no capture ID or
/// conditional stop, so an external client can still race the checked stop.
pub(super) struct ObsCapture {
    rpc: Option<ObsRpc>,
    ownership: Ownership,
    scene: Option<String>,
}

impl ObsCapture {
    pub(super) async fn connect(settings: ObsSettings) -> io::Result<Self> {
        let rpc = timeout(CONNECT_TIMEOUT, ObsRpc::connect_recording(settings))
            .await
            .map_err(|_| timed_out("OBS connection timed out"))?
            .map_err(|error| {
                io::Error::new(error.kind(), format!("OBS connection failed: {error}"))
            })?;
        Ok(Self {
            rpc: Some(rpc),
            ownership: Ownership::Unowned,
            scene: None,
        })
    }

    pub(super) async fn recording_directory(&mut self) -> io::Result<PathBuf> {
        let result = self
            .request("GetRecordDirectory", Instant::now() + REQUEST_TIMEOUT)
            .await?;
        result["recordDirectory"]
            .as_str()
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "OBS returned no recording directory",
                )
            })
    }

    pub(super) async fn require_idle(&mut self) -> io::Result<()> {
        if self.status(Instant::now() + REQUEST_TIMEOUT).await? != RecordState::Idle {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "OBS is already recording or paused",
            ));
        }
        Ok(())
    }

    pub(super) async fn prepare_scene(&mut self, scene: String) -> io::Result<()> {
        self.require_scene(&scene).await?;
        self.scene = Some(scene);
        Ok(())
    }

    async fn require_scene(&mut self, scene: &str) -> io::Result<()> {
        let result = self
            .request("GetCurrentProgramScene", Instant::now() + REQUEST_TIMEOUT)
            .await?;
        if result["currentProgramSceneName"].as_str() != Some(scene) {
            return Err(io::Error::other(format!(
                "请先在 OBS 中切换到录制场景「{scene}」"
            )));
        }
        Ok(())
    }

    pub(super) async fn start(&mut self) -> io::Result<()> {
        if self.ownership != Ownership::Unowned {
            return Err(io::Error::other(
                "OBS capture is already owned or unconfirmed",
            ));
        }
        self.require_idle().await?;
        if let Some(scene) = self.scene.clone() {
            self.require_scene(&scene).await?;
        }
        // Neither a lost acknowledgement nor an active status on its own proves ownership.
        self.ownership = Ownership::Starting {
            observed_start: false,
        };
        self.request("StartRecord", Instant::now() + REQUEST_TIMEOUT)
            .await?;
        self.wait_for("StartRecord", RecordState::Recording, STATE_TIMEOUT)
            .await?;
        self.ownership = Ownership::Owned;
        Ok(())
    }

    pub(super) async fn stop(&mut self) -> io::Result<Option<PathBuf>> {
        self.stop_owned(false).await
    }

    pub(super) async fn pause(&mut self) -> io::Result<()> {
        self.change_transport("PauseRecord", RecordState::Recording, RecordState::Paused).await
    }

    pub(super) async fn resume(&mut self) -> io::Result<()> {
        self.change_transport("ResumeRecord", RecordState::Paused, RecordState::Recording).await
    }

    async fn change_transport(
        &mut self,
        method: &str,
        before: RecordState,
        after: RecordState,
    ) -> io::Result<()> {
        self.require_owned()?;
        if self.status(Instant::now() + REQUEST_TIMEOUT).await? != before {
            return Err(io::Error::other("OBS 录制状态已变化，无法继续拼接片段"));
        }
        if after == RecordState::Recording {
            if let Some(scene) = self.scene.clone() {
                self.require_scene(&scene).await?;
            }
        }
        self.require_owned()?;
        self.request(method, Instant::now() + REQUEST_TIMEOUT).await?;
        self.wait_for(method, after, STATE_TIMEOUT).await
    }

    /// Stop a confirmed capture still active on this connection. An ambiguous
    /// acknowledgement requires recovery outside this controller.
    pub(super) async fn finish_if_owned(&mut self) -> io::Result<Option<PathBuf>> {
        if matches!(self.ownership, Ownership::Unowned | Ownership::Revoked) {
            return Ok(None);
        }
        self.stop_owned(true).await
    }

    async fn stop_owned(&mut self, already_idle_ok: bool) -> io::Result<Option<PathBuf>> {
        self.require_owned()?;
        let status = self.status(Instant::now() + REQUEST_TIMEOUT).await?;
        if status == RecordState::Idle || self.ownership == Ownership::Revoked {
            self.ownership = Ownership::Revoked;
            return if already_idle_ok {
                Ok(None)
            } else {
                Err(ownership_lost())
            };
        }
        // Do not retry a stop whose outcome is unknown, or stop a new recording after our ack.
        self.ownership = Ownership::Stopping;
        let result = self
            .request("StopRecord", Instant::now() + STOP_TIMEOUT)
            .await?;
        self.wait_for("StopRecord", RecordState::Idle, STOP_TIMEOUT)
            .await?;
        self.ownership = Ownership::Unowned;
        let path = result["outputPath"].as_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "OBS StopRecord returned no valid outputPath",
            )
        })?;
        Ok((!path.trim().is_empty()).then(|| PathBuf::from(path)))
    }

    fn require_owned(&self) -> io::Result<()> {
        match self.ownership {
            Ownership::Owned => Ok(()),
            Ownership::Unowned => Err(io::Error::other("This controller owns no OBS recording")),
            Ownership::Revoked => Err(ownership_lost()),
            Ownership::Starting { .. } | Ownership::Stopping => Err(io::Error::other(
                "OBS recording ownership could not be confirmed",
            )),
        }
    }

    async fn wait_for(
        &mut self,
        method: &str,
        expected: RecordState,
        limit: Duration,
    ) -> io::Result<()> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if self.ownership == Ownership::Revoked {
                return Err(ownership_lost());
            }
            let status = self
                .status(deadline.min(Instant::now() + REQUEST_TIMEOUT))
                .await?;
            if self.ownership == Ownership::Revoked {
                return Err(ownership_lost());
            }
            if status == expected
                && self.ownership
                    != (Ownership::Starting {
                        observed_start: false,
                    })
            {
                return Ok(());
            }
            sleep_until(deadline.min(Instant::now() + POLL_INTERVAL)).await;
        }
        Err(timed_out(format!(
            "OBS {method} did not reach recording state {expected:?}"
        )))
    }

    async fn status(&mut self, deadline: Instant) -> io::Result<RecordState> {
        let value = self.request("GetRecordStatus", deadline).await?;
        match (
            value["outputActive"].as_bool(),
            value["outputPaused"].as_bool(),
        ) {
            (Some(false), Some(false)) => Ok(RecordState::Idle),
            (Some(true), Some(false)) => Ok(RecordState::Recording),
            (Some(true), Some(true)) => Ok(RecordState::Paused),
            _ => {
                self.rpc = None;
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "OBS GetRecordStatus returned an invalid recording state",
                ))
            }
        }
    }

    async fn request(&mut self, method: &str, deadline: Instant) -> io::Result<Value> {
        // If an await is cancelled or the reply is lost, do not send another request on a
        // connection that may still deliver the previous reply.
        let mut rpc = self.rpc.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "OBS recording connection is no longer usable",
            )
        })?;
        let mut observe = |event: &Value| self.ownership.observe(event);
        let reply = timeout_at(
            deadline,
            rpc.request_observing(method, Value::Null, &mut observe),
        )
        .await
        .map_err(|_| timed_out(format!("OBS {method} acknowledgement timed out")))?
        .map_err(|error| io::Error::new(error.kind(), format!("OBS {method} failed: {error}")))?;
        rpc.drain_events(observe)?;
        self.rpc = Some(rpc);
        Ok(reply)
    }
}

fn invalid_event() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid OBS recording state event",
    )
}

fn ownership_lost() -> io::Error {
    io::Error::other("The owned OBS recording stopped or was replaced")
}

fn timed_out(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::{net::TcpListener, task::JoinHandle};
    use tokio_websockets::{Message, ServerBuilder};

    struct Reply {
        method: &'static str,
        payload: Value,
        before: Vec<Value>,
        after: Vec<Value>,
    }

    impl Reply {
        fn ok(method: &'static str, payload: Value) -> Self {
            Self {
                method,
                payload,
                before: Vec::new(),
                after: Vec::new(),
            }
        }

        fn status(active: bool) -> Self {
            Self::ok(
                "GetRecordStatus",
                json!({"outputActive":active,"outputPaused":false}),
            )
        }
    }

    fn event(state: &str, active: bool) -> Value {
        json!({"op":5,"d":{"eventType":"RecordStateChanged","eventIntent":64,
                           "eventData":{"outputActive":active,"outputState":state}}})
    }

    async fn serve(
        replies: Vec<Reply>,
        subscriptions: u64,
        start_reply_lost: bool,
    ) -> (ObsSettings, JoinHandle<()>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let (_, mut socket) = ServerBuilder::new().accept(tcp).await.unwrap();
            socket
                .send(Message::text(
                    json!({"op":0,"d":{"rpcVersion":1}}).to_string(),
                ))
                .await
                .unwrap();
            let identify = socket.next().await.unwrap().unwrap();
            let identify: Value = serde_json::from_str(identify.as_text().unwrap()).unwrap();
            assert_eq!(identify["op"], 1);
            assert_eq!(identify["d"]["eventSubscriptions"], subscriptions);
            socket
                .send(Message::text(
                    json!({"op":2,"d":{"negotiatedRpcVersion":1}}).to_string(),
                ))
                .await
                .unwrap();
            for step in replies {
                let request = socket.next().await.unwrap().unwrap();
                let request: Value = serde_json::from_str(request.as_text().unwrap()).unwrap();
                assert_eq!(request["op"], 6);
                assert_eq!(request["d"]["requestType"], step.method);
                for message in step.before {
                    socket
                        .send(Message::text(message.to_string()))
                        .await
                        .unwrap();
                }
                if step.method == "StartRecord" && start_reply_lost {
                    return;
                }
                let response = json!({"op":7,"d":{
                    "requestType":step.method,"requestId":request["d"]["requestId"],
                    "requestStatus":{"result":true,"code":100},"responseData":step.payload
                }});
                socket
                    .feed(Message::text(response.to_string()))
                    .await
                    .unwrap();
                for message in step.after {
                    socket
                        .feed(Message::text(message.to_string()))
                        .await
                        .unwrap();
                }
                socket.flush().await.unwrap();
            }
            // A StopRecord after the last status would stop somebody else's recording.
            if let Some(Ok(message)) = socket.next().await {
                assert!(
                    message.as_text().is_none(),
                    "unexpected additional OBS request"
                );
            }
        });
        (
            serde_json::from_value(json!({"host":"localhost","port":port,"password":""})).unwrap(),
            task,
        )
    }

    fn startup(start_before_ack: bool) -> Vec<Reply> {
        let mut start = Reply::ok("StartRecord", Value::Null);
        start
            .before
            .push(event("OBS_WEBSOCKET_OUTPUT_STARTING", false));
        let mut started = event("OBS_WEBSOCKET_OUTPUT_STARTED", true);
        if start_before_ack {
            started["d"]["eventData"]["outputPath"] = Value::Null;
            start.before.push(started);
        } else {
            started["d"]["eventData"]["outputPath"] = json!("C:\\fixture\\recording.mkv");
            start.after.push(started);
        }
        vec![Reply::status(false), start, Reply::status(true)]
    }

    fn bounded_test(test: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { timeout(Duration::from_secs(5), test).await.unwrap() });
    }

    #[test]
    fn never_starts_recording_after_the_program_scene_changes() {
        bounded_test(async {
            let (settings, server) = serve(
                vec![
                    Reply::ok(
                        "GetCurrentProgramScene",
                        json!({"currentProgramSceneName":"fixture game"}),
                    ),
                    Reply::status(false),
                    Reply::ok(
                        "GetCurrentProgramScene",
                        json!({"currentProgramSceneName":"fixture desktop"}),
                    ),
                ],
                OUTPUTS_SUBSCRIPTION,
                false,
            )
            .await;
            let mut capture = ObsCapture::connect(settings).await.unwrap();
            capture
                .prepare_scene("fixture game".to_owned())
                .await
                .unwrap();
            assert!(capture.start().await.is_err());
            assert!(capture.finish_if_owned().await.unwrap().is_none());
            drop(capture);
            server.await.unwrap();
        });
    }

    #[test]
    fn starts_and_stops_the_same_capture_when_started_events_bracket_the_ack() {
        bounded_test(async {
            for start_before_ack in [true, false] {
                let mut replies = vec![
                    Reply::ok(
                        "GetRecordDirectory",
                        json!({"recordDirectory":"C:\\fixture"}),
                    ),
                    Reply::status(false),
                ];
                replies.extend(startup(start_before_ack));
                replies.push(Reply::status(true));
                let mut stop = Reply::ok(
                    "StopRecord",
                    json!({"outputPath":"C:\\fixture\\recording.mkv"}),
                );
                stop.before
                    .push(event("OBS_WEBSOCKET_OUTPUT_STOPPING", true));
                stop.after
                    .push(event("OBS_WEBSOCKET_OUTPUT_STOPPED", false));
                replies.extend([stop, Reply::status(false)]);
                let (settings, server) = serve(replies, 64, false).await;
                let mut capture = ObsCapture::connect(settings).await.unwrap();
                assert_eq!(
                    capture.recording_directory().await.unwrap(),
                    PathBuf::from("C:\\fixture")
                );
                capture.require_idle().await.unwrap();
                capture.start().await.unwrap();
                assert_eq!(
                    capture.stop().await.unwrap(),
                    Some(PathBuf::from("C:\\fixture\\recording.mkv"))
                );
                assert_eq!(capture.finish_if_owned().await.unwrap(), None);
                drop(capture);
                server.await.unwrap();
            }
        });
    }

    #[test]
    fn pauses_and_resumes_one_owned_capture_between_segments() {
        bounded_test(async {
            let paused = || Reply::ok("GetRecordStatus", json!({"outputActive":true,"outputPaused":true}));
            let mut replies = startup(true);
            let stop = Reply::ok("StopRecord", json!({"outputPath":"C:\\fixture\\joined.mkv"}));
            replies.extend([
                Reply::status(true), Reply::ok("PauseRecord", Value::Null), paused(),
                paused(), Reply::ok("ResumeRecord", Value::Null), Reply::status(true),
                Reply::status(true), stop, Reply::status(false),
            ]);
            let (settings, server) = serve(replies, OUTPUTS_SUBSCRIPTION, false).await;
            let mut capture = ObsCapture::connect(settings).await.unwrap();
            assert!(capture.pause().await.is_err());
            capture.start().await.unwrap();
            capture.pause().await.unwrap();
            capture.resume().await.unwrap();
            assert_eq!(capture.stop().await.unwrap(), Some(PathBuf::from("C:\\fixture\\joined.mkv")));
            drop(capture);
            server.await.unwrap();
        });
    }

    #[test]
    fn subsequent_active_status_cannot_restore_ownership_after_observed_replacement() {
        bounded_test(async {
            for (boundary, after_status_reply) in [
                ("OBS_WEBSOCKET_OUTPUT_STOPPING", false),
                ("OBS_WEBSOCKET_OUTPUT_STOPPED", false),
                ("OBS_WEBSOCKET_OUTPUT_STARTING", false),
                ("OBS_WEBSOCKET_OUTPUT_STOPPED", true),
            ] {
                let mut replies = startup(true);
                let mut preflight = Reply::status(true);
                let events = if after_status_reply {
                    &mut preflight.after
                } else {
                    &mut preflight.before
                };
                events.push(event(boundary, false));
                events.push(event("OBS_WEBSOCKET_OUTPUT_STARTED", true));
                replies.push(preflight);
                let (settings, server) = serve(replies, 64, false).await;
                let mut capture = ObsCapture::connect(settings).await.unwrap();
                capture.start().await.unwrap();
                assert!(capture.stop().await.is_err());
                assert_eq!(capture.finish_if_owned().await.unwrap(), None);
                drop(capture);
                server.await.unwrap();
            }
        });
    }

    #[test]
    fn malformed_record_event_invalidates_mutation_on_the_connection() {
        bounded_test(async {
            let mut replies = startup(false);
            let mut malformed = event("OBS_WEBSOCKET_OUTPUT_STOPPED", false);
            malformed["d"]["eventData"]["outputState"] = json!(10);
            let mut preflight = Reply::status(true);
            preflight.before.push(malformed);
            replies.push(preflight);
            let (settings, server) = serve(replies, 64, false).await;
            let mut capture = ObsCapture::connect(settings).await.unwrap();
            capture.start().await.unwrap();
            assert_eq!(
                capture.stop().await.unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert!(capture.finish_if_owned().await.is_err());
            drop(capture);
            server.await.unwrap();
        });
    }

    #[test]
    fn a_started_event_without_the_start_reply_does_not_authorize_cleanup() {
        bounded_test(async {
            let (settings, server) = serve(startup(true), 64, true).await;
            let mut capture = ObsCapture::connect(settings).await.unwrap();
            assert!(capture.start().await.is_err());
            assert!(capture.finish_if_owned().await.is_err());
            server.await.unwrap();
        });
    }

    #[test]
    fn blank_directory_and_already_active_status_fail_without_recording_actions() {
        bounded_test(async {
            let (settings, server) = serve(
                vec![
                    Reply::ok("GetRecordDirectory", json!({"recordDirectory":"  "})),
                    Reply::status(true),
                    Reply::status(true),
                ],
                64,
                false,
            )
            .await;
            let mut capture = ObsCapture::connect(settings).await.unwrap();
            assert_eq!(
                capture.recording_directory().await.unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(
                capture.require_idle().await.unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(
                capture.start().await.unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(capture.finish_if_owned().await.unwrap(), None);
            drop(capture);
            server.await.unwrap();
        });
    }

    #[test]
    fn direct_rpc_keeps_events_disabled_and_recording_requires_parsed_loopback() {
        bounded_test(async {
            let (settings, server) = serve(vec![Reply::status(false)], 0, false).await;
            let mut direct = ObsRpc::connect(settings).await.unwrap();
            direct
                .request("GetRecordStatus", Value::Null)
                .await
                .unwrap();
            drop(direct);
            server.await.unwrap();
            let outside =
                serde_json::from_value(json!({"host":"192.0.2.1","port":4455,"password":""}))
                    .unwrap();
            assert_eq!(
                ObsCapture::connect(outside).await.err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
        });
    }
}
