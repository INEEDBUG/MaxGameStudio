//! Commands are bound in the session's private Demo cfg before it starts.
//! Only the window of the owned playback process receives these keys.

use super::{
    execute_single::{DemoControl, GameCommand, Segment, View},
    playback_gsi::GameUpdate,
    playback_process::PlaybackProcess,
    playback_spec::{seek_commands, PlaybackSpec},
};
use futures_util::future::{select, Either};
use std::{
    collections::{btree_map::Entry, BTreeMap},
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

pub(super) struct Bindings {
    seek: BTreeMap<i64, u16>,
    select: BTreeMap<i64, u16>,
}

impl Bindings {
    pub(super) fn configure(
        spec: &mut PlaybackSpec,
        segments: &[Segment<'_>],
        warmup: &[String],
        after_selection: &[&str],
    ) -> io::Result<Self> {
        let mut bindings = Self {
            seek: BTreeMap::new(),
            select: BTreeMap::new(),
        };
        // These per-process keys supplement the original four numpad bindings.
        // Each distinct operation has its own key, even within the same round.
        let mut additional = (b'A'..=b'Z').chain(b'0'..=b'9').map(|key| {
            (u16::from(key), char::from(key.to_ascii_lowercase()).to_string())
        });
        let mut next_key = || {
            additional.next().ok_or_else(|| {
                io::Error::other("本次录制的跳转和观战目标过多，请将片段分成两次录制")
            })
        };
        let mut commands = String::from(
            "bind \"KP_5\" \"demo_pause\"\r\nbind \"KP_6\" \"demo_resume\"\r\n",
        );
        for (index, segment) in segments.iter().enumerate() {
            if let Entry::Vacant(entry) = bindings.seek.entry(segment.seek_tick) {
                let (key, name) = if index == 0 {
                    (VK_NUMPAD7, "KP_7".to_owned())
                } else {
                    next_key()?
                };
                let seek = seek_commands(segment.seek_tick)?;
                let commands_to_send = warmup.iter().chain(&seek)
                    .map(String::as_str).collect::<Vec<_>>().join("; ");
                write!(commands, "bind \"{name}\" \"{commands_to_send}\"\r\n").expect("append to cfg");
                entry.insert(key);
            }
            let slot = segment.view.slot;
            if let Entry::Vacant(entry) = bindings.select.entry(slot) {
                if !(0..=i64::from(i32::MAX)).contains(&slot) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "无效的 Demo 观战槽位",
                    ));
                }
                let (key, name) = if index == 0 {
                    (VK_NUMPAD8, "KP_8".to_owned())
                } else {
                    next_key()?
                };
                let select = format!("spec_mode 5; spec_player {slot}");
                let commands_to_send = std::iter::once(select.as_str())
                    .chain(after_selection.iter().copied()).collect::<Vec<_>>().join("; ");
                write!(commands, "bind \"{name}\" \"{commands_to_send}\"\r\n").expect("append to cfg");
                entry.insert(key);
            }
        }
        spec.demo_cfg.push_str(&commands);
        Ok(bindings)
    }

    fn key(&self, command: GameCommand) -> io::Result<u16> {
        match command {
            GameCommand::SeekPaused(tick) => self.seek.get(&tick).copied(),
            GameCommand::SelectPlayer(slot) => self.select.get(&slot).copied(),
            GameCommand::PauseWithoutConsole => Some(VK_NUMPAD5),
            GameCommand::ResumeWithoutConsole => Some(VK_NUMPAD6),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "本次录制未绑定该 Demo 操作"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_http::recording_plan::playback_spec::{build, GsiTarget};
    use std::{num::NonZeroU16, path::Path};

    #[test]
    fn binds_distinct_seeks_and_povs_without_a_sequential_console_state() {
        let mut spec = build(
            Path::new(r"C:\Fixture Steam\game\bin\win64\cs2.exe"),
            Path::new(r"D:\Fixture Work\match.dem"),
            "0123456789abcdef0123456789abcdef", "F10", None,
            GsiTarget { port: NonZeroU16::new(19871).unwrap(), token: &"1".repeat(64) },
        ).unwrap();
        let segment = |seek_tick, slot, round| Segment {
            seek_tick, pre_roll: Duration::from_secs(2), duration: Duration::from_secs(1),
            view: View { map: "de_fixture", round, steam_id: 76561198000000000, slot },
        };
        let controls = Bindings::configure(&mut spec, &[
            segment(500, 3, 2), segment(900, 3, 2), segment(1200, 4, 3),
        ], &[], &["cl_demo_predict 0"]).unwrap();
        assert_eq!(controls.key(GameCommand::SeekPaused(500)).unwrap(), VK_NUMPAD7);
        assert_eq!(controls.key(GameCommand::SeekPaused(900)).unwrap(), u16::from(b'A'));
        assert_eq!(controls.key(GameCommand::SeekPaused(1200)).unwrap(), u16::from(b'B'));
        assert_eq!(controls.key(GameCommand::SelectPlayer(3)).unwrap(), VK_NUMPAD8);
        assert_eq!(controls.key(GameCommand::SelectPlayer(4)).unwrap(), u16::from(b'C'));
        assert!(controls.key(GameCommand::SeekPaused(999)).is_err());
        assert!(spec.demo_cfg.contains("bind \"a\" \"demo_pause; demo_gototick 900\""));
        assert!(spec.demo_cfg.contains("bind \"c\" \"spec_mode 5; spec_player 4; cl_demo_predict 0\""));
    }
}
