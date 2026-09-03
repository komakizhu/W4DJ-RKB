//! Deterministic, relative-path extended M3U8 export for imported DJ playlists.
//!
//! This module never mutates audio or playlist state.  It receives the
//! current output paths resolved from stable W4DJ track keys and renders an
//! explicit UTF-8 file only after the caller has selected a destination.

use crate::dj_playlist::ImportedDjPlaylist;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedDjPlaylistTrack {
    pub position: u64,
    pub title: String,
    pub artist_display: String,
    pub duration_seconds: Option<f64>,
    pub destination_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct M3u8OmittedTrack {
    pub position: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct M3u8ExportSummary {
    pub matched_count: usize,
    pub total: usize,
    pub omitted: Vec<M3u8OmittedTrack>,
}

#[derive(Debug)]
pub enum M3u8Error {
    InvalidPath(String),
    InvalidContent(String),
    Incomplete { omitted: Vec<M3u8OmittedTrack> },
    Empty,
    Io(io::Error),
}

impl std::fmt::Display for M3u8Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath(message) | Self::InvalidContent(message) => {
                formatter.write_str(message)
            }
            Self::Incomplete { omitted } => {
                write!(formatter, "歌单仍有 {} 首歌曲未能解析", omitted.len())
            }
            Self::Empty => formatter.write_str("不能导出空的 M3U8 歌单"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for M3u8Error {}

impl From<io::Error> for M3u8Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Render a complete extended M3U8.  A complete export rejects every
/// unresolved, missing, unreadable, or non-relative output instead of
/// silently omitting it.
pub fn build_relative_m3u8(
    playlist: &ImportedDjPlaylist,
    resolved_tracks: &[ResolvedDjPlaylistTrack],
    playlist_path: &Path,
) -> Result<String, M3u8Error> {
    build_relative_m3u8_with_summary(playlist, resolved_tracks, playlist_path)
        .map(|(contents, _)| contents)
}

/// Render an extended M3U8. Every playlist row must have a valid resolved
/// output; omitted positions are returned as an error and can never be
/// silently exported.
pub fn build_relative_m3u8_with_summary(
    playlist: &ImportedDjPlaylist,
    resolved_tracks: &[ResolvedDjPlaylistTrack],
    playlist_path: &Path,
) -> Result<(String, M3u8ExportSummary), M3u8Error> {
    if !playlist_path.is_absolute() {
        return Err(M3u8Error::InvalidPath(
            "M3U8 保存路径必须是绝对路径".to_string(),
        ));
    }
    let parent = playlist_path
        .parent()
        .ok_or_else(|| M3u8Error::InvalidPath("无法确定 M3U8 保存目录".to_string()))?;

    let mut by_position = std::collections::HashMap::new();
    for resolved in resolved_tracks {
        if by_position.insert(resolved.position, resolved).is_some() {
            return Err(M3u8Error::InvalidContent(
                "M3U8 解析结果包含重复歌单位置".to_string(),
            ));
        }
    }

    let mut lines = vec!["#EXTM3U".to_string()];
    let mut omitted = Vec::new();
    let mut matched_count = 0usize;
    let mut ordered_tracks = playlist.tracks.iter().collect::<Vec<_>>();
    ordered_tracks.sort_by_key(|track| track.position);
    for track in ordered_tracks {
        let Some(resolved) = by_position.get(&track.position).copied() else {
            omitted.push(M3u8OmittedTrack {
                position: track.position,
                reason: "没有匹配到 W4DJ 输出".to_string(),
            });
            continue;
        };

        let Some(reason) = invalid_output_reason(resolved) else {
            let relative = relative_path(parent, &resolved.destination_path).ok_or_else(|| {
                M3u8Error::InvalidPath(format!(
                    "输出路径无法相对于 M3U8 目录表示：{}",
                    resolved.destination_path.display()
                ))
            });
            let relative = match relative {
                Ok(path) => path,
                Err(error) => {
                    omitted.push(M3u8OmittedTrack {
                        position: track.position,
                        reason: error.to_string(),
                    });
                    continue;
                }
            };
            let duration = resolved
                .duration_seconds
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map(|value| value.round() as i64)
                .unwrap_or(-1);
            let display = extinf_display(&resolved.artist_display, &resolved.title);
            lines.push(format!("#EXTINF:{duration},{display}"));
            lines.push(relative);
            matched_count += 1;
            continue;
        };
        omitted.push(M3u8OmittedTrack {
            position: track.position,
            reason: reason.to_string(),
        });
    }

    if !omitted.is_empty() {
        return Err(M3u8Error::Incomplete { omitted });
    }
    if matched_count == 0 {
        return Err(M3u8Error::Empty);
    }

    let contents = lines.join("\n");
    let summary = M3u8ExportSummary {
        matched_count,
        total: playlist.tracks.len(),
        omitted,
    };
    Ok((contents, summary))
}

/// Atomically write an already-rendered M3U8 in the selected directory.
pub fn write_relative_m3u8_atomic(path: &Path, contents: &str) -> Result<(), M3u8Error> {
    if !path.is_absolute() {
        return Err(M3u8Error::InvalidPath(
            "M3U8 保存路径必须是绝对路径".to_string(),
        ));
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("m3u8"))
    {
        return Err(M3u8Error::InvalidPath(
            "M3U8 保存路径必须使用 .m3u8 扩展名".to_string(),
        ));
    }
    if contents.starts_with('\u{feff}') || contents.contains('\r') {
        return Err(M3u8Error::InvalidContent(
            "M3U8 必须是无 BOM、仅使用 LF 换行的 UTF-8 文本".to_string(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| M3u8Error::InvalidPath("无法确定 M3U8 保存目录".to_string()))?;
    if !parent.is_dir() {
        return Err(M3u8Error::InvalidPath("M3U8 保存目录不存在".to_string()));
    }

    let mut temporary = tempfile::Builder::new()
        .prefix(".w4dj-m3u8-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(contents.as_bytes())?;
    temporary.flush()?;
    #[cfg(unix)]
    {
        let mut permissions = temporary.as_file().metadata()?.permissions();
        permissions.set_mode(0o644);
        temporary.as_file().set_permissions(permissions)?;
    }
    temporary.as_file().sync_all()?;
    let temporary_path = temporary.into_temp_path();
    fs::rename(&temporary_path, path)?;
    temporary_path
        .keep()
        .map_err(|error| M3u8Error::Io(error.error))?;
    Ok(())
}

fn invalid_output_reason(track: &ResolvedDjPlaylistTrack) -> Option<&'static str> {
    if !track.destination_path.is_absolute() {
        return Some("输出路径不是绝对路径");
    }
    let Ok(metadata) = fs::metadata(&track.destination_path) else {
        return Some("输出文件不存在或不可读");
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return Some("输出文件不存在或为空");
    }
    if File::open(&track.destination_path).is_err() {
        return Some("输出文件不可读");
    }
    None
}

fn extinf_display(artist: &str, title: &str) -> String {
    let artist = sanitize_display(artist);
    let title = sanitize_display(title);
    if artist.is_empty() {
        title
    } else if title.is_empty() {
        artist
    } else {
        format!("{artist} - {title}")
    }
}

fn sanitize_display(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if matches!(character, '\r' | '\n') {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn relative_path(base: &Path, destination: &Path) -> Option<String> {
    let base = base.components().collect::<Vec<_>>();
    let destination = destination.components().collect::<Vec<_>>();
    let common = base
        .iter()
        .zip(&destination)
        .take_while(|(left, right)| left == right)
        .count();
    if base
        .first()
        .zip(destination.first())
        .is_some_and(|(left, right)| matches!((left, right), (Component::Prefix(a), Component::Prefix(b)) if a != b))
    {
        return None;
    }
    let mut parts = Vec::new();
    for component in &base[common..] {
        if !matches!(component, Component::CurDir) {
            parts.push("..".to_string());
        }
    }
    for component in &destination[common..] {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
            Component::ParentDir => parts.push("..".to_string()),
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
        }
    }
    if parts.is_empty() {
        return Some(".".to_string());
    }
    Some(parts.join("/"))
}
