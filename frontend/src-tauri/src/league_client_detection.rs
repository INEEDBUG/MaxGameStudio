//! Lightweight local League client process detection.
//!
//! This intentionally reports process presence only.  It does not inspect
//! process memory, read credentials, launch a shell, or modify the game.

/// Return the small status object consumed by the League runtime auto manager.
///
/// On Windows, only the exact `LeagueClientUx.exe` process name is accepted.
/// Other platforms safely report that the client is unavailable.
#[tauri::command]
pub(crate) fn detect_league_client() -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        let pid = find_league_client_pid()?;
        Ok(serde_json::json!({
            "connected": false,
            "client_process_detected": pid.is_some(),
            "client_window_detected": false,
            "client_pid": pid,
        }))
    }

    #[cfg(not(windows))]
    Ok(serde_json::json!({
        "connected": false,
        "client_process_detected": false,
        "client_window_detected": false,
        "client_pid": serde_json::Value::Null,
    }))
}

#[cfg(windows)]
fn find_league_client_pid() -> Result<Option<u32>, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };

    // ToolHelp is read-only and does not require elevation for this process
    // name/ID enumeration.  Do not retain the snapshot beyond this call.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "Unable to enumerate processes (Win32 error {})",
                GetLastError()
            ));
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut has_entry = Process32FirstW(snapshot, &mut entry);
        if has_entry == 0 {
            let error = GetLastError();
            CloseHandle(snapshot);
            return if error == ERROR_NO_MORE_FILES {
                Ok(None)
            } else {
                Err(format!(
                    "Unable to enumerate processes (Win32 error {error})"
                ))
            };
        }
        while has_entry != 0 {
            let length = entry
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(entry.szExeFile.len());
            let executable_name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if executable_name.eq_ignore_ascii_case("LeagueClientUx.exe") {
                found = Some(entry.th32ProcessID);
                break;
            }
            has_entry = Process32NextW(snapshot, &mut entry);
        }
        let error = GetLastError();
        CloseHandle(snapshot);
        if found.is_some() {
            return Ok(found);
        }
        if error != ERROR_NO_MORE_FILES {
            return Err(format!(
                "Unable to enumerate processes (Win32 error {error})"
            ));
        }
        Ok(found)
    }
}
