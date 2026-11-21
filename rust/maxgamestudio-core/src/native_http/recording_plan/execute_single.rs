//! Execute the ordinary single-POV segment with an owned OBS capture.
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
    pub(super) elapsed: Duration,
}

pub(super) async fn run(
    game: &mut impl DemoControl,
    capture: &mut impl SegmentCapture,
    segment: Segment<'_>,
    cancel: &mut watch::Receiver<bool>,
) -> io::Result<Artifact> {
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

    // Never cancel the lifecycle call in flight. Exactly one acknowledged
    // start must precede our stop, even if a stop request arrives meanwhile.
    capture.start().await?;
    let started = Instant::now();
    let recorded = async {
        if *cancel.borrow() {
            return Err(interrupted());
        }
        game.command(GameCommand::ResumeWithoutConsole).await?;
        let remaining = segment
            .duration
            .saturating_sub(overshoot)
            .max(Duration::from_millis(100));
        match select(
            Box::pin(sleep_or_cancel(remaining, cancel)),
            Box::pin(game.wait_exit()),
        )
        .await
        {
            Either::Left((result, _)) | Either::Right((result, _)) => result,
        }
    }
    .await;
    let elapsed = started.elapsed();
    let stopped = capture.stop().await;
    let path = match (recorded, stopped) {
        (Ok(()), Ok(Some(path))) => path,
        (Ok(()), Ok(None)) => return Err(io::Error::other("OBS did not return a recording path")),
        (Err(error), Ok(_)) => return Err(error),
        (Ok(()), Err(error)) => return Err(error),
        (Err(error), Err(stop)) => {
            return Err(io::Error::other(format!(
                "{error}; OBS recording cleanup could not be confirmed: {stop}"
            )));
        }
    };
    Ok(Artifact {
        file: path,
        elapsed,
    })
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
                    Segment {
                        seek_tick: 500,
                        pre_roll: Duration::from_millis(20),
                        duration: Duration::from_millis(20),
                        view: View {
                            map: "de_fixture",
                            round: 2,
                            steam_id: 76561198000000000,
                            slot: 3,
                        },
                    },
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
                    assert!(artifact.elapsed >= Duration::from_millis(100));
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
}
