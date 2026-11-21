use futures_util::future::{select, Either};
use serde_json::Value;
use std::{
    env,
    future::pending,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::watch, time::timeout};

pub(super) fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}

pub(super) fn find_ffprobe(ffmpeg: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let adjacent = ffmpeg.with_file_name(name);
    if adjacent.is_file() {
        return adjacent.canonicalize().ok();
    }
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(name))
            .find(|path| path.is_file())
            .and_then(|path| path.canonicalize().ok())
    })
}

pub(super) async fn inspect(
    ffprobe: &Path,
    file: &Path,
    cancel: Option<&watch::Receiver<bool>>,
) -> io::Result<Value> {
    let interrupted = || io::Error::new(io::ErrorKind::Interrupted, "FFprobe cancelled");
    if cancel.is_some_and(|cancel| *cancel.borrow()) {
        return Err(interrupted());
    }
    const MAX_METADATA: u64 = 128 * 1024;
    let mut child = command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,codec_name,width,height,r_frame_rate,duration",
            "-of",
            "json",
        ])
        .arg(file)
        .stdout(Stdio::piped())
        .spawn()?;
    let capture = async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .expect("FFprobe stdout")
            .take(MAX_METADATA + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() as u64 > MAX_METADATA || !child.wait().await?.success() {
            return Err(io::Error::other(
                "FFprobe did not return bounded media metadata",
            ));
        }
        Ok(bytes)
    };
    let cancellation = async {
        if let Some(signal) = cancel {
            let mut signal = signal.clone();
            let _ = signal.wait_for(|value| *value).await;
        } else {
            pending::<()>().await;
        }
    };
    let result = match select(
        Box::pin(timeout(Duration::from_secs(12), capture)),
        Box::pin(cancellation),
    )
    .await
    {
        Either::Left((Ok(result), _)) => result,
        Either::Left((Err(_), _)) => {
            Err(io::Error::new(io::ErrorKind::TimedOut, "FFprobe timed out"))
        }
        Either::Right(_) => Err(interrupted()),
    };
    if result.is_err() {
        let _ = child.kill().await;
    }
    let bytes = result?;
    let data: Value = serde_json::from_slice(&bytes)?;
    if !data.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FFprobe returned an invalid document",
        ));
    }
    Ok(data)
}
