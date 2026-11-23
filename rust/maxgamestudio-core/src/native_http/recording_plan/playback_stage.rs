#![cfg(windows)]

//! Stages the Demo and the two uniquely named recording configurations.

use super::playback_spec::{gsi_config_name, hexadecimal_id, PlaybackSpec};
use sha2::{Digest, Sha256};
use std::{
    ffi::c_void,
    fs::{File, Metadata, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, RawHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    sync::atomic::{AtomicBool, Ordering},
};

const SHARE_READ: u32 = 0x0001;
const READ_ATTRIBUTES: u32 = 0x0080;
const DELETE: u32 = 0x0001_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const REPARSE_POINT: u32 = 0x0400;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_DISPOSITION_INFO: i32 = 4;
const FILE_ID_INFO: i32 = 18;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandleEx(
        file: RawHandle,
        class: i32,
        buffer: *mut c_void,
        size: u32,
    ) -> i32;
    fn SetFileInformationByHandle(
        file: RawHandle,
        class: i32,
        buffer: *const c_void,
        size: u32,
    ) -> i32;
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
struct FileId {
    volume: u64,
    id: [u8; 16],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct SourceStamp {
    id: FileId,
    length: u64,
    written: u64,
}

impl SourceStamp {
    pub(super) fn open(path: &Path) -> io::Result<(File, Self)> {
        let file = open_source(path)?;
        let stamp = source_stamp(&file)?;
        Ok((file, stamp))
    }
}

#[derive(PartialEq, Eq)]
struct ContentStamp {
    bytes: u64,
    digest: [u8; 32],
}

struct OwnedFile {
    path: PathBuf,
    id: FileId,
    content: ContentStamp,
}

pub(super) struct PlaybackStage {
    demo: Option<OwnedFile>,
    cfg: Option<OwnedFile>,
    gsi: Option<OwnedFile>,
}

impl PlaybackStage {
    pub(super) fn stage(spec: &PlaybackSpec, cancelled: &AtomicBool) -> io::Result<Self> {
        check_cancelled(Some(cancelled))?;
        let cfg_dir = validate_destinations(spec)?;
        let _directories = lock_directories(&cfg_dir)?;
        let mut source = open_source(&spec.source_demo)?;
        let before = source_stamp(&source)?;
        if spec.source_stamp.is_some_and(|expected| expected != before) {
            return Err(io::Error::other(
                "排队期间 Demo 文件已变化，请重新选择录制片段",
            ));
        }
        verify_space(
            &cfg_dir,
            before
                .length
                .saturating_add(spec.demo_cfg.len() as u64)
                .saturating_add(spec.gsi_cfg.len() as u64),
        )?;
        let (demo_file, demo) = write_new(&spec.staged_demo, |target| {
            let content = transfer(&mut source, target, Some(cancelled))?;
            let verified_path = open_source(&spec.source_demo)?;
            if content.bytes != before.length
                || source_stamp(&source)? != before
                || source_stamp(&verified_path)? != before
            {
                return Err(io::Error::other(
                    "The recording Demo changed during staging",
                ));
            }
            Ok(content)
        })?;
        let (cfg_file, cfg) = match write_new(&spec.demo_cfg_path, |target| {
            transfer(&mut spec.demo_cfg.as_bytes(), target, Some(cancelled))
        }) {
            Ok(created) => created,
            Err(error) => return Err(with_rollback(error, &demo_file)),
        };
        let (gsi_file, gsi) = match write_new(&spec.gsi_cfg_path, |target| {
            transfer(&mut spec.gsi_cfg.as_bytes(), target, Some(cancelled))
        }) {
            Ok(created) => created,
            Err(error) => {
                let error = with_rollback(error, &cfg_file);
                return Err(with_rollback(error, &demo_file));
            }
        };
        // A caller that cancels after staging returns must finish the stage.
        if let Err(error) = check_cancelled(Some(cancelled)) {
            let error = with_rollback(error, &gsi_file);
            let error = with_rollback(error, &cfg_file);
            return Err(with_rollback(error, &demo_file));
        }
        Ok(Self {
            demo: Some(demo),
            cfg: Some(cfg),
            gsi: Some(gsi),
        })
    }

    /// Call only after the consuming process is stopped. All paths are tried
    /// independently, and a failed cleanup remains available for a safe retry.
    pub(super) fn finish(&mut self) -> io::Result<()> {
        let mut result = Ok(());
        for slot in [&mut self.gsi, &mut self.cfg, &mut self.demo] {
            if let Some(file) = slot {
                match file.remove_if_unchanged() {
                    Ok(()) => *slot = None,
                    Err(error) if result.is_ok() => result = Err(error),
                    Err(_) => {}
                }
            }
        }
        result
    }
}

fn verify_space(directory: &Path, required: u64) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let path: Vec<_> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0_u64;
    // The staging files are all children of game/csgo, whose plain ancestors
    // stay open throughout the placement.
    let available = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if available == 0 {
        return Err(io::Error::last_os_error());
    }
    if free < required {
        return Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "Not enough disk space for the playback Demo",
        ));
    }
    Ok(())
}

fn validate_destinations(spec: &PlaybackSpec) -> io::Result<PathBuf> {
    let csgo = spec.working_dir.join("csgo");
    let cfg = csgo.join("cfg");
    let stem = spec
        .staged_demo
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".dem"));
    let Some(stem) = stem else {
        return Err(invalid("Missing the generated recording Demo filename"));
    };
    let Some(session) = stem
        .strip_prefix("_insight_")
        .filter(|id| hexadecimal_id(id, 32))
    else {
        return Err(invalid("Recording stage path has no valid session ID"));
    };
    if spec.staged_demo.parent() != Some(csgo.as_path())
        || spec.demo_cfg_path != cfg.join(format!("{stem}.cfg"))
        || spec.gsi_cfg_path != cfg.join(gsi_config_name(session))
    {
        return Err(invalid(
            "Recording stage paths do not match the generated session",
        ));
    }
    Ok(cfg)
}

pub(super) fn lock_directories(path: &Path) -> io::Result<Vec<File>> {
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)))
        || components.next() != Some(Component::RootDir)
        || !components.all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(invalid(
            "Recording stage must use an absolute Windows drive directory",
        ));
    }
    let ancestors: Vec<_> = path.ancestors().collect();
    ancestors
        .into_iter()
        .rev()
        .map(|ancestor| {
            // Hold the parent while opening the next child, so a directory cannot be swapped
            // for a junction between inspection and use.
            let directory = OpenOptions::new()
                .access_mode(READ_ATTRIBUTES)
                .share_mode(SHARE_READ)
                .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS)
                .open(ancestor)?;
            let metadata = directory.metadata()?;
            if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
                return Err(invalid(
                    "Recording stage directory must be a plain directory",
                ));
            }
            Ok(directory)
        })
        .collect()
}

fn open_source(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
}

fn source_stamp(file: &File) -> io::Result<SourceStamp> {
    let metadata = regular_metadata(file)?;
    Ok(SourceStamp {
        id: file_id(file)?,
        length: metadata.len(),
        written: metadata.last_write_time(),
    })
}

fn write_new(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<ContentStamp>,
) -> io::Result<(File, OwnedFile)> {
    // Retain an exclusive DELETE handle until the whole stage is ready so a rollback
    // can target the new file even when writing failed before hashing completed.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .share_mode(0)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)?;
    let result = (|| {
        regular_metadata(&file)?;
        let id = file_id(&file)?;
        let content = write(&mut file)?;
        file.sync_all()?;
        Ok(OwnedFile {
            path: path.to_owned(),
            id,
            content,
        })
    })();
    match result {
        Ok(owned) => Ok((file, owned)),
        Err(error) => Err(with_rollback(error, &file)),
    }
}

impl OwnedFile {
    fn remove_if_unchanged(&self) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| invalid("Missing recording file parent"))?;
        let _directories = lock_directories(parent)?;
        let mut file = match OpenOptions::new()
            .access_mode(GENERIC_READ | DELETE)
            .share_mode(0)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if regular_metadata(&file)?.len() != self.content.bytes
            || file_id(&file)? != self.id
            || transfer(&mut file, &mut io::sink(), None)? != self.content
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "A staged recording file has changed",
            ));
        }
        mark_deleted(&file)
    }
}

fn transfer(
    input: &mut impl Read,
    output: &mut impl Write,
    cancelled: Option<&AtomicBool>,
) -> io::Result<ContentStamp> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0;
    let mut digest = Sha256::new();
    loop {
        check_cancelled(cancelled)?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let chunk = &buffer[..count];
        output.write_all(chunk)?;
        digest.update(chunk);
        bytes += count as u64;
    }
    Ok(ContentStamp {
        bytes,
        digest: digest.finalize().into(),
    })
}

fn regular_metadata(file: &File) -> io::Result<Metadata> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(invalid("Recording staging requires a plain file"));
    }
    Ok(metadata)
}

fn file_id(file: &File) -> io::Result<FileId> {
    let mut identity = FileId::default();
    // SAFETY: the buffer matches FILE_ID_INFO and the file handle remains live.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FILE_ID_INFO,
            (&mut identity as *mut FileId).cast(),
            std::mem::size_of::<FileId>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(identity)
}

fn mark_deleted(file: &File) -> io::Result<()> {
    let delete: u8 = 1;
    // SAFETY: FILE_DISPOSITION_INFO contains one BOOLEAN and this handle has DELETE access.
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FILE_DISPOSITION_INFO,
            (&delete as *const u8).cast(),
            1,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn with_rollback(error: io::Error, file: &File) -> io::Error {
    match mark_deleted(file) {
        Ok(()) => error,
        Err(rollback) => io::Error::new(
            error.kind(),
            format!("{error}. Could not retire a staged file: {rollback}"),
        ),
    }
}

fn check_cancelled(cancelled: Option<&AtomicBool>) -> io::Result<()> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Recording staging cancelled",
        ));
    }
    Ok(())
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::super::playback_spec;
    use super::*;
    use std::{fs, num::NonZeroU16};

    fn fixture(root: &Path) -> PlaybackSpec {
        let game = root.join("Game");
        fs::create_dir_all(game.join("csgo/cfg")).unwrap();
        let file = root.join("Synthetic Demo.dem");
        fs::write(&file, b"test fixture demo content").unwrap();
        playback_spec::build(
            &game.join("bin/win64/cs2.exe"),
            &file,
            "0123456789abcdef0123456789abcdef",
            "F10",
            None,
            playback_spec::GsiTarget {
                port: NonZeroU16::new(19871).unwrap(),
                token: "111122223333444455556666777788889999aaaabbbbccccddddeeeeffff0000",
            },
        )
        .unwrap()
    }

    #[test]
    fn rejects_a_demo_replaced_after_queue_planning() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = fixture(root.path());
        let (locked, stamp) = SourceStamp::open(&spec.source_demo).unwrap();
        drop(locked);
        spec.source_stamp = Some(stamp);
        fs::write(&spec.source_demo, b"a different Demo").unwrap();

        assert!(PlaybackStage::stage(&spec, &AtomicBool::new(false)).is_err());
        assert!(!spec.staged_demo.exists());
        assert!(!spec.demo_cfg_path.exists());
        assert!(!spec.gsi_cfg_path.exists());
        assert_eq!(fs::read(&spec.source_demo).unwrap(), b"a different Demo");
    }

    #[test]
    fn stages_and_retires_only_owned_files() {
        let root = tempfile::tempdir().unwrap();
        let spec = fixture(root.path());
        let old_gsi = spec
            .working_dir
            .join("csgo/cfg/gamestate_integration_cs2_insight_agent.cfg");
        fs::write(&old_gsi, b"existing GSI config").unwrap();
        let mut stage = PlaybackStage::stage(&spec, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            fs::read(&spec.staged_demo).unwrap(),
            fs::read(&spec.source_demo).unwrap()
        );
        assert_eq!(
            fs::read(&spec.demo_cfg_path).unwrap(),
            spec.demo_cfg.as_bytes()
        );
        assert_eq!(
            fs::read(&spec.gsi_cfg_path).unwrap(),
            spec.gsi_cfg.as_bytes()
        );
        stage.finish().unwrap();
        stage.finish().unwrap();
        assert!(
            !spec.staged_demo.exists()
                && !spec.demo_cfg_path.exists()
                && !spec.gsi_cfg_path.exists()
        );
        assert_eq!(fs::read(&old_gsi).unwrap(), b"existing GSI config");
        assert_eq!(
            fs::read(&spec.source_demo).unwrap(),
            b"test fixture demo content"
        );
    }

    #[test]
    fn exclusive_staging_never_replaces_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let spec = fixture(root.path());
        fs::write(&spec.demo_cfg_path, b"foreign cfg").unwrap();
        assert!(PlaybackStage::stage(&spec, &AtomicBool::new(false)).is_err());
        assert!(!spec.staged_demo.exists());
        assert_eq!(fs::read(&spec.demo_cfg_path).unwrap(), b"foreign cfg");
        fs::write(&spec.staged_demo, b"foreign demo").unwrap();
        assert!(PlaybackStage::stage(&spec, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&spec.staged_demo).unwrap(), b"foreign demo");
        assert_eq!(fs::read(&spec.demo_cfg_path).unwrap(), b"foreign cfg");
    }

    #[test]
    fn gsi_collision_rolls_back_the_other_two_files() {
        let root = tempfile::tempdir().unwrap();
        let spec = fixture(root.path());
        fs::write(&spec.gsi_cfg_path, b"foreign GSI cfg").unwrap();
        assert!(PlaybackStage::stage(&spec, &AtomicBool::new(false)).is_err());
        assert!(!spec.staged_demo.exists() && !spec.demo_cfg_path.exists());
        assert_eq!(fs::read(&spec.gsi_cfg_path).unwrap(), b"foreign GSI cfg");
    }

    #[test]
    fn cleanup_preserves_in_place_changes_and_byte_identical_replacements() {
        let root = tempfile::tempdir().unwrap();
        let spec = fixture(root.path());
        let mut stage = PlaybackStage::stage(&spec, &AtomicBool::new(false)).unwrap();
        let previous = spec.working_dir.join("old-owned-demo.dem");
        fs::rename(&spec.staged_demo, previous).unwrap();
        fs::copy(&spec.source_demo, &spec.staged_demo).unwrap();
        let content = vec![b'x'; spec.demo_cfg.len()];
        fs::write(&spec.demo_cfg_path, &content).unwrap();
        assert!(stage.finish().is_err());
        assert_eq!(
            fs::read(&spec.staged_demo).unwrap(),
            b"test fixture demo content"
        );
        assert_eq!(fs::read(&spec.demo_cfg_path).unwrap(), content);
    }

    #[test]
    fn cancel_and_active_source_writer_leave_no_staged_files() {
        let root = tempfile::tempdir().unwrap();
        let spec = fixture(root.path());
        assert!(PlaybackStage::stage(&spec, &AtomicBool::new(true)).is_err());
        let _writer = OpenOptions::new()
            .write(true)
            .open(&spec.source_demo)
            .unwrap();
        assert!(PlaybackStage::stage(&spec, &AtomicBool::new(false)).is_err());
        assert!(!spec.staged_demo.exists() && !spec.demo_cfg_path.exists());
        assert_eq!(
            fs::read(&spec.source_demo).unwrap(),
            b"test fixture demo content"
        );
    }

    #[test]
    fn cancellation_during_copy_removes_the_partially_written_file() {
        struct CancelAfterRead<'a>(&'a AtomicBool);
        impl Read for CancelAfterRead<'_> {
            fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
                destination[0] = 7;
                self.0.store(true, Ordering::Relaxed);
                Ok(1)
            }
        }

        let root = tempfile::tempdir().unwrap();
        let spec = fixture(root.path());
        let cancelled = AtomicBool::new(false);
        let result = write_new(&spec.staged_demo, |file| {
            transfer(&mut CancelAfterRead(&cancelled), file, Some(&cancelled))
        });
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::Interrupted));
        assert!(!spec.staged_demo.exists());
    }
}
