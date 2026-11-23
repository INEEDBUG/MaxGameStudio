//! Execute planned POV segments with one owned OBS capture.
//! GSI verifies the observed map/round/player. Demo tick and pause timing
//! remain estimates, as in the released recorder.

use futures_util::future::{select, Either};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::{sync::watch, time::sleep};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GameCommand {
    SeekPaused(i64),
    SelectPlayer(i64),
    PauseWithoutConsole,
    ResumeWithoutConsole,
}

pub(super) trait DemoControl {
    async fn command(&mut self, command: GameCommand) -> io::Result<()>;
    async fn wait_view(&mut self, requested: &View<'_>, after: Instant) -> io::Result<()>;
    async fn wait_exit(&mut self) -> io::Result<()>;
}

pub(super) trait SegmentCapture {
    async fn start(&mut self) -> io::Result<()>;
    async fn pause(&mut self) -> io::Result<()>;
    async fn resume(&mut self) -> io::Result<()>;
    async fn stop(&mut self) -> io::Result<Option<PathBuf>>;
}

pub(super) struct View<'a> {
    pub(super) map: &'a str,
    pub(super) round: i64,
    pub(super) steam_id: u64,
    pub(super) slot: i64,
}

pub(super) struct Segment<'a> {
    pub(super) seek_tick: i64,
    pub(super) pre_roll: Duration,
    pub(super) duration: Duration,
    pub(super) view: View<'a>,
}

pub(super) struct Artifact {
    pub(super) file: PathBuf,
    pub(super) durations: Vec<Duration>,
}

pub(super) async fn run(
    game: &mut impl DemoControl,
    capture: &mut impl SegmentCapture,
    segments: &[Segment<'_>],
    cancel: &mut watch::Receiver<bool>,
) -> io::Result<Artifact> {
    if segments.is_empty() {
        return Err(io::Error::other("没有可录制的片段"));
    }
    let mut durations = Vec::with_capacity(segments.len());
    let mut started = false;
    let record = async {
        for (index, segment) in segments.iter().enumerate() {
            let overshoot = prepare(game, segment, cancel).await?;
            // Wait for acknowledgement before allowing cancellation to stop the capture.
            if index == 0 {
                capture.start().await?;
                started = true;
            } else {
                capture.resume().await?;
            }
            let measuring = Instant::now();
            capture_segment(game, segment, overshoot, cancel).await?;
            durations.push(measuring.elapsed());
            if index + 1 < segments.len() {
                capture.pause().await?;
            }
        }
        Ok::<_, io::Error>(())
    }
    .await;
    let stopped = if started {
        capture.stop().await
    } else {
        Ok(None)
    };
    let file = match (record, stopped) {
        (Ok(()), Ok(Some(path))) => path,
        (Ok(()), Ok(None)) => return Err(io::Error::other("OBS did not return a recording path")),
        (Err(error), Ok(_)) | (Ok(()), Err(error)) => return Err(error),
        (Err(error), Err(stop)) => {
            return Err(io::Error::other(format!(
                "{error}; OBS recording cleanup could not be confirmed: {stop}"
            )));
        }
    };
    Ok(Artifact { file, durations })
}

async fn prepare(
    game: &mut impl DemoControl,
    segment: &Segment<'_>,
    cancel: &mut watch::Receiver<bool>,
) -> io::Result<Duration> {
    if *cancel.borrow() {
        return Err(interrupted());
    }
    game.command(GameCommand::SeekPaused(segment.seek_tick))
        .await?;
    sleep_or_cancel(Duration::from_millis(1200), cancel).await?;

    game.command(GameCommand::ResumeWithoutConsole).await?;
    let before_selection = Instant::now();
    game.command(GameCommand::SelectPlayer(segment.view.slot))
        .await?;
    let view = game.wait_view(&segment.view, before_selection);
    let cancel_view = cancel.wait_for(|value| *value);
    match select(Box::pin(view), Box::pin(cancel_view)).await {
        Either::Left((result, _)) => result?,
        Either::Right(_) => return Err(interrupted()),
    }
    let elapsed = before_selection.elapsed();
    let overshoot = elapsed.saturating_sub(segment.pre_roll);
    if overshoot > Duration::from_millis(200) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "POV confirmation overran the recording start",
        ));
    }
    sleep_or_cancel(segment.pre_roll.saturating_sub(elapsed), cancel).await?;
    game.command(GameCommand::PauseWithoutConsole).await?;
    sleep_or_cancel(Duration::from_millis(50), cancel).await?;
    Ok(overshoot)
}

async fn capture_segment(
    game: &mut impl DemoControl,
    segment: &Segment<'_>,
    overshoot: Duration,
    cancel: &mut watch::Receiver<bool>,
) -> io::Result<()> {
    if *cancel.borrow() {
        return Err(interrupted());
    }
    game.command(GameCommand::ResumeWithoutConsole).await?;
    let remaining = segment.duration.saturating_sub(overshoot).max(Duration::from_millis(100));
    match select(
        Box::pin(sleep_or_cancel(remaining, cancel)),
        Box::pin(game.wait_exit()),
    )
    .await
    {
        Either::Left((result, _)) | Either::Right((result, _)) => result,
    }
}

async fn sleep_or_cancel(duration: Duration, cancel: &mut watch::Receiver<bool>) -> io::Result<()> {
    if *cancel.borrow() {
        return Err(interrupted());
    }
    match select(
        Box::pin(sleep(duration)),
        Box::pin(cancel.wait_for(|value| *value)),
    )
    .await
    {
        Either::Left(_) => Ok(()),
        Either::Right(_) => Err(interrupted()),
    }
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Recording cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    struct FakeGame {
        matched: bool,
        trace: Rc<RefCell<Vec<&'static str>>>,
    }

    impl DemoControl for FakeGame {
        async fn wait_exit(&mut self) -> io::Result<()> {
            std::future::pending().await
        }
        async fn command(&mut self, command: GameCommand) -> io::Result<()> {
            let label = match command {
                GameCommand::SeekPaused(500) => "seek",
                GameCommand::SeekPaused(900) => "second seek",
                GameCommand::SelectPlayer(3) => "select",
                GameCommand::PauseWithoutConsole => "pause",
                GameCommand::ResumeWithoutConsole => "resume",
                _ => panic!("unexpected recording command"),
            };
            self.trace.borrow_mut().push(label);
            Ok(())
        }

        async fn wait_view(&mut self, requested: &View<'_>, after: Instant) -> io::Result<()> {
            assert_eq!(
                (requested.map, requested.round, requested.steam_id),
                ("de_fixture", 2, 76561198000000000)
            );
            assert!(Instant::now() >= after);
            self.trace.borrow_mut().push("view");
            self.matched
                .then_some(())
                .ok_or_else(|| io::Error::other("wrong spectator"))
        }
    }

    struct FakeCapture {
        trace: Rc<RefCell<Vec<&'static str>>>,
        on_start: Option<watch::Sender<bool>>,
    }

    impl SegmentCapture for FakeCapture {
        async fn start(&mut self) -> io::Result<()> {
            self.trace.borrow_mut().push("obs start");
            if let Some(sender) = &self.on_start {
                sender.send_replace(true);
            }
            Ok(())
        }

        async fn stop(&mut self) -> io::Result<Option<PathBuf>> {
            self.trace.borrow_mut().push("obs stop");
            Ok(Some(PathBuf::from("synthetic-video.mp4")))
        }

        async fn pause(&mut self) -> io::Result<()> {
            self.trace.borrow_mut().push("obs pause");
            Ok(())
        }

        async fn resume(&mut self) -> io::Result<()> {
            self.trace.borrow_mut().push("obs resume");
            Ok(())
        }
    }

    fn fixture_segment(seek_tick: i64) -> Segment<'static> {
        Segment {
            seek_tick,
            pre_roll: Duration::from_millis(20),
            duration: Duration::from_millis(20),
            view: View {
                map: "de_fixture", round: 2, steam_id: 76561198000000000, slot: 3,
            },
        }
    }

    #[test]
    fn verifies_view_and_closes_only_its_started_capture_on_cancel() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            for (matched, cancel_on_start) in [(true, false), (true, true), (false, false)] {
                let trace = Rc::new(RefCell::new(Vec::new()));
                let (send, mut cancel) = watch::channel(false);
                let mut game = FakeGame {
                    matched,
                    trace: Rc::clone(&trace),
                };
                let mut capture = FakeCapture {
                    trace: Rc::clone(&trace),
                    on_start: cancel_on_start.then(|| send.clone()),
                };
                let result = run(
                    &mut game,
                    &mut capture,
                    &[fixture_segment(500)],
                    &mut cancel,
                )
                .await;
                let trace = trace.borrow();
                if !matched {
                    assert!(result.is_err());
                    assert_eq!(trace.as_slice(), ["seek", "resume", "select", "view"]);
                } else if cancel_on_start {
                    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::Interrupted);
                    assert_eq!(
                        trace.as_slice(),
                        [
                            "seek",
                            "resume",
                            "select",
                            "view",
                            "pause",
                            "obs start",
                            "obs stop"
                        ]
                    );
                } else {
                    let artifact = result.unwrap();
                    assert_eq!(artifact.file, PathBuf::from("synthetic-video.mp4"));
                    assert!(artifact.durations[0] >= Duration::from_millis(100));
                    assert_eq!(
                        trace.as_slice(),
                        [
                            "seek",
                            "resume",
                            "select",
                            "view",
                            "pause",
                            "obs start",
                            "resume",
                            "obs stop"
                        ]
                    );
                }
            }
        });
    }

    #[test]
    fn pauses_before_seeking_to_another_position_in_the_same_round() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        runtime.block_on(async {
            let trace = Rc::new(RefCell::new(Vec::new()));
            let (_send, mut cancel) = watch::channel(false);
            let mut game = FakeGame { matched: true, trace: Rc::clone(&trace) };
            let mut capture = FakeCapture { trace: Rc::clone(&trace), on_start: None };
            let artifact = run(&mut game, &mut capture, &[fixture_segment(500), fixture_segment(900)], &mut cancel)
                .await.unwrap();
            assert_eq!(artifact.file, PathBuf::from("synthetic-video.mp4"));
            assert_eq!(artifact.durations.len(), 2);
            assert_eq!(trace.borrow().as_slice(), [
                "seek", "resume", "select", "view", "pause", "obs start", "resume", "obs pause",
                "second seek", "resume", "select", "view", "pause", "obs resume", "resume", "obs stop",
            ]);
        });
    }
}
