#![allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[path = "../src/metadata.rs"]
mod metadata;
#[path = "../src/sync.rs"]
mod sync;
#[path = "../src/task.rs"]
mod task;

use config::{LosslessFormat, Mode};
use id3::TagLike;
use ncmdump::NcmInfo;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use sync::{
    TargetProfile, cleanup_temporary_outputs, compare_music_dicts, get_destination_music_dict,
    resolve_output_policy,
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
fn cleanup_temporary_outputs_removes_internal_temp_files() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-temp-cleanup-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let temp_path = temp_dir.join(".w4dj-same.flac");
    fs::write(&temp_path, b"temp").unwrap();

    cleanup_temporary_outputs(temp_dir.to_str().unwrap()).unwrap();

    assert!(!temp_path.exists());

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
