use rusqlite::Connection;
use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;
use w4dj::analysis::{
    AnalysisLabel, ContinuousEmotionResult, DiscogsEffnetAnalysis, DiscogsEffnetHeadResult,
    EmotionCandidates, EmotionHeadStatus, HighLevelAnalysis, TrackAnalysis,
};
use w4dj::dj_playlist::{ImportedDjPlaylist, ImportedDjPlaylistTrack};
use w4dj::m3u8::{ResolvedDjPlaylistTrack, build_relative_m3u8_with_summary};
use w4dj::w4dj_library::{CommittedOutputFacts, OutputFileSnapshot, W4djLibrary};

#[test]
fn sqlite_snapshot_is_readable_and_contains_output_index() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("out");
    fs::create_dir_all(&output_root).unwrap();
    let output = output_root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let database_path = directory.path().join("w4dj.sqlite3");
    let mut library = W4djLibrary::open(&database_path).unwrap();
    library
        .upsert_output_file(0, &output_root, None, &output)
        .unwrap();

    let snapshot = library.sqlite_snapshot_bytes().unwrap();
    assert!(snapshot.starts_with(b"SQLite format 3\0"));
    let snapshot_path = directory.path().join("snapshot.sqlite3");
    fs::write(&snapshot_path, snapshot).unwrap();
    let connection = Connection::open(&snapshot_path).unwrap();
    let output_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM w4dj_track_meta", [], |row| row.get(0))
        .unwrap();
    assert_eq!(output_count, 1);
    drop(connection);

    let read_only = W4djLibrary::open_read_only(&database_path).unwrap();
    assert_eq!(read_only.stats().unwrap().available, 1);
}

#[test]
fn w4dj_output_and_playlist_rows_never_backfill_netease_identity() {
    let directory = tempdir().unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    let output = directory.path().join("exports/eat-your-man.mp3");
    library
        .upsert_lightweight_output(
            0,
            None,
            &output,
            "Eat Your Man (with Nelly Furtado) Extended Mix",
            "Dom Dolla, Nelly Furtado",
        )
        .unwrap();
    let playlist = ImportedDjPlaylist {
        playlist_id: "playlist-backfill".to_string(),
        format_version: 2,
        name: "Backfill".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks: vec![ImportedDjPlaylistTrack {
            position: 1,
            title: "Eat Your Man (with Nelly Furtado) Extended Mix".to_string(),
            artist_display: "Dom Dolla, Nelly Furtado".to_string(),
            dedupe_key: "title-artist:eatyourmanwithnellyfurtadoextendedmix:domdollanellyfurtado"
                .to_string(),
            netease_import_line:
                "Eat Your Man (with Nelly Furtado) Extended Mix - Dom Dolla, Nelly Furtado"
                    .to_string(),
        }],
        warnings: Vec::new(),
    };
    library.upsert_imported_dj_playlist(&playlist).unwrap();

    assert_eq!(
        library.available_dj_output_candidates().unwrap()[0].title,
        "Eat Your Man (with Nelly Furtado) Extended Mix"
    );
    let loaded = library
        .get_imported_dj_playlist("playlist-backfill")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.tracks[0].dedupe_key, playlist.tracks[0].dedupe_key);
}

#[test]
fn ordinary_output_indexing_does_not_write_w4dj_manifest() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let output_root = fs::canonicalize(output_root).unwrap();
    let output = output_root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let source = directory.path().join("downloads/song.ncm");
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();

    library
        .upsert_lightweight_output(0, Some(&source), &output, "Song", "Artist")
        .unwrap();
    library
        .reconcile_output_roots(&[(0, output_root.clone(), vec![output])])
        .unwrap();
    let scanned_output = output_root.join("scanned.mp3");
    fs::write(&scanned_output, b"scanned audio").unwrap();
    library
        .upsert_output_file(0, &output_root, None, &scanned_output)
        .unwrap();

    assert!(!output_root.join(".w4dj-output-identities.json").exists());
}

#[test]
fn explicit_w4dj_operation_persists_output_manifest() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let output_root = fs::canonicalize(output_root).unwrap();
    let output = output_root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let source = directory.path().join("downloads/song.ncm");
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();

    library
        .upsert_lightweight_output(0, Some(&source), &output, "Song", "Artist")
        .unwrap();
    assert!(!output_root.join(".w4dj-output-identities.json").exists());

    assert_eq!(
        library
            .persist_output_identity_manifests(&[(0, output_root.clone())])
            .unwrap(),
        1
    );
    let manifest_path = output_root.join(".w4dj-output-identities.json");
    assert!(manifest_path.is_file());
    let manifest_before_ordinary_update = fs::read(&manifest_path).unwrap();
    let second_output = output_root.join("second.mp3");
    fs::write(&second_output, b"second audio").unwrap();
    library
        .upsert_lightweight_output(
            0,
            Some(&directory.path().join("downloads/second.ncm")),
            &second_output,
            "Second",
            "Artist",
        )
        .unwrap();
    assert_eq!(
        fs::read(&manifest_path).unwrap(),
        manifest_before_ordinary_update
    );
}

#[test]
fn ordinary_rescan_does_not_restore_w4dj_manifest() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let output_root = fs::canonicalize(output_root).unwrap();
    let output = output_root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let source = directory.path().join("downloads/song.ncm");

    let database = directory.path().join("w4dj.sqlite3");
    let mut w4dj_library = W4djLibrary::open(&database).unwrap();
    w4dj_library
        .upsert_lightweight_output(0, Some(&source), &output, "Song", "Artist")
        .unwrap();
    assert_eq!(
        w4dj_library
            .persist_output_identity_manifests(&[(0, output_root.clone())])
            .unwrap(),
        1
    );
    assert!(output_root.join(".w4dj-output-identities.json").is_file());
    drop(w4dj_library);

    let mut ordinary_library =
        W4djLibrary::open(&directory.path().join("ordinary.sqlite3")).unwrap();
    ordinary_library
        .reconcile_output_roots(&[(0, output_root, vec![output])])
        .unwrap();
    assert_eq!(
        ordinary_library
            .available_dj_output_candidates()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn output_identity_survives_library_clear_and_output_rescan() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let output_root = fs::canonicalize(output_root).unwrap();
    let output = output_root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let source = directory.path().join("downloads/song.ncm");
    let database = directory.path().join("w4dj.sqlite3");

    let mut library = W4djLibrary::open(&database).unwrap();
    library
        .upsert_lightweight_output(0, Some(&source), &output, "Song", "Artist")
        .unwrap();
    assert_eq!(
        library
            .persist_output_identity_manifests(&[(0, output_root.clone())])
            .unwrap(),
        1
    );
    assert!(output_root.join(".w4dj-output-identities.json").is_file());
    library
        .reconcile_output_roots(&[(0, output_root.clone(), vec![output.clone()])])
        .unwrap();

    library.clear_output_library().unwrap();
    assert!(output_root.join(".w4dj-output-identities.json").is_file());
    assert_eq!(library.restore_output_identity_manifests(&[]).unwrap(), 1);

    library
        .reconcile_output_roots(&[(0, output_root.clone(), vec![output.clone()])])
        .unwrap();
    let candidates = library.available_dj_output_candidates().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].title, "Song");
    assert_eq!(candidates[0].artist_display, "Artist");

    drop(library);
    let mut rebuilt_library = W4djLibrary::open(&directory.path().join("rebuilt.sqlite3")).unwrap();
    assert_eq!(
        rebuilt_library
            .restore_output_identity_manifests(&[(0, output_root)])
            .unwrap(),
        1
    );
    let rebuilt_candidates = rebuilt_library.available_dj_output_candidates().unwrap();
    assert_eq!(rebuilt_candidates[0].title, "Song");
}

#[test]
fn reviewed_playlist_binding_survives_explicit_manifest_restore() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let output_root = fs::canonicalize(output_root).unwrap();
    let output = output_root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let database = directory.path().join("w4dj.sqlite3");

    let playlist = ImportedDjPlaylist {
        playlist_id: "reviewed-playlist".to_string(),
        format_version: 2,
        name: "Reviewed playlist".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks: vec![ImportedDjPlaylistTrack {
            position: 1,
            title: "Song".to_string(),
            artist_display: "Artist".to_string(),
            dedupe_key: "title-artist:song:artist".to_string(),
            netease_import_line: "Song - Artist".to_string(),
        }],
        warnings: Vec::new(),
    };

    let mut library = W4djLibrary::open(&database).unwrap();
    library
        .upsert_lightweight_output(0, None, &output, "Song", "Artist")
        .unwrap();
    library.upsert_imported_dj_playlist(&playlist).unwrap();
    let report = library
        .compute_imported_dj_playlist_matches(&playlist.playlist_id)
        .unwrap();
    library
        .replace_imported_dj_playlist_matches(&playlist.playlist_id, &report)
        .unwrap();
    library
        .set_imported_dj_playlist_match_confirmed(&playlist.playlist_id, 1, true)
        .unwrap();
    assert_eq!(
        library
            .persist_output_identity_manifests(&[(0, output_root.clone())])
            .unwrap(),
        1
    );
    let manifest_path = output_root.join(".w4dj-output-identities.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest.contains("reviewed-playlist"));
    assert!(manifest.contains("relativePath"));
    assert!(!manifest.contains("netease_track_id"));
    drop(library);

    let rebuilt_database = directory.path().join("rebuilt-w4dj.sqlite3");
    let mut rebuilt = W4djLibrary::open(&rebuilt_database).unwrap();
    assert_eq!(
        rebuilt
            .restore_output_identity_manifests(&[(0, output_root.clone())])
            .unwrap(),
        1
    );
    rebuilt.upsert_imported_dj_playlist(&playlist).unwrap();
    assert_eq!(
        rebuilt
            .restore_imported_dj_playlist_review_manifests(
                &playlist.playlist_id,
                &[(0, output_root)],
            )
            .unwrap(),
        1
    );
    let restored = rebuilt
        .get_imported_dj_playlist_match_report(&playlist.playlist_id)
        .unwrap();
    assert_eq!(restored.matched_count, 1);
    assert!(restored.matches[0].confirmed);
    assert_eq!(
        restored.matches[0].match_method.as_deref(),
        Some("libraryBm25f")
    );
    assert_eq!(
        restored.matches[0].destination_path.as_deref(),
        Some(output.as_path())
    );
}

#[test]
fn playlist_with_existing_and_new_outputs_matches_both_after_index_rebuild() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let first = output_root.join("a.mp3");
    let second = output_root.join("b.mp3");
    fs::write(&first, b"a").unwrap();
    fs::write(&second, b"b").unwrap();
    let database = directory.path().join("w4dj.sqlite3");
    let mut library = W4djLibrary::open(&database).unwrap();
    for (path, title) in [(&first, "Song A"), (&second, "Song B")] {
        library
            .upsert_lightweight_output(0, None, path, title, "Artist")
            .unwrap();
    }
    assert_eq!(
        library
            .persist_output_identity_manifests(&[(0, output_root.clone())])
            .unwrap(),
        2
    );
    library.clear_output_library().unwrap();
    library
        .restore_output_identity_manifests(&[(0, output_root)])
        .unwrap();

    let playlist = ImportedDjPlaylist {
        playlist_id: "a-plus-b".to_string(),
        format_version: 2,
        name: "A + B".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks: vec![
            ImportedDjPlaylistTrack {
                position: 1,
                title: "Song A".to_string(),
                artist_display: "Artist".to_string(),
                dedupe_key: "title-artist:songa:artist".to_string(),
                netease_import_line: "Song A - Artist".to_string(),
            },
            ImportedDjPlaylistTrack {
                position: 2,
                title: "Song B".to_string(),
                artist_display: "Artist".to_string(),
                dedupe_key: "title-artist:songb:artist".to_string(),
                netease_import_line: "Song B - Artist".to_string(),
            },
        ],
        warnings: Vec::new(),
    };
    library.upsert_imported_dj_playlist(&playlist).unwrap();
    let report = library
        .compute_imported_dj_playlist_matches("a-plus-b")
        .unwrap();
    assert_eq!(report.matched_count, 2);
    assert!(report.matches.iter().all(|row| row.status == "matched"));

    let candidates = library.available_dj_output_candidates().unwrap();
    let resolved = report
        .matches
        .iter()
        .filter_map(|row| {
            let track_key = row.track_key.as_deref()?;
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.track_key == track_key)?;
            Some(ResolvedDjPlaylistTrack {
                position: row.position,
                title: row.title.clone(),
                artist_display: row.artist_display.clone(),
                duration_seconds: candidate.duration_seconds,
                destination_path: candidate.destination_path.clone(),
            })
        })
        .collect::<Vec<_>>();
    let (contents, summary) = build_relative_m3u8_with_summary(
        &playlist,
        &resolved,
        &directory.path().join("a-plus-b.m3u8"),
    )
    .unwrap();
    assert_eq!(summary.matched_count, 2);
    assert!(contents.contains("outputs/a.mp3"));
    assert!(contents.contains("outputs/b.mp3"));
}

#[test]
fn playlist_claims_the_latest_unclaimed_committed_batch() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("outputs");
    fs::create_dir_all(&root).unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library
        .upsert_committed_output_in_root(
            0,
            &root,
            None,
            &root.join("old.mp3"),
            "Old",
            "Artist",
            &CommittedOutputFacts {
                conversion_batch_id: Some("batch-old".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    library
        .upsert_committed_output_in_root(
            0,
            &root,
            None,
            &root.join("new.mp3"),
            "New",
            "Artist",
            &CommittedOutputFacts {
                conversion_batch_id: Some("batch-new".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    let playlist = ImportedDjPlaylist {
        playlist_id: "playlist-a".to_string(),
        format_version: 2,
        name: "Playlist A".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks: vec![ImportedDjPlaylistTrack {
            position: 1,
            title: "New".to_string(),
            artist_display: "Artist".to_string(),
            dedupe_key: "title-artist:new:artist".to_string(),
            netease_import_line: "New - Artist".to_string(),
        }],
        warnings: Vec::new(),
    };
    library.upsert_imported_dj_playlist(&playlist).unwrap();
    assert_eq!(
        library
            .claim_latest_conversion_batch("playlist-a")
            .unwrap()
            .as_deref(),
        Some("batch-new")
    );
    assert_eq!(
        library
            .claim_latest_conversion_batch("playlist-a")
            .unwrap()
            .as_deref(),
        Some("batch-new")
    );

    let mut second_playlist = playlist.clone();
    second_playlist.playlist_id = "playlist-b".to_string();
    library
        .upsert_imported_dj_playlist(&second_playlist)
        .unwrap();
    assert_eq!(
        library.claim_latest_conversion_batch("playlist-b").unwrap(),
        None
    );
}

#[test]
fn changing_a_manual_file_keeps_the_new_binding_exportable() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("outputs");
    fs::create_dir_all(&root).unwrap();
    let first = root.join("first.mp3");
    let second = root.join("second.mp3");
    fs::write(&first, b"first audio").unwrap();
    fs::write(&second, b"second audio").unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library
        .upsert_lightweight_output(0, None, &first, "First", "Artist")
        .unwrap();
    library
        .upsert_lightweight_output(0, None, &second, "Second", "Artist")
        .unwrap();
    let playlist = ImportedDjPlaylist {
        playlist_id: "manual-playlist".to_string(),
        format_version: 2,
        name: "Manual playlist".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks: vec![ImportedDjPlaylistTrack {
            position: 1,
            title: "First".to_string(),
            artist_display: "Artist".to_string(),
            dedupe_key: "title-artist:first:artist".to_string(),
            netease_import_line: "First - Artist".to_string(),
        }],
        warnings: Vec::new(),
    };
    library.upsert_imported_dj_playlist(&playlist).unwrap();
    let report = library
        .compute_imported_dj_playlist_matches("manual-playlist")
        .unwrap();
    library
        .replace_imported_dj_playlist_matches("manual-playlist", &report)
        .unwrap();
    library
        .set_imported_dj_playlist_match_confirmed("manual-playlist", 1, true)
        .unwrap();
    library
        .set_imported_dj_playlist_match_by_path("manual-playlist", 1, &second)
        .unwrap();
    let row = &library
        .get_imported_dj_playlist_match_report("manual-playlist")
        .unwrap()
        .matches[0];
    assert_eq!(row.destination_path.as_deref(), Some(second.as_path()));
    assert!(row.confirmed);
    assert_eq!(row.match_method.as_deref(), Some("manual"));
}

#[test]
fn excluding_playlist_rows_persists_only_in_the_export_review() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    fs::create_dir_all(&output_root).unwrap();
    let output_root = fs::canonicalize(output_root).unwrap();
    let outputs = [
        output_root.join("song-a.mp3"),
        output_root.join("song-b.mp3"),
        output_root.join("song-c.mp3"),
    ];
    for (index, output) in outputs.iter().enumerate() {
        fs::write(output, format!("audio-{index}")).unwrap();
    }
    let playlist = ImportedDjPlaylist {
        playlist_id: "excluded-playlist".to_string(),
        format_version: 2,
        name: "Excluded playlist".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks: ["A", "B", "C"]
            .into_iter()
            .enumerate()
            .map(|(index, suffix)| ImportedDjPlaylistTrack {
                position: (index + 1) as u64,
                title: format!("Song {suffix}"),
                artist_display: "Artist".to_string(),
                dedupe_key: format!("song-{suffix}"),
                netease_import_line: format!("Song {suffix} - Artist"),
            })
            .collect(),
        warnings: Vec::new(),
    };
    let database = directory.path().join("w4dj.sqlite3");
    let mut library = W4djLibrary::open(&database).unwrap();
    for (index, output) in outputs.iter().enumerate() {
        library
            .upsert_lightweight_output(
                0,
                None,
                output,
                &format!("Song {}", ["A", "B", "C"][index]),
                "Artist",
            )
            .unwrap();
    }
    library.upsert_imported_dj_playlist(&playlist).unwrap();
    let report = library
        .compute_imported_dj_playlist_matches(&playlist.playlist_id)
        .unwrap();
    assert_eq!(report.matched_count, 3);
    library
        .replace_imported_dj_playlist_matches(&playlist.playlist_id, &report)
        .unwrap();

    library
        .set_imported_dj_playlist_matches_excluded(&playlist.playlist_id, &[2], true)
        .unwrap();
    let excluded_report = library
        .get_imported_dj_playlist_match_report(&playlist.playlist_id)
        .unwrap();
    assert!(
        excluded_report
            .matches
            .iter()
            .find(|row| row.position == 2)
            .unwrap()
            .excluded
    );
    assert_eq!(library.available_dj_output_candidates().unwrap().len(), 3);
    assert!(outputs[1].is_file());

    library
        .persist_output_identity_manifests(&[(0, output_root.clone())])
        .unwrap();
    let manifest = fs::read_to_string(output_root.join(".w4dj-output-identities.json")).unwrap();
    assert!(manifest.contains("song-a.mp3"));
    assert!(manifest.contains("song-c.mp3"));
    assert!(!manifest.contains("\"position\": 2"));

    drop(library);
    let mut reopened = W4djLibrary::open(&database).unwrap();
    let reopened_report = reopened
        .get_imported_dj_playlist_match_report(&playlist.playlist_id)
        .unwrap();
    assert!(
        reopened_report
            .matches
            .iter()
            .find(|row| row.position == 2)
            .unwrap()
            .excluded
    );
    reopened
        .set_imported_dj_playlist_matches_excluded(&playlist.playlist_id, &[2], false)
        .unwrap();
    let restored_report = reopened
        .get_imported_dj_playlist_match_report(&playlist.playlist_id)
        .unwrap();
    assert!(
        !restored_report
            .matches
            .iter()
            .find(|row| row.position == 2)
            .unwrap()
            .excluded
    );
}

#[test]
fn replacing_a_converted_path_with_a_new_source_does_not_reuse_the_old_id() {
    let directory = tempdir().unwrap();
    let output = directory.path().join("outputs/song.mp3");
    let source_a = directory.path().join("downloads/a.ncm");
    let source_b = directory.path().join("downloads/b.ncm");
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();

    library
        .upsert_lightweight_output(0, Some(&source_a), &output, "Song A", "Artist")
        .unwrap();
    library
        .upsert_lightweight_output(0, Some(&source_b), &output, "Song B", "Artist")
        .unwrap();

    let candidate = &library.available_dj_output_candidates().unwrap()[0];
    assert_eq!(candidate.title, "Song B");
}

#[test]
fn committed_nested_output_uses_the_configured_root_manifest() {
    let directory = tempdir().unwrap();
    let output_root = directory.path().join("outputs");
    let output = output_root.join("house/song.mp3");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, b"audio").unwrap();
    let source = directory.path().join("downloads/song.ncm");
    let database = directory.path().join("w4dj.sqlite3");

    let mut library = W4djLibrary::open(&database).unwrap();
    library
        .upsert_committed_output_in_root(
            0,
            &output_root,
            Some(&source),
            &output,
            "Song",
            "Artist",
            &CommittedOutputFacts::default(),
        )
        .unwrap();
    assert_eq!(
        library
            .persist_output_identity_manifests(&[(0, output_root.clone())])
            .unwrap(),
        1
    );
    assert!(output_root.join(".w4dj-output-identities.json").is_file());
    assert!(
        !output
            .parent()
            .unwrap()
            .join(".w4dj-output-identities.json")
            .is_file()
    );
    drop(library);

    let mut rebuilt = W4djLibrary::open(&directory.path().join("rebuilt.sqlite3")).unwrap();
    assert_eq!(
        rebuilt
            .restore_output_identity_manifests(&[(0, output_root)])
            .unwrap(),
        1
    );
    assert_eq!(
        rebuilt.available_dj_output_candidates().unwrap()[0].title,
        "Song"
    );
}

#[test]
fn output_metadata_is_not_replaced_by_a_netease_identity_backfill() {
    let directory = tempdir().unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    let output = directory.path().join("exports/song.mp3");
    library
        .upsert_lightweight_output(0, None, &output, "Song", "Artist")
        .unwrap();
    let candidate = &library.available_dj_output_candidates().unwrap()[0];
    assert_eq!(candidate.title, "Song");
    assert_eq!(candidate.artist_display, "Artist");
}

#[test]
fn high_level_json_round_trips_old_and_new_emotion_fields() {
    let old = serde_json::json!({
        "status": "completed",
        "modelVersion": "legacy",
        "genre": [],
        "mood": [{"label": "happy", "confidence": 0.9}],
        "instrument": [],
        "filtered": []
    });
    let parsed_old: HighLevelAnalysis = serde_json::from_value(old).unwrap();
    assert!(parsed_old.style.is_empty());
    assert!(parsed_old.emotion_candidates.is_none());
    assert!(parsed_old.mood_cluster.is_empty());
    assert_eq!(parsed_old.mood[0].label, "happy");

    let current = HighLevelAnalysis {
        status: "completed".into(),
        model_version: Some("emotion-v1".into()),
        reason: None,
        genre: Vec::new(),
        style: vec![AnalysisLabel {
            label: "House".into(),
            confidence: 0.8,
        }],
        mood: vec![AnalysisLabel {
            label: "happy".into(),
            confidence: 0.9,
        }],
        instrument: Vec::new(),
        emotion_candidates: Some(EmotionCandidates {
            emomusic: Some(ContinuousEmotionResult {
                model: "emomusic".into(),
                status: EmotionHeadStatus::Completed,
                valence: Some(7.0),
                arousal: Some(6.0),
                reason: None,
            }),
            muse: Some(ContinuousEmotionResult {
                model: "muse".into(),
                status: EmotionHeadStatus::ModelMissing,
                valence: None,
                arousal: None,
                reason: Some("missing".into()),
            }),
        }),
        mood_cluster: vec![AnalysisLabel {
            label: "passionate".into(),
            confidence: 0.7,
        }],
        mood_cluster_status: Some(EmotionHeadStatus::Completed),
        mood_cluster_reason: None,
        filtered: Vec::new(),
        discogs_effnet: None,
    };
    let value = serde_json::to_value(&current).unwrap();
    assert_eq!(value["emotionCandidates"]["emomusic"]["valence"], 7.0);
    assert_eq!(value["moodCluster"][0]["label"], "passionate");
    let round_trip: HighLevelAnalysis = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip, current);
}

#[test]
fn continuous_emotion_json_rejects_invalid_completed_or_non_completed_values() {
    let invalid_completed = serde_json::json!({
        "model": "emomusic",
        "status": "completed",
        "valence": 10.0,
        "arousal": 5.0
    });
    assert!(serde_json::from_value::<ContinuousEmotionResult>(invalid_completed).is_err());

    let invalid_missing = serde_json::json!({
        "model": "muse",
        "status": "model_missing",
        "valence": 5.0,
        "arousal": null
    });
    assert!(serde_json::from_value::<ContinuousEmotionResult>(invalid_missing).is_err());
}

#[test]
fn output_root_switch_marks_old_slot_records_out_of_scope() {
    let directory = tempdir().unwrap();
    let root_a = directory.path().join("A");
    let root_b = directory.path().join("B");
    fs::create_dir_all(&root_a).unwrap();
    fs::create_dir_all(&root_b).unwrap();
    let output_a = root_a.join("a.mp3");
    let output_b = root_b.join("b.mp3");
    fs::write(&output_a, b"a").unwrap();
    fs::write(&output_b, b"b").unwrap();

    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library
        .upsert_output_file(0, &root_a, None, &output_a)
        .unwrap();
    assert_eq!(library.stats().unwrap().available, 1);
    library
        .upsert_output_file(0, &root_b, None, &output_b)
        .unwrap();
    let stats = library.stats().unwrap();
    assert_eq!(stats.total, 2);
    assert_eq!(stats.available, 1);
    assert_eq!(stats.invalid, 1);
}

#[test]
fn invalid_cleanup_only_removes_database_rows() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let database_path = directory.path().join("w4dj.sqlite3");
    let mut library = W4djLibrary::open(&database_path).unwrap();
    library.upsert_output_file(0, &root, None, &output).unwrap();
    fs::remove_file(&output).unwrap();

    let stats = library.scan_invalid(|| false, |_, _, _| {}).unwrap();
    assert_eq!(stats.invalid, 1);
    assert_eq!(library.remove_invalid().unwrap(), 1);
    assert_eq!(library.stats().unwrap().total, 0);
    assert!(!output.exists());
}

#[test]
fn output_without_analysis_can_be_removed_from_the_independent_library() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("not-yet-analyzed.mp3");
    fs::write(&output, b"audio").unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    let key = library.upsert_output_file(0, &root, None, &output).unwrap();

    assert!(library.remove_analyzed_track(&key).unwrap());
    assert_eq!(library.stats().unwrap().total, 0);
    assert!(output.is_file());
}

#[test]
fn lightweight_output_registration_never_requires_the_files_to_exist() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("sources/song.ncm");
    let destination = directory.path().join("exports/song.mp3");
    let database_path = directory.path().join("w4dj.sqlite3");
    let mut library = W4djLibrary::open(&database_path).unwrap();

    let key = library
        .upsert_lightweight_output(1, Some(&source), &destination, "Song", "Artist")
        .unwrap();

    assert!(key.starts_with("source:"));
    assert_eq!(library.stats().unwrap().total, 1);
    let track = library.track_detail(&key).unwrap().unwrap();
    assert_eq!(track.title, "Song");
    assert_eq!(track.artists, "Artist");
    assert_eq!(track.netease_track_id, None);
    assert_eq!(
        library.local_files_for_track(&key).unwrap()[0].path,
        destination
    );
    // Neither the source nor destination exists.  The successful registration
    // is evidence that no metadata probe/stat/open was hidden in this path.
    assert!(!source.exists());
    assert!(
        library
            .local_files_for_track(&key)
            .unwrap()
            .first()
            .unwrap()
            .readable
    );
    drop(library);
    let connection = Connection::open(database_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM output_roots", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM slot_output_roots", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn lightweight_registration_updates_one_identity_when_output_moves() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("source.flac");
    let first = directory.path().join("A/first.mp3");
    let second = directory.path().join("B/second.mp3");
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();

    let first_key = library
        .upsert_lightweight_output(0, Some(&source), &first, "First", "Artist")
        .unwrap();
    let second_key = library
        .upsert_lightweight_output(0, Some(&source), &second, "Second", "Artist")
        .unwrap();

    assert_eq!(first_key, second_key);
    assert_eq!(library.stats().unwrap().total, 1);
    let candidates = library.available_dj_output_candidates().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].destination_path, second);
}

#[test]
fn lightweight_registration_without_netease_id_reuses_source_identity() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("source.flac");
    let first = directory.path().join("A/first.mp3");
    let second = directory.path().join("B/second.mp3");
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();

    let first_key = library
        .upsert_lightweight_output(0, Some(&source), &first, "First", "Artist")
        .unwrap();
    let second_key = library
        .upsert_lightweight_output(0, Some(&source), &second, "Second", "Artist")
        .unwrap();

    assert_eq!(first_key, "source:".to_string() + &source.to_string_lossy());
    assert_eq!(second_key, first_key);
    assert_eq!(library.stats().unwrap().total, 1);
    assert_eq!(
        library
            .available_dj_output_candidates()
            .unwrap()
            .first()
            .unwrap()
            .destination_path,
        second
    );
}

#[test]
fn committed_output_bindings_store_fingerprint_and_naming_context() {
    let directory = tempdir().unwrap();
    let source = directory.path().join("source.flac");
    let destination = directory.path().join("out/song.mp3");
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    let facts = CommittedOutputFacts {
        source_size_bytes: Some(123),
        source_modified_at_ms: Some(456),
        conversion_mode: Some("scan_then_convert".into()),
        lossless_format: Some("wav".into()),
        filename_rule: Some("title_artist".into()),
        netease_filename_format: Some("title_artist".into()),
        filename_normalization_policy: Some("soundcloud".into()),
        conversion_batch_id: Some("batch-1".into()),
    };
    library
        .upsert_committed_output(0, Some(&source), &destination, "Song", "Artist", &facts)
        .unwrap();

    let bindings = library.committed_output_bindings().unwrap();
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    assert_eq!(binding.source_size_bytes, Some(123));
    assert_eq!(binding.source_modified_at_ms, Some(456));
    assert_eq!(binding.mode.as_deref(), Some("scan_then_convert"));
    assert_eq!(binding.filename_rule.as_deref(), Some("title_artist"));
}

#[test]
fn schema_v2_track_meta_migration_preserves_rows_and_adds_v4_facts() {
    let directory = tempdir().unwrap();
    let database_path = directory.path().join("w4dj-v2.sqlite3");
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let mut library = W4djLibrary::open(&database_path).unwrap();
    library.upsert_output_file(0, &root, None, &output).unwrap();
    drop(library);

    let connection = Connection::open(&database_path).unwrap();
    let stored_root: String = connection
        .query_row("SELECT root_path FROM output_roots LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    connection
        .execute_batch("DROP INDEX IF EXISTS w4dj_track_meta_source_path; DROP INDEX IF EXISTS w4dj_track_meta_status; DROP TABLE w4dj_track_meta;")
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE w4dj_track_meta(
                track_key TEXT PRIMARY KEY,
                source_path TEXT,
                destination_path TEXT NOT NULL UNIQUE,
                slot_index INTEGER NOT NULL,
                output_root TEXT NOT NULL,
                status TEXT NOT NULL,
                analysis_status TEXT NOT NULL DEFAULT 'notAnalyzed',
                analysis_error TEXT,
                measured_duration_seconds REAL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                FOREIGN KEY(output_root) REFERENCES output_roots(root_path)
             );",
        )
        .unwrap();
    let track_key: String = connection
        .query_row("SELECT track_key FROM tracks LIMIT 1", [], |row| row.get(0))
        .unwrap();
    connection
        .execute(
            "INSERT INTO w4dj_track_meta VALUES(?1,?2,?3,0,?4,'available','notAnalyzed',NULL,NULL,1,1)",
            rusqlite::params![
                track_key,
                "/in/song.flac",
                output.to_string_lossy(),
                stored_root
            ],
        )
        .unwrap();
    drop(connection);

    let library = W4djLibrary::open(&database_path).unwrap();
    let bindings = library.committed_output_bindings().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].source_path, "/in/song.flac");
    let connection = Connection::open(database_path).unwrap();
    let columns = connection
        .prepare("SELECT name FROM pragma_table_info('w4dj_track_meta')")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(columns.iter().any(|column| column == "source_size_bytes"));
    assert!(columns.iter().any(|column| column == "filename_rule"));
}

#[test]
fn reconcile_output_snapshots_uses_captured_file_facts() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("song.mp3");
    fs::write(&output, b"audio").unwrap();
    let size = fs::metadata(&output).unwrap().len();
    let modified = fs::metadata(&output)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library
        .reconcile_output_snapshots(&[(
            0,
            root.clone(),
            vec![OutputFileSnapshot {
                path: output.clone(),
                size_bytes: size,
                modified_at_ms: Some(modified),
            }],
        )])
        .unwrap();
    assert_eq!(library.stats().unwrap().available, 1);
}

#[test]
fn emotion_manifest_samples_available_outputs_and_preserves_legacy_mood() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(root.join("Album")).unwrap();
    let first = root.join("Album/first.mp3");
    let second = root.join("Album/second.mp3");
    let missing = root.join("Album/missing.mp3");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    fs::write(&missing, b"missing").unwrap();

    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    library.upsert_output_file(0, &root, None, &first).unwrap();
    library.upsert_output_file(0, &root, None, &second).unwrap();
    library
        .upsert_output_file(0, &root, None, &missing)
        .unwrap();
    let analysis = TrackAnalysis {
        path: first.display().to_string(),
        title: "First".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        genre: String::new(),
        duration_seconds: Some(20.0),
        bpm: Some(120.0),
        key: None,
        scale: None,
        key_strength: None,
        integrated_loudness_lufs: None,
        loudness_range_lu: None,
        energy: Some(0.5),
        danceability: Some(0.5),
        beat_positions: Vec::new(),
        analyzed_at: String::new(),
        analyzer: "test".into(),
        analysis_version: "test".into(),
        source_size_bytes: None,
        source_modified_at: None,
        source_filename_format: None,
        drop_loudness_lufs: None,
        drop_analysis: None,
        high_level: Some(HighLevelAnalysis {
            status: "completed".into(),
            model_version: None,
            reason: None,
            genre: Vec::new(),
            style: Vec::new(),
            mood: vec![AnalysisLabel {
                label: "happy".into(),
                confidence: 0.91,
            }],
            instrument: Vec::new(),
            emotion_candidates: Some(EmotionCandidates {
                emomusic: Some(ContinuousEmotionResult {
                    model: "emomusic".into(),
                    status: EmotionHeadStatus::Completed,
                    valence: Some(7.0),
                    arousal: Some(6.0),
                    reason: None,
                }),
                muse: Some(ContinuousEmotionResult {
                    model: "muse".into(),
                    status: EmotionHeadStatus::Completed,
                    valence: Some(5.0),
                    arousal: Some(4.0),
                    reason: None,
                }),
            }),
            mood_cluster: vec![AnalysisLabel {
                label: "passionate".into(),
                confidence: 0.8,
            }],
            mood_cluster_status: Some(EmotionHeadStatus::Completed),
            mood_cluster_reason: None,
            filtered: Vec::new(),
            discogs_effnet: None,
        }),
    };
    library
        .apply_analysis_for_destination(&first, &analysis)
        .unwrap();

    let first_manifest = library.emotion_evaluation_manifest(0, 42).unwrap();
    let second_manifest = library.emotion_evaluation_manifest(0, 42).unwrap();
    assert_eq!(first_manifest.seed, second_manifest.seed);
    assert_eq!(first_manifest.tracks, second_manifest.tracks);
    let json = serde_json::to_value(&first_manifest).unwrap();
    assert!(json.get("schemaVersion").is_some());
    assert!(json["tracks"][0].get("clipSelection").is_some());
    assert!(json["tracks"][0].get("legacyMood").is_some());
    assert_eq!(first_manifest.sample_size, 3);
    assert!(
        first_manifest
            .tracks
            .iter()
            .all(|track| track.relative_path.starts_with("Album/"))
    );
    let first_entry = first_manifest
        .tracks
        .iter()
        .find(|track| track.track_id.contains("first.mp3"))
        .unwrap();
    assert_eq!(first_entry.legacy_mood["status"], "completed");
    assert_eq!(first_entry.legacy_mood["labels"][0]["label"], "happy");
    assert_eq!(first_entry.emomusic["status"], "completed");
    assert_eq!(first_entry.emomusic["valence"], 7.0);
    assert_eq!(first_entry.muse["arousal"], 4.0);
    assert_eq!(first_entry.mirex["status"], "completed");
    assert_eq!(first_entry.mirex["labels"][0]["label"], "passionate");
    let second_entry = first_manifest
        .tracks
        .iter()
        .find(|track| track.track_id.contains("second.mp3"))
        .unwrap();
    assert_eq!(second_entry.emomusic["status"], "model_missing");
    assert_eq!(second_entry.muse["status"], "model_missing");
    assert_eq!(second_entry.mirex["status"], "model_missing");
    assert!(matches!(
        first_entry.clip_selection.as_str(),
        "peakEnergy" | "startFallback" | "fullTrack" | "drop"
    ));
    assert!(
        first_manifest
            .tracks
            .iter()
            .all(|track| track.clip_duration_seconds <= 10.0)
    );
    assert_eq!(
        library
            .emotion_evaluation_manifest(2, 42)
            .unwrap()
            .sample_size,
        2
    );
}

#[test]
fn discogs_heads_use_independent_projection_columns_and_preserve_completed_siblings() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("out");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("discogs.mp3");
    fs::write(&output, b"audio").unwrap();
    let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    let track_key = library.upsert_output_file(0, &root, None, &output).unwrap();

    let mut heads = BTreeMap::new();
    for (id, label) in [
        ("moodTheme", "dark"),
        ("approachability", "approachable"),
        ("instrumentation", "synthesizer"),
        ("timbre", "bright"),
        ("danceability", "danceable"),
    ] {
        heads.insert(
            id.to_string(),
            DiscogsEffnetHeadResult {
                model: id.to_string(),
                status: "completed".into(),
                version: "discogs-test".into(),
                labels: vec![AnalysisLabel {
                    label: label.into(),
                    confidence: 0.9,
                }],
                scores: BTreeMap::from([(label.to_string(), 0.9)]),
                frame_count: 3,
                threshold: Some(0.35),
                selected_class: Some(label.into()),
                selected_confidence: Some(0.9),
                reason: None,
            },
        );
    }
    let base = TrackAnalysis {
        path: output.display().to_string(),
        title: "Discogs".into(),
        artist: "Artist".into(),
        album: String::new(),
        genre: String::new(),
        duration_seconds: Some(12.0),
        bpm: Some(124.0),
        key: Some("C".into()),
        scale: Some("major".into()),
        key_strength: None,
        integrated_loudness_lufs: None,
        loudness_range_lu: None,
        energy: Some(0.4),
        danceability: Some(0.25),
        beat_positions: Vec::new(),
        analyzed_at: String::new(),
        analyzer: "test".into(),
        analysis_version: "discogs-test".into(),
        source_size_bytes: None,
        source_modified_at: None,
        source_filename_format: None,
        drop_loudness_lufs: None,
        drop_analysis: None,
        high_level: Some(HighLevelAnalysis {
            status: "completed".into(),
            model_version: Some("discogs-test".into()),
            reason: None,
            genre: Vec::new(),
            style: vec![AnalysisLabel {
                label: "House".into(),
                confidence: 0.8,
            }],
            mood: Vec::new(),
            instrument: Vec::new(),
            emotion_candidates: None,
            mood_cluster: Vec::new(),
            mood_cluster_status: None,
            mood_cluster_reason: None,
            filtered: Vec::new(),
            discogs_effnet: Some(DiscogsEffnetAnalysis {
                embedding_model: "discogs-effnet-bs64-1".into(),
                embedding_dimensions: 1280,
                input_shape: vec![64, 128, 96],
                heads,
            }),
        }),
    };
    library
        .apply_analysis_for_destination(&output, &base)
        .unwrap();
    let detail = library.track_detail(&track_key).unwrap().unwrap();
    assert!(detail.discogs_mood_theme_json.contains("dark"));
    assert!(detail.discogs_approachability_json.contains("approachable"));
    assert!(detail.discogs_instrumentation_json.contains("synthesizer"));
    assert!(detail.discogs_timbre_json.contains("bright"));
    assert!(detail.discogs_danceability_json.contains("danceable"));
    assert_eq!(detail.style_json, r#"[{"label":"House","confidence":0.8}]"#);
    assert_eq!(detail.danceability, Some(0.25));

    let mut failed = base.clone();
    if let Some(high_level) = failed.high_level.as_mut()
        && let Some(discogs) = high_level.discogs_effnet.as_mut()
    {
        discogs.heads.get_mut("moodTheme").unwrap().status = "failed".into();
        discogs.heads.get_mut("danceability").unwrap().status = "model_missing".into();
    }
    library
        .apply_analysis_for_destination(&output, &failed)
        .unwrap();
    let after = library.track_detail(&track_key).unwrap().unwrap();
    assert!(after.discogs_mood_theme_json.contains("dark"));
    assert!(after.discogs_danceability_json.contains("danceable"));
}
