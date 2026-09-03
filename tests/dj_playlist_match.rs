use std::path::PathBuf;

use w4dj::dj_playlist::{ImportedDjPlaylist, ImportedDjPlaylistTrack};
use w4dj::dj_playlist_match::{
    DjOutputCandidate, DjPlaylistMatchKind, bm25f_track_score, match_imported_playlist,
    match_imported_playlist_with_priority, normalize_identity_text,
};

fn track(position: u64, title: &str, artist: &str) -> ImportedDjPlaylistTrack {
    ImportedDjPlaylistTrack {
        position,
        title: title.to_string(),
        artist_display: artist.to_string(),
        dedupe_key: format!("title-artist:{position}"),
        netease_import_line: format!("{title} - {artist}"),
    }
}

fn playlist(tracks: Vec<ImportedDjPlaylistTrack>) -> ImportedDjPlaylist {
    ImportedDjPlaylist {
        playlist_id: "playlist-1".to_string(),
        format_version: 2,
        name: "Test".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks,
        warnings: Vec::new(),
    }
}

fn candidate(key: &str, title: &str, artist: &str) -> DjOutputCandidate {
    DjOutputCandidate {
        track_key: key.to_string(),
        title: title.to_string(),
        artist_display: artist.to_string(),
        duration_seconds: None,
        destination_path: PathBuf::from(format!("/music/{key}.mp3")),
        status: "available".to_string(),
        readable: true,
        conversion_batch_id: None,
        committed_at_ms: None,
    }
}

fn batched_candidate(key: &str, title: &str, artist: &str, batch_id: &str) -> DjOutputCandidate {
    DjOutputCandidate {
        conversion_batch_id: Some(batch_id.to_string()),
        committed_at_ms: Some(100),
        ..candidate(key, title, artist)
    }
}

#[test]
fn normalizes_fullwidth_case_punctuation_and_whitespace() {
    assert_eq!(
        normalize_identity_text("  Ｈｅｌｌｏ，  WORLD!  "),
        "hello world"
    );
}

#[test]
fn exact_title_and_multiple_artists_match_with_bm25f() {
    let report = match_imported_playlist(
        &playlist(vec![track(
            1,
            "Eat Your Man (with Nelly Furtado) Extended Mix",
            "Dom Dolla, Nelly Furtado",
        )]),
        &[candidate(
            "output:1",
            "Eat Your Man (with Nelly Furtado) Extended Mix",
            "Dom Dolla / Nelly Furtado",
        )],
    );

    assert_eq!(report.matched_count, 1);
    assert_eq!(report.matches[0].track_key.as_deref(), Some("output:1"));
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::LibraryBm25f);
    assert_eq!(
        report.matches[0].match_method.as_deref(),
        Some("libraryBm25f")
    );
    assert!(report.matches[0].score.unwrap_or_default() >= 90);
}

#[test]
fn version_labels_and_artist_region_suffixes_are_softly_matched() {
    let report = match_imported_playlist(
        &playlist(vec![track(
            1,
            "Atmosphere Extended Mix",
            "FISHER (OZ), Kita Alexander",
        )]),
        &[candidate(
            "output:1",
            "Atmosphere [Extended]",
            "FISHER, Kita Alexander",
        )],
    );

    assert_eq!(report.matched_count, 1);
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::LibraryBm25f);
    assert!(report.matches[0].score.unwrap_or_default() >= 50);
}

#[test]
fn unrelated_artist_does_not_cross_match_even_when_title_is_similar() {
    let report = match_imported_playlist(
        &playlist(vec![track(
            1,
            "Atmosphere Extended Mix",
            "FISHER, Kita Alexander",
        )]),
        &[candidate(
            "wrong",
            "Atmosphere [Extended]",
            "Paul de Fol, Kris Max",
        )],
    );

    assert_eq!(report.unmatched_count, 1);
    assert!(report.matches[0].track_key.is_none());
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::Unmatched);
}

#[test]
fn recent_batch_is_preferred_even_when_its_text_score_is_below_library_threshold() {
    let report = match_imported_playlist_with_priority(
        &playlist(vec![track(1, "Playlist title", "Playlist artist")]),
        &[batched_candidate(
            "recent",
            "Completely different",
            "Unknown",
            "batch-1",
        )],
        &[candidate("library", "Playlist title", "Playlist artist")],
    );

    assert_eq!(report.matched_count, 1);
    assert_eq!(report.matches[0].track_key.as_deref(), Some("recent"));
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::RecentBm25f);
    assert_eq!(report.matches[0].score, Some(0));
}

#[test]
fn a_short_recent_batch_is_filled_from_the_historical_library() {
    let report = match_imported_playlist_with_priority(
        &playlist(vec![
            track(1, "New One", "New Artist"),
            track(2, "New Two", "New Artist"),
            track(3, "Previously Converted", "Known Artist"),
        ]),
        &[
            batched_candidate("recent-1", "New One", "New Artist", "batch-1"),
            batched_candidate("recent-2", "New Two", "New Artist", "batch-1"),
        ],
        &[candidate(
            "library-3",
            "Previously Converted",
            "Known Artist",
        )],
    );

    assert_eq!(report.matched_count, 3);
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::RecentBm25f);
    assert_eq!(report.matches[1].kind, DjPlaylistMatchKind::RecentBm25f);
    assert_eq!(report.matches[2].kind, DjPlaylistMatchKind::LibraryBm25f);
    assert_eq!(report.matches[2].track_key.as_deref(), Some("library-3"));
}

#[test]
fn automatic_assignment_is_deterministic_and_consumes_distinct_outputs() {
    let playlist = playlist(vec![track(1, "Same", "Artist"), track(2, "Same", "Artist")]);
    let candidates = vec![
        candidate("b", "Same", "Artist"),
        candidate("a", "Same", "Artist"),
    ];

    let first = match_imported_playlist(&playlist, &candidates);
    let second = match_imported_playlist(&playlist, &candidates);
    assert_eq!(first, second);
    assert_eq!(first.matches[0].track_key.as_deref(), Some("a"));
    assert_eq!(first.matches[1].track_key.as_deref(), Some("b"));
}

#[test]
fn identical_playlist_rows_may_reuse_one_output_but_different_rows_may_not() {
    let repeated = match_imported_playlist_with_priority(
        &playlist(vec![
            track(1, "Same Song", "Same Artist"),
            track(2, "Same Song", "Same Artist"),
        ]),
        &[batched_candidate(
            "one-output",
            "Different",
            "Different",
            "batch-1",
        )],
        &[],
    );
    assert_eq!(repeated.matched_count, 2);
    assert_eq!(
        repeated.matches[0].track_key,
        Some("one-output".to_string())
    );
    assert_eq!(
        repeated.matches[1].track_key,
        Some("one-output".to_string())
    );

    let different = match_imported_playlist_with_priority(
        &playlist(vec![
            track(1, "Song One", "Artist"),
            track(2, "Song Two", "Artist"),
        ]),
        &[batched_candidate(
            "one-output",
            "Unrelated",
            "Unknown",
            "batch-1",
        )],
        &[],
    );
    assert_eq!(different.matched_count, 1);
    assert!(
        different
            .matches
            .iter()
            .any(|row| row.status == "unmatched")
    );
}

#[test]
fn suggestions_remain_available_for_manual_review_below_the_auto_threshold() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Rare Song", "Artist")]),
        &[candidate("lookalike", "Song Live", "Other")],
    );

    assert_eq!(report.unmatched_count, 1);
    assert_eq!(report.matches[0].candidates.len(), 1);
    assert!(report.matches[0].candidates[0].score > 0);
}

#[test]
fn bm25f_partial_title_and_artist_prefers_the_intended_track() {
    let corpus = vec![
        candidate("right", "Where You Are Extended Mix", "John Summit / HAYLA"),
        candidate("wrong", "Where Are You Now", "Lost Frequencies"),
    ];
    let right = bm25f_track_score(
        "Where You Are (Extended Mix)",
        "John Summit, HAYLA",
        &corpus[0].title,
        &corpus[0].artist_display,
        &corpus,
    );
    let wrong = bm25f_track_score(
        "Where You Are (Extended Mix)",
        "John Summit, HAYLA",
        &corpus[1].title,
        &corpus[1].artist_display,
        &corpus,
    );
    assert!(right > wrong);
    assert!(right >= 50);
}
