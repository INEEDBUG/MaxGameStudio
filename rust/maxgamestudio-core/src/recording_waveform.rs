use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, UNIX_EPOCH},
};
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

const CACHE_BYTES: u64 = 1024 * 1024;
const SAMPLE_RATE: usize = 400;
const CACHE_BUCKETS: usize = 16_384;
const PCM_LIMIT: u64 = 86_400 * 400 * 4;

#[derive(Debug, Clone, Copy)]
pub(crate) struct WaveformSlice {
    pub buckets: usize,
    pub start: f64,
    pub end: Option<f64>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct WaveformCache {
    version: u8,
    source_size: u64,
    source_mtime_ns: i64,
    duration_sec: f64,
    peaks: Vec<f64>,
}

pub(crate) struct WaveformSource {
    pub path: PathBuf,
    sidecar: PathBuf,
    size: u64,
    modified: i64,
}

impl WaveformSource {
    pub(crate) fn open(path: PathBuf) -> io::Result<Self> {
        let (size, modified) = fingerprint(&path)?;
        let mut name = path
            .file_stem()
            .ok_or_else(|| io::Error::other("素材文件名无效"))?
            .to_os_string();
        name.push(".waveform-v1.json");
        let sidecar = path.with_file_name(name);
        Ok(Self {
            path,
            sidecar,
            size,
            modified,
        })
    }

    pub(crate) fn cached(&self) -> Option<WaveformCache> {
        let mut bytes = Vec::new();
        File::open(&self.sidecar)
            .ok()?
            .take(CACHE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > CACHE_BYTES {
            return None;
        }
        let cache: WaveformCache = serde_json::from_slice(&bytes).ok()?;
        (cache.version == 1
            && cache.source_size == self.size
            && cache.source_mtime_ns == self.modified
            && cache.peaks.len() <= CACHE_BUCKETS
            && cache.duration_sec.is_finite())
        .then_some(cache)
    }

    pub(crate) fn publish(
        self,
        pcm: Vec<u8>,
        measured_duration: Option<f64>,
    ) -> io::Result<WaveformCache> {
        if fingerprint(&self.path)? != (self.size, self.modified) {
            return Err(io::Error::other("生成波形期间素材发生变化"));
        }
        let length = pcm.len() / 4;
        let duration = measured_duration
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(length as f64 / SAMPLE_RATE as f64);
        let peaks = bucket_peaks(length, CACHE_BUCKETS, |sample| {
            let bytes = pcm[sample * 4..sample * 4 + 4]
                .try_into()
                .expect("one PCM sample");
            f32::from_le_bytes(bytes)
        });
        let cache = WaveformCache {
            version: 1,
            source_size: self.size,
            source_mtime_ns: self.modified,
            duration_sec: duration,
            peaks,
        };
        let parent = self.sidecar.parent().expect("canonical source parent");
        let mut temporary = tempfile::Builder::new()
            .prefix(".waveform-")
            .tempfile_in(parent)?;
        serde_json::to_writer(temporary.as_file_mut(), &cache)?;
        temporary.as_file_mut().flush()?;
        temporary
            .persist(&self.sidecar)
            .map_err(|error| error.error)?;
        Ok(cache)
    }
}

impl WaveformCache {
    pub(crate) fn view(&self, query: WaveformSlice, cached: bool) -> Value {
        let duration = self.duration_sec.max(0.001);
        let start = query.start.min(duration);
        let end = query.end.map_or(duration, |end| end.clamp(start, duration));
        let end = if end <= start {
            duration.min(start + 0.001)
        } else {
            end
        };
        let len = self.peaks.len();
        let left = ((start / duration) * len as f64) as usize;
        let right = (((end / duration) * len as f64 + 0.999) as usize)
            .max(left + 1)
            .min(len);
        let selected = &self.peaks[left..right];
        let values = bucket_peaks(selected.len(), query.buckets, |index| {
            selected[index].max(0.0) as f32
        });
        let peak = values.iter().copied().fold(0.0001, f64::max);
        let peaks = values
            .into_iter()
            .map(|value| round((value / peak).clamp(0.04, 1.0), 10_000.0))
            .collect::<Vec<_>>();
        json!({ "peaks": peaks, "duration_sec": duration, "start_sec": start, "end_sec": end,
            "buckets": query.buckets, "cached": cached })
    }
}

fn bucket_peaks(length: usize, buckets: usize, sample: impl Fn(usize) -> f32) -> Vec<f64> {
    if length == 0 {
        return vec![0.0; buckets];
    }
    (0..buckets)
        .map(|index| {
            let boundary = |position| (position as u64 * length as u64 / buckets as u64) as usize;
            let start = boundary(index).min(length - 1);
            let end = boundary(index + 1).max(start + 1).min(length);
            let peak = (start..end)
                .map(|offset| f64::from(sample(offset).abs()))
                .filter(|value| value.is_finite())
                .fold(0.0, f64::max);
            round(peak, 1_000_000.0)
        })
        .collect()
}

fn round(value: f64, factor: f64) -> f64 {
    (value * factor).round_ties_even() / factor
}

fn fingerprint(path: &Path) -> io::Result<(u64, i64)> {
    let metadata = fs::metadata(path)?;
    let nanos = match metadata.modified()?.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    Ok((
        metadata.len(),
        i64::try_from(nanos).map_err(io::Error::other)?,
    ))
}

pub(crate) async fn decode(ffmpeg: &Path, source: &Path) -> io::Result<Vec<u8>> {
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-threads",
            "1",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(source)
        .args([
            "-map", "0:a:0", "-vn", "-ac", "1", "-ar", "400", "-f", "f32le", "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    let mut child = command.spawn()?;
    timeout(Duration::from_secs(230), async {
        let mut pcm = Vec::new();
        child
            .stdout
            .take()
            .expect("piped PCM")
            .take(PCM_LIMIT + 4)
            .read_to_end(&mut pcm)
            .await?;
        if pcm.len() as u64 > PCM_LIMIT {
            return Err(io::Error::other("素材时长超过 24 小时"));
        }
        if !child.wait().await?.success() || pcm.len() < 4 || !pcm.len().is_multiple_of(4) {
            return Err(io::Error::other("素材没有可用于生成波形的音轨"));
        }
        Ok(pcm)
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "生成素材波形超时"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_the_existing_sidecar_and_invalidates_it_when_recording_changes() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("历史片段.mp4");
        let original = b"recording bytes";
        fs::write(&path, original).unwrap();
        let source = WaveformSource::open(path.clone()).unwrap();
        let pcm = [0.125f32, -0.25, 0.5, -1.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let query = WaveformSlice {
            buckets: 8,
            start: 0.0,
            end: None,
        };
        let generated = source.publish(pcm, Some(4.0)).unwrap();
        let expected = json!({
            "peaks":[0.125,0.125,0.25,0.25,0.5,0.5,1.0,1.0], "duration_sec":4.0,
            "start_sec":0.0, "end_sec":4.0, "buckets":8, "cached":false,
        });
        assert_eq!(generated.view(query, false), expected);
        assert_eq!(fs::read(&path).unwrap(), original);
        let source = WaveformSource::open(path.clone()).unwrap();
        let previous = fs::read(&source.sidecar).unwrap();
        let cached = source.cached().unwrap();
        assert_eq!(cached.view(query, false), expected);
        assert_eq!(
            cached.view(
                WaveformSlice {
                    start: 2.0,
                    end: Some(3.0),
                    ..query
                },
                true
            )["peaks"],
            json!(vec![1.0; 8])
        );
        fs::write(&path, b"newer recording bytes").unwrap();
        let source = WaveformSource::open(path).unwrap();
        assert!(source.cached().is_none());
        assert_eq!(fs::read(source.sidecar).unwrap(), previous);
    }
}
