#![allow(dead_code)]
#[path = "../src/analysis.rs"]
mod analysis;
#[path = "../src/concurrency.rs"]
mod concurrency;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/filename_policy.rs"]
mod filename_policy;
#[path = "../src/metadata.rs"]
mod metadata;
#[path = "../src/netease.rs"]
mod netease;
#[path = "../src/netease_cache.rs"]
mod netease_cache;
#[path = "../src/scan_cache.rs"]
mod scan_cache;
#[path = "../src/sync.rs"]
mod sync;
#[path = "../src/task.rs"]
mod task;

use config::{FilenameNormalizationPolicy, LosslessFormat, Mode, NeteaseFilenameFormat};
use id3::{Tag, TagLike};
use ncmdump::NcmInfo;
use netease::NeteaseMetadataResolver;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use sync::{
    ConversionMetadataContext, TargetProfile, cleanup_temporary_outputs, compare_music_dicts,
    get_destination_music_dict, resolve_output_policy,
    update_existing_metadata_transactionally_with_context_and_policy,
};
use tempfile::tempdir;

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
fn real_mass_destruction_metadata_round_trip_uses_database_identity() {
    let Ok(database) = env::var("W4DJ_REAL_NETEASE_DB") else {
        return;
    };
    let Ok(source) = env::var("W4DJ_REAL_NETEASE_SOURCE") else {
        return;
    };
    let source_path = PathBuf::from(&source);
    if !source_path.is_file() {
        return;
    }
    let temporary = tempdir().unwrap();
    let output = temporary.path().join("database-identity.mp3");
    fs::write(&output, b"temporary audio").unwrap();
    let source_before = fs::read(&source_path).unwrap();
    let resolver =
        Arc::new(NeteaseMetadataResolver::load_exact(PathBuf::from(database).as_path()).unwrap());
    let context = ConversionMetadataContext { netease: resolver };

    update_existing_metadata_transactionally_with_context_and_policy(
        &source_path,
        &output,
        NeteaseFilenameFormat::TitleArtist,
        |_| Ok(()),
        &context,
        FilenameNormalizationPolicy::PreserveSource,
    )
    .unwrap();

    let tag = Tag::read_from_path(&output).unwrap();
    assert_eq!(
        tag.title(),
        Some("Mass Destruction (\"P3\" + \"P3F\" ver.)")
    );
    assert_eq!(tag.artist(), Some("川村ゆみ, Lotus Juice"));
    assert_eq!(tag.album(), Some("『P3D』＆『P5D』フルサウンドトラック"));
    assert_eq!(fs::read(&source_path).unwrap(), source_before);
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
fn get_music_dict_preserves_distinct_duplicate_stems() {
    let temp_dir = std::env::temp_dir().join(format!("w4dj-sync-policy-{}", std::process::id()));
    fs::create_dir_all(&temp_dir).unwrap();

    let mp3_path = temp_dir.join("same.mp3");
    let flac_path = temp_dir.join("same.flac");
    fs::write(&mp3_path, b"mp3").unwrap();
    fs::write(&flac_path, b"flac").unwrap();

    let dict = sync::get_music_dict(temp_dir.to_str().unwrap());
    assert_eq!(dict.len(), 2);
    assert!(dict.values().any(|(_, path)| path == &mp3_path));
    assert!(dict.values().any(|(_, path)| path == &flac_path));

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
fn observed_scan_reports_each_song_and_supports_cancellation() {
    let temp_dir =
        std::env::temp_dir().join(format!("w4dj-sync-policy-observer-{}", std::process::id()));
    fs::create_dir_all(temp_dir.join("nested")).unwrap();
    fs::write(temp_dir.join("one.mp3"), b"mp3").unwrap();
    fs::write(temp_dir.join("nested/two.flac"), b"flac").unwrap();

    let mut phases = Vec::new();
    let (dict, issues, cancelled) = sync::get_music_dict_with_scan_issues_with_rule_and_observer(
        temp_dir.to_str().unwrap(),
        config::FilenameRule::default(),
        &mut |phase, _path| {
            phases.push(phase);
            true
        },
    );

    assert!(!cancelled);
    assert!(issues.is_empty());
    assert_eq!(dict.len(), 2);
    assert_eq!(phases.len(), 2);
    assert!(
        phases
            .iter()
            .all(|phase| matches!(phase, sync::ScanPhase::Source))
    );

    let (count, cancelled) = sync::count_music_files_with_cancel(
        temp_dir.to_str().unwrap(),
        sync::SUPPORTED_SOURCE_EXTENSIONS,
        || true,
    );
    assert_eq!(count, 0);
    assert!(cancelled);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn enumerate_music_files_reports_one_pass_paths_issues_and_total() {
    let temp_dir = std::env::temp_dir().join(format!("w4dj-enumerate-once-{}", std::process::id()));
    fs::create_dir_all(temp_dir.join("nested")).unwrap();
    fs::write(temp_dir.join("one.mp3"), b"mp3").unwrap();
    fs::write(temp_dir.join("nested/two.flac"), b"flac").unwrap();
    fs::write(temp_dir.join("ignored.txt"), b"not audio").unwrap();

    let mut observations = Vec::new();
    let result = sync::enumerate_music_files_observed(
        temp_dir.to_str().unwrap(),
        sync::SUPPORTED_SOURCE_EXTENSIONS,
        &AtomicBool::new(false),
        |processed, total, path| {
            observations.push((processed, total, path.to_path_buf()));
        },
    )
    .unwrap();

    assert_eq!(result.paths.len(), 2);
    assert!(result.issues.is_empty());
    assert_eq!(observations.last().unwrap().0, 2);
    assert_eq!(observations.last().unwrap().1, Some(2));

    let cancelled = AtomicBool::new(true);
    let error = sync::enumerate_music_files_observed(
        temp_dir.to_str().unwrap(),
        sync::SUPPORTED_SOURCE_EXTENSIONS,
        &cancelled,
        |_, _, _| {},
    )
    .unwrap_err();
    assert_eq!(error, sync::ScanEnumerationError::Cancelled);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn scan_counter_handles_a_hundreds_track_library() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-large-library-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    for index in 0..300 {
        fs::write(temp_dir.join(format!("track-{index:03}.mp3")), b"mp3").unwrap();
    }

    let (count, cancelled) = sync::count_music_files_with_cancel(
        temp_dir.to_str().unwrap(),
        sync::SUPPORTED_SOURCE_EXTENSIONS,
        || false,
    );
    assert_eq!(count, 300);
    assert!(!cancelled);

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
fn get_music_dict_ignores_dot_underscore_track_even_without_appledouble_magic() {
    let temp_dir = std::env::temp_dir().join(format!(
        "w4dj-sync-policy-dot-underscore-track-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let source_path = temp_dir.join("._Song.flac");
    fs::write(&source_path, b"real-audio-placeholder").unwrap();

    let dict = sync::get_music_dict(temp_dir.to_str().unwrap());

    assert!(dict.is_empty());

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
    flac_tag.vorbis_comments_mut().set_genre(vec!["Electronic"]);
    flac_tag.add_picture(
        "image/png",
        metaflac::block::PictureType::CoverFront,
        vec![0x89, 0x50, 0x4e, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
    );

    let tag = metadata::build_id3_tag_from_flac(&flac_tag);

    assert_eq!(tag.title(), Some("Song"));
    assert_eq!(tag.album(), Some("Album"));
    assert_eq!(tag.artist(), Some("Artist"));
    assert_eq!(tag.genre(), Some("Electronic"));
    assert_eq!(tag.pictures().count(), 1);
}
