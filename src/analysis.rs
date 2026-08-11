use crate::netease::recover_local_metadata;
use id3::TagLike;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use ncmdump::Ncmdump;

/// Music-analysis data produced by Essentia.js.
///
/// The structure is deliberately independent from the converter history. A
/// conversion can be deleted without losing the analysis data for the music
/// library, and a track can be re-analysed without creating a fake conversion
/// history entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrackAnalysis {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: Option<f64>,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub scale: Option<String>,
    pub key_strength: Option<f64>,
    pub integrated_loudness_lufs: Option<f64>,
    pub loudness_range_lu: Option<f64>,
    pub energy: Option<f64>,
    pub danceability: Option<f64>,
    pub beat_positions: Vec<f64>,
    pub analyzed_at: String,
    pub analyzer: String,
    pub analysis_version: String,
    #[serde(default)]
    pub source_size_bytes: Option<u64>,
    #[serde(default)]
    pub source_modified_at: Option<u64>,
    #[serde(default)]
    pub source_filename_format: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrackMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
}

pub fn read_track_metadata(path: &Path) -> TrackMetadata {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut metadata = match extension.as_str() {
        "flac" => metaflac::Tag::read_from_path(path)
            .ok()
            .and_then(|tag| {
                let comments = tag.vorbis_comments()?;
                Some(TrackMetadata {
                    title: first_metadata_value(comments.title()),
                    artist: comments
                        .artist()
                        .map(|values| values.join(", "))
                        .unwrap_or_default(),
                    album: first_metadata_value(comments.album()),
                })
            })
            .unwrap_or_default(),
        "mp3" | "wav" | "aiff" | "aif" => id3::Tag::read_from_path(path)
            .ok()
            .map(|tag| TrackMetadata {
                title: tag.title().unwrap_or_default().to_string(),
                artist: tag
                    .artist()
                    .or_else(|| tag.album_artist())
                    .unwrap_or_default()
                    .to_string(),
                album: tag.album().unwrap_or_default().to_string(),
            })
            .unwrap_or_default(),
        "ncm" => File::open(path)
            .ok()
            .and_then(|file| Ncmdump::from_reader(file).ok())
            .and_then(|mut ncm| ncm.get_info().ok())
            .map(|info| TrackMetadata {
                title: info.name.to_string(),
                artist: info
                    .artist
                    .iter()
                    .map(|item| item.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                album: info.album.to_string(),
            })
            .unwrap_or_default(),
        _ => TrackMetadata::default(),
    };

    // Plain MP3/FLAC downloads from NetEase may have no embedded tags even
    // though the desktop client still knows the track.  Merge only missing
    // values so user-authored tags remain authoritative.
    if !matches!(extension.as_str(), "ncm")
        && let Some(recovered) = recover_local_metadata(path)
    {
        if metadata.title.trim().is_empty() {
            metadata.title = recovered.title;
        }
        if metadata.artist.trim().is_empty() {
            metadata.artist = recovered.artist;
        }
        if metadata.album.trim().is_empty() {
            metadata.album = recovered.album;
        }
    }

    metadata
}

fn first_metadata_value(values: Option<&Vec<String>>) -> String {
    values
        .and_then(|values| values.first())
        .cloned()
        .unwrap_or_default()
}

impl TrackAnalysis {
    pub fn track_id(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.path.hash(&mut hasher);
        hasher.finish()
    }
}

pub fn load_analysis_file(path: &Path) -> Result<Vec<TrackAnalysis>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents =
        fs::read_to_string(path).map_err(|error| format!("读取音乐分析库失败：{error}"))?;
    serde_json::from_str(&contents).map_err(|error| format!("解析音乐分析库失败：{error}"))
}

pub fn clear_analysis_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清除音乐分析缓存失败：{error}")),
    }
}

pub fn save_analysis_file(path: &Path, entries: &[TrackAnalysis]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建音乐分析目录失败：{error}"))?;
    }

    let temp_path = path.with_extension("json.tmp");
    let contents = serde_json::to_string_pretty(entries)
        .map_err(|error| format!("生成音乐分析库失败：{error}"))?;
    fs::write(&temp_path, contents).map_err(|error| format!("写入音乐分析库失败：{error}"))?;
    fs::rename(&temp_path, path).map_err(|error| format!("保存音乐分析库失败：{error}"))
}

pub fn merge_analysis_entries(
    existing: Vec<TrackAnalysis>,
    updates: Vec<TrackAnalysis>,
) -> Vec<TrackAnalysis> {
    let mut merged = existing
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<std::collections::HashMap<_, _>>();

    for entry in updates {
        merged.insert(entry.path.clone(), entry);
    }

    let mut entries = merged.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

pub fn build_rekordbox_xml(entries: &[TrackAnalysis], product_version: &str) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DJ_PLAYLISTS Version=\"1.0.0\">\n",
    );
    let _ = writeln!(
        xml,
        "  <PRODUCT Name=\"W4DJ RKB\" Version=\"{}\" Company=\"W4DJ\"/>",
        escape_xml(product_version)
    );
    let _ = writeln!(xml, "  <COLLECTION Entries=\"{}\">", entries.len());

    for entry in entries {
        let track_id = entry.track_id();
        let name = if entry.title.trim().is_empty() {
            Path::new(&entry.path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("Untitled")
        } else {
            entry.title.trim()
        };
        let artist = entry.artist.trim();
        let album = entry.album.trim();
        let average_bpm = entry
            .bpm
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| format!("{value:.2}"))
            .unwrap_or_default();
        let total_time = entry
            .duration_seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.round().to_string())
            .unwrap_or_default();
        let tonality = tonality(entry.key.as_deref(), entry.scale.as_deref());
        let comments = analysis_comments(entry);
        let location = file_uri(Path::new(&entry.path));

        let _ = writeln!(
            xml,
            "    <TRACK TrackID=\"{}\" Name=\"{}\" Artist=\"{}\" Album=\"{}\" Genre=\"\" Kind=\"{}\" TotalTime=\"{}\" AverageBpm=\"{}\" Tonality=\"{}\" Comments=\"{}\" Location=\"{}\">",
            track_id,
            escape_xml(name),
            escape_xml(artist),
            escape_xml(album),
            escape_xml(&kind_for_path(&entry.path)),
            escape_xml(&total_time),
            escape_xml(&average_bpm),
            escape_xml(&tonality),
            escape_xml(&comments),
            escape_xml(&location),
        );

        for (beat_index, beat_position) in entry
            .beat_positions
            .iter()
            .copied()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .take(2000)
            .enumerate()
        {
            let bpm = entry.bpm.unwrap_or_default();
            if bpm.is_finite() && bpm > 0.0 {
                let _ = writeln!(
                    xml,
                    "      <TEMPO Inizio=\"{beat_position:.3}\" Bpm=\"{bpm:.3}\" Metro=\"4/4\" Battito=\"{}\"/>",
                    beat_index % 4 + 1
                );
            }
        }

        xml.push_str("    </TRACK>\n");
    }

    xml.push_str("  </COLLECTION>\n");
    let _ = writeln!(
        xml,
        "  <PLAYLISTS>\n    <NODE Type=\"0\" Name=\"ROOT\" Count=\"1\">\n      <NODE Type=\"1\" Name=\"W4DJ RKB Analysis\" Entries=\"{}\" KeyType=\"0\">",
        entries.len()
    );
    for entry in entries {
        let _ = writeln!(xml, "      <TRACK Key=\"{}\"/>", entry.track_id());
    }
    xml.push_str("      </NODE>\n    </NODE>\n  </PLAYLISTS>\n</DJ_PLAYLISTS>\n");
    xml
}

fn kind_for_path(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_uppercase())
        .unwrap_or_else(|| String::from("AUDIO"))
}

fn analysis_comments(entry: &TrackAnalysis) -> String {
    let mut values = Vec::new();
    if let Some(value) = entry
        .integrated_loudness_lufs
        .filter(|value| value.is_finite())
    {
        values.push(format!("Loudness {value:.1} LUFS"));
    }
    if let Some(value) = entry.energy.filter(|value| value.is_finite()) {
        values.push(format!("Energy {value:.3}"));
    }
    if let Some(value) = entry.danceability.filter(|value| value.is_finite()) {
        values.push(format!("Danceability {value:.3}"));
    }
    if let Some(value) = entry.key_strength.filter(|value| value.is_finite()) {
        values.push(format!("Key confidence {value:.3}"));
    }
    if values.is_empty() {
        String::from("W4DJ Essentia analysis")
    } else {
        format!("W4DJ Essentia | {}", values.join(" | "))
    }
}

fn tonality(key: Option<&str>, scale: Option<&str>) -> String {
    let Some(key) = key.map(str::trim).filter(|value| !value.is_empty()) else {
        return String::new();
    };
    let key = key.replace('♯', "#").replace('♭', "b");
    if scale
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("minor"))
    {
        format!("{key}m")
    } else {
        key
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn file_uri(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    let value = if value.starts_with('/') {
        value
    } else {
        format!("/{value}")
    };
    let mut uri = String::from("file://localhost");
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'.' | b'-' | b'_' | b'~' | b':')
        {
            uri.push(*byte as char);
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

pub fn analysis_file_path(history_path: &Path) -> PathBuf {
    history_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("track-analysis.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TrackAnalysis {
        TrackAnalysis {
            path: String::from("/Users/test/Mr & DJ - Song.mp3"),
            title: String::from("Song"),
            artist: String::from("Mr & DJ"),
            album: String::from("Album"),
            duration_seconds: Some(180.4),
            bpm: Some(140.25),
            key: Some(String::from("F#")),
            scale: Some(String::from("minor")),
            key_strength: Some(0.92),
            integrated_loudness_lufs: Some(-7.3),
            loudness_range_lu: Some(4.2),
            energy: Some(0.81),
            danceability: Some(0.76),
            beat_positions: vec![0.0, 0.428],
            analyzed_at: String::from("2026-07-27T00:00:00Z"),
            analyzer: String::from("Essentia.js"),
            analysis_version: String::from("0.1.5"),
            source_size_bytes: Some(1024),
            source_modified_at: Some(1_754_000_000_000),
            source_filename_format: Some(String::from("title_artist")),
        }
    }

    #[test]
    fn rekordbox_xml_contains_native_and_visible_analysis_fields() {
        let xml = build_rekordbox_xml(&[sample()], "3.0.0");

        assert!(xml.contains("AverageBpm=\"140.25\""));
        assert!(xml.contains("Tonality=\"F#m\""));
        assert!(xml.contains("Loudness -7.3 LUFS"));
        assert!(xml.contains("Energy 0.810"));
        assert!(xml.contains("<TEMPO Inizio=\"0.000\""));
        assert!(xml.contains("Mr &amp; DJ"));
        assert!(xml.contains("Mr%20%26%20DJ%20-%20Song.mp3"));
    }

    #[test]
    fn rekordbox_xml_uses_root_and_nested_playlist_nodes() {
        let xml = build_rekordbox_xml(&[sample()], "3.0.0");

        assert!(xml.contains("<NODE Type=\"0\" Name=\"ROOT\" Count=\"1\">"));
        assert!(
            xml.contains(
                "<NODE Type=\"1\" Name=\"W4DJ RKB Analysis\" Entries=\"1\" KeyType=\"0\">"
            )
        );
        assert!(!xml.contains("<PLAYLISTS>\n    <NODE Type=\"0\" Name=\"W4DJ"));
    }

    #[test]
    fn rekordbox_xml_numbers_beats_within_each_bar() {
        let mut entry = sample();
        entry.beat_positions = vec![0.0, 0.428, 0.856, 1.284, 1.712];
        let xml = build_rekordbox_xml(&[entry], "3.0.0");

        assert!(xml.contains("Inizio=\"0.000\" Bpm=\"140.250\" Metro=\"4/4\" Battito=\"1\""));
        assert!(xml.contains("Inizio=\"0.428\" Bpm=\"140.250\" Metro=\"4/4\" Battito=\"2\""));
        assert!(xml.contains("Inizio=\"1.712\" Bpm=\"140.250\" Metro=\"4/4\" Battito=\"1\""));
    }

    #[test]
    fn rekordbox_xml_uses_a_valid_windows_file_uri() {
        let mut entry = sample();
        entry.path = String::from("C:\\Music\\Mr & DJ - Song.mp3");
        let xml = build_rekordbox_xml(&[entry], "3.0.0");

        assert!(
            xml.contains("Location=\"file://localhost/C:/Music/Mr%20%26%20DJ%20-%20Song.mp3\"")
        );
    }

    #[test]
    fn merge_replaces_same_path_without_duplicate_entries() {
        let original = sample();
        let mut updated = sample();
        updated.bpm = Some(141.0);

        let merged = merge_analysis_entries(vec![original], vec![updated]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].bpm, Some(141.0));
    }

    #[test]
    fn reads_existing_id3_metadata_before_filename_fallback() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("Title - Artist.mp3");
        std::fs::File::create(&path).expect("test audio file should be created");
        let mut tag = id3::Tag::new();
        tag.set_title("Tagged title");
        tag.set_artist("Tagged artist");
        tag.set_album("Tagged album");
        tag.write_to_path(&path, id3::Version::Id3v24)
            .expect("test ID3 tag should be written");

        assert_eq!(
            read_track_metadata(&path),
            TrackMetadata {
                title: String::from("Tagged title"),
                artist: String::from("Tagged artist"),
                album: String::from("Tagged album"),
            }
        );
    }
}
