use rusqlite::Connection;
use std::path::Path;

use w4dj::analysis::{AnalysisLabel, HighLevelAnalysis, TrackAnalysis};
use w4dj::library_catalog::{
    CATALOG_SCHEMA_VERSION, CatalogLocalFile, CatalogSnapshot, CatalogSource, CatalogSourceRecord,
    CatalogTrack, DurationSource, LibraryCatalog, LocalStatus, effective_duration,
};
use w4dj::library_query::{
    FilterLogic, LibraryField, LibraryFilter, LibraryOperator, LibraryQuery, LibrarySort,
    SortDirection,
};
use w4dj::media_probe::probe_local_audio;
use w4dj::metadata::{
    MetadataWriteProfile, SourceMetadata, build_output_metadata, split_supported_fields,
};

#[test]
fn analysis_wire_accepts_null_unavailable_filtered_confidence() {
    let wire = serde_json::json!({
        "path": "/music/song.mp3",
        "title": "Song",
        "artist": "Artist",
        "album": "",
        "genre": "",
        "durationSeconds": 180.0,
        "bpm": 120.0,
        "key": "C",
        "scale": "major",
        "keyStrength": null,
        "integratedLoudnessLufs": -12.0,
        "loudnessRangeLu": null,
        "energy": 0.4,
        "danceability": 0.7,
        "beatPositions": [],
        "analyzedAt": "2026-08-24T00:00:00Z",
        "analyzer": "Essentia.js",
        "analysisVersion": "0.2.0",
        "highLevel": {
            "status": "failed",
            "filtered": [{
                "label": "genre_discogs400",
                "confidence": null,
                "reason": "model missing"
            }]
        }
    });
    let parsed: TrackAnalysis = serde_json::from_value(wire).unwrap();
    assert_eq!(parsed.high_level.unwrap().filtered[0].confidence, None);
}

#[test]
fn catalog_keeps_netease_and_essentia_genres_separate() {
    let directory = tempfile::tempdir().unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("library.sqlite3")).unwrap();
    catalog.migrate().unwrap();
    let track = CatalogTrack {
        track_key: "netease:28712318".to_string(),
        netease_genre: "J-Pop".to_string(),
        essentia_genre: "City Pop".to_string(),
        local_status: LocalStatus::DatabaseOnly,
        ..CatalogTrack::default()
    };
    catalog.upsert_track(&track).unwrap();

    let stored = catalog.track_detail("netease:28712318").unwrap().unwrap();
    assert_eq!(stored.netease_genre, "J-Pop");
    assert_eq!(stored.essentia_genre, "City Pop");
    assert_eq!(catalog.count_tracks().unwrap(), 1);
}

#[test]
fn catalog_round_trips_instrument_and_acoustic_electronic_labels() {
    let directory = tempfile::tempdir().unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("library.sqlite3")).unwrap();
    catalog.migrate().unwrap();
    let track = CatalogTrack {
        track_key: "analysis:labels".to_string(),
        instrument_json: r#"[{"label":"instrumental","confidence":0.91}]"#.to_string(),
        style_json:
            r#"[{"label":"acoustic","confidence":0.84},{"label":"electronic","confidence":0.8}]"#
                .to_string(),
        local_status: LocalStatus::Available,
        ..CatalogTrack::default()
    };
    catalog.upsert_track(&track).unwrap();

    let stored = catalog.track_detail("analysis:labels").unwrap().unwrap();
    assert_eq!(stored.instrument_json, track.instrument_json);
    assert_eq!(stored.style_json, track.style_json);
}

#[test]
fn catalog_creates_the_private_schema_without_touching_source_paths() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested/library.sqlite3");
    let mut catalog = LibraryCatalog::open(&path).unwrap();
    catalog.migrate().unwrap();
    assert_eq!(catalog.path(), Path::new(&path));
    assert!(path.exists());
    assert_eq!(CATALOG_SCHEMA_VERSION, 1);
}

#[test]
fn damaged_private_catalog_cache_is_backed_up_and_rebuilt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("library-dashboard.sqlite3");
    std::fs::write(&path, b"not a sqlite database").unwrap();

    let (mut catalog, backup) = LibraryCatalog::open_or_recover(&path).unwrap();
    assert!(backup.as_ref().is_some_and(|value| value.is_file()));
    catalog.migrate().unwrap();
    assert_eq!(catalog.count_tracks().unwrap(), 0);
    assert!(path.is_file());
}

#[test]
fn effective_duration_prefers_essentia_then_measured_then_netease() {
    assert_eq!(
        effective_duration(Some(180.0), Some(181.0), Some(182.0)),
        (Some(180.0), Some(DurationSource::Essentia))
    );
    assert_eq!(
        effective_duration(None, Some(181.0), Some(182.0)),
        (Some(181.0), Some(DurationSource::Measured))
    );
    assert_eq!(
        effective_duration(Some(0.0), None, Some(182.0)),
        (Some(182.0), Some(DurationSource::Netease))
    );
    assert_eq!(effective_duration(None, None, None), (None, None));
}

#[test]
fn media_probe_rejects_unknown_or_empty_files_without_using_extension() {
    let directory = tempfile::tempdir().unwrap();
    let unknown = directory.path().join("looks-like.flac");
    std::fs::write(&unknown, b"not an audio file").unwrap();
    assert!(probe_local_audio(&unknown).is_err());

    let empty = directory.path().join("empty.mp3");
    std::fs::write(&empty, []).unwrap();
    assert!(probe_local_audio(&empty).is_err());
}

#[test]
fn metadata_profiles_keep_netease_identity_separate_from_analysis() {
    let source = SourceMetadata {
        title: "FRAGILE".to_string(),
        artists: vec!["山下達郎".to_string(), "Guest".to_string()],
        album: "COZY".to_string(),
        genre: "J-Pop".to_string(),
        aliases: "[\"fragile\"]".to_string(),
        ..SourceMetadata::default()
    };
    let core = build_output_metadata(MetadataWriteProfile::NcmCore, &source, None, None);
    assert_eq!(
        core.fields.get("artist").map(String::as_str),
        Some("山下達郎, Guest")
    );
    assert!(!core.fields.contains_key("genre"));
    let enriched = build_output_metadata(MetadataWriteProfile::Enriched, &source, None, None);
    let (written, unsupported) = split_supported_fields(&enriched, "flac");
    assert!(written.contains(&"genre".to_string()));
    assert!(unsupported.is_empty());
}

#[test]
fn catalog_query_supports_text_numeric_filter_sort_and_paging() {
    let directory = tempfile::tempdir().unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("library.sqlite3")).unwrap();
    catalog.migrate().unwrap();
    for (key, title, artist, bitrate) in [
        ("netease:1", "First", "Tyla", 320_000),
        ("netease:2", "Second", "Other", 128_000),
    ] {
        let track = CatalogTrack {
            track_key: key.to_string(),
            title: title.to_string(),
            artists: artist.to_string(),
            effective_bitrate_bps: Some(bitrate),
            local_status: LocalStatus::Available,
            ..CatalogTrack::default()
        };
        catalog.upsert_track(&track).unwrap();
    }
    let page = catalog
        .query(&LibraryQuery {
            text: "Tyla".to_string(),
            filters: vec![LibraryFilter {
                field: LibraryField::Bitrate,
                operator: LibraryOperator::GreaterOrEqual,
                value: Some("320000".to_string()),
                second_value: None,
            }],
            filter_logic: FilterLogic::And,
            sorts: vec![LibrarySort {
                field: LibraryField::Title,
                direction: SortDirection::Asc,
            }],
            limit: 100,
            offset: 0,
        })
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].track_key, "netease:1");
}

#[test]
fn catalog_query_matches_multi_character_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("library.sqlite3")).unwrap();
    catalog.migrate().unwrap();
    catalog
        .upsert_track(&CatalogTrack {
            track_key: "netease:alias".to_string(),
            title: "嘻勋 (Ulei)".to_string(),
            aliases_json: "[\"弹舌\"]".to_string(),
            ..CatalogTrack::default()
        })
        .unwrap();

    let page = catalog
        .query(&LibraryQuery {
            text: "弹舌".to_string(),
            ..LibraryQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].track_key, "netease:alias");
}

#[test]
fn analysis_projection_is_the_dashboard_source_and_can_be_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let song_path = directory.path().join("Song.mp3");
    std::fs::write(&song_path, b"analysis fixture").unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    catalog.migrate().unwrap();

    let entry = TrackAnalysis {
        path: song_path.to_string_lossy().into_owned(),
        title: "Song".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        genre: "Embedded genre".into(),
        duration_seconds: Some(180.0),
        bpm: Some(124.0),
        key: Some("F".into()),
        scale: Some("minor".into()),
        key_strength: Some(0.9),
        integrated_loudness_lufs: Some(-9.0),
        loudness_range_lu: Some(4.0),
        energy: Some(0.8),
        danceability: Some(1.2),
        beat_positions: vec![0.0, 0.5],
        analyzed_at: "2026-08-19T00:00:00Z".into(),
        analyzer: "Essentia.js".into(),
        analysis_version: "test".into(),
        source_size_bytes: Some(16),
        source_modified_at: None,
        source_filename_format: None,
        drop_loudness_lufs: Some(-7.0),
        drop_analysis: None,
        high_level: Some(HighLevelAnalysis {
            status: "completed".into(),
            model_version: Some("test".into()),
            reason: None,
            genre: vec![AnalysisLabel {
                label: "House".into(),
                confidence: 0.95,
            }],
            style: vec![
                AnalysisLabel {
                    label: "acoustic".into(),
                    confidence: 0.84,
                },
                AnalysisLabel {
                    label: "electronic".into(),
                    confidence: 0.8,
                },
            ],
            mood: vec![],
            instrument: vec![AnalysisLabel {
                label: "instrumental".into(),
                confidence: 0.91,
            }],
            emotion_candidates: None,
            mood_cluster: Vec::new(),
            mood_cluster_status: None,
            mood_cluster_reason: None,
            filtered: vec![],
            discogs_effnet: None,
        }),
    };

    catalog.replace_analysis_entries(&[entry]).unwrap();
    assert_eq!(catalog.count_tracks().unwrap(), 1);
    assert_eq!(catalog.count_analyzed_tracks().unwrap(), 1);
    let page = catalog.query_analyzed(&LibraryQuery::default()).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].netease_track_id, None);
    assert_eq!(page.items[0].essentia_genre, "House");
    assert_eq!(page.items[0].title, "Song");
    assert!(page.items[0].instrument_json.contains("instrumental"));
    assert!(page.items[0].style_json.contains("acoustic"));
    assert!(page.items[0].style_json.contains("electronic"));

    catalog.replace_analysis_entries(&[]).unwrap();
    assert_eq!(catalog.count_analyzed_tracks().unwrap(), 0);
    assert_eq!(
        catalog
            .query_analyzed(&LibraryQuery::default())
            .unwrap()
            .total,
        0
    );
    assert!(
        catalog
            .track_detail(&page.items[0].track_key)
            .unwrap()
            .is_none()
    );
}

fn completed_analysis(path: &Path, title: &str) -> TrackAnalysis {
    TrackAnalysis {
        path: path.to_string_lossy().into_owned(),
        title: title.into(),
        artist: "Artist".into(),
        album: "Album".into(),
        genre: "House".into(),
        duration_seconds: Some(180.0),
        bpm: Some(124.0),
        key: Some("F".into()),
        scale: Some("minor".into()),
        key_strength: Some(0.9),
        integrated_loudness_lufs: Some(-9.0),
        loudness_range_lu: Some(4.0),
        energy: Some(0.8),
        danceability: Some(1.2),
        beat_positions: vec![0.0, 0.5],
        analyzed_at: "2026-08-19T00:00:00Z".into(),
        analyzer: "Essentia.js".into(),
        analysis_version: "test".into(),
        source_size_bytes: Some(16),
        source_modified_at: None,
        source_filename_format: None,
        drop_loudness_lufs: Some(-7.0),
        drop_analysis: None,
        high_level: Some(HighLevelAnalysis {
            status: "completed".into(),
            model_version: Some("test".into()),
            reason: None,
            genre: vec![AnalysisLabel {
                label: "House".into(),
                confidence: 0.95,
            }],
            style: Vec::new(),
            mood: vec![],
            instrument: vec![],
            emotion_candidates: None,
            mood_cluster: Vec::new(),
            mood_cluster_status: None,
            mood_cluster_reason: None,
            filtered: vec![],
            discogs_effnet: None,
        }),
    }
}

#[test]
fn relocate_analyzed_track_rebinds_only_the_private_local_file() {
    let directory = tempfile::tempdir().unwrap();
    let old_path = directory.path().join("old.mp3");
    let new_path = directory.path().join("new.flac");
    std::fs::write(&old_path, b"old analysis fixture").unwrap();
    std::fs::write(&new_path, b"new replacement fixture").unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    catalog.migrate().unwrap();
    let entry = completed_analysis(&old_path, "Keep analysis");
    catalog
        .replace_analysis_entries(std::slice::from_ref(&entry))
        .unwrap();
    let key = catalog
        .query_analyzed(&LibraryQuery::default())
        .unwrap()
        .items[0]
        .track_key
        .clone();

    catalog.relocate_analyzed_track(&key, &new_path).unwrap();
    let track = catalog.track_detail(&key).unwrap().unwrap();
    assert_eq!(track.title, "Keep analysis");
    assert_eq!(track.essentia_duration_seconds, Some(180.0));
    assert_eq!(track.effective_format.as_deref(), Some("flac"));
    assert!(catalog.local_file_by_path(&old_path).unwrap().is_none());
    assert_eq!(
        catalog.local_files_for_track(&key).unwrap()[0].path,
        new_path
    );
    assert!(old_path.is_file());
}

#[test]
fn removing_analyzed_track_only_removes_the_w4dj_projection() {
    let directory = tempfile::tempdir().unwrap();
    let song_path = directory.path().join("song.mp3");
    std::fs::write(&song_path, b"keep local file").unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    catalog.migrate().unwrap();
    let entry = completed_analysis(&song_path, "Remove me");
    catalog
        .replace_analysis_entries(std::slice::from_ref(&entry))
        .unwrap();
    let key = catalog
        .query_analyzed(&LibraryQuery::default())
        .unwrap()
        .items[0]
        .track_key
        .clone();

    assert!(catalog.remove_analyzed_track(&key).unwrap());
    assert!(!catalog.remove_analyzed_track(&key).unwrap());
    assert!(catalog.track_detail(&key).unwrap().is_none());
    assert!(catalog.local_file_by_path(&song_path).unwrap().is_none());
    assert!(song_path.is_file());
}

#[test]
fn bulk_invalid_cleanup_removes_missing_rows_after_confirmation() {
    let directory = tempfile::tempdir().unwrap();
    let available_path = directory.path().join("available.mp3");
    let missing_path = directory.path().join("missing.mp3");
    std::fs::write(&available_path, b"available fixture").unwrap();
    let mut catalog = LibraryCatalog::open(&directory.path().join("w4dj.sqlite3")).unwrap();
    catalog.migrate().unwrap();
    let entries = [
        completed_analysis(&available_path, "Available"),
        completed_analysis(&missing_path, "Missing"),
    ];
    catalog.replace_analysis_entries(&entries).unwrap();
    assert_eq!(catalog.count_analyzed_tracks().unwrap(), 2);

    assert_eq!(catalog.remove_invalid_analyzed_tracks().unwrap(), 1);
    assert_eq!(catalog.count_analyzed_tracks().unwrap(), 1);
    assert_eq!(
        catalog
            .query_analyzed(&LibraryQuery::default())
            .unwrap()
            .items[0]
            .title,
        "Available"
    );
    assert!(!missing_path.is_file());
}

#[test]
fn netease_snapshot_projects_extended_metadata_without_touching_source_database() {
    let directory = tempfile::tempdir().unwrap();
    let database_path = directory.path().join("sqlite_storage.sqlite3");
    let music_path = directory.path().join("网易云音乐");
    std::fs::create_dir_all(&music_path).unwrap();
    let song_path = music_path.join("Song - Artist.mp3");
    std::fs::write(&song_path, b"not-a-real-mp3").unwrap();

    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE track (
                file TEXT, title TEXT, artist TEXT, album TEXT,
                filesize INTEGER, duration INTEGER, tid INTEGER, album_id INTEGER,
                cover_path TEXT, detail TEXT
            );",
        )
        .unwrap();
    let detail = serde_json::json!({
        "track": {
            "id": 28712318,
            "name": "Song",
            "artists": [{"name": "Artist"}],
            "album": {"id": 7, "name": "Album"},
            "genre": "Electronic",
            "alias": ["Radio edit"],
            "publishTime": 1704067200000_i64,
            "copyright": "Licensed",
            "lyric": "[00:01.00]Hello\n[00:02.00]World",
            "tlyric": {"lyric": "[00:01.00]你好"}
        }
    })
    .to_string();
    connection
        .execute(
            "INSERT INTO track (file,title,artist,album,filesize,duration,tid,album_id,cover_path,detail)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![
                song_path.to_string_lossy().as_ref(),
                "Song",
                "Artist",
                "Album",
                14_i64,
                180_000_i64,
                28712318_i64,
                7_i64,
                "28712318.jpg",
                detail,
            ],
        )
        .unwrap();

    let snapshot =
        w4dj::netease_library::build_catalog_snapshot(&database_path, Some(&music_path)).unwrap();
    let track = snapshot
        .tracks
        .iter()
        .find(|track| track.netease_track_id.as_deref() == Some("28712318"))
        .unwrap();
    assert_eq!(track.netease_genre, "Electronic");
    assert_eq!(track.aliases_json, "[\"Radio edit\"]");
    assert_eq!(track.lyric_plain_text, "Hello\nWorld");
    assert_eq!(track.lyric_translated_text, "你好");
    assert_eq!(track.lyric_sync_type, "timed");
    assert_eq!(snapshot.local_files.len(), 1);
    assert!(!snapshot.local_files[0].readable);
    let before = std::fs::metadata(&database_path).unwrap();
    assert!(before.len() > 0);
}

#[test]
fn snapshot_marks_local_files_missing_when_they_disappear() {
    let directory = tempfile::tempdir().unwrap();
    let database_path = directory.path().join("library.sqlite3");
    let local_path = directory.path().join("Song.mp3");
    std::fs::write(&local_path, b"not-a-real-mp3").unwrap();
    let mut catalog = LibraryCatalog::open(&database_path).unwrap();
    catalog.migrate().unwrap();

    let track = CatalogTrack {
        track_key: "netease:1".into(),
        title: "Song".into(),
        local_status: LocalStatus::Available,
        ..CatalogTrack::default()
    };
    let source = CatalogSource {
        database_path: directory.path().join("source.sqlite"),
        database_size_bytes: 1,
        database_modified_at_ms: None,
        last_imported_at_ms: 1,
        import_status: "success".into(),
        last_error: None,
    };
    let local_file = CatalogLocalFile {
        id: None,
        track_key: track.track_key.clone(),
        path: local_path,
        size_bytes: 1,
        modified_at_ms: None,
        measured_format: Some("mp3".into()),
        measured_bitrate_bps: None,
        measured_duration_seconds: None,
        sample_rate_hz: None,
        channels: None,
        readable: true,
        probe_error: None,
    };
    catalog
        .upsert_snapshot(&CatalogSnapshot {
            tracks: vec![track.clone()],
            local_files: vec![local_file],
            source_records: vec![],
            sources: vec![source.clone()],
        })
        .unwrap();
    assert_eq!(
        catalog
            .track_detail(&track.track_key)
            .unwrap()
            .unwrap()
            .local_status,
        LocalStatus::Available
    );

    catalog
        .upsert_snapshot(&CatalogSnapshot {
            tracks: vec![track],
            local_files: vec![],
            source_records: vec![],
            sources: vec![source],
        })
        .unwrap();
    assert_eq!(
        catalog
            .track_detail("netease:1")
            .unwrap()
            .unwrap()
            .local_status,
        LocalStatus::Missing
    );
}

#[test]
fn cancelled_snapshot_commit_keeps_the_previous_catalog() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("library.sqlite3");
    let mut catalog = LibraryCatalog::open(&path).unwrap();
    catalog.migrate().unwrap();
    catalog
        .upsert_track(&CatalogTrack {
            track_key: "netease:old".into(),
            title: "Old".into(),
            ..CatalogTrack::default()
        })
        .unwrap();

    let mut checks = 0;
    let result = catalog.upsert_snapshot_with_analysis(
        &CatalogSnapshot {
            tracks: vec![CatalogTrack {
                track_key: "netease:new".into(),
                title: "New".into(),
                ..CatalogTrack::default()
            }],
            ..CatalogSnapshot::default()
        },
        &[],
        || {
            checks += 1;
            checks > 1
        },
        |_, _, _| {},
    );

    assert!(result.is_err());
    assert!(catalog.track_detail("netease:old").unwrap().is_some());
    assert!(catalog.track_detail("netease:new").unwrap().is_none());
}

#[test]
fn source_records_can_be_loaded_for_a_track_without_exposing_other_tracks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("library.sqlite3");
    let mut catalog = LibraryCatalog::open(&path).unwrap();
    catalog.migrate().unwrap();
    let first = CatalogTrack {
        track_key: "netease:1".into(),
        title: "First".into(),
        ..CatalogTrack::default()
    };
    let second = CatalogTrack {
        track_key: "netease:2".into(),
        title: "Second".into(),
        ..CatalogTrack::default()
    };
    catalog
        .upsert_snapshot(&CatalogSnapshot {
            tracks: vec![first, second],
            source_records: vec![
                CatalogSourceRecord {
                    track_key: "netease:1".into(),
                    source_table: "track".into(),
                    source_primary_key: "1".into(),
                    source_version: Some("v1".into()),
                    raw_json: r#"{"title":"First"}"#.into(),
                    imported_at_ms: 1,
                },
                CatalogSourceRecord {
                    track_key: "netease:2".into(),
                    source_table: "track".into(),
                    source_primary_key: "2".into(),
                    source_version: Some("v1".into()),
                    raw_json: r#"{"title":"Second"}"#.into(),
                    imported_at_ms: 1,
                },
            ],
            ..CatalogSnapshot::default()
        })
        .unwrap();

    let records = catalog.source_records_for_track("netease:1").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source_primary_key, "1");
    assert!(
        catalog
            .source_records_for_track("netease:missing")
            .unwrap()
            .is_empty()
    );
}
