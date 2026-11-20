//! Read the local League UI's connection arguments when a client omits its lockfile.

use std::{
    mem::size_of,
    os::windows::io::{AsRawHandle, OwnedHandle},
    ptr::null_mut,
    slice,
};
use windows_sys::{
    Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation},
    Win32::{
        Foundation::{
            LocalFree, STATUS_BUFFER_TOO_SMALL, STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING,
        },
        UI::Shell::CommandLineToArgvW,
    },
};

const MAX_COMMAND_LINE_BYTES: usize = 65_536 + size_of::<UNICODE_STRING>();

#[derive(PartialEq, Eq)]
pub(super) struct LaunchDetails {
    pub(super) owner_pid: u32,
    pub(super) port: u16,
    pub(super) token: String,
}

impl LaunchDetails {
    pub(super) fn read(process: &OwnedHandle) -> Option<Self> {
        let mut length = 0;
        // The retained handle was opened for the same-session LeagueClientUx.exe.
        let status = unsafe {
            NtQueryInformationProcess(
                process.as_raw_handle(),
                ProcessCommandLineInformation,
                null_mut(),
                0,
                &mut length,
            )
        };
        if !matches!(
            status,
            STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_TOO_SMALL
        ) || !(size_of::<UNICODE_STRING>()..=MAX_COMMAND_LINE_BYTES).contains(&(length as usize))
        {
            return None;
        }
        let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        let status = unsafe {
            NtQueryInformationProcess(
                process.as_raw_handle(),
                ProcessCommandLineInformation,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        };
        if status < 0 {
            return None;
        }
        let header = unsafe { &*buffer.as_ptr().cast::<UNICODE_STRING>() };
        // The native string must point into this result buffer and contain complete UTF-16 units.
        let offset = (header.Buffer as usize).checked_sub(buffer.as_ptr() as usize)?;
        let bytes = usize::from(header.Length);
        if offset < size_of::<UNICODE_STRING>()
            || offset % 2 != 0
            || bytes == 0
            || bytes % 2 != 0
            || offset.checked_add(bytes)? > buffer.len() * size_of::<usize>()
        {
            return None;
        }
        let mut command_line = unsafe { slice::from_raw_parts(header.Buffer, bytes / 2) }.to_vec();
        command_line.push(0);
        Self::from_command_line(&command_line)
    }

    fn from_command_line(command_line: &[u16]) -> Option<Self> {
        if command_line.last().copied() != Some(0) || command_line.len() < 2 {
            return None;
        }
        let mut count = 0;
        let argv = unsafe { CommandLineToArgvW(command_line.as_ptr(), &mut count) };
        if argv.is_null() {
            return None;
        }
        let mut owner_pid = None;
        let mut port = None;
        let mut token = None;
        let result = (|| {
            let args = unsafe { slice::from_raw_parts(argv, count as usize) };
            for &ptr in args.iter().skip(1) {
                let len = (0..command_line.len()).find(|&index| unsafe { *ptr.add(index) } == 0)?;
                let argument =
                    String::from_utf16(unsafe { slice::from_raw_parts(ptr, len) }).ok()?;
                if let Some(value) = argument.strip_prefix("--app-pid=") {
                    if owner_pid
                        .replace(value.parse::<u32>().ok().filter(|&pid| pid > 0)?)
                        .is_some()
                    {
                        return None;
                    }
                } else if let Some(value) = argument.strip_prefix("--app-port=") {
                    if port
                        .replace(value.parse::<u16>().ok().filter(|&port| port > 0)?)
                        .is_some()
                    {
                        return None;
                    }
                } else if let Some(value) = argument.strip_prefix("--remoting-auth-token=") {
                    if !(1..=128).contains(&value.len())
                        || !value
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                        || token.replace(value.to_owned()).is_some()
                    {
                        return None;
                    }
                }
            }
            Some(Self {
                owner_pid: owner_pid?,
                port: port?,
                token: token?,
            })
        })();
        unsafe { LocalFree(argv.cast()) };
        result
    }
}

#[cfg(test)]
mod tests {
    use super::LaunchDetails;

    fn parse(text: &str) -> Option<LaunchDetails> {
        LaunchDetails::from_command_line(&text.encode_utf16().chain([0]).collect::<Vec<_>>())
    }

    #[test]
    fn reads_only_the_three_fixed_ui_arguments_using_windows_quoting() {
        let parsed = parse(r#""D:\Synthetic Folder\LeagueClientUx.exe" "--app-pid=40" --app-port=32109 --remoting-auth-token=fixture_only-x --riotclient-app-port=32110"#).unwrap();
        assert_eq!(parsed.owner_pid, 40);
        assert_eq!(parsed.port, 32109);
        assert_eq!(parsed.token, "fixture_only-x");
        for input in [
            "LeagueClientUx.exe --app-pid=40 --app-port=0 --remoting-auth-token=fixture",
            "LeagueClientUx.exe --app-pid=40 --app-port=2 --app-port=1 --remoting-auth-token=fixture",
            "LeagueClientUx.exe --app-pid=0 --app-port=32109 --remoting-auth-token=fixture",
            "LeagueClientUx.exe --app-pid=40 --app-port=32109 --remoting-auth-token=fixture!",
            "LeagueClientUx.exe --app-pid=40 --riotclient-app-port=32109 --remoting-auth-token=fixture",
        ] {
            assert!(parse(input).is_none());
        }
    }
}
