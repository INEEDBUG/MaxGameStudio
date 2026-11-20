#![cfg(windows)]

use maxgamestudio_league_core::profile_path::{league_profile_path, ProfilePathError};
use std::{fs, os::windows::process::CommandExt, path::Path, process::Command};

#[test]
fn recorded_windows_profile_paths() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("CAFÉ 玩家");
    fs::create_dir(&directory).unwrap();
    let file = root.path().join("ordinary.txt");
    fs::write(&file, "fixture").unwrap();
    let nested = directory.join("新目录").join("Profile");
    let alternate_case = root.path().join("café 玩家");

    // Captured from leagueProfilePath with Node 24 on Windows in an equivalent temporary tree.
    assert_eq!(league_profile_path(&directory).unwrap(), directory);
    assert_eq!(
        league_profile_path(&alternate_case).unwrap(),
        alternate_case
    );
    let mut drive_case = directory.to_str().unwrap().to_owned();
    drive_case[..1].make_ascii_lowercase();
    assert_eq!(
        league_profile_path(Path::new(&drive_case))
            .unwrap()
            .as_os_str(),
        Path::new(&drive_case).as_os_str()
    );
    let unix_separators = directory.to_str().unwrap().replace('\\', "/");
    assert_eq!(
        league_profile_path(Path::new(&unix_separators)).unwrap(),
        directory
    );
    let normalized = root.path().join("absent").join("..").join("CAFÉ 玩家");
    assert_eq!(league_profile_path(&normalized).unwrap(), directory);
    assert_eq!(league_profile_path(&nested).unwrap(), nested);
    assert!(!nested.parent().unwrap().exists());
    assert_eq!(
        league_profile_path(&file).unwrap_err().to_string(),
        "数据路径必须是普通文件夹。"
    );

    for input in [
        "relative",
        "C:relative",
        r"\\fixture.invalid\share\Profile",
        r"\\?\C:\Profile",
    ] {
        assert_eq!(
            league_profile_path(Path::new(input))
                .unwrap_err()
                .to_string(),
            "请选择本机磁盘上的数据文件夹。"
        );
    }
    let drive = root.path().ancestors().last().unwrap();
    for input in [drive.to_owned(), drive.join("absent").join("..")] {
        assert_eq!(
            league_profile_path(&input).unwrap_err().to_string(),
            "不能将磁盘根目录用作数据文件夹。"
        );
    }
}

#[test]
fn linked_paths_and_non_directory_ancestors_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("target");
    let junction = root.path().join("junction");
    let file = root.path().join("ordinary.txt");
    fs::create_dir(&directory).unwrap();
    fs::write(&file, "fixture").unwrap();

    // An NTFS junction is testable without Developer Mode or privileges for symbolic links.
    let command = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:MGS_FIXTURE_JUNCTION -Target $env:MGS_FIXTURE_JUNCTION_TARGET | Out-Null"])
        .env("MGS_FIXTURE_JUNCTION", &junction)
        .env("MGS_FIXTURE_JUNCTION_TARGET", &directory)
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert!(
        command.status.success(),
        "junction creation failed on the Windows fixture: {:?}",
        command.status
    );

    assert_eq!(
        league_profile_path(&junction).unwrap_err().to_string(),
        "数据路径必须是普通文件夹。"
    );
    assert_eq!(
        league_profile_path(&junction.join("absent"))
            .unwrap_err()
            .to_string(),
        "数据路径不能经过快捷方式、联接或符号链接。"
    );

    // The original accepts a file ancestor and an unresolved junction. Both are unsafe parents.
    assert!(matches!(
        league_profile_path(&file.join("absent")),
        Err(ProfilePathError::NotDirectory)
    ));
    fs::remove_dir(&directory).unwrap();
    assert!(matches!(
        league_profile_path(&junction),
        Err(ProfilePathError::NotDirectory)
    ));
    assert!(matches!(
        league_profile_path(&junction.join("absent")),
        Err(ProfilePathError::Link)
    ));
    fs::remove_dir(&junction).unwrap();
}

#[test]
fn future_directories_cannot_use_windows_aliases_or_devices() {
    let root = tempfile::tempdir().unwrap();
    for segment in [
        "absent.",
        "absent ",
        "absent:stream",
        "NUL",
        "con.txt",
        "COM¹",
        "LPT9",
        "bad?name",
    ] {
        assert!(
            matches!(
                league_profile_path(&root.path().join(segment)),
                Err(ProfilePathError::InvalidName)
            ),
            "{segment:?}"
        );
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
