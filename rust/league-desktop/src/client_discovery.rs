//! Locate the temporary lockfile of a League client in the current Windows session.

use crate::lcu::{LcuError, LcuTarget};
use maxgamestudio_league_core::lcu_lockfile::LcuLockfile;
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Read},
    mem::{offset_of, size_of},
    os::windows::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_FILES, ERROR_SUCCESS, FILETIME,
        INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    },
    NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    },
    Networking::WinSock::AF_INET,
    Storage::FileSystem::{
        GetDriveTypeW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
        WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE},
    },
};

const CLIENT_PROCESS: &str = "LeagueClient.exe";
const LOCKFILE_LIMIT: u64 = 4096;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ClientInstance {
    pub(crate) pid: u32,
    pub(crate) created_at: u64,
}

pub(crate) struct LocalClient {
    process: OwnedHandle,
    pub(crate) instance: ClientInstance,
    port: u16,
    token: String,
}

impl LocalClient {
    pub(crate) fn alive(&self) -> bool {
        // The retained handle refers to the selected process even if Windows recycles its PID.
        unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) == WAIT_TIMEOUT }
    }

    pub(crate) fn target(&self) -> Result<LcuTarget<'_>, LcuError> {
        let listeners = ipv4_listeners().map_err(|_| LcuError::InvalidTarget)?;
        if !self.alive() || !owns_loopback_listener(&listeners, self.instance.pid, self.port) {
            return Err(LcuError::InvalidTarget);
        }
        LcuTarget::new(self.port, &self.token)
    }
}

pub(crate) fn discover_clients() -> io::Result<Vec<LocalClient>> {
    let session = session_id(std::process::id()).ok_or_else(io::Error::last_os_error)?;
    let candidates = client_process_ids()?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let listeners = ipv4_listeners()?;
    let mut clients = Vec::new();
    for pid in candidates
        .into_iter()
        .filter(|&pid| session_id(pid) == Some(session))
    {
        if let Some(client) = read_client(pid, &listeners) {
            clients.push(client);
        }
    }
    Ok(clients)
}

fn session_id(pid: u32) -> Option<u32> {
    let mut session = 0;
    (unsafe { ProcessIdToSessionId(pid, &mut session) } != 0).then_some(session)
}

fn client_process_ids() -> io::Result<Vec<u32>> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut process = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut available = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut process) } != 0;
    let mut pids = Vec::new();
    while available {
        let end = process
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(process.szExeFile.len());
        let executable = &process.szExeFile[..end];
        let name_matches = executable.len() == CLIENT_PROCESS.len()
            && executable
                .iter()
                .zip(CLIENT_PROCESS.bytes())
                .all(|(&unit, expected)| {
                    u8::try_from(unit).is_ok_and(|byte| byte.eq_ignore_ascii_case(&expected))
                });
        if name_matches {
            pids.push(process.th32ProcessID);
        }
        available = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut process) } != 0;
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
        return Err(error);
    }
    Ok(pids)
}

fn process_image(process: &OwnedHandle) -> Option<PathBuf> {
    let mut name = vec![0u16; 32_768];
    let mut units = name.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(process.as_raw_handle(), 0, name.as_mut_ptr(), &mut units)
    } == 0
    {
        return None;
    }
    Some(OsString::from_wide(&name[..units as usize]).into())
}

fn creation_time(process: &OwnedHandle) -> Option<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    let ok = unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    (ok != 0)
        .then_some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn is_local_install_directory(directory: &Path) -> bool {
    let mut components = directory.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return false;
    };
    let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
        return false;
    };
    if components.next() != Some(Component::RootDir) {
        return false;
    }
    let volume = [u16::from(letter), b':' as u16, b'\\' as u16, 0];
    if !matches!(
        unsafe { GetDriveTypeW(volume.as_ptr()) },
        DRIVE_FIXED | DRIVE_REMOVABLE
    ) {
        return false;
    }
    let mut checked = PathBuf::from(prefix.as_os_str());
    checked.push(r"\");
    for part in components {
        let Component::Normal(name) = part else {
            return false;
        };
        checked.push(name);
        if !fs::symlink_metadata(&checked).is_ok_and(|metadata| {
            metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
        }) {
            return false;
        }
    }
    true
}

fn read_client(pid: u32, listeners: &[MIB_TCPROW_OWNER_PID]) -> Option<LocalClient> {
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return None;
    }
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    let image = process_image(&process)?;
    if !image
        .file_name()?
        .to_str()?
        .eq_ignore_ascii_case(CLIENT_PROCESS)
    {
        return None;
    }

    let install_directory = image.parent()?;
    if !is_local_install_directory(install_directory) {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(install_directory.join("lockfile"))
        .ok()?;
    let metadata = file.metadata().ok()?;
    let created_at = creation_time(&process)?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || metadata.last_write_time() < created_at
    {
        return None;
    }
    let mut contents = String::new();
    if file
        .take(LOCKFILE_LIMIT + 1)
        .read_to_string(&mut contents)
        .ok()? as u64
        > LOCKFILE_LIMIT
    {
        return None;
    }
    let lockfile = LcuLockfile::parse(&contents)?;
    if lockfile.pid != pid || !owns_loopback_listener(listeners, pid, lockfile.port) {
        return None;
    }

    let result = LocalClient {
        process,
        instance: ClientInstance { pid, created_at },
        port: lockfile.port,
        token: lockfile.token.to_owned(),
    };
    result.alive().then_some(result)
}

fn owns_loopback_listener(listeners: &[MIB_TCPROW_OWNER_PID], pid: u32, port: u16) -> bool {
    let localhost = u32::from_ne_bytes([127, 0, 0, 1]);
    let mut matching = listeners.iter().filter(|entry| {
        u16::from_be(entry.dwLocalPort as u16) == port
            && (entry.dwLocalAddr == 0 || entry.dwLocalAddr == localhost)
    });
    matching
        .next()
        .is_some_and(|entry| entry.dwOwningPid == pid)
        && matching.all(|entry| entry.dwOwningPid == pid)
}

fn ipv4_listeners() -> io::Result<Vec<MIB_TCPROW_OWNER_PID>> {
    let mut length = 0;
    let status = unsafe {
        GetExtendedTcpTable(
            null_mut(),
            &mut length,
            0,
            AF_INET.into(),
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if status != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    for _ in 0..3 {
        // Use a DWORD-aligned buffer for the header and variable-length rows.
        let mut data = vec![0u32; (length as usize).div_ceil(size_of::<u32>())];
        let status = unsafe {
            GetExtendedTcpTable(
                data.as_mut_ptr().cast(),
                &mut length,
                0,
                AF_INET.into(),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }

        let offset = offset_of!(MIB_TCPTABLE_OWNER_PID, table);
        let count = data.first().copied().unwrap_or_default() as usize;
        let capacity = (data.len() * size_of::<u32>()).saturating_sub(offset)
            / size_of::<MIB_TCPROW_OWNER_PID>();
        if count > capacity {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let rows = unsafe {
            std::slice::from_raw_parts(data.as_ptr().cast::<u8>().add(offset).cast(), count)
        };
        return Ok(rows.to_vec());
    }
    Err(io::Error::from_raw_os_error(
        ERROR_INSUFFICIENT_BUFFER as i32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};

    #[test]
    fn rejects_a_second_owner_on_the_same_loopback_port() {
        let wildcard = MIB_TCPROW_OWNER_PID {
            dwLocalPort: u32::from(58031u16.to_be()),
            dwOwningPid: 5432,
            ..Default::default()
        };
        let other = MIB_TCPROW_OWNER_PID {
            dwLocalAddr: u32::from_ne_bytes([127, 0, 0, 1]),
            dwOwningPid: 5433,
            ..wildcard
        };
        assert!(!owns_loopback_listener(&[wildcard, other], 5432, 58031));
    }

    #[test]
    fn binds_the_actual_ipv4_loopback_listener_to_its_owner() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let listeners = ipv4_listeners().unwrap();
        assert!(owns_loopback_listener(&listeners, std::process::id(), port));
        assert!(!owns_loopback_listener(&listeners, 0, port));
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, std::process::id()) };
        assert!(!process.is_null());
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        assert!(is_local_install_directory(
            process_image(&process).unwrap().parent().unwrap()
        ));
        assert!(creation_time(&process).unwrap() > 0);
    }
}
