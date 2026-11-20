//! Inspect a selected watcher directory without opening demos or archives.

use crate::profile_platform::{expand_user, path_text};
use serde_json::{json, Value};
use std::{
    fs, io,
    path::Path,
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 50_000;
const INSPECTION_TIME: Duration = Duration::from_secs(3);
const UNAVAILABLE: &str = "目录不存在或无法访问";
const LIMIT_REACHED: &str = "目录检查已达到扫描上限，请缩小目录范围或扫描深度";

pub(super) fn inspect(path: &str, depth: usize) -> Value {
    let candidate = match expand_user(path) {
        Ok(candidate) => candidate,
        Err(error) => return invalid(path, &error.to_string()),
    };
    let input = path_text(&candidate);
    if !candidate.is_dir() {
        return invalid(&input, UNAVAILABLE);
    }
    let resolved = match fs::canonicalize(&candidate) {
        Ok(resolved) => resolved,
        Err(error) => return invalid(&input, &error.to_string()),
    };
    match count_candidates(
        &resolved,
        depth,
        MAX_ENTRIES,
        Instant::now() + INSPECTION_TIME,
    ) {
        Ok((demo_count, zip_count)) => json!({
            "valid": true, "path": path_text(&resolved),
            "demo_count": demo_count, "zip_count": zip_count, "max_depth": depth,
        }),
        Err(error) => invalid(&input, &error.to_string()),
    }
}

fn invalid(path: &str, error: &str) -> Value {
    json!({"valid": false, "path": path, "demo_count": 0, "zip_count": 0, "error": error})
}

fn count_candidates(
    root: &Path,
    max_depth: usize,
    max_entries: usize,
    deadline: Instant,
) -> io::Result<(usize, usize)> {
    let mut stack = vec![(fs::read_dir(root)?, 0usize)];
    let mut visited = 0;
    let mut demos = 0;
    let mut zips = 0;
    while let Some((entries, depth)) = stack.last_mut() {
        let Some(entry) = entries.next() else {
            stack.pop();
            continue;
        };
        if visited == max_entries || Instant::now() >= deadline {
            return Err(io::Error::other(LIMIT_REACHED));
        }
        visited += 1;
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if *depth < max_depth && !is_reparse_directory(&entry)? {
                let next_depth = *depth + 1;
                // The original os.walk skips subdirectories it cannot enumerate.
                if let Ok(children) = fs::read_dir(entry.path()) {
                    stack.push((children, next_depth));
                }
            }
        } else if !(kind.is_symlink() && entry.path().is_dir()) {
            // Python os.walk reports file links in filenames and excludes directory links.
            let filename = entry.file_name();
            let extension = Path::new(&filename)
                .extension()
                .and_then(|value| value.to_str());
            match extension {
                Some(extension) if extension.eq_ignore_ascii_case("dem") => demos += 1,
                Some(extension) if extension.eq_ignore_ascii_case("zip") => zips += 1,
                _ => {}
            }
        }
    }
    if Instant::now() >= deadline {
        return Err(io::Error::other(LIMIT_REACHED));
    }
    Ok((demos, zips))
}

#[cfg(windows)]
fn is_reparse_directory(entry: &fs::DirEntry) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    Ok(entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

#[cfg(not(windows))]
fn is_reparse_directory(_entry: &fs::DirEntry) -> io::Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_reports_a_budget_limit_instead_of_partial_counts() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("one.dem"), []).unwrap();
        fs::write(tmp.path().join("two.zip"), []).unwrap();
        assert_eq!(
            count_candidates(tmp.path(), 0, 2, Instant::now() + INSPECTION_TIME).unwrap(),
            (1, 1)
        );
        assert_eq!(
            count_candidates(tmp.path(), 0, 1, Instant::now() + INSPECTION_TIME)
                .unwrap_err()
                .to_string(),
            LIMIT_REACHED
        );
        assert_eq!(
            count_candidates(tmp.path(), 0, 2, Instant::now())
                .unwrap_err()
                .to_string(),
            LIMIT_REACHED
        );
    }
}
