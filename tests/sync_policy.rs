#![allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[path = "../src/metadata.rs"]
mod metadata;
#[path = "../src/sync.rs"]
mod sync;
#[path = "../src/task.rs"]
mod task;

use config::{FilenameRule, LosslessFormat, Mode};
use id3::{TagLike, Version};
use ncmdump::NcmInfo;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use sync::{
    TargetProfile, cleanup_temporary_outputs, compare_music_dicts, get_destination_music_dict,
    resolve_output_policy, sync_music_library_with_policy,
};

#[test]
fn compat_mode_always_targets_mp3() {
    let policy = resolve_output_policy(Mode::Compat, None, "flac");
    assert_eq!(policy.output_extension, "mp3");
}

#[test]
fn lossless_mode_uses_requested_format() {
    let wav_policy = resolve_output_policy(Mode::Lossless, Some(LosslessFormat::Wav), "ncm");
    assert_eq!(wav_policy.output_extension, "wav");

    let aiff_policy = resolve_output_policy(Mode::Lossless, Some(LosslessFormat::Aiff), "ncm");
    assert_eq!(aiff_policy.output_extension, "aiff");
}

#[test]
fn lossless_mode_defaults_to_wav_when_format_missing() {
    let policy = resolve_output_policy(Mode::Lossless, None, "ncm");
    assert_eq!(policy.output_extension, "wav");
}

#[test]
fn lossless_mode_preserves_mp3_sources() {
    let policy = resolve_output_policy(Mode::Lossless, Some(LosslessFormat::Aiff), "mp3");
    assert_eq!(policy.output_extension, "mp3");
    assert!(matches!(policy.target_profile, TargetProfile::CompatMp3));
}

#[test]
fn compare_music_dicts_keeps_mp3_sources_when_destination_matches() {
    let mut wf_dict = HashMap::new();
    wf_dict.insert(
        "Song".to_string(),
        ("100".to_string(), PathBuf::from("/music/source/Song.mp3")),
    );

    let mut sf_dict = HashMap::new();
    sf_dict.insert(
        "Song".to_string(),
        ("4096".to_string(), PathBuf::from("/music/dest/Song.mp3")),
    );

    let diff = compare_music_dicts(
        &wf_dict,
        &sf_dict,
        &Mode::Lossless,
        Some(LosslessFormat::Aiff),
    );
    assert!(diff.is_empty());
}

#[test]
fn compare_music_dicts_skips_lossless_mp3_sources_when_a_lossless_output_already_exists() {
    let mut wf_dict = HashMap::new();
    wf_dict.insert(
        "Song".to_string(),
        ("100".to_string(), PathBuf::from("/music/source/Song.mp3")),
    );

    let mut sf_dict = HashMap::new();
    sf_dict.insert(
        "Song".to_string(),
        ("4096".to_string(), PathBuf::from("/music/dest/Song.wav")),
    );

    let diff = compare_music_dicts(
        &wf_dict,
        &sf_dict,
        &Mode::Lossless,
        Some(LosslessFormat::Aiff),
    );

    assert!(diff.is_empty());
}

#[test]
fn compare_music_dicts_still_regenerates_compat_mp3_when_destination_has_lossless_output() {
    let mut wf_dict = HashMap::new();
    wf_dict.insert(
        "Song".to_string(),
        ("100".to_string(), PathBuf::from("/music/source/Song.mp3")),
    );

    let mut sf_dict = HashMap::new();
    sf_dict.insert(
        "Song".to_string(),
        ("4096".to_string(), PathBuf::from("/music/dest/Song.wav")),
    );

    let diff = compare_music_dicts(&wf_dict, &sf_dict, &Mode::Compat, None);

    assert_eq!(diff.len(), 1);
}

#[test]
fn compare_music_dicts_rebuilds_zero_byte_destination_files() {
    let mut wf_dict = HashMap::new();
    wf_dict.insert(
        "Song".to_string(),
        ("100".to_string(), PathBuf::from("/music/source/Song.flac")),
    );

    let mut sf_dict = HashMap::new();
    sf_dict.insert(
        "Song".to_string(),
        ("0".to_string(), PathBuf::from("/music/dest/Song.aiff")),
    );

    let diff = compare_music_dicts(
        &wf_dict,
        &sf_dict,
        &Mode::Lossless,
        Some(LosslessFormat::Aiff),
    );

    assert_eq!(diff.len(), 1);
}

#[test]
fn get_music_dict_prefers_higher_quality_duplicate_stem() {
    let temp_dir = std::env::temp_dir().join(format!("w4dj-sync-policy-{}", std::process::id()));
    fs::create_dir_all(&temp_dir).unwrap();

    let mp3_path = temp_dir.join("same.mp3");
    let flac_path = temp_dir.join("same.flac");
    fs::write(&mp3_path, b"mp3").unwrap();
    fs::write(&flac_path, b"flac").unwrap();

    let dict = sync::get_music_dict(temp_dir.to_str().unwrap());
    let (_, selected_path) = dict.get("same").unwrap();

    assert_eq!(selected_path, &flac_path);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn get_music_dict_accepts_a_single_audio_file_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-single-file-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let source_path = temp_dir.join("single-track.flac");
    fs::write(&source_path, b"flac-placeholder").unwrap();

    let dict = sync::get_music_dict(source_path.to_str().unwrap());

    assert_eq!(dict.len(), 1);
    assert_eq!(dict.get("single-track").unwrap().1, source_path);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn get_music_dict_ignores_macos_appledouble_sidecars() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-appledouble-ignore-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let source_path = temp_dir.join("Song.flac");
    let sidecar_path = temp_dir.join("._Song.flac");
    fs::write(&source_path, b"real-audio-placeholder").unwrap();
    fs::write(&sidecar_path, b"\x00\x05\x16\x07macos-metadata").unwrap();

    let dict = sync::get_music_dict(temp_dir.to_str().unwrap());

    assert_eq!(dict.len(), 1);
    assert_eq!(dict.get("Song").unwrap().1, source_path);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn get_music_dict_keeps_non_appledouble_track_with_dot_underscore_name() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-dot-underscore-track-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let source_path = temp_dir.join("._Song.flac");
    fs::write(&source_path, b"real-audio-placeholder").unwrap();

    let dict = sync::get_music_dict(temp_dir.to_str().unwrap());

    assert_eq!(dict.len(), 1);
    assert!(dict.values().any(|(_, path)| path == &source_path));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn get_music_dict_prefers_wav_over_mp3_for_same_stem() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-wav-over-mp3-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let mp3_path = temp_dir.join("same.mp3");
    let wav_path = temp_dir.join("same.wav");
    fs::write(&mp3_path, b"mp3").unwrap();
    fs::write(&wav_path, b"wav-data").unwrap();

    let dict = sync::get_music_dict(temp_dir.to_str().unwrap());
    let (_, selected_path) = dict.get("same").unwrap();

    assert_eq!(selected_path, &wav_path);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn destination_music_dict_ignores_temporary_w4dj_files() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-temp-ignore-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let final_path = temp_dir.join("same.wav");
    let temp_path = temp_dir.join(".w4dj-same.flac");
    fs::write(&final_path, b"final").unwrap();
    fs::write(&temp_path, b"temp").unwrap();

    let dict = get_destination_music_dict(temp_dir.to_str().unwrap());
    let (_, selected_path) = dict.get("same").unwrap();

    assert_eq!(selected_path, &final_path);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn destination_music_dict_ignores_macos_appledouble_sidecars() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-destination-appledouble-ignore-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let final_path = temp_dir.join("Song.wav");
    let sidecar_path = temp_dir.join("._Song.wav");
    fs::write(&final_path, b"final").unwrap();
    fs::write(&sidecar_path, b"\x00\x05\x16\x07macos-metadata").unwrap();

    let dict = get_destination_music_dict(temp_dir.to_str().unwrap());

    assert_eq!(dict.len(), 1);
    assert_eq!(dict.get("Song").unwrap().1, final_path);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn destination_music_dict_ignores_non_output_flac_files() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-ignore-flac-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let final_path = temp_dir.join("same.mp3");
    let ignored_path = temp_dir.join("same.flac");
    fs::write(&final_path, b"final").unwrap();
    fs::write(&ignored_path, b"ignored").unwrap();

    let dict = get_destination_music_dict(temp_dir.to_str().unwrap());
    let (_, selected_path) = dict.get("same").unwrap();

    assert_eq!(selected_path, &final_path);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn cleanup_temporary_outputs_never_deletes_user_files_by_prefix() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-temp-cleanup-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let temp_path = temp_dir.join(".w4dj-same.flac");
    fs::write(&temp_path, b"temp").unwrap();

    cleanup_temporary_outputs(temp_dir.to_str().unwrap()).unwrap();

    assert!(temp_path.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn cleanup_temporary_outputs_preserves_macos_appledouble_sidecars() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-preserve-appledouble-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let sidecar_path = temp_dir.join("._Song.wav");
    fs::write(&sidecar_path, b"\x00\x05\x16\x07macos-metadata").unwrap();

    cleanup_temporary_outputs(temp_dir.to_str().unwrap()).unwrap();

    assert!(sidecar_path.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn build_id3_tag_carries_cover_and_text() {
    let info = NcmInfo {
        album: "Album".into(),
        artist: vec![("Artist".into(), 0)],
        alias: None,
        bitrate: 320,
        duration: 180,
        format: "flac".into(),
        id: 42,
        name: "Song".into(),
        mv_id: None,
    };

    let tag = metadata::build_id3_tag(&info, &[0x89, 0x50, 0x4e, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

    assert_eq!(tag.title(), Some("Song"));
    assert_eq!(tag.album(), Some("Album"));
    assert_eq!(tag.artist(), Some("Artist"));
    assert_eq!(tag.pictures().count(), 1);
}

#[test]
fn build_id3_tag_from_flac_carries_cover_and_text() {
    let mut flac_tag = metaflac::Tag::new();
    flac_tag.vorbis_comments_mut().set_title(vec!["Song"]);
    flac_tag.vorbis_comments_mut().set_album(vec!["Album"]);
    flac_tag.vorbis_comments_mut().set_artist(vec!["Artist"]);
    flac_tag.add_picture(
        "image/png",
        metaflac::block::PictureType::CoverFront,
        vec![0x89, 0x50, 0x4e, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
    );

    let tag = metadata::build_id3_tag_from_flac(&flac_tag);

    assert_eq!(tag.title(), Some("Song"));
    assert_eq!(tag.album(), Some("Album"));
    assert_eq!(tag.artist(), Some("Artist"));
    assert_eq!(tag.pictures().count(), 1);
}

#[test]
fn compat_export_uses_tags_to_read_netease_artist_first_filename() {
    let root = tempfile::tempdir().unwrap();
    let source_dir = root.path().join("网易云音乐");
    let output_dir = root.path().join("output");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&output_dir).unwrap();

    let source_path = source_dir.join("KIMERU - Overlap.mp3");
    fs::write(&source_path, b"audio").unwrap();
    let mut source_tag = id3::Tag::new();
    source_tag.set_title("Overlap");
    source_tag.set_artist("KIMERU");
    source_tag.add_frame(id3::frame::ExtendedText {
        description: "163 key".into(),
        value: "netease-source".into(),
    });
    source_tag
        .write_to_path(&source_path, Version::Id3v24)
        .unwrap();

    let (source_music, issues) = sync::get_music_dict_with_scan_issues_with_rule(
        source_dir.to_str().unwrap(),
        FilenameRule::TitleArtist,
    );
    assert!(issues.is_empty());
    assert!(source_music.contains_key("Overlap - KIMERU"));

    let pending = source_music.iter().collect::<HashMap<_, _>>();
    sync_music_library_with_policy(&pending, output_dir.to_str().unwrap(), &Mode::Compat, None)
        .unwrap();

    let output_path = output_dir.join("Overlap - KIMERU.mp3");
    let output_tag = id3::Tag::read_from_path(output_path).unwrap();
    assert_eq!(output_tag.title(), Some("Overlap"));
    assert_eq!(output_tag.artist(), Some("KIMERU"));
}

#[test]
fn compat_export_uses_tags_when_netease_filename_only_contains_the_title() {
    let root = tempfile::tempdir().unwrap();
    let source_dir = root.path().join("网易云音乐");
    let output_dir = root.path().join("output");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&output_dir).unwrap();

    let source_path = source_dir.join("Overlap.mp3");
    fs::write(&source_path, b"audio").unwrap();
    let mut source_tag = id3::Tag::new();
    source_tag.set_title("Overlap");
    source_tag.set_artist("KIMERU");
    source_tag
        .write_to_path(&source_path, Version::Id3v24)
        .unwrap();

    let (source_music, issues) = sync::get_music_dict_with_scan_issues_with_rule(
        source_dir.to_str().unwrap(),
        FilenameRule::TitleArtist,
    );
    assert!(issues.is_empty());
    assert!(source_music.contains_key("Overlap - KIMERU"));

    let pending = source_music.iter().collect::<HashMap<_, _>>();
    sync_music_library_with_policy(&pending, output_dir.to_str().unwrap(), &Mode::Compat, None)
        .unwrap();

    let output_tag = id3::Tag::read_from_path(output_dir.join("Overlap - KIMERU.mp3")).unwrap();
    assert_eq!(output_tag.title(), Some("Overlap"));
    assert_eq!(output_tag.artist(), Some("KIMERU"));
}

#[test]
fn compat_export_treats_untagged_netease_filename_as_title_then_artist() {
    let root = tempfile::tempdir().unwrap();
    let source_dir = root.path().join("网易云音乐");
    let output_dir = root.path().join("output");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&output_dir).unwrap();

    let source_path = source_dir.join("Overlap - KIMERU.mp3");
    fs::write(&source_path, b"audio").unwrap();

    let (source_music, issues) = sync::get_music_dict_with_scan_issues_with_rule(
        source_dir.to_str().unwrap(),
        FilenameRule::TitleArtist,
    );
    assert!(issues.is_empty());
    assert!(source_music.contains_key("Overlap - KIMERU"));

    let pending = source_music.iter().collect::<HashMap<_, _>>();
    sync_music_library_with_policy(&pending, output_dir.to_str().unwrap(), &Mode::Compat, None)
        .unwrap();

    let output_tag = id3::Tag::read_from_path(output_dir.join("Overlap - KIMERU.mp3")).unwrap();
    assert_eq!(output_tag.title(), Some("Overlap"));
    assert_eq!(output_tag.artist(), Some("KIMERU"));
}

#[test]
fn compat_export_keeps_netease_title_first_filename_and_metadata_identity() {
    let root = tempfile::tempdir().unwrap();
    let source_dir = root.path().join("网易云音乐");
    let output_dir = root.path().join("output");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&output_dir).unwrap();

    let source_path = source_dir.join("巴适 (Bāshì) - BikaBreezy, Jaytrue.mp3");
    fs::write(&source_path, b"audio").unwrap();
    let mut source_tag = id3::Tag::new();
    source_tag.set_title("巴适 (Bāshì)");
    source_tag.set_artist("BikaBreezy, Jaytrue");
    source_tag.add_frame(id3::frame::ExtendedText {
        description: "163 key".into(),
        value: "netease-source".into(),
    });
    source_tag
        .write_to_path(&source_path, Version::Id3v24)
        .unwrap();

    let (source_music, issues) = sync::get_music_dict_with_scan_issues_with_rule(
        source_dir.to_str().unwrap(),
        FilenameRule::TitleArtist,
    );
    assert!(issues.is_empty());
    assert!(source_music.contains_key("巴适 (Bāshì) - BikaBreezy, Jaytrue"));

    let pending = source_music.iter().collect::<HashMap<_, _>>();
    sync_music_library_with_policy(&pending, output_dir.to_str().unwrap(), &Mode::Compat, None)
        .unwrap();

    let output_path = output_dir.join("巴适 (Bāshì) - BikaBreezy, Jaytrue.mp3");
    let output_tag = id3::Tag::read_from_path(&output_path).unwrap();
    assert_eq!(output_tag.title(), Some("巴适 (Bāshì)"));
    assert_eq!(output_tag.artist(), Some("BikaBreezy, Jaytrue"));

    let diagnostic = sync::inspect_metadata_decision(&source_path, &output_path);
    assert_eq!(diagnostic.detected_filename_layout, "歌名 - 歌手");
    assert_eq!(diagnostic.decision, "采用完整内嵌元数据");
    assert_eq!(diagnostic.resolved_title, "巴适 (Bāshì)");
    assert_eq!(diagnostic.resolved_artist, "BikaBreezy, Jaytrue");
    assert_eq!(diagnostic.output_title.as_deref(), Some("巴适 (Bāshì)"));
    assert_eq!(
        diagnostic.output_artist.as_deref(),
        Some("BikaBreezy, Jaytrue")
    );
}

#[test]
fn compat_export_does_not_reverse_trustworthy_title_first_metadata() {
    let root = tempfile::tempdir().unwrap();
    let source_dir = root.path().join("source");
    let output_dir = root.path().join("output");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&output_dir).unwrap();

    let source_path = source_dir.join("Song - Artist.mp3");
    fs::write(&source_path, b"audio").unwrap();
    let mut source_tag = id3::Tag::new();
    source_tag.set_title("Song");
    source_tag.set_artist("Artist");
    source_tag
        .write_to_path(&source_path, Version::Id3v24)
        .unwrap();

    let (source_music, issues) = sync::get_music_dict_with_scan_issues_with_rule(
        source_dir.to_str().unwrap(),
        FilenameRule::TitleArtist,
    );
    assert!(issues.is_empty());
    assert!(source_music.contains_key("Song - Artist"));

    let pending = source_music.iter().collect::<HashMap<_, _>>();
    sync_music_library_with_policy(&pending, output_dir.to_str().unwrap(), &Mode::Compat, None)
        .unwrap();

    let output_tag = id3::Tag::read_from_path(output_dir.join("Song - Artist.mp3")).unwrap();
    assert_eq!(output_tag.title(), Some("Song"));
    assert_eq!(output_tag.artist(), Some("Artist"));
}

#[test]
fn compat_export_normalizes_a_generic_embedded_cover_for_dj_software() {
    let root = tempfile::tempdir().unwrap();
    let source_dir = root.path().join("source");
    let output_dir = root.path().join("output");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&output_dir).unwrap();

    let source_path = source_dir.join("Artist - Song.mp3");
    fs::write(&source_path, b"audio").unwrap();
    let jpeg_cover = vec![0xff, 0xd8, 0xff, 0xdb, 0x00, 0x43, 0x01, 0x02];
    let mut source_tag = id3::Tag::new();
    source_tag.set_title("Song");
    source_tag.set_artist("Artist");
    source_tag.add_frame(id3::frame::Picture {
        mime_type: "image/*".into(),
        picture_type: id3::frame::PictureType::CoverFront,
        description: String::new(),
        data: jpeg_cover.clone(),
    });
    source_tag
        .write_to_path(&source_path, Version::Id3v24)
        .unwrap();

    let (source_music, issues) = sync::get_music_dict_with_scan_issues_with_rule(
        source_dir.to_str().unwrap(),
        FilenameRule::TitleArtist,
    );
    assert!(issues.is_empty());
    let pending = source_music.iter().collect::<HashMap<_, _>>();
    sync_music_library_with_policy(&pending, output_dir.to_str().unwrap(), &Mode::Compat, None)
        .unwrap();

    let output_path = output_dir.join("Song - Artist.mp3");
    let output_tag = id3::Tag::read_from_path(&output_path).unwrap();
    let output_cover = output_tag.pictures().next().expect("missing output cover");
    assert_eq!(output_cover.mime_type, "image/jpeg");
    assert_eq!(
        output_cover.picture_type,
        id3::frame::PictureType::CoverFront
    );
    assert_eq!(output_cover.data, jpeg_cover);
    assert_eq!(fs::read(output_path).unwrap()[3], 3);
}
