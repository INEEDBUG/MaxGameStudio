use serde_json::{json, Value};
use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const EXECUTABLE: &str = "maxgamestudio-parser-worker.exe";
const PARSER_COMMIT: &str = "db3ba5f7a1b02b8cb27d0063d52649883a98469c";
const INPUT_LIMIT: usize = 64 * 1024;
const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
const REPLAY_TIMEOUT: Duration = Duration::from_secs(240);

#[derive(Clone)]
pub(crate) struct ParserWorker {
    executable: PathBuf,
}

impl ParserWorker {
    pub(crate) fn installed_with(bundle_data: Option<&Path>) -> Option<Self> {
        let root = bundle_data?.parent()?;
        let executable = root.join(EXECUTABLE);
        if !executable.is_absolute()
            || !executable
                .symlink_metadata()
                .is_ok_and(|info| info.is_file())
        {
            return None;
        }
        Some(Self { executable })
    }

    pub(crate) async fn write_replay(
        &self,
        demo: &Path,
        staged_parquet: &Path,
        tick_rate: f64,
        fps: f64,
        rounds: &Value,
    ) -> io::Result<Value> {
        let request = serde_json::to_vec(&json!({
            "protocol": 1, "action": "write_replay_parquet", "path": demo, "output_path": staged_parquet,
            "tick_rate": tick_rate, "fps": fps, "rounds": rounds,
        }))?;
        if request.len() > INPUT_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Too many replay rounds for the parser worker",
            ));
        }

        let mut command = Command::new(&self.executable);
        command
            .current_dir(self.executable.parent().expect("installed parser path"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        let mut child = command.spawn()?;
        tokio::time::timeout(REPLAY_TIMEOUT, async {
            let mut stdin = child.stdin.take().expect("parser stdin was piped");
            stdin.write_all(&request).await?;
            drop(stdin);
            let mut output = Vec::new();
            child
                .stdout
                .take()
                .expect("parser stdout was piped")
                .take(OUTPUT_LIMIT + 1)
                .read_to_end(&mut output)
                .await?;
            if output.len() as u64 > OUTPUT_LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Parser worker output exceeded its size limit",
                ));
            }
            let status = child.wait().await?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "Parser worker failed with exit status {status}"
                )));
            }
            let response: Value = serde_json::from_slice(&output)?;
            if response["protocol"] != 1
                || response["parser_commit"] != PARSER_COMMIT
                || response["ok"] != true
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unexpected parser worker protocol or source identity",
                ));
            }
            response.get("result").cloned().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Parser worker reply is incomplete",
                )
            })
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Replay parser worker timed out"))?
    }
}
