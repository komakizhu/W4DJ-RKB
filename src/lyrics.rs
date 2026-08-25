//! Local helpers for exporting lyrics alongside converted audio.
//!
//! Audio conversion must not fail just because a lyric sidecar cannot be
//! written. The helper normalizes line endings and writes through a temporary
//! file before replacing the intended sidecar. If a user already has a
//! different sidecar, a numbered W4DJ sidecar is used instead.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeteaseLyricRecord {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizedLyrics {
    pub plain: String,
    pub translated: String,
    pub romanized: String,
    pub lrc: String,
    pub warnings: Vec<String>,
}

pub fn normalize_lyrics(records: &[NeteaseLyricRecord]) -> NormalizedLyrics {
    let mut result = NormalizedLyrics::default();
    for record in records {
        let normalized = normalize_lrc(&record.text);
        if normalized.is_empty() {
            continue;
        }
        match record.kind.to_ascii_lowercase().as_str() {
            "translated" | "translation" => result.translated = normalized,
            "romanized" | "romaji" => result.romanized = normalized,
            "lrc" | "original_lrc" => {
                if normalized.lines().any(|line| !line.starts_with('[')) {
                    result.warnings.push("LRC 中存在未带时间戳的行".to_string());
                }
                result.lrc = normalized.clone();
                result.plain = strip_lrc_timestamps(&normalized);
            }
            _ => result.plain = normalized,
        }
    }
    result
}

fn strip_lrc_timestamps(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let mut rest = line.trim();
            while rest.starts_with('[') {
                let Some(end) = rest.find(']') else {
                    break;
                };
                let timestamp = &rest[1..end];
                let valid = timestamp.split_once(':').is_some_and(|(minutes, seconds)| {
                    minutes.parse::<u32>().is_ok()
                        && seconds
                            .parse::<f32>()
                            .is_ok_and(|value| value.is_finite() && value >= 0.0)
                });
                if !valid {
                    break;
                }
                rest = &rest[end + 1..];
            }
            rest.trim()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn normalize_lrc(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn write_lrc_sidecar(audio_path: &Path, value: &str) -> io::Result<Option<PathBuf>> {
    let contents = normalize_lrc(value);
    if contents.is_empty() {
        return Ok(None);
    }

    let parent = audio_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("track");
    let preferred = parent.join(format!("{stem}.lrc"));
    let target = if !preferred.exists()
        || fs::read_to_string(&preferred).ok().as_deref() == Some(contents.as_str())
    {
        preferred
    } else {
        (1..1000)
            .map(|index| parent.join(format!("{stem}.w4dj-{index}.lrc")))
            .find(|candidate| !candidate.exists())
            .unwrap_or_else(|| parent.join(format!("{stem}.w4dj.lrc")))
    };

    let mut temporary = NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut temporary, contents.as_bytes())?;
    temporary.persist(&target).map_err(|error| error.error)?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::{NeteaseLyricRecord, normalize_lrc, normalize_lyrics, write_lrc_sidecar};

    #[test]
    fn normalizes_line_endings_and_ignores_blank_lines() {
        assert_eq!(
            normalize_lrc("[00:01.00]a\r\n\r\n[00:02.00]b\r"),
            "[00:01.00]a\n[00:02.00]b"
        );
    }

    #[test]
    fn preserves_a_different_user_sidecar() {
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("track.mp3");
        let sidecar = directory.path().join("track.lrc");
        std::fs::write(&sidecar, "user lyric\n").unwrap();

        let written = write_lrc_sidecar(&audio, "[00:01.00]w4dj\n")
            .unwrap()
            .unwrap();
        assert_ne!(written, sidecar);
        assert_eq!(std::fs::read_to_string(sidecar).unwrap(), "user lyric\n");
        assert_eq!(std::fs::read_to_string(written).unwrap(), "[00:01.00]w4dj");
    }

    #[test]
    fn normalizes_original_translation_and_romanized_lyrics() {
        let normalized = normalize_lyrics(&[
            NeteaseLyricRecord {
                kind: "lrc".to_string(),
                text: "[00:01.00]hello\n[00:02.00]world".to_string(),
            },
            NeteaseLyricRecord {
                kind: "translated".to_string(),
                text: "你好\n世界".to_string(),
            },
            NeteaseLyricRecord {
                kind: "romanized".to_string(),
                text: "ni hao\nshi jie".to_string(),
            },
        ]);
        assert_eq!(normalized.plain, "hello\nworld");
        assert_eq!(normalized.translated, "你好\n世界");
        assert_eq!(normalized.romanized, "ni hao\nshi jie");
        assert_eq!(normalized.lrc, "[00:01.00]hello\n[00:02.00]world");
        assert!(normalized.warnings.is_empty());
    }
}
