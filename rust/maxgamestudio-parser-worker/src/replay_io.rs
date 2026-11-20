use crate::Failure;
use parser::first_pass::parser::HEADER_ENDS_AT_BYTE;
use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::Path,
};

pub(crate) fn validate_destination(
    path: &Path,
    suffix: &str,
    invalid_path: &'static str,
) -> Result<(), Failure> {
    if !path.is_absolute()
        || !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(suffix))
    {
        return Err(Failure::request(invalid_path));
    }
    match fs::symlink_metadata(path) {
        Ok(_) => Err(Failure::replay_write("staged replay output already exists")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Failure::replay_write(
            "could not inspect staged output path",
        )),
    }
}

pub(crate) fn stage<T>(
    path: &Path,
    encode: impl FnOnce(&mut File) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let parent = path
        .parent()
        .ok_or_else(|| Failure::request("expected a staged replay output directory"))?;
    let mut file = tempfile::Builder::new()
        .prefix(".mgs-replay-")
        .tempfile_in(parent)
        .map_err(|_| Failure::replay_write("could not create replay scratch file"))?;
    let result = encode(file.as_file_mut())?;
    file.as_file()
        .sync_all()
        .map_err(|_| Failure::replay_write("could not flush replay scratch file"))?;
    file.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == ErrorKind::AlreadyExists {
            Failure::replay_write("staged replay output already exists")
        } else {
            Failure::replay_write("could not stage completed replay output")
        }
    })?;
    Ok(result)
}

pub(crate) fn with_demo_bytes<T>(
    path: &Path,
    run: impl FnOnce(&[u8]) -> Result<T, Failure>,
) -> Result<T, Failure> {
    if !path.is_absolute() {
        return Err(Failure::request("expected an absolute demo path"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        options.share_mode(FILE_SHARE_READ);
    }
    let file = options
        .open(path)
        .map_err(|_| Failure::read("could not open demo for stable reading"))?;
    let metadata = file
        .metadata()
        .map_err(|_| Failure::read("could not inspect demo"))?;
    if !metadata.is_file() {
        return Err(Failure::read("expected a regular demo file"));
    }
    if metadata.len() < HEADER_ENDS_AT_BYTE as u64 {
        return Err(Failure::replay_parse("invalid Source 2 demo framing"));
    }
    #[cfg(windows)]
    // SAFETY: The Windows handle denies write sharing and stays live for the mapping.
    let bytes = unsafe { memmap2::MmapOptions::new().map(&file) }
        .map_err(|_| Failure::read("could not map demo"))?;
    #[cfg(not(windows))]
    let bytes = {
        use std::io::Read;
        let mut file = file;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| Failure::read("could not read demo"))?;
        bytes
    };
    if bytes.len() < HEADER_ENDS_AT_BYTE || !bytes.starts_with(b"PBDEMS2\0") {
        return Err(Failure::replay_parse("invalid Source 2 demo framing"));
    }
    run(&bytes)
}
