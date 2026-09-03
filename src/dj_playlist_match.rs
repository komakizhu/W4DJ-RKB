//! Deterministic matching between imported DJ playlist rows and W4DJ outputs.
//!
//! NetEase identifiers deliberately do not participate here. They belong to
//! the conversion-retrieval boundary only. Once a W4DJ playlist enters the
//! application, the only identity evidence is the title and artist text that
//! the user supplied, scored with a small dependency-free BM25F equivalent.

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
    pub duration_seconds: Option<f64>,
    pub destination_path: PathBuf,
    pub status: String,
    pub readable: bool,
    /// The successful conversion batch that produced this output. This is
    /// provenance, not a song identity and is never written to `.w4dj`.
    #[serde(default)]
    pub conversion_batch_id: Option<String>,
    #[serde(default)]
    pub committed_at_ms: Option<i64>,
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
    RecentBm25f,
    LibraryBm25f,
    Unmatched,
    Missing,
    Manual,
    /// Kept only so old cached reports can be deserialized by a newer build.
    /// New reports never emit this value.
    #[serde(alias = "ambiguous")]
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DjPlaylistTrackMatch {
    pub position: u64,
    pub dedupe_key: String,
    pub title: String,
    pub artist_display: String,
    pub kind: DjPlaylistMatchKind,
    pub status: String,
    pub track_key: Option<String>,
    pub match_method: Option<String>,
    pub score: Option<i32>,
    pub reason: String,
    pub candidates: Vec<DjPlaylistMatchCandidate>,
    pub manual: bool,
    /// The selected local output. Keeping the path in the report makes the
    /// review screen and exporter independent of a second metadata lookup.
    #[serde(default)]
    pub destination_path: Option<PathBuf>,
    /// `recent`, `library`, or `manual`; this is displayed as provenance only.
    #[serde(default)]
    pub candidate_source: Option<String>,
    /// Kept for compatibility with older reports. A matched row is now
    /// accepted by default; the review UI has no per-row confirmation gate.
    #[serde(default)]
    pub confirmed: bool,
    /// Excluded from this playlist's export list. This never deletes the
    /// W4DJ library record or the local audio file.
    #[serde(default)]
    pub excluded: bool,
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

#[derive(Debug, Clone, Default)]
struct FieldStats {
    document_count: usize,
    document_frequency: HashMap<String, usize>,
    average_length: f64,
}

impl FieldStats {
    fn from_documents(documents: impl IntoIterator<Item = Vec<String>>) -> Self {
        let mut document_count = 0usize;
        let mut total_length = 0usize;
        let mut document_frequency = HashMap::new();
        for tokens in documents {
            document_count += 1;
            total_length += tokens.len();
            let unique = tokens.into_iter().collect::<HashSet<_>>();
            for token in unique {
                *document_frequency.entry(token).or_insert(0) += 1;
            }
        }
        Self {
            document_count,
            document_frequency,
            average_length: if document_count == 0 {
                1.0
            } else {
                (total_length as f64 / document_count as f64).max(1.0)
            },
        }
    }

    fn idf(&self, token: &str) -> f64 {
        let document_count = self.document_count.max(1) as f64;
        let document_frequency = self.document_frequency.get(token).copied().unwrap_or(0) as f64;
        ((document_count - document_frequency + 0.5) / (document_frequency + 0.5) + 1.0).ln()
    }
}

#[derive(Debug, Clone, Copy)]
struct MatchScore {
    total: i32,
    title: i32,
    artist: i32,
}

#[derive(Debug, Clone)]
struct Edge<'a> {
    row_index: usize,
    candidate: &'a DjOutputCandidate,
    score: MatchScore,
}

/// Match using the normal library-only policy: no recent batch is preferred,
/// so only title/artist candidates scoring at least 50 are automatically
/// bound. This keeps the old pure function useful for callers and tests while
/// the W4DJ workflow uses [`match_imported_playlist_with_priority`].
pub fn match_imported_playlist(
    playlist: &ImportedDjPlaylist,
    candidates: &[DjOutputCandidate],
) -> DjPlaylistMatchReport {
    match_imported_playlist_with_priority(playlist, &[], candidates)
}

/// Match a playlist against a recent successful conversion batch and the full
/// W4DJ output library.
///
/// The recent batch is consumed one-to-one first. Its candidates are retained
/// even below the 50-point library threshold because the user explicitly
/// converted them for this playlist. If it contains fewer rows than the
/// playlist, the remaining rows are filled from the historical library at a
/// score of at least 50. The global edge ordering is deterministic, so a
/// collision cannot change the result between runs.
pub fn match_imported_playlist_with_priority(
    playlist: &ImportedDjPlaylist,
    recent_candidates: &[DjOutputCandidate],
    library_candidates: &[DjOutputCandidate],
) -> DjPlaylistMatchReport {
    let candidates = unique_candidates(recent_candidates, library_candidates);
    let recent_keys = recent_candidates
        .iter()
        .map(|candidate| candidate.track_key.as_str())
        .collect::<HashSet<_>>();
    let stats = corpus_stats(&candidates);

    let mut rows = playlist
        .tracks
        .iter()
        .map(|track| empty_match(track, &candidates, &stats))
        .collect::<Vec<_>>();

    let mut recent_edges = Vec::new();
    for (row_index, track) in playlist.tracks.iter().enumerate() {
        for candidate in &candidates {
            if recent_keys.contains(candidate.track_key.as_str()) {
                recent_edges.push(Edge {
                    row_index,
                    candidate,
                    score: score_track(
                        track.title.as_str(),
                        track.artist_display.as_str(),
                        candidate,
                        &stats,
                    ),
                });
            }
        }
    }
    sort_edges(&mut recent_edges);

    let desired_recent = recent_keys.len().min(playlist.tracks.len());
    let mut used_rows = HashSet::new();
    let mut used_candidates = HashSet::new();
    let mut assigned_candidate_identities = HashMap::<String, (String, Vec<String>)>::new();
    let mut recent_assigned = 0usize;
    let mut recent_reuse_edges = Vec::new();
    for edge in &recent_edges {
        if used_rows.contains(&edge.row_index) {
            continue;
        }
        let row_identity = identity_key(
            &playlist.tracks[edge.row_index].title,
            &playlist.tracks[edge.row_index].artist_display,
        );
        if used_candidates.contains(&edge.candidate.track_key) {
            if assigned_candidate_identities.get(&edge.candidate.track_key) == Some(&row_identity) {
                recent_reuse_edges.push(edge);
            }
            continue;
        }
        if recent_assigned >= desired_recent {
            continue;
        }
        used_candidates.insert(edge.candidate.track_key.clone());
        assigned_candidate_identities.insert(edge.candidate.track_key.clone(), row_identity);
        assign_row(
            &mut rows[edge.row_index],
            edge.candidate,
            edge.score,
            "recent",
            "recentBm25f",
            "最近转换批次候选，按标题/歌手 BM25F 优先绑定",
        );
        used_rows.insert(edge.row_index);
        recent_assigned += 1;
    }
    for edge in recent_reuse_edges {
        if used_rows.contains(&edge.row_index) {
            continue;
        }
        let row_identity = identity_key(
            &playlist.tracks[edge.row_index].title,
            &playlist.tracks[edge.row_index].artist_display,
        );
        if assigned_candidate_identities.get(&edge.candidate.track_key) != Some(&row_identity) {
            continue;
        }
        assign_row(
            &mut rows[edge.row_index],
            edge.candidate,
            edge.score,
            "recent",
            "recentBm25f",
            "最近转换批次候选，按标题/歌手 BM25F 优先绑定",
        );
        used_rows.insert(edge.row_index);
    }

    let mut library_edges = Vec::new();
    for (row_index, track) in playlist.tracks.iter().enumerate() {
        if used_rows.contains(&row_index) {
            continue;
        }
        for candidate in &candidates {
            if used_candidates.contains(&candidate.track_key)
                || recent_keys.contains(candidate.track_key.as_str())
            {
                continue;
            }
            let score = score_track(
                track.title.as_str(),
                track.artist_display.as_str(),
                candidate,
                &stats,
            );
            // A title-only overlap is too easy to get wrong (especially for
            // short dance titles). The historical-library fallback therefore
            // needs at least one artist token in common; the recent batch is
            // intentionally exempt because it represents the user's explicit
            // pre-import conversion set.
            if score.total >= 50 && score.artist > 0 {
                library_edges.push(Edge {
                    row_index,
                    candidate,
                    score,
                });
            }
        }
    }
    sort_edges(&mut library_edges);
    let mut library_reuse_edges = Vec::new();
    for edge in &library_edges {
        if used_rows.contains(&edge.row_index) {
            continue;
        }
        let row_identity = identity_key(
            &playlist.tracks[edge.row_index].title,
            &playlist.tracks[edge.row_index].artist_display,
        );
        if used_candidates.contains(&edge.candidate.track_key) {
            if assigned_candidate_identities.get(&edge.candidate.track_key) == Some(&row_identity) {
                library_reuse_edges.push(edge);
            }
            continue;
        }
        used_candidates.insert(edge.candidate.track_key.clone());
        assigned_candidate_identities.insert(edge.candidate.track_key.clone(), row_identity);
        assign_row(
            &mut rows[edge.row_index],
            edge.candidate,
            edge.score,
            "library",
            "libraryBm25f",
            "W4DJ 曲库候选，标题/歌手 BM25F 置信度达到 50%",
        );
        used_rows.insert(edge.row_index);
    }
    for edge in library_reuse_edges {
        if used_rows.contains(&edge.row_index) {
            continue;
        }
        let row_identity = identity_key(
            &playlist.tracks[edge.row_index].title,
            &playlist.tracks[edge.row_index].artist_display,
        );
        if assigned_candidate_identities.get(&edge.candidate.track_key) != Some(&row_identity) {
            continue;
        }
        assign_row(
            &mut rows[edge.row_index],
            edge.candidate,
            edge.score,
            "library",
            "libraryBm25f",
            "W4DJ 曲库候选，标题/歌手 BM25F 置信度达到 50%",
        );
        used_rows.insert(edge.row_index);
    }

    build_report(&playlist.playlist_id, rows)
}

fn unique_candidates<'a>(
    recent_candidates: &'a [DjOutputCandidate],
    library_candidates: &'a [DjOutputCandidate],
) -> Vec<&'a DjOutputCandidate> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for candidate in recent_candidates.iter().chain(library_candidates.iter()) {
        if seen.insert(candidate.track_key.as_str()) {
            result.push(candidate);
        }
    }
    result
}

fn corpus_stats(candidates: &[&DjOutputCandidate]) -> (FieldStats, FieldStats) {
    (
        FieldStats::from_documents(
            candidates
                .iter()
                .map(|candidate| title_tokens(&candidate.title)),
        ),
        FieldStats::from_documents(
            candidates
                .iter()
                .map(|candidate| artist_tokens(&candidate.artist_display)),
        ),
    )
}

fn empty_match(
    track: &crate::dj_playlist::ImportedDjPlaylistTrack,
    candidates: &[&DjOutputCandidate],
    stats: &(FieldStats, FieldStats),
) -> DjPlaylistTrackMatch {
    let suggestions = suggestions_for(track, candidates, stats);
    DjPlaylistTrackMatch {
        position: track.position,
        dedupe_key: track.dedupe_key.clone(),
        title: track.title.clone(),
        artist_display: track.artist_display.clone(),
        kind: DjPlaylistMatchKind::Unmatched,
        status: "unmatched".to_string(),
        track_key: None,
        match_method: None,
        score: None,
        reason: if suggestions.is_empty() {
            "没有找到标题或歌手有重叠的输出候选".to_string()
        } else {
            "候选低于曲库自动绑定阈值，请从右侧选择本地歌曲".to_string()
        },
        candidates: suggestions,
        manual: false,
        destination_path: None,
        candidate_source: None,
        confirmed: false,
        excluded: false,
    }
}

fn assign_row(
    row: &mut DjPlaylistTrackMatch,
    candidate: &DjOutputCandidate,
    score: MatchScore,
    source: &str,
    method: &str,
    reason: &str,
) {
    row.kind = if source == "recent" {
        DjPlaylistMatchKind::RecentBm25f
    } else {
        DjPlaylistMatchKind::LibraryBm25f
    };
    row.status = "matched".to_string();
    row.track_key = Some(candidate.track_key.clone());
    row.match_method = Some(method.to_string());
    row.score = Some(score.total);
    row.reason = reason.to_string();
    row.manual = false;
    row.destination_path = Some(candidate.destination_path.clone());
    row.candidate_source = Some(source.to_string());
    row.confirmed = true;
    row.excluded = false;
    if !row
        .candidates
        .iter()
        .any(|item| item.track_key == candidate.track_key)
    {
        row.candidates
            .push(candidate_to_suggestion(candidate, score.total, reason));
    }
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

fn sort_edges(edges: &mut [Edge<'_>]) {
    edges.sort_by(|left, right| {
        right
            .score
            .total
            .cmp(&left.score.total)
            .then_with(|| right.score.title.cmp(&left.score.title))
            .then_with(|| right.score.artist.cmp(&left.score.artist))
            .then_with(|| left.row_index.cmp(&right.row_index))
            .then_with(|| left.candidate.track_key.cmp(&right.candidate.track_key))
    });
}

fn score_track(
    query_title: &str,
    query_artist: &str,
    candidate: &DjOutputCandidate,
    stats: &(FieldStats, FieldStats),
) -> MatchScore {
    let title = normalized_field_score(
        &title_tokens(query_title),
        &title_tokens(&candidate.title),
        &stats.0,
    );
    let artist = normalized_field_score(
        &artist_tokens(query_artist),
        &artist_tokens(&candidate.artist_display),
        &stats.1,
    );
    let total = if title == 0 && artist == 0 {
        0
    } else {
        ((title as f64 * 0.65) + (artist as f64 * 0.35)).round() as i32
    };
    MatchScore {
        total: total.clamp(0, 100),
        title,
        artist,
    }
}

/// Return the BM25F-style title/artist score used by the matcher. The corpus
/// controls IDF, so the same pair has a stable absolute score regardless of
/// which other rows happen to be displayed in the review UI. The score is
/// intentionally a soft confidence value, not a proof of song identity.
pub fn bm25f_track_score(
    query_title: &str,
    query_artist: &str,
    candidate_title: &str,
    candidate_artist: &str,
    corpus: &[DjOutputCandidate],
) -> u8 {
    let candidate = DjOutputCandidate {
        track_key: "score-only".to_string(),
        title: candidate_title.to_string(),
        artist_display: candidate_artist.to_string(),
        duration_seconds: None,
        destination_path: PathBuf::new(),
        status: "available".to_string(),
        readable: true,
        conversion_batch_id: None,
        committed_at_ms: None,
    };
    let mut corpus_candidates = corpus.iter().collect::<Vec<_>>();
    if corpus_candidates.is_empty() {
        corpus_candidates.push(&candidate);
    }
    let stats = corpus_stats(&corpus_candidates);
    score_track(query_title, query_artist, &candidate, &stats)
        .total
        .clamp(0, 100) as u8
}

/// Compatibility helper for callers that only need to know whether a title /
/// artist pair is plausible. Unlike the old exact gate, it intentionally uses
/// the same soft BM25F score as export review.
pub fn title_artist_match_score(
    left_title: &str,
    left_artists: &str,
    right_title: &str,
    right_artists: &str,
) -> Option<i32> {
    let candidate = DjOutputCandidate {
        track_key: "score-only".to_string(),
        title: right_title.to_string(),
        artist_display: right_artists.to_string(),
        duration_seconds: None,
        destination_path: PathBuf::new(),
        status: "available".to_string(),
        readable: true,
        conversion_batch_id: None,
        committed_at_ms: None,
    };
    let score = bm25f_track_score(
        left_title,
        left_artists,
        right_title,
        right_artists,
        std::slice::from_ref(&candidate),
    ) as i32;
    (score >= 50).then_some(score)
}

fn normalized_field_score(query: &[String], document: &[String], stats: &FieldStats) -> i32 {
    if query.is_empty() || document.is_empty() {
        return 0;
    }
    let query_score = bm25_raw(query, document, stats);
    let ideal_score = bm25_raw(query, query, stats);
    if ideal_score <= f64::EPSILON {
        return 0;
    }
    ((query_score / ideal_score) * 100.0)
        .round()
        .clamp(0.0, 100.0) as i32
}

fn bm25_raw(query: &[String], document: &[String], stats: &FieldStats) -> f64 {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let length = document.len() as f64;
    let average_length = stats.average_length.max(1.0);
    let mut frequencies = HashMap::<&str, usize>::new();
    for token in document {
        *frequencies.entry(token.as_str()).or_insert(0) += 1;
    }
    query.iter().fold(0.0, |total, token| {
        let frequency = frequencies.get(token.as_str()).copied().unwrap_or(0) as f64;
        if frequency == 0.0 {
            return total;
        }
        let idf = stats.idf(token);
        let saturation =
            frequency * (K1 + 1.0) / (frequency + K1 * (1.0 - B + B * length / average_length));
        total + idf * saturation
    })
}

fn suggestions_for(
    track: &crate::dj_playlist::ImportedDjPlaylistTrack,
    candidates: &[&DjOutputCandidate],
    stats: &(FieldStats, FieldStats),
) -> Vec<DjPlaylistMatchCandidate> {
    let mut scored = candidates
        .iter()
        .map(|candidate| {
            let score = score_track(&track.title, &track.artist_display, candidate, stats).total;
            let reason = if score >= 80 {
                "标题/歌手高度重叠"
            } else if score >= 50 {
                "标题/歌手部分重叠"
            } else {
                "低置信度候选，仅供手动复核"
            };
            candidate_to_suggestion(candidate, score, reason)
        })
        .filter(|candidate| candidate.score > 0)
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
        destination_filename: candidate_filename(candidate),
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

fn title_tokens(value: &str) -> Vec<String> {
    expand_match_tokens(
        normalize_title_for_match(value)
            .split_whitespace()
            .map(str::to_string)
            .collect(),
    )
}

fn artist_tokens(value: &str) -> Vec<String> {
    expand_match_tokens(
        normalize_artist_tokens(value)
            .into_iter()
            .flat_map(|entity| {
                entity
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect(),
    )
}

/// Keep full-word evidence while adding character trigrams for long words.
/// The prefixed namespaces stop a short word such as `mix` from accidentally
/// counting as a trigram match for an unrelated long token.
fn expand_match_tokens(tokens: Vec<String>) -> Vec<String> {
    let mut expanded = Vec::new();
    for token in tokens {
        expanded.push(format!("word:{token}"));
        let characters = token.chars().collect::<Vec<_>>();
        if characters.len() >= 4 {
            for window in characters.windows(3) {
                expanded.push(format!("tri:{}{}{}", window[0], window[1], window[2]));
            }
        }
    }
    expanded
}

fn identity_key(title: &str, artist_display: &str) -> (String, Vec<String>) {
    (
        normalize_title_for_match(title),
        normalize_artist_tokens(artist_display),
    )
}

pub fn identity_key_for(title: &str, artist_display: &str) -> (String, Vec<String>) {
    identity_key(title, artist_display)
}

fn normalize_artist_tokens(value: &str) -> Vec<String> {
    let mut separated = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            ',' | '，' | '、' | ';' | '；' | '&' | '+' | '/' | '×' | '|'
        ) {
            separated.push('|');
        } else {
            separated.push(character);
        }
    }
    let mut tokens = Vec::new();
    for raw_part in separated.split('|') {
        let part = strip_known_region_suffix(raw_part);
        let mut entity = Vec::new();
        let normalized_part = normalize_identity_text(&part);
        for token in normalized_part.split_whitespace() {
            if matches!(token, "feat" | "featuring" | "ft" | "with") {
                if !entity.is_empty() {
                    tokens.push(entity.join(" "));
                    entity.clear();
                }
            } else {
                entity.push(token);
            }
        }
        if !entity.is_empty() {
            tokens.push(entity.join(" "));
        }
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

fn strip_known_region_suffix(value: &str) -> String {
    let mut raw = value.trim();
    while let Some(last) = raw.chars().last() {
        let opening = match last {
            ')' => raw.rfind('('),
            ']' => raw.rfind('['),
            _ => None,
        };
        let Some(opening) = opening else {
            break;
        };
        let closing_length = last.len_utf8();
        let inner = &raw[opening + 1..raw.len() - closing_length];
        if !matches!(
            normalize_identity_text(inner).as_str(),
            "au" | "br"
                | "ca"
                | "cn"
                | "de"
                | "es"
                | "fr"
                | "it"
                | "jp"
                | "kr"
                | "mx"
                | "nl"
                | "oz"
                | "uk"
                | "us"
        ) {
            break;
        }
        raw = raw[..opening].trim_end();
    }
    raw.to_string()
}

/// Normalize common release-label spelling differences without collapsing
/// meaningful versions such as Live or Radio Edit.
pub fn normalize_title_for_match(value: &str) -> String {
    let tokens = normalize_identity_text(value)
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut folded = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if matches!(token.as_str(), "extended" | "original")
            && tokens.get(index + 1).is_some_and(|next| next == "mix")
        {
            folded.push(token.clone());
            index += 2;
        } else {
            folded.push(token.clone());
            index += 1;
        }
    }
    folded.join(" ")
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
