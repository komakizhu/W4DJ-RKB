//! Strict parsing and normalization for the W4DJ playlist interchange format.
//!
//! `.w4dj` is deliberately a small protocol boundary.  Version 2 is the only
//! accepted wire format; older files and fields are rejected instead of being
//! silently migrated.  All derived values (such as `dedupe_key` and the
//! NetEase import line) exist only in memory and in the local application
//! database.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const W4DJ_PLAYLIST_FORMAT: &str = "w4dj";
pub const W4DJ_PLAYLIST_FORMAT_VERSION: u32 = 2;
pub const W4DJ_PLAYLIST_MAX_BYTES: usize = 10 * 1024 * 1024;
pub const W4DJ_PLAYLIST_MAX_TRACKS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DjPlaylistImportWarning {
    pub code: String,
    pub message: String,
    pub position: u64,
    pub dedupe_key: String,
}

/// A normalized track row used by the local W4DJ workflow.
///
/// NetEase IDs intentionally do not exist on this boundary.  They are only
/// conversion-retrieval data and must never influence playlist matching or
/// export. `dedupe_key` and `netease_import_line` are derived application
/// fields; they are never accepted from or emitted to `.w4dj` files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedDjPlaylistTrack {
    pub position: u64,
    pub title: String,
    pub artist_display: String,
    pub dedupe_key: String,
    pub netease_import_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedDjPlaylist {
    pub playlist_id: String,
    pub format_version: u32,
    pub name: String,
    pub source_path: Option<PathBuf>,
    pub imported_at_ms: Option<i64>,
    pub tracks: Vec<ImportedDjPlaylistTrack>,
    pub warnings: Vec<DjPlaylistImportWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedDjPlaylistSummary {
    pub playlist_id: String,
    pub name: String,
    pub track_count: usize,
    pub warning_count: usize,
    pub imported_at_ms: i64,
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DjPlaylistError {
    TooLarge { actual: usize, maximum: usize },
    InvalidJson(String),
    InvalidField(String),
    UnsupportedFormat(String),
    UnsupportedVersion(u32),
}

impl std::fmt::Display for DjPlaylistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { actual, maximum } => {
                write!(f, "DJ 歌单文件过大：{actual} 字节，最大允许 {maximum} 字节")
            }
            Self::InvalidJson(message) => write!(f, "DJ 歌单 JSON 无效：{message}"),
            Self::InvalidField(message) => write!(f, "DJ 歌单字段无效：{message}"),
            Self::UnsupportedFormat(format) => write!(f, "不支持的 DJ 歌单格式：{format}"),
            Self::UnsupportedVersion(version) => {
                write!(
                    f,
                    "不支持的 DJ 歌单版本：{version}；请重新导出全新的 v2 文件"
                )
            }
        }
    }
}

impl std::error::Error for DjPlaylistError {}

/// The wire structs intentionally deny unknown fields.  This makes v1 fields
/// and future fields fail closed until a new protocol version is designed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePlaylist {
    format: String,
    #[serde(rename = "format_version")]
    _format_version: u32,
    export_id: String,
    playlist: WirePlaylistInfo,
    tracks: Vec<WireTrack>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePlaylistInfo {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTrack {
    position: u64,
    title: String,
    artist_display: String,
    /// Kept only as a v2 wire compatibility slot. It is deliberately ignored
    /// after parsing, whether it is null, missing, or a string from an older
    /// exporter. Numeric IDs remain invalid so malformed data is not silently
    /// accepted as a protocol value.
    #[serde(default)]
    #[serde(rename = "netease_track_id")]
    _netease_track_id: Option<String>,
}

pub fn parse_w4dj_playlist(
    bytes: &[u8],
    source_path: Option<&Path>,
) -> Result<ImportedDjPlaylist, DjPlaylistError> {
    if bytes.len() > W4DJ_PLAYLIST_MAX_BYTES {
        return Err(DjPlaylistError::TooLarge {
            actual: bytes.len(),
            maximum: W4DJ_PLAYLIST_MAX_BYTES,
        });
    }

    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| DjPlaylistError::InvalidJson(error.to_string()))?;
    let version = value
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| DjPlaylistError::InvalidField("缺少或无效 format_version".to_string()))?;
    if version > u32::MAX as u64 {
        return Err(DjPlaylistError::InvalidField(
            "format_version 超出支持范围".to_string(),
        ));
    }
    if version as u32 != W4DJ_PLAYLIST_FORMAT_VERSION {
        return Err(DjPlaylistError::UnsupportedVersion(version as u32));
    }
    reject_non_string_netease_ids(&value)?;

    let wire: WirePlaylist = serde_json::from_value(value)
        .map_err(|error| DjPlaylistError::InvalidJson(error.to_string()))?;
    if wire.format != W4DJ_PLAYLIST_FORMAT {
        return Err(DjPlaylistError::UnsupportedFormat(wire.format));
    }
    let playlist_id = required_text(wire.export_id, "export_id")?;
    let name = required_text(wire.playlist.name, "playlist.name")?;
    if wire.tracks.len() > W4DJ_PLAYLIST_MAX_TRACKS {
        return Err(DjPlaylistError::InvalidField(format!(
            "tracks 超过 {} 条",
            W4DJ_PLAYLIST_MAX_TRACKS
        )));
    }

    let mut normalized = Vec::with_capacity(wire.tracks.len());
    let mut positions = HashSet::new();
    for track in wire.tracks {
        let position = track.position;
        if position == 0 || !positions.insert(position) {
            return Err(DjPlaylistError::InvalidField(format!(
                "track.position 无效或重复：{position}"
            )));
        }
        let title = normalize_line_text(&track.title, &format!("track[{position}].title"))?;
        let artist_display = normalize_line_text(
            &track.artist_display,
            &format!("track[{position}].artist_display"),
        )?;
        normalized.push(ImportedDjPlaylistTrack {
            position,
            title: title.clone(),
            artist_display: artist_display.clone(),
            dedupe_key: dedupe_key(&title, &artist_display),
            netease_import_line: netease_import_line(&title, &artist_display)?,
        });
    }
    normalized.sort_by_key(|track| track.position);

    Ok(ImportedDjPlaylist {
        playlist_id,
        format_version: W4DJ_PLAYLIST_FORMAT_VERSION,
        name,
        source_path: source_path.map(Path::to_path_buf),
        imported_at_ms: None,
        tracks: normalized,
        warnings: Vec::new(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
struct MinimalW4djExport {
    format: &'static str,
    format_version: u32,
    export_id: String,
    playlist: MinimalW4djPlaylist,
    tracks: Vec<MinimalW4djTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MinimalW4djPlaylist {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MinimalW4djTrack {
    position: u64,
    title: String,
    artist_display: String,
    /// The v2 handoff keeps the key explicit and empty. It documents that an
    /// ID was intentionally not supplied and prevents consumers from treating
    /// a missing value as an invitation to look it up during export.
    netease_track_id: Option<String>,
}

pub fn serialize_w4dj_playlist(playlist: &ImportedDjPlaylist) -> Result<Vec<u8>, DjPlaylistError> {
    if playlist.playlist_id.trim().is_empty() || playlist.name.trim().is_empty() {
        return Err(DjPlaylistError::InvalidField(
            "export_id 和 playlist.name 不能为空".to_string(),
        ));
    }
    if playlist.format_version != W4DJ_PLAYLIST_FORMAT_VERSION {
        return Err(DjPlaylistError::UnsupportedVersion(playlist.format_version));
    }
    let export = MinimalW4djExport {
        format: W4DJ_PLAYLIST_FORMAT,
        format_version: W4DJ_PLAYLIST_FORMAT_VERSION,
        export_id: playlist.playlist_id.clone(),
        playlist: MinimalW4djPlaylist {
            name: playlist.name.clone(),
        },
        tracks: playlist
            .tracks
            .iter()
            .map(|track| MinimalW4djTrack {
                position: track.position,
                title: track.title.clone(),
                artist_display: track.artist_display.clone(),
                netease_track_id: None,
            })
            .collect(),
    };
    serde_json::to_vec_pretty(&export)
        .map_err(|error| DjPlaylistError::InvalidJson(error.to_string()))
}

pub fn netease_import_line(title: &str, artist_display: &str) -> Result<String, DjPlaylistError> {
    let title = normalize_line_text(title, "title")?;
    let artist_display = normalize_line_text(artist_display, "artist_display")?;
    Ok(format!("{title} - {artist_display}"))
}

fn required_text(value: String, field: &str) -> Result<String, DjPlaylistError> {
    if value.trim().is_empty() {
        return Err(DjPlaylistError::InvalidField(format!("{field} 为空")));
    }
    Ok(value)
}

fn reject_non_string_netease_ids(value: &Value) -> Result<(), DjPlaylistError> {
    let Some(tracks) = value.get("tracks").and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, track) in tracks.iter().enumerate() {
        let Some(id) = track
            .as_object()
            .and_then(|track| track.get("netease_track_id"))
        else {
            continue;
        };
        if !id.is_null() && !id.is_string() {
            return Err(DjPlaylistError::InvalidField(format!(
                "track[{index}].netease_track_id 必须是字符串"
            )));
        }
    }
    Ok(())
}

fn dedupe_key(title: &str, artist_display: &str) -> String {
    format!(
        "title-artist:{}:{}",
        normalize_dedupe_component(title),
        normalize_dedupe_component(artist_display)
    )
}

fn normalize_dedupe_component(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn normalize_line_text(value: &str, field: &str) -> Result<String, DjPlaylistError> {
    let mut normalized = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\r' | '\n' | '\t') {
            normalized.push(' ');
        } else if character.is_control() {
            return Err(DjPlaylistError::InvalidField(format!(
                "{field} 包含不允许的控制字符"
            )));
        } else {
            normalized.push(character);
        }
    }
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Err(DjPlaylistError::InvalidField(format!("{field} 为空")));
    }
    Ok(normalized)
}
