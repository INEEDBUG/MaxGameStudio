#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn install_panic_log() {
    use std::{
        backtrace::Backtrace,
        fs::{self, OpenOptions},
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    std::panic::set_hook(Box::new(|info| {
        let Some(logs) = cs2_insight_agent_desktop_lib::desktop_logs_dir() else {
            return;
        };
        if fs::create_dir_all(&logs).is_err() {
            return;
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs.join("desktop-panic.log"))
        {
            let _ = writeln!(
                file,
                "[{timestamp}] {info}\n{}\n",
                Backtrace::force_capture()
            );
        }
    }));
}

fn main() {
    #[cfg(windows)]
    install_panic_log();
    cs2_insight_agent_desktop_lib::run();
}
