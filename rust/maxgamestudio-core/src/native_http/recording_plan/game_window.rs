//! Commands are bound in the session's private Demo cfg. Only the window of
//! the PlayDemo subprocess receives the four keys, including during capture.

use super::{
    execute_single::{DemoControl, GameCommand, View},
    playback_gsi::GameUpdate,
    playback_process::PlaybackProcess,
    playback_spec::{seek_commands, PlaybackSpec},
};
use futures_util::future::{select, Either};
use std::{
    fmt::Write,
    io,
    time::{Duration, Instant},
};
use tokio::{
    sync::watch,
    time::{sleep, timeout},
};
use windows_sys::core::BOOL;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            MapVirtualKeyW, MAPVK_VK_TO_VSC, VK_NUMPAD5, VK_NUMPAD6, VK_NUMPAD7, VK_NUMPAD8,
        },
        WindowsAndMessaging::{
            EnumWindows, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
            PostMessageW, GW_OWNER, WM_KEYDOWN, WM_KEYUP,
        },
    },
};

#[derive(Clone, Copy)]
pub(super) struct Bindings {
    seek_tick: i64,
    slot: i64,
}

impl Bindings {
    pub(super) fn configure(
        spec: &mut PlaybackSpec,
        seek_tick: i64,
        slot: i64,
        warmup: &[String],
        after_selection: &[&str],
    ) -> io::Result<Self> {
        let seek = seek_commands(seek_tick)?;
        if !(0..=i64::from(i32::MAX)).contains(&slot) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "无效的 Demo 观战槽位",
            ));
        }
        let select = format!("spec_mode 5; spec_player {slot}");
        let bindings = [
            ("KP_5", "demo_pause".to_owned()),
            ("KP_6", "demo_resume".to_owned()),
            (
                "KP_7",
                warmup
                    .iter()
                    .chain(&seek)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            (
                "KP_8",
                std::iter::once(select.as_str())
                    .chain(after_selection.iter().copied())
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        ];
        for (key, command) in bindings {
            write!(spec.demo_cfg, "bind \"{key}\" \"{command}\"\r\n").expect("append to cfg");
        }
        Ok(Self { seek_tick, slot })
    }

    fn key(self, command: GameCommand) -> io::Result<u16> {
        match command {
            GameCommand::SeekPaused(tick) if tick == self.seek_tick => Ok(VK_NUMPAD7),
            GameCommand::SelectPlayer(slot) if slot == self.slot => Ok(VK_NUMPAD8),
            GameCommand::PauseWithoutConsole => Ok(VK_NUMPAD5),
            GameCommand::ResumeWithoutConsole => Ok(VK_NUMPAD6),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "本次录制未绑定该 Demo 操作",
            )),
        }
    }
}

pub(super) struct GameWindow<'game, 'lease> {
    process: &'game mut PlaybackProcess<'lease>,
    // HWND is a Win32 handle and may be used by a different Tokio thread.
    // Resolve and recheck its owner before posting a message.
    window: isize,
    bindings: Bindings,
    updates: watch::Receiver<Option<GameUpdate>>,
}

impl<'game, 'lease> GameWindow<'game, 'lease> {
    pub(super) async fn find(
        process: &'game mut PlaybackProcess<'lease>,
        bindings: Bindings,
        updates: watch::Receiver<Option<GameUpdate>>,
        cancel: &mut watch::Receiver<bool>,
    ) -> io::Result<Self> {
        let discover = async {
            loop {
                let pid = process.running_pid()?;
                if let Some(window) = owned_window(pid)? {
                    return Ok::<_, io::Error>(window);
                }
                sleep(Duration::from_millis(80)).await;
            }
        };
        let window = match select(
            Box::pin(timeout(Duration::from_secs(5), discover)),
            Box::pin(cancel.wait_for(|cancelled| *cancelled)),
        )
        .await
        {
            Either::Left((Ok(result), _)) => result?,
            Either::Left(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "未找到本次录制的 CS2 窗口",
                ))
            }
            Either::Right(_) => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "录制已取消"))
            }
        };
        Ok(Self {
            process,
            window,
            bindings,
            updates,
        })
    }

    fn post(&mut self, message: u32, key: u16, flags: isize) -> io::Result<()> {
        let pid = self.process.running_pid()?;
        let window = self.window as HWND;
        if !belongs_to(window, pid) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "本次录制的游戏窗口已关闭或不可见",
            ));
        }
        // PostMessageW follows normal UIPI permissions. No global input or focus changes.
        if unsafe { PostMessageW(window, message, usize::from(key), flags) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl DemoControl for GameWindow<'_, '_> {
    async fn command(&mut self, command: GameCommand) -> io::Result<()> {
        let key = self.bindings.key(command)?;
        let scan = unsafe { MapVirtualKeyW(u32::from(key), MAPVK_VK_TO_VSC) } & 0xff;
        if scan == 0 {
            return Err(io::Error::other("无法定位 Demo 控制键"));
        }
        let flags = (1 | scan << 16) as isize;
        self.post(WM_KEYDOWN, key, flags)?;
        sleep(Duration::from_millis(25)).await;
        self.post(WM_KEYUP, key, flags | (0xc000_0000_u32 as i32 as isize))
    }

    async fn wait_view(&mut self, requested: &View<'_>, after: Instant) -> io::Result<()> {
        let update = self.updates.wait_for(|message| {
            message.as_ref().is_some_and(|message| {
                message.received >= after
                    && message.map.eq_ignore_ascii_case(requested.map)
                    && message.player == Some(requested.steam_id)
                    && message.round == Some(requested.round)
            })
        });
        match timeout(
            Duration::from_secs(4),
            select(Box::pin(update), Box::pin(self.process.wait_for_exit())),
        )
        .await
        {
            Ok(Either::Left((result, _))) => result.map(|_| ()).map_err(io::Error::other),
            Ok(Either::Right((exit, _))) => exit,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "未确认 Demo 已切到选定玩家",
            )),
        }
    }

    async fn wait_exit(&mut self) -> io::Result<()> {
        self.process.wait_for_exit().await
    }
}

struct Search {
    pid: u32,
    first: isize,
    ambiguous: bool,
}

fn owned_window(pid: u32) -> io::Result<Option<isize>> {
    let mut search = Search {
        pid,
        first: 0,
        ambiguous: false,
    };
    // EnumWindows calls the callback synchronously while the stack value is valid.
    if unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((search.first != 0 && !search.ambiguous).then_some(search.first))
}

unsafe extern "system" fn visit(window: HWND, context: LPARAM) -> BOOL {
    let search = unsafe { &mut *(context as *mut Search) };
    if belongs_to(window, search.pid) && unsafe { GetWindow(window, GW_OWNER) }.is_null() {
        search.ambiguous |= search.first != 0;
        search.first = window as isize;
    }
    1
}

fn belongs_to(window: HWND, pid: u32) -> bool {
    let mut actual = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut actual) != 0
            && actual == pid
            && IsWindowVisible(window) != 0
            && IsIconic(window) == 0
    }
}
