use std::fs;
use std::path::PathBuf;

use tempfile::tempdir;
use w4dj::dj_playlist::{ImportedDjPlaylist, ImportedDjPlaylistTrack};
use w4dj::m3u8::{
    M3u8Error, ResolvedDjPlaylistTrack, build_relative_m3u8, build_relative_m3u8_with_summary,
    write_relative_m3u8_atomic,
};

fn playlist(tracks: Vec<ImportedDjPlaylistTrack>) -> ImportedDjPlaylist {
    ImportedDjPlaylist {
        playlist_id: "playlist-1".to_string(),
        format_version: 2,
        name: "测试歌单".to_string(),
        source_path: None,
        imported_at_ms: None,
        tracks,
        warnings: Vec::new(),
    }
}

fn track(position: u64, title: &str, artist: &str) -> ImportedDjPlaylistTrack {
    ImportedDjPlaylistTrack {
        position,
        title: title.to_string(),
        artist_display: artist.to_string(),
        dedupe_key: format!("key-{position}"),
        netease_import_line: format!("{title} - {artist}"),
    }
}

fn resolved(
    position: u64,
    title: &str,
    artist: &str,
    duration_seconds: Option<f64>,
    path: PathBuf,
) -> ResolvedDjPlaylistTrack {
    ResolvedDjPlaylistTrack {
        position,
        title: title.to_string(),
        artist_display: artist.to_string(),
        duration_seconds,
        destination_path: path,
    }
}

#[test]
fn renders_ordered_utf8_relative_extended_m3u8() {
    let root = tempdir().unwrap();
    let music = root.path().join("音乐 # 目录");
    fs::create_dir_all(&music).unwrap();
    let first = music.join("第一首 #.mp3");
    let second = music.join("第二首 🎵.mp3");
    fs::write(&first, b"audio").unwrap();
    fs::write(&second, b"audio").unwrap();
    let playlist_path = root.path().join("exports").join("测试.m3u8");
    fs::create_dir_all(playlist_path.parent().unwrap()).unwrap();

    let contents = build_relative_m3u8(
        &playlist(vec![
            track(2, "第二\n首", "歌手 2"),
            track(1, "第一首", "歌手 1"),
        ]),
        &[
            resolved(1, "第一首", "歌手 1", Some(90.6), first),
            resolved(2, "第二\n首", "歌手 2", Some(91.4), second),
        ],
        &playlist_path,
    )
    .unwrap();

    assert_eq!(
        contents,
        "#EXTM3U\n#EXTINF:91,歌手 1 - 第一首\n../音乐 # 目录/第一首 #.mp3\n#EXTINF:91,歌手 2 - 第二 首\n../音乐 # 目录/第二首 🎵.mp3"
    );
    assert!(!contents.starts_with('\u{feff}'));
    assert!(!contents.contains('\r'));
}

#[test]
fn rejects_incomplete_export_instead_of_reporting_a_partial_success() {
    let root = tempdir().unwrap();
    let path = root.path().join("one.mp3");
    fs::write(&path, b"audio").unwrap();
    let playlist = playlist(vec![track(1, "One", "A"), track(2, "Two", "B")]);
    let resolved_tracks = vec![resolved(1, "One", "A", None, path)];

    let error =
        build_relative_m3u8(&playlist, &resolved_tracks, &root.path().join("x.m3u8")).unwrap_err();
    assert!(matches!(error, M3u8Error::Incomplete { .. }));

    let error =
        build_relative_m3u8_with_summary(&playlist, &resolved_tracks, &root.path().join("x.m3u8"))
            .unwrap_err();
    assert!(matches!(error, M3u8Error::Incomplete { .. }));
}

#[test]
fn missing_output_is_never_silently_omitted_from_complete_export() {
    let root = tempdir().unwrap();
    let playlist = playlist(vec![track(1, "Missing", "A")]);
    let result = build_relative_m3u8(
        &playlist,
        &[resolved(
            1,
            "Missing",
            "A",
            None,
            root.path().join("gone.mp3"),
        )],
        &root.path().join("x.m3u8"),
    );
    assert!(matches!(result, Err(M3u8Error::Incomplete { .. })));
    assert!(matches!(
        build_relative_m3u8(
            &playlist,
            &[resolved(
                1,
                "Missing",
                "A",
                None,
                root.path().join("gone.mp3"),
            )],
            &root.path().join("x.m3u8"),
        ),
        Err(M3u8Error::Incomplete { .. })
    ));
}

#[test]
fn preserves_repeated_positions_for_the_same_output_path() {
    let root = tempdir().unwrap();
    let path = root.path().join("same.mp3");
    fs::write(&path, b"audio").unwrap();
    let playlist = playlist(vec![track(1, "Same", "Artist"), track(2, "Same", "Artist")]);

    let contents = build_relative_m3u8(
        &playlist,
        &[
            resolved(1, "Same", "Artist", None, path.clone()),
            resolved(2, "Same", "Artist", None, path),
        ],
        &root.path().join("playlist.m3u8"),
    )
    .unwrap();

    assert_eq!(contents.matches("#EXTINF:-1,Artist - Same").count(), 2);
    assert_eq!(contents.matches("same.mp3").count(), 2);
}

#[test]
fn writes_utf8_m3u8_atomically_and_rejects_wrong_path() {
    let root = tempdir().unwrap();
    let path = root.path().join("playlist.m3u8");
    write_relative_m3u8_atomic(&path, "#EXTM3U\n#EXTINF:-1,歌手 - 歌名\n音频.mp3").unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "#EXTM3U\n#EXTINF:-1,歌手 - 歌名\n音频.mp3"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
    assert!(matches!(
        write_relative_m3u8_atomic(&root.path().join("playlist.txt"), "#EXTM3U"),
        Err(M3u8Error::InvalidPath(_))
    ));
    assert!(matches!(
        write_relative_m3u8_atomic(&root.path().join("playlist.m3u8"), "#EXTM3U\r\n"),
        Err(M3u8Error::InvalidContent(_))
    ));
}
