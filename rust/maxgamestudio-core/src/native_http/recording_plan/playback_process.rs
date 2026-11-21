//! Launch an off-line Demo under an exclusive runtime lease. Only the child
//! launched by this session is eligible for termination.

use super::{
    playback_gsi::{GameUpdate, PlaybackFeed},
    playback_spec::{PlaybackSpec, STEAM_ENV},
    playback_stage::{lock_directories, PlaybackStage},
};
use crate::{profile_platform::probe_process, runtime_session::RuntimeLease};
use futures_util::future::{select, Either};
use std::{
    io,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    process::{Child, Command},
    sync::watch,
    time::timeout,
};

struct Prepared {
    spec: PlaybackSpec,
    stage: Option<PlaybackStage>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(stage) = self.stage.as_mut() {
            let _ = stage.finish();
        }
    }
}

pub(super) struct PlaybackProcess<'a> {
    _lease: &'a RuntimeLease<'a>,
    child: Child,
    stage: Option<PlaybackStage>,
    launched: Instant,
}

impl<'a> PlaybackProcess<'a> {
    pub(super) async fn launch(
        lease: &'a RuntimeLease<'a>,
        spec: PlaybackSpec,
        cancelled: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let staging_cancel = Arc::clone(&cancelled);
        let mut prepared = tokio::task::spawn_blocking(move || {
            require_idle()?;
            let stage = PlaybackStage::stage(&spec, &staging_cancel)?;
            Ok::<_, io::Error>(Prepared {
                spec,
                stage: Some(stage),
            })
        })
        .await
        .map_err(io::Error::other)??;
        if cancelled.load(Ordering::Acquire) {
            return Err(interrupted());
        }
        require_idle()?;
        // Keep the file open across process creation to prevent replacement of
        // the verified executable between opening it and launching it.
        let _directories = lock_directories(
            prepared
                .spec
                .executable
                .parent()
                .expect("canonical game executable parent"),
        )?;
        let _image = hold_executable(&prepared.spec)?;
        let launched = Instant::now();
        let child = Command::new(&prepared.spec.executable)
            .args(&prepared.spec.args)
            .envs(STEAM_ENV)
            .current_dir(&prepared.spec.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .spawn()?;
        let stage = prepared.stage.take();
        Ok(Self {
            _lease: lease,
            child,
            stage,
            launched,
        })
    }

    pub(super) fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub(super) fn running_pid(&mut self) -> io::Result<u32> {
        if self.child.try_wait()?.is_none() {
            return self.pid().ok_or_else(exited);
        }
        Err(exited())
    }

    pub(super) async fn wait_for_exit(&mut self) -> io::Result<()> {
        self.child.wait().await?;
        Err(exited())
    }

    pub(super) async fn wait_ready(
        &mut self,
        feed: &mut PlaybackFeed<'_>,
        map: &str,
        cancel: &mut watch::Receiver<bool>,
        limit: Duration,
    ) -> io::Result<GameUpdate> {
        if *cancel.borrow() {
            return Err(interrupted());
        }
        let launched = self.launched;
        let world = async {
            let update = feed
                .updates
                .wait_for(|message| {
                    message.as_ref().is_some_and(|message| {
                        message.received >= launched && message.map.eq_ignore_ascii_case(map)
                    })
                })
                .await
                .map_err(io::Error::other)?;
            Ok::<_, io::Error>(update.as_ref().expect("matching GSI payload").clone())
        };
        let stop = cancel.wait_for(|cancelled| *cancelled);
        let ready = select(Box::pin(world), Box::pin(stop));
        let result = timeout(limit, select(Box::pin(self.child.wait()), Box::pin(ready))).await;
        match result {
            Ok(Either::Left((status, _))) => {
                status?;
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Owned CS2 exited before playback became ready",
                ))
            }
            Ok(Either::Right((Either::Left((world, _)), _))) => world,
            Ok(Either::Right((Either::Right(_), _))) => Err(interrupted()),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "CS2 playback did not become ready",
            )),
        }
    }

    /// Reap our child before removing the stage files. On a cleanup error the
    /// caller can retry; a replacement file always stays outside our ownership.
    pub(super) async fn finish(&mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }
        let Some(mut stage) = self.stage.take() else {
            return Ok(());
        };
        let (stage, finished) = tokio::task::spawn_blocking(move || {
            let finished = stage.finish();
            (stage, finished)
        })
        .await
        .map_err(io::Error::other)?;
        if finished.is_err() {
            self.stage = Some(stage);
        }
        finished
    }
}

fn require_idle() -> io::Result<()> {
    if probe_process("cs2.exe")? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "CS2 is already running",
        ));
    }
    Ok(())
}

fn hold_executable(spec: &PlaybackSpec) -> io::Result<std::fs::File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    };
    let image = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&spec.executable)?;
    let metadata = image.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CS2 executable is not an ordinary file",
        ));
    }
    Ok(image)
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Playback cancelled")
}

fn exited() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "录制启动的 CS2 已退出")
}

#[cfg(test)]
mod tests {
    use super::super::{
        playback_gsi::PlaybackGsi,
        playback_spec::{self, GsiTarget},
    };
    use super::*;
    use crate::runtime_session::{OperationOwner, RuntimeSession};
    use serde_json::json;
    use std::{fs, num::NonZeroU16, path::PathBuf};

    #[test]
    #[ignore = "requires an isolated, synthetic executable; never run with a real game"]
    fn owns_only_the_fixture_process_and_cleans_its_recording_files() {
        let test_root = PathBuf::from(std::env::var_os("MGS_PLAYBACK_TEST_ROOT").unwrap());
        let fixture = test_root.join("synthetic-playback-fixture.exe");
        assert!(fixture.is_file());
        let scratch = tempfile::tempdir_in(&test_root).unwrap();
        let game = scratch.path().join("game");
        fs::create_dir_all(game.join("bin/win64")).unwrap();
        fs::create_dir_all(game.join("csgo/cfg")).unwrap();
        let executable = game.join("bin/win64/cs2.exe");
        fs::copy(fixture, &executable).unwrap();
        let source = scratch.path().join("Synthetic Demo.dem");
        fs::write(&source, b"unchanged input bytes").unwrap();
        let token = "a".repeat(64);
        let spec = playback_spec::build(
            &executable,
            &source,
            "0123456789abcdef0123456789abcdef",
            "F10",
            None,
            GsiTarget {
                port: NonZeroU16::new(19111).unwrap(),
                token: &token,
            },
        )
        .unwrap();
        let artifacts = [
            spec.staged_demo.clone(),
            spec.demo_cfg_path.clone(),
            spec.gsi_cfg_path.clone(),
        ];
        let service = RuntimeSession::default();
        let guard = service
            .try_begin(OperationOwner {
                id: "isolated-playback-fixture".to_owned(),
                operation: "recording".to_owned(),
                started_at: String::new(),
            })
            .unwrap();
        let gsi = PlaybackGsi::default();
        let mut feed = gsi.begin(token.clone()).unwrap();
        let message = json!({
            "provider":{"appid":730},"auth":{"token":token},
            "map":{"name":"de_synthetic","round":3,"team_ct":{"score":1},"team_t":{"score":2}},"round":{"phase":"live"}
        });
        // A packet from before the process launch must never fulfill readiness.
        gsi.notify(&message).unwrap();
        let (stop, mut cancel) = watch::channel(false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut process =
                PlaybackProcess::launch(&guard, spec, Arc::new(AtomicBool::new(false)))
                    .await
                    .unwrap();
            let pid = process.pid().expect("owned CS2 fixture");
            assert!(pid > 0 && artifacts.iter().all(|path| path.is_file()));
            let stale = process
                .wait_ready(
                    &mut feed,
                    "de_synthetic",
                    &mut cancel,
                    Duration::from_millis(100),
                )
                .await
                .err()
                .expect("stale payload");
            assert_eq!(stale.kind(), io::ErrorKind::TimedOut);
            gsi.notify(&message).unwrap();
            let fresh = process
                .wait_ready(
                    &mut feed,
                    "de_synthetic",
                    &mut cancel,
                    Duration::from_secs(2),
                )
                .await
                .unwrap();
            assert_eq!(fresh.round, Some(4));
            stop.send_replace(true);
            let cancelled = process
                .wait_ready(
                    &mut feed,
                    "de_synthetic",
                    &mut cancel,
                    Duration::from_secs(2),
                )
                .await
                .err()
                .expect("cancelled wait");
            assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
            process.finish().await.unwrap();
            assert!(process.pid().is_none() && artifacts.iter().all(|path| !path.exists()));
        });
        drop(guard);
        assert!(fs::read(&source).unwrap() == b"unchanged input bytes" && !service.state().busy);
    }
}
