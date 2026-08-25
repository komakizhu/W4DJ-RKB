use std::path::Path;

use w4dj::dj_playlist::{
    DjPlaylistError, W4DJ_PLAYLIST_MAX_BYTES, netease_import_line, parse_w4dj_playlist,
    serialize_w4dj_playlist,
};
use w4dj::w4dj_library::W4djLibrary;

fn minimal_playlist() -> serde_json::Value {
    serde_json::json!({
        "format": "w4dj",
        "format_version": 2,
        "export_id": "playlist-1",
        "playlist": {"name": "Club"},
        "tracks": [
            {
                "position": 2,
                "title": " Second\nSong ",
                "artist_display": "Artist\tTwo",
                "netease_track_id": "123456789012345678"
            },
            {
                "position": 1,
                "title": " First ",
                "artist_display": "Artist One"
            }
        ]
    })
}

fn parse_value(
    value: serde_json::Value,
) -> Result<w4dj::dj_playlist::ImportedDjPlaylist, DjPlaylistError> {
    parse_w4dj_playlist(&serde_json::to_vec(&value).unwrap(), None)
}

#[test]
fn parses_minimal_v2_and_preserves_string_id() {
    let playlist = parse_value(minimal_playlist()).unwrap();
    assert_eq!(playlist.playlist_id, "playlist-1");
    assert_eq!(playlist.format_version, 2);
    assert_eq!(playlist.tracks.len(), 2);
    assert_eq!(playlist.tracks[0].position, 1);
    assert_eq!(playlist.tracks[0].netease_track_id, None);
    assert_eq!(
        playlist.tracks[1].netease_track_id.as_deref(),
        Some("123456789012345678")
    );
    assert_eq!(playlist.tracks[0].netease_import_line, "First - Artist One");
    assert_eq!(
        playlist.tracks[1].netease_import_line,
        "Second Song - Artist Two"
    );
    assert_eq!(playlist.tracks[1].dedupe_key, "netease:123456789012345678");
    assert_eq!(playlist.source_path, None);
}

#[test]
fn rejects_v1_without_migration() {
    let mut value = minimal_playlist();
    value["format_version"] = serde_json::json!(1);
    assert!(matches!(
        parse_value(value),
        Err(DjPlaylistError::UnsupportedVersion(1))
    ));
}

#[test]
fn rejects_legacy_fields_and_unknown_fields_in_v2() {
    for field in [
        "created_at",
        "output_mode",
        "record_id",
        "artists",
        "album_or_ep",
        "duration",
        "bpm",
        "musical_key",
        "platform_refs",
        "dedupe_key",
        "expected_filename_hint",
    ] {
        let mut value = minimal_playlist();
        if field == "created_at" {
            value["created_at"] = serde_json::json!("2026-08-25");
        } else {
            value["tracks"][0][field] = serde_json::json!("legacy");
        }
        assert!(
            matches!(parse_value(value), Err(DjPlaylistError::InvalidJson(_))),
            "legacy or unknown field should be rejected: {field}"
        );
    }
}

#[test]
fn rejects_non_string_netease_id() {
    let mut value = minimal_playlist();
    value["tracks"][0]["netease_track_id"] = serde_json::json!(3409113568_u64);
    assert!(matches!(
        parse_value(value),
        Err(DjPlaylistError::InvalidField(_))
    ));

    let mut value = minimal_playlist();
    value["tracks"][0]["netease_track_id"] = serde_json::Value::Null;
    assert!(matches!(
        parse_value(value),
        Err(DjPlaylistError::InvalidField(_))
    ));

    for invalid_id in ["", " 42", "42 "] {
        let mut value = minimal_playlist();
        value["tracks"][0]["netease_track_id"] = serde_json::json!(invalid_id);
        assert!(matches!(
            parse_value(value),
            Err(DjPlaylistError::InvalidField(_))
        ));
    }
}

#[test]
fn preserves_same_track_id_at_distinct_positions() {
    let mut value = minimal_playlist();
    value["tracks"] = serde_json::json!([
        {
            "position": 1,
            "title": "Song",
            "artist_display": "Artist",
            "netease_track_id": "42"
        },
        {
            "position": 2,
            "title": "Song",
            "artist_display": "Artist",
            "netease_track_id": "42"
        }
    ]);
    let playlist = parse_value(value).unwrap();
    assert_eq!(playlist.tracks.len(), 2);
    assert_eq!(playlist.tracks[0].dedupe_key, "netease:42");
    assert_eq!(playlist.tracks[1].dedupe_key, "netease:42");
    assert!(playlist.warnings.is_empty());
}

#[test]
fn rejects_duplicate_positions_empty_required_fields_and_control_characters() {
    let mut value = minimal_playlist();
    value["tracks"][1]["position"] = serde_json::json!(2);
    assert!(matches!(
        parse_value(value),
        Err(DjPlaylistError::InvalidField(_))
    ));
    let mut value = minimal_playlist();
    value["tracks"][0]["title"] = serde_json::json!(" ");
    assert!(matches!(
        parse_value(value),
        Err(DjPlaylistError::InvalidField(_))
    ));
    let mut value = minimal_playlist();
    value["tracks"][0]["title"] = serde_json::json!("bad\u{0001}title");
    assert!(matches!(
        parse_value(value),
        Err(DjPlaylistError::InvalidField(_))
    ));
}

#[test]
fn serializes_only_minimal_v2_fields_and_round_trips() {
    let playlist = parse_value(minimal_playlist()).unwrap();
    let bytes = serialize_w4dj_playlist(&playlist).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["format_version"], 2);
    assert!(value.get("created_at").is_none());
    assert!(value["tracks"][0].get("dedupe_key").is_none());
    assert!(value["tracks"][0].get("platform_refs").is_none());
    let round_trip = parse_w4dj_playlist(&bytes, Some(Path::new("/tmp/list.w4dj"))).unwrap();
    assert_eq!(round_trip.tracks, playlist.tracks);
    assert_eq!(
        round_trip.source_path.as_deref(),
        Some(Path::new("/tmp/list.w4dj"))
    );
}

#[test]
fn rejects_oversized_and_malformed_input() {
    let oversized = vec![b' '; W4DJ_PLAYLIST_MAX_BYTES + 1];
    assert!(matches!(
        parse_w4dj_playlist(&oversized, None),
        Err(DjPlaylistError::TooLarge { .. })
    ));
    assert!(matches!(
        parse_w4dj_playlist(b"{not json", None),
        Err(DjPlaylistError::InvalidJson(_))
    ));
}

#[test]
fn builds_exact_netease_import_line() {
    assert_eq!(
        netease_import_line("  Title\n", "Artist\tTwo").unwrap(),
        "Title - Artist Two"
    );
}

#[test]
fn persists_v2_playlist_and_reimports_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("w4dj.sqlite3");
    let mut library = W4djLibrary::open(&path).unwrap();
    let playlist = parse_value(minimal_playlist()).unwrap();
    library.upsert_imported_dj_playlist(&playlist).unwrap();
    assert_eq!(
        library.list_imported_dj_playlists().unwrap()[0].track_count,
        2
    );
    let loaded = library
        .get_imported_dj_playlist("playlist-1")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.playlist_id, playlist.playlist_id);
    assert_eq!(loaded.name, playlist.name);
    assert_eq!(loaded.tracks, playlist.tracks);

    let mut replacement = playlist.clone();
    replacement.name = "Updated".to_string();
    replacement.tracks.pop();
    library.upsert_imported_dj_playlist(&replacement).unwrap();
    let loaded = library
        .get_imported_dj_playlist("playlist-1")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.name, "Updated");
    assert_eq!(loaded.tracks.len(), 1);

    let mut repeated = replacement.clone();
    let mut repeated_track = repeated.tracks[0].clone();
    repeated_track.position = 2;
    repeated.tracks.push(repeated_track);
    library.upsert_imported_dj_playlist(&repeated).unwrap();
    assert_eq!(
        library
            .get_imported_dj_playlist("playlist-1")
            .unwrap()
            .unwrap()
            .tracks
            .len(),
        2
    );

    let mut repeated_id = parse_value(minimal_playlist()).unwrap();
    let mut repeated_id_track = repeated_id.tracks[1].clone();
    repeated_id_track.position = 3;
    repeated_id.tracks.push(repeated_id_track);
    library.upsert_imported_dj_playlist(&repeated_id).unwrap();
    let loaded = library
        .get_imported_dj_playlist("playlist-1")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.tracks.len(), 3);
    assert_eq!(
        loaded.tracks[1].netease_track_id,
        loaded.tracks[2].netease_track_id
    );
}
