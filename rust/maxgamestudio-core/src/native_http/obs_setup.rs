use super::{
    detail, json_reply, optional_model_body, profile_io_error, profile_work, HttpResponse,
    ProfileState,
};
use crate::{
    obs_wire::{ObsRpc, ObsSettings},
    profile_platform::process_is_running,
    wire_json::integer_from_text,
};
use hyper::{body::Incoming, Request, StatusCode};
use serde_json::{json, Value};
use std::{
    io,
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::time::{sleep, timeout};

pub(super) enum Route {
    Check,
    Running,
}

impl Route {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        match path {
            "/api/obs/config-check" => Some(Self::Check),
            "/api/obs/is-running" => Some(Self::Running),
            _ => None,
        }
    }
}

pub(super) async fn respond(
    profile: &Arc<ProfileState>,
    route: Route,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> HttpResponse {
    match route {
        Route::Check => check(profile, request, extended_request).await,
        Route::Running => {
            match profile_work(profile, |profile| {
                let (_, program) = profile.config.obs_profile()?;
                Ok::<_, io::Error>(
                    json!({"running":running(Path::new(&program)), "obs_path":program}),
                )
            })
            .await
            {
                Ok(Ok(result)) => json_reply(StatusCode::OK, result),
                Ok(Err(error)) => profile_io_error(&error),
                Err(error) => profile_io_error(&error),
            }
        }
    }
}

async fn check(
    profile: &Arc<ProfileState>,
    request: Request<Incoming>,
    extended_request: &AtomicBool,
) -> HttpResponse {
    let input = match optional_model_body(request, "OBS configuration is too large").await {
        Ok(Some(input)) => match CheckSettings::parse(input) {
            Some(input) => Some(input),
            None => return detail(StatusCode::UNPROCESSABLE_ENTITY, "OBS 连接配置无效"),
        },
        Ok(None) => None,
        Err(error) => return *error,
    };
    let Ok(_permit) = profile.obs_writes.try_acquire() else {
        return detail(StatusCode::CONFLICT, "正在检查或修改 OBS 配置，请稍后重试");
    };
    extended_request.store(true, Ordering::Relaxed);
    let (saved, program, launched) = match profile_work(profile, |profile| {
        let (saved, program) = profile.config.obs_profile()?;
        let launched = start_if_needed(&program);
        Ok::<_, io::Error>((saved, program, launched))
    })
    .await
    {
        Ok(Ok((saved, program, Ok(launched)))) => (saved, program, launched),
        Ok(Ok((_, _, Err(problem)))) => return json_reply(StatusCode::OK, problem.response()),
        Ok(Err(error)) => return profile_io_error(&error),
        Err(error) => return profile_io_error(&error),
    };
    let settings = input.map_or_else(
        || saved.clone(),
        |input| saved.with_check_input(&input.host, input.port, &input.password),
    );
    let connected = timeout(Duration::from_secs(15), wait_for_obs(&settings))
        .await
        .is_ok();
    if connected && settings == saved {
        let persisted = profile_work(profile, move |profile| {
            profile.config.mark_obs_verified(&saved, &program)
        })
        .await;
        if !matches!(persisted, Ok(Ok(()))) {
            return detail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "无法保存 OBS 配置检查结果",
            );
        }
    }
    json_reply(
        StatusCode::OK,
        json!({"path_ok":true,"connected":connected,"launched_obs":launched}),
    )
}

struct CheckSettings {
    host: String,
    port: u16,
    password: String,
}

impl CheckSettings {
    fn parse(input: Value) -> Option<Self> {
        let fields = input.as_object()?;
        let host = fields
            .get("host")
            .map_or(Some("localhost"), Value::as_str)?;
        let password = fields.get("password").map_or(Some(""), Value::as_str)?;
        let port = match fields.get("port") {
            None => 4455,
            Some(Value::String(raw)) => integer_from_text(raw)?.as_u64()?,
            Some(raw) => {
                let number = raw.as_f64()?;
                if !(1.0..=65535.0).contains(&number) || number.fract() != 0.0 {
                    return None;
                }
                number as u64
            }
        };
        let port = u16::try_from(port).ok().filter(|&port| port > 0)?;
        Some(Self {
            host: host.to_owned(),
            port,
            password: password.to_owned(),
        })
    }
}

async fn wait_for_obs(settings: &ObsSettings) {
    loop {
        let result = timeout(Duration::from_secs(2), async {
            let mut obs = ObsRpc::connect(settings.clone()).await?;
            obs.request("GetVersion", Value::Null).await
        })
        .await;
        if matches!(result, Ok(Ok(_))) {
            return;
        }
        sleep(Duration::from_secs(1)).await;
    }
}

enum LaunchProblem {
    Unconfigured,
    Missing,
    Spawn,
}

impl LaunchProblem {
    fn response(self) -> Value {
        match self {
            Self::Unconfigured => json!({
                "path_ok":false, "connected":false, "error":"请先配置 OBS 路径，再点击配置检查"
            }),
            Self::Missing => json!({"path_ok":false,"error":"OBS 路径不存在"}),
            Self::Spawn => json!({"path_ok":false,"error":"无法启动 OBS，请检查程序路径"}),
        }
    }
}

fn start_if_needed(program: &str) -> Result<bool, LaunchProblem> {
    if program.is_empty() {
        return Err(LaunchProblem::Unconfigured);
    }
    let executable = Path::new(program)
        .canonicalize()
        .ok()
        .filter(|path| path.is_file())
        .ok_or(LaunchProblem::Missing)?;
    #[cfg(windows)]
    if !executable
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("exe"))
    {
        return Err(LaunchProblem::Spawn);
    }
    if running(&executable) {
        return Ok(false);
    }
    Command::new(&executable)
        .current_dir(
            executable
                .parent()
                .expect("canonical OBS executable has a parent"),
        )
        .spawn()
        .map_err(|_| LaunchProblem::Spawn)?;
    Ok(true)
}

fn running(program: &Path) -> bool {
    program
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(process_is_running)
}
