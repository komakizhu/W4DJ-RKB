//! Deterministic matching between imported DJ playlist rows and W4DJ outputs.
//!
//! The matcher deliberately accepts only exact title/artist identity at the
//! automatic tier.  Filename hints and partial matches are suggestions only;
//! they never silently become playlist assignments.

use crate::dj_playlist::ImportedDjPlaylist;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DjOutputCandidate {
    pub track_key: String,
    pub title: String,
    pub artist_display: String,
    pub netease_track_id: Option<String>,
    pub duration_seconds: Option<f64>,
    pub destination_path: PathBuf,
    pub status: String,
    pub readable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DjPlaylistMatchCandidate {
    pub track_key: String,
    pub title: String,
    pub artist_display: String,
    pub duration_seconds: Option<f64>,
    pub destination_filename: String,
    pub score: i32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DjPlaylistMatchKind {
    NeteaseTrackId,
    UniqueTitleArtistFallback,
    Ambiguous,
    Unmatched,
    Missing,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DjPlaylistTrackMatch {
    pub position: u64,
    pub dedupe_key: String,
    pub title: String,
    pub artist_display: String,
    pub netease_track_id: Option<String>,
    pub kind: DjPlaylistMatchKind,
    pub status: String,
    pub track_key: Option<String>,
    pub match_method: Option<String>,
    pub score: Option<i32>,
    pub reason: String,
    pub candidates: Vec<DjPlaylistMatchCandidate>,
    pub manual: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DjPlaylistMatchReport {
    pub playlist_id: String,
    pub total: usize,
    pub matched_count: usize,
    pub ambiguous_count: usize,
    pub unmatched_count: usize,
    pub missing_count: usize,
    pub matches: Vec<DjPlaylistTrackMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IdentityKey {
    title: String,
    artists: Vec<String>,
}

/// Match the imported order against currently available output candidates.
/// A candidate is consumed only after an automatic exact match, so one output
/// cannot satisfy two imported rows in the same report.
pub fn match_imported_playlist(
    playlist: &ImportedDjPlaylist,
    candidates: &[DjOutputCandidate],
) -> DjPlaylistMatchReport {
    let available = candidates
        .iter()
        .filter(|candidate| candidate.status == "available" && candidate.readable)
        .collect::<Vec<_>>();
    let mut by_identity: HashMap<IdentityKey, Vec<&DjOutputCandidate>> = HashMap::new();
    let mut by_netease_id: HashMap<&str, Vec<&DjOutputCandidate>> = HashMap::new();
    for candidate in &available {
        by_identity
            .entry(identity_key(&candidate.title, &candidate.artist_display))
            .or_default()
            .push(candidate);
        if let Some(id) = candidate.netease_track_id.as_deref() {
            by_netease_id.entry(id).or_default().push(candidate);
        }
    }

    let mut used = HashSet::new();
    let mut matches = Vec::with_capacity(playlist.tracks.len());
    for track in &playlist.tracks {
        let key = identity_key(&track.title, &track.artist_display);
        let mut exact = track
            .netease_track_id
            .as_deref()
            .and_then(|id| by_netease_id.get(id))
            .into_iter()
            .flat_map(|items| items.iter().copied())
            .collect::<Vec<_>>();
        let match_method = if track.netease_track_id.is_some() {
            "neteaseTrackId"
        } else {
            "uniqueTitleArtistFallback"
        };
        if exact.is_empty() && track.netease_track_id.is_none() {
            exact = by_identity
                .get(&key)
                .into_iter()
                .flat_map(|items| items.iter().copied())
                .filter(|candidate| !used.contains(&candidate.track_key))
                .collect::<Vec<_>>();
        }
        exact.sort_by(|left, right| left.track_key.cmp(&right.track_key));

        let suggestions = suggestions_for(track, &available, &used);
        let row = if exact.len() == 1 {
            let candidate = exact[0];
            if track.netease_track_id.is_none() {
                used.insert(candidate.track_key.clone());
            }
            DjPlaylistTrackMatch {
                position: track.position,
                dedupe_key: track.dedupe_key.clone(),
                title: track.title.clone(),
                artist_display: track.artist_display.clone(),
                netease_track_id: track.netease_track_id.clone(),
                kind: if match_method == "neteaseTrackId" {
                    DjPlaylistMatchKind::NeteaseTrackId
                } else {
                    DjPlaylistMatchKind::UniqueTitleArtistFallback
                },
                status: "matched".to_string(),
                track_key: Some(candidate.track_key.clone()),
                match_method: Some(match_method.to_string()),
                score: Some(100),
                reason: if match_method == "neteaseTrackId" {
                    "网易云歌曲 ID 精确匹配".to_string()
                } else {
                    "标题和歌手唯一匹配".to_string()
                },
                candidates: vec![candidate_to_suggestion(
                    candidate,
                    100,
                    "标题和歌手完全匹配",
                )],
                manual: false,
            }
        } else if exact.len() > 1 {
            DjPlaylistTrackMatch {
                position: track.position,
                dedupe_key: track.dedupe_key.clone(),
                title: track.title.clone(),
                artist_display: track.artist_display.clone(),
                netease_track_id: track.netease_track_id.clone(),
                kind: DjPlaylistMatchKind::Ambiguous,
                status: "ambiguous".to_string(),
                track_key: None,
                match_method: Some(if track.netease_track_id.is_some() {
                    "neteaseTrackId".to_string()
                } else {
                    "uniqueTitleArtistFallback".to_string()
                }),
                score: Some(100),
                reason: "存在多个同等精确的可用输出，需要手动选择".to_string(),
                candidates: exact
                    .into_iter()
                    .map(|candidate| candidate_to_suggestion(candidate, 100, "同等精确候选"))
                    .collect(),
                manual: false,
            }
        } else {
            DjPlaylistTrackMatch {
                position: track.position,
                dedupe_key: track.dedupe_key.clone(),
                title: track.title.clone(),
                artist_display: track.artist_display.clone(),
                netease_track_id: track.netease_track_id.clone(),
                kind: DjPlaylistMatchKind::Unmatched,
                status: "unmatched".to_string(),
                track_key: None,
                match_method: None,
                score: None,
                reason: if track.netease_track_id.is_some() {
                    "网易云歌曲 ID 未找到当前可用输出".to_string()
                } else if suggestions.is_empty() {
                    "没有找到可用输出候选".to_string()
                } else {
                    "没有达到自动匹配条件，请从候选中手动选择".to_string()
                },
                candidates: suggestions,
                manual: false,
            }
        };
        matches.push(row);
    }
    build_report(&playlist.playlist_id, matches)
}

pub fn identity_key_for(title: &str, artist_display: &str) -> (String, Vec<String>) {
    let key = identity_key(title, artist_display);
    (key.title, key.artists)
}

fn build_report(playlist_id: &str, matches: Vec<DjPlaylistTrackMatch>) -> DjPlaylistMatchReport {
    let matched_count = matches.iter().filter(|row| row.status == "matched").count();
    let ambiguous_count = matches
        .iter()
        .filter(|row| row.status == "ambiguous")
        .count();
    let unmatched_count = matches
        .iter()
        .filter(|row| row.status == "unmatched")
        .count();
    let missing_count = matches.iter().filter(|row| row.status == "missing").count();
    DjPlaylistMatchReport {
        playlist_id: playlist_id.to_string(),
        total: matches.len(),
        matched_count,
        ambiguous_count,
        unmatched_count,
        missing_count,
        matches,
    }
}

fn identity_key(title: &str, artist_display: &str) -> IdentityKey {
    IdentityKey {
        title: normalize_identity_text(title),
        artists: normalize_artist_tokens(artist_display),
    }
}

fn normalize_artist_tokens(value: &str) -> Vec<String> {
    let normalized = normalize_identity_text(value)
        .replace(" featuring ", " ")
        .replace(" feat ", " ")
        .replace(" ft ", " ");
    let mut tokens = normalized
        .split_whitespace()
        .filter(|token| !matches!(*token, "feat" | "featuring" | "ft"))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

/// A dependency-free compatibility fold covering the NFKC forms encountered
/// in music metadata (full-width text, common ligatures, and case). Punctuation
/// is mapped to spaces only for comparison; stored display values are untouched.
pub fn normalize_identity_text(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for character in value.chars() {
        let character = match character {
            '\u{3000}' => ' ',
            '\u{ff01}'..='\u{ff5e}' => {
                char::from_u32(character as u32 - 0xfee0).unwrap_or(character)
            }
            '\u{fb00}' => 'f',
            '\u{fb01}' | '\u{fb02}' => 'i',
            '\u{fb03}' | '\u{fb04}' => 'f',
            '\u{fb05}' | '\u{fb06}' => 's',
            _ => character,
        };
        for lower in character.to_lowercase() {
            if lower.is_alphanumeric() {
                folded.push(lower);
            } else {
                folded.push(' ');
            }
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn suggestions_for(
    track: &crate::dj_playlist::ImportedDjPlaylistTrack,
    candidates: &[&DjOutputCandidate],
    used: &HashSet<String>,
) -> Vec<DjPlaylistMatchCandidate> {
    if track.netease_track_id.is_some() {
        return Vec::new();
    }
    let title = normalize_identity_text(&track.title);
    let artist_tokens = normalize_artist_tokens(&track.artist_display);
    let mut scored = candidates
        .iter()
        .filter(|candidate| {
            track.netease_track_id.is_some() || !used.contains(&candidate.track_key)
        })
        .filter_map(|candidate| {
            let candidate_title = normalize_identity_text(&candidate.title);
            let candidate_artists = normalize_artist_tokens(&candidate.artist_display);
            let title_match = candidate_title == title;
            let artist_overlap = artist_tokens
                .iter()
                .filter(|token| candidate_artists.contains(token))
                .count();
            if !title_match && artist_overlap == 0 {
                return None;
            }
            let score = (if title_match { 60 } else { 0 }) + (artist_overlap as i32 * 10);
            Some(candidate_to_suggestion(
                candidate,
                score,
                "低置信度候选，仅供手动复核",
            ))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.track_key.cmp(&right.track_key))
    });
    scored.truncate(8);
    scored
}

fn candidate_to_suggestion(
    candidate: &DjOutputCandidate,
    score: i32,
    reason: &str,
) -> DjPlaylistMatchCandidate {
    DjPlaylistMatchCandidate {
        track_key: candidate.track_key.clone(),
        title: candidate.title.clone(),
        artist_display: candidate.artist_display.clone(),
        duration_seconds: candidate.duration_seconds,
        destination_filename: candidate
            .destination_path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        score,
        reason: reason.to_string(),
    }
}

pub fn candidate_filename(candidate: &DjOutputCandidate) -> String {
    candidate
        .destination_path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| Path::new(&candidate.track_key).display().to_string())
}
