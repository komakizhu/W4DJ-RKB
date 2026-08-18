//! Conservative local-media probing for the library catalog.
//!
//! The catalog must distinguish facts measured from an output file from values
//! imported from NetEase.  We use the existing FFmpeg runtime when available
//! for stream facts, while keeping container detection independent of the
//! filename extension.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredAudioFacts {
    pub path: PathBuf,
    pub format: String,
    pub size_bytes: i64,
    pub duration_seconds: Option<f64>,
    pub average_bitrate_bps: Option<i64>,
    pub sample_rate_hz: Option<i64>,
    pub channels: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    NotAFile,
    EmptyFile,
    UnknownContainer,
    FfmpegUnavailable,
    FfmpegFailed(String),
    InvalidOutput(String),
    Io(String),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAFile => write!(formatter, "path is not a regular file"),
            Self::EmptyFile => write!(formatter, "audio file is empty"),
            Self::UnknownContainer => write!(formatter, "unsupported or unknown audio container"),
            Self::FfmpegUnavailable => write!(formatter, "FFmpeg is not available"),
            Self::FfmpegFailed(message) => write!(formatter, "FFmpeg probe failed: {message}"),
            Self::InvalidOutput(message) => {
                write!(formatter, "invalid FFmpeg probe output: {message}")
            }
            Self::Io(message) => write!(formatter, "I/O error: {message}"),
        }
    }
}

impl std::error::Error for ProbeError {}

pub fn probe_local_audio(path: &Path) -> Result<MeasuredAudioFacts, ProbeError> {
    let metadata = fs::metadata(path).map_err(|error| ProbeError::Io(error.to_string()))?;
    if !metadata.is_file() {
        return Err(ProbeError::NotAFile);
    }
    if metadata.len() == 0 {
        return Err(ProbeError::EmptyFile);
    }

    let bytes = fs::read(path).map_err(|error| ProbeError::Io(error.to_string()))?;
    let format = detect_container(&bytes).ok_or(ProbeError::UnknownContainer)?;
    let size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    let mut facts = MeasuredAudioFacts {
        path: path.to_path_buf(),
        format: format.to_string(),
        size_bytes,
        duration_seconds: None,
        average_bitrate_bps: None,
        sample_rate_hz: None,
        channels: None,
    };

    let probe = probe_with_ffmpeg(path)?;
    facts.duration_seconds = probe.duration_seconds;
    facts.average_bitrate_bps = probe
        .average_bitrate_bps
        .or_else(|| calculate_average_bitrate(size_bytes, facts.duration_seconds));
    facts.sample_rate_hz = probe.sample_rate_hz;
    facts.channels = probe.channels;
    Ok(facts)
}

fn detect_container(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"fLaC") {
        return Some("flac");
    }
    if bytes.len() >= 12
        && bytes.starts_with(b"RIFF")
        && bytes.get(8..12) == Some(b"WAVE".as_slice())
    {
        return Some("wav");
    }
    if bytes.len() >= 12
        && bytes.starts_with(b"FORM")
        && matches!(bytes.get(8..12), Some(b"AIFF" | b"AIFC"))
    {
        return Some("aiff");
    }
    if bytes.starts_with(b"ID3") || looks_like_mp3_frame(bytes) {
        return Some("mp3");
    }
    None
}

fn looks_like_mp3_frame(bytes: &[u8]) -> bool {
    bytes
        .windows(2)
        .take(4096)
        .any(|window| window[0] == 0xFF && (window[1] & 0xE0) == 0xE0 && (window[1] & 0x06) != 0)
}

#[derive(Debug, Default)]
struct FfmpegProbe {
    duration_seconds: Option<f64>,
    average_bitrate_bps: Option<i64>,
    sample_rate_hz: Option<i64>,
    channels: Option<i64>,
}

fn probe_with_ffmpeg(path: &Path) -> Result<FfmpegProbe, ProbeError> {
    let executable = crate::sync::find_ffmpeg().ok_or(ProbeError::FfmpegUnavailable)?;
    let output = Command::new(executable)
        .args(["-hide_banner", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .map_err(|error| ProbeError::Io(error.to_string()))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        let message = stderr
            .lines()
            .rev()
            .find(|line| line.contains("Invalid") || line.contains("Error"))
            .unwrap_or("unknown media error")
            .trim()
            .to_string();
        return Err(ProbeError::FfmpegFailed(message));
    }
    parse_ffmpeg_probe(&stderr)
}

fn parse_ffmpeg_probe(text: &str) -> Result<FfmpegProbe, ProbeError> {
    let mut probe = FfmpegProbe::default();
    for line in text.lines() {
        if let Some(value) = line.trim().strip_prefix("Duration: ") {
            let value = value.split(',').next().unwrap_or_default().trim();
            probe.duration_seconds = parse_clock_duration(value);
        }
        if let Some(value) = line.split("bitrate:").nth(1) {
            probe.average_bitrate_bps = parse_kbps(value);
        }
        if line.contains("Audio:") {
            let details = line.split("Audio:").nth(1).unwrap_or_default();
            for token in details.split(',').map(str::trim) {
                if probe.sample_rate_hz.is_none() {
                    probe.sample_rate_hz = parse_sample_rate(token);
                }
                if probe.channels.is_none() {
                    probe.channels = parse_channels(token);
                }
            }
        }
    }
    if probe.duration_seconds.is_none()
        && probe.average_bitrate_bps.is_none()
        && probe.sample_rate_hz.is_none()
        && probe.channels.is_none()
    {
        return Err(ProbeError::InvalidOutput(
            "no audio stream facts found".to_string(),
        ));
    }
    Ok(probe)
}

fn parse_clock_duration(value: &str) -> Option<f64> {
    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<f64>().ok()?;
    let minutes = parts.next()?.parse::<f64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn parse_kbps(value: &str) -> Option<i64> {
    let value = value.split_whitespace().next()?.parse::<f64>().ok()?;
    if value.is_finite() && value > 0.0 {
        Some((value * 1000.0).round() as i64)
    } else {
        None
    }
}

fn parse_sample_rate(token: &str) -> Option<i64> {
    let value = token
        .split_whitespace()
        .next()?
        .trim_end_matches("Hz")
        .parse::<i64>()
        .ok()?;
    (value >= 8000).then_some(value)
}

fn parse_channels(token: &str) -> Option<i64> {
    let normalized = token.to_ascii_lowercase();
    if normalized.contains("mono") {
        Some(1)
    } else if normalized.contains("stereo") {
        Some(2)
    } else {
        None
    }
}

fn calculate_average_bitrate(size_bytes: i64, duration_seconds: Option<f64>) -> Option<i64> {
    let duration = duration_seconds.filter(|value| value.is_finite() && *value > 0.0)?;
    Some(((size_bytes as f64 * 8.0) / duration).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::{detect_container, parse_clock_duration, parse_ffmpeg_probe};

    #[test]
    fn detects_containers_from_signatures() {
        assert_eq!(detect_container(b"fLaC\0\0"), Some("flac"));
        assert_eq!(detect_container(b"RIFF\0\0\0\0WAVEfmt "), Some("wav"));
        assert_eq!(detect_container(b"FORM\0\0\0\0AIFF"), Some("aiff"));
        assert_eq!(detect_container(b"ID3\x04\0\0"), Some("mp3"));
        assert_eq!(detect_container(b"not audio"), None);
    }

    #[test]
    fn parses_ffmpeg_audio_facts() {
        let output = "Duration: 00:01:02.50, start: 0.000000, bitrate: 320 kb/s\n    Stream #0:0: Audio: flac, 48000 Hz, stereo, s32";
        let probe = parse_ffmpeg_probe(output).unwrap();
        assert_eq!(probe.duration_seconds, Some(62.5));
        assert_eq!(probe.average_bitrate_bps, Some(320_000));
        assert_eq!(probe.sample_rate_hz, Some(48_000));
        assert_eq!(probe.channels, Some(2));
        assert_eq!(parse_clock_duration("00:02:03.25"), Some(123.25));
    }
}
