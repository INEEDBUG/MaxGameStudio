use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

/// Locations read from the standalone toolbox's existing control directory.
/// The caller validates and completes any migration before changing the active locator.
#[derive(Debug, Eq, PartialEq)]
pub struct ProfileLocation {
    pub active: Option<PathBuf>,
    pub selected: PathBuf,
}

impl ProfileLocation {
    pub fn read(control: &Path) -> io::Result<Self> {
        let active = read_location(&control.join("data-location.txt"))?;
        let pending = read_location(&control.join("installer-data-location.txt"))?;
        let selected = pending
            .or_else(|| active.clone())
            .unwrap_or_else(|| control.join("Profile"));
        Ok(Self { active, selected })
    }
}

fn read_location(file: &Path) -> io::Result<Option<PathBuf>> {
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().copied().map(u16::from_le_bytes).collect();
    let decoded = String::from_utf16_lossy(&units);
    // The Electron launcher used String.trim(), whose whitespace set also includes the BOM.
    let path = decoded.trim_matches(|character| {
        matches!(
            character,
            '\u{0009}'..='\u{000d}'
                | '\u{0020}'
                | '\u{00a0}'
                | '\u{1680}'
                | '\u{2000}'..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
        )
    });
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}
