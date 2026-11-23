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
const EFFECT_OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;

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
        self.run(
            json!({
                "protocol": 1, "action": "write_replay_parquet", "path": demo,
                "output_path": staged_parquet, "tick_rate": tick_rate, "fps": fps, "rounds": rounds,
            }),
            Duration::from_secs(240),
            OUTPUT_LIMIT,
        )
        .await
    }

    pub(crate) async fn inspect_demo(&self, path: &Path) -> io::Result<Value> {
        self.run(
            json!({"protocol": 1, "action": "inspect_demo", "path": path}),
            Duration::from_secs(30),
            OUTPUT_LIMIT,
        )
        .await
    }

    pub(crate) async fn inspect_roster(&self, path: &Path) -> io::Result<Value> {
        self.run(
            json!({"protocol": 1, "action": "inspect_roster", "path": path}),
            Duration::from_secs(90),
            OUTPUT_LIMIT,
        )
        .await
    }

    pub(crate) async fn analyze(
        &self,
        path: &Path,
        players: &[String],
        rounds: Option<&[i64]>,
    ) -> io::Result<Value> {
        self.run(
            json!({
                "protocol":1, "action":"inspect_match_timeline", "path":path, "tick_rate":64.0,
                "include_match_facts":true,
                "round_compilations":{"players":players,"rounds":rounds},
            }),
            Duration::from_secs(240),
            16 * 1024 * 1024,
        )
        .await
    }

    pub(crate) async fn inspect_match(&self, path: &Path, tick_rate: f64) -> io::Result<Value> {
        self.run(
            json!({
                "protocol":1,"action":"inspect_match_timeline","path":path,
                "tick_rate":tick_rate,"include_match_facts":true,
            }),
            Duration::from_secs(180),
            16 * 1024 * 1024,
        )
        .await
    }

    pub(crate) async fn replay_effects(
        &self,
        path: &Path,
        tick_rate: f64,
        end: i64,
    ) -> io::Result<Value> {
        self.run(
            json!({
                "protocol":1, "action":"inspect_effect_tracks", "path":path,
                "tick_rate":tick_rate, "start_tick":0, "end_tick":end,
            }),
            Duration::from_secs(180),
            EFFECT_OUTPUT_LIMIT,
        )
        .await
    }

    pub(crate) async fn replay_trajectories(
        &self,
        path: &Path,
        tick_rate: f64,
        end: i64,
    ) -> io::Result<Value> {
        self.run(
            json!({
                "protocol":1, "action":"inspect_projectile_trajectories", "path":path,
                "tick_rate":tick_rate, "start_tick":0, "end_tick":end,
            }),
            Duration::from_secs(120),
            OUTPUT_LIMIT,
        )
        .await
    }

    async fn run(&self, input: Value, limit: Duration, max_output: u64) -> io::Result<Value> {
        let request = serde_json::to_vec(&input)?;
        if request.len() > INPUT_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Parser worker request exceeded its size limit",
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
        tokio::time::timeout(limit, async {
            let mut stdin = child.stdin.take().expect("parser stdin was piped");
            stdin.write_all(&request).await?;
            drop(stdin);
            let mut output = Vec::new();
            child
                .stdout
                .take()
                .expect("parser stdout was piped")
                .take(max_output + 1)
                .read_to_end(&mut output)
                .await?;
            if output.len() as u64 > max_output {
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
            let mut response: Value = serde_json::from_slice(&output)?;
            if response["protocol"] != 1
                || response["parser_commit"] != PARSER_COMMIT
                || response["ok"] != true
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unexpected parser worker protocol or source identity",
                ));
            }
            response.get_mut("result").map(Value::take).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Parser worker reply is incomplete",
                )
            })
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Parser worker timed out"))?
    }
}
