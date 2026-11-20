#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod profile;

#[cfg(windows)]
mod desktop;

#[cfg(windows)]
mod client_discovery;

#[cfg(windows)]
mod client_runtime;

#[cfg(windows)]
mod lcu;

#[cfg(windows)]
mod resg_window;

#[cfg(windows)]
mod opgg_window;

#[cfg(windows)]
mod aram_automation;

#[cfg(windows)]
fn main() {
    if let Err(error) = desktop::run() {
        rfd::MessageDialog::new()
            .set_title("英雄联盟工具箱启动失败")
            .set_description(error.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("英雄联盟工具箱桌面版需要 Windows。");
}
