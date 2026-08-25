use std::fs;
use std::path::PathBuf;

use w4dj::dj_playlist::{ImportedDjPlaylist, ImportedDjPlaylistTrack};
use w4dj::dj_playlist_match::{
    DjOutputCandidate, DjPlaylistMatchKind, match_imported_playlist, normalize_identity_text,
};
use w4dj::w4dj_library::W4djLibrary;

fn track(
    position: u64,
    title: &str,
    artist: &str,
    netease_track_id: Option<&str>,
) -> ImportedDjPlaylistTrack {
    let id = netease_track_id.map(str::to_string);
    ImportedDjPlaylistTrack {
        position,
        title: title.to_string(),
        artist_display: artist.to_string(),
        netease_track_id: id.clone(),
        dedupe_key: id
            .as_deref()
            .map(|value| format!("netease:{value}"))
            .unwrap_or_else(|| format!("title-artist:{position}")),
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

fn candidate(
    key: &str,
    title: &str,
    artist: &str,
    netease_track_id: Option<&str>,
) -> DjOutputCandidate {
    DjOutputCandidate {
        track_key: key.to_string(),
        title: title.to_string(),
        artist_display: artist.to_string(),
        netease_track_id: netease_track_id.map(str::to_string),
        duration_seconds: None,
        destination_path: PathBuf::from(format!("/music/{key}.mp3")),
        status: "available".to_string(),
        readable: true,
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
fn netease_id_is_the_first_and_only_identity_when_present() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Wrong title", "Wrong artist", Some("42"))]),
        &[candidate(
            "output:1",
            "Official title",
            "Official artist",
            Some("42"),
        )],
    );
    assert_eq!(report.matched_count, 1);
    assert_eq!(report.matches[0].track_key.as_deref(), Some("output:1"));
    assert_eq!(
        report.matches[0].match_method.as_deref(),
        Some("neteaseTrackId")
    );
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::NeteaseTrackId);
}

#[test]
fn an_id_miss_does_not_fall_back_to_a_lookalike_title() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Song", "Artist", Some("42"))]),
        &[candidate("output:1", "Song", "Artist", Some("99"))],
    );
    assert_eq!(report.unmatched_count, 1);
    assert_eq!(report.matches[0].track_key, None);
}

#[test]
fn unique_title_artist_is_the_only_automatic_fallback() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Track (Radio Edit)", "Main, Guest", None)]),
        &[candidate(
            "output:1",
            "track (radio edit)",
            "Guest feat. Main",
            None,
        )],
    );
    assert_eq!(report.matched_count, 1);
    assert_eq!(report.matches[0].track_key.as_deref(), Some("output:1"));
    assert_eq!(
        report.matches[0].match_method.as_deref(),
        Some("uniqueTitleArtistFallback")
    );
    assert_eq!(
        report.matches[0].kind,
        DjPlaylistMatchKind::UniqueTitleArtistFallback
    );
}

#[test]
fn full_title_keeps_mix_versions_distinct() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Song (Live)", "Artist", None)]),
        &[candidate("live", "Song", "Artist", None)],
    );
    assert_eq!(report.unmatched_count, 1);
    assert_eq!(report.matches[0].track_key, None);
}

#[test]
fn ambiguous_fallback_is_not_auto_selected() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Song", "Artist", None)]),
        &[
            candidate("a", "Song", "Artist", None),
            candidate("b", "Song", "Artist", None),
        ],
    );
    assert_eq!(report.ambiguous_count, 1);
    assert!(report.matches[0].track_key.is_none());
    assert_eq!(report.matches[0].candidates.len(), 2);
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::Ambiguous);
}

#[test]
fn repeated_id_positions_reuse_one_confirmed_output_without_dropping_rows() {
    let report = match_imported_playlist(
        &playlist(vec![
            track(1, "Song", "Artist", Some("42")),
            track(2, "Song", "Artist", Some("42")),
        ]),
        &[candidate("available", "Song", "Artist", Some("42"))],
    );
    assert_eq!(report.matched_count, 2);
    assert_eq!(report.matches.len(), 2);
    assert_eq!(report.matches[0].track_key.as_deref(), Some("available"));
    assert_eq!(report.matches[1].track_key.as_deref(), Some("available"));
}

#[test]
fn duplicate_outputs_for_one_id_are_ambiguous_but_remain_id_matches() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Same", "Artist", Some("42"))]),
        &[
            candidate("a", "First output", "Artist", Some("42")),
            candidate("b", "Second output", "Artist", Some("42")),
        ],
    );
    assert_eq!(report.ambiguous_count, 1);
    assert_eq!(
        report.matches[0].kind,
        w4dj::dj_playlist_match::DjPlaylistMatchKind::Ambiguous
    );
    assert_eq!(
        report.matches[0].match_method.as_deref(),
        Some("neteaseTrackId")
    );
}

#[test]
fn non_available_output_is_ignored() {
    let report = match_imported_playlist(
        &playlist(vec![track(1, "Song", "Artist", None)]),
        &[DjOutputCandidate {
            status: "missing".to_string(),
            ..candidate("missing", "Song", "Artist", None)
        }],
    );
    assert_eq!(report.unmatched_count, 1);
}

#[test]
fn w4dj_database_identity_resolves_id_to_the_current_readable_output() {
    let directory = tempfile::tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let output = output_root.join("renamed-official-file.mp3");
    fs::write(&output, b"audio").unwrap();

    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library
        .upsert_output_file(0, &output_root, None, &output)
        .unwrap();
    library
        .set_output_identity(&output, Some("123456789012345678"), None)
        .unwrap();

    let candidates = library.available_dj_output_candidates().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].netease_track_id.as_deref(),
        Some("123456789012345678")
    );
    assert_eq!(
        candidates[0].destination_path,
        fs::canonicalize(&output).unwrap()
    );

    let report = match_imported_playlist(
        &playlist(vec![track(
            1,
            "Different title",
            "Different artist",
            Some("123456789012345678"),
        )]),
        &candidates,
    );
    assert_eq!(report.matched_count, 1);
    assert_eq!(report.matches[0].kind, DjPlaylistMatchKind::NeteaseTrackId);
    assert_eq!(
        report.matches[0].track_key.as_deref(),
        Some(candidates[0].track_key.as_str())
    );
}
