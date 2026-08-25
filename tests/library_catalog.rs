use rusqlite::Connection;
use std::path::Path;

use w4dj::library_catalog::{
    CATALOG_SCHEMA_VERSION, CatalogLocalFile, CatalogSnapshot, CatalogSource, CatalogSourceRecord,
    CatalogTrack, DurationSource, LibraryCatalog, LocalStatus, effective_duration,
};
use w4dj::library_query::{
    FilterLogic, LibraryField, LibraryFilter, LibraryOperator, LibraryQuery, LibrarySort,
    SortDirection,
};
use w4dj::media_probe::probe_local_audio;

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
fn damaged_private_catalog_is_backed_up_and_rebuilt() {
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
