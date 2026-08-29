use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use id3::{Tag, TagLike, Version};
use rusqlite::{Connection, params};
use tempfile::tempdir;
use w4dj::concurrency::GlobalConcurrencyBudget;
use w4dj::config::{ConflictStrategy, FilenameNormalizationPolicy, FilenameRule, Mode};
use w4dj::history::{ErrorCategory, FailedFile, HistoryEntry, HistoryStatus, PendingFile};
use w4dj::netease::NeteaseMetadataResolver;
use w4dj::preview::{
    OutputTrackIdentity, PreviewCandidate, PreviewOperation, SyncPreview, build_retry_preview,
    build_sync_preview, build_sync_preview_with_settings,
    build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget_and_policy_and_resolver,
    build_sync_preview_with_settings_and_netease_observed_with_policy_and_resolver,
    disambiguate_duplicate_output_names,
};
use w4dj::scan_cache::ScanCache;
use w4dj::sync::remove_replaced_output;

fn write_file(path: impl AsRef<std::path::Path>, size: usize) {
    fs::write(path, vec![b'x'; size]).unwrap();
}

#[test]
fn task1_preview_uses_database_identity_before_building_output_path() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let database = tempdir().unwrap();
    let source_path = source
        .path()
        .join("Mass Destruction (＂P3＂ + ＂P3F＂ ver.) - 川村ゆみ,Lotus Juice.flac");
    fs::write(&source_path, b"untagged flac placeholder").unwrap();
    let database_path = database.path().join("sqlite_storage.sqlite3");
    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE web_track (tid TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 0, track TEXT NOT NULL);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO web_track(tid,version,track) VALUES (?1,0,?2)",
            params![
                "864433756",
                r#"{"album":{"id":71720241,"name":"『P3D』＆『P5D』フルサウンドトラック"},"artists":[{"name":"川村ゆみ"},{"name":"Lotus Juice"}],"id":864433756,"name":"Mass Destruction (\"P3\" + \"P3F\" ver.)"}"#
            ],
        )
        .unwrap();
    drop(connection);
    let resolver = NeteaseMetadataResolver::load_exact(&database_path).unwrap();

    let preview = build_sync_preview_with_settings_and_netease_observed_with_policy_and_resolver(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Skip,
        FilenameRule::TitleArtist,
        Default::default(),
        FilenameNormalizationPolicy::PreserveSource,
        None,
        &resolver,
    )
    .unwrap()
    .unwrap();

    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(
        preview.candidates[0].name,
        "Mass Destruction (\"P3\" + \"P3F\" ver.) - 川村ゆみ, Lotus Juice"
    );
    assert_eq!(
        preview.candidates[0].netease_track_id.as_deref(),
        Some("864433756")
    );
    assert_eq!(
        preview.candidates[0].netease_title.as_deref(),
        Some("Mass Destruction (\"P3\" + \"P3F\" ver.)")
    );
    assert_eq!(
        preview.candidates[0].netease_artist.as_deref(),
        Some("川村ゆみ, Lotus Juice")
    );
}

#[test]
fn real_mass_destruction_preview_uses_database_identity_when_available() {
    let Ok(database) = env::var("W4DJ_REAL_NETEASE_DB") else {
        return;
    };
    let Ok(_cache) = env::var("W4DJ_REAL_NETEASE_CACHE") else {
        return;
    };
    let Ok(source) = env::var("W4DJ_REAL_NETEASE_SOURCE") else {
        return;
    };
    let source_path = std::path::Path::new(&source);
    if !source_path.is_file() {
        return;
    }
    let destination = tempdir().unwrap();
    let resolver = NeteaseMetadataResolver::load_exact(std::path::Path::new(&database)).unwrap();
    let preview = build_sync_preview_with_settings_and_netease_observed_with_policy_and_resolver(
        &source,
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Skip,
        FilenameRule::TitleArtist,
        Default::default(),
        FilenameNormalizationPolicy::PreserveSource,
        None,
        &resolver,
    )
    .unwrap()
    .unwrap();

    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(
        preview.candidates[0].name,
        "Mass Destruction (\"P3\" + \"P3F\" ver.) - 川村ゆみ, Lotus Juice"
    );
    assert_eq!(
        preview.candidates[0].netease_track_id.as_deref(),
        Some("864433756")
    );
}

#[test]
fn real_lazy_netease_index_scans_the_current_folder_without_loading_full_rows() {
    let Ok(database) = env::var("W4DJ_REAL_NETEASE_DB") else {
        return;
    };
    let Ok(cache) = env::var("W4DJ_REAL_NETEASE_CACHE") else {
        return;
    };
    let Ok(source_directory) = env::var("W4DJ_REAL_NETEASE_SOURCE_DIR") else {
        return;
    };
    let database_path = std::path::Path::new(&database);
    let source_path = std::path::Path::new(&source_directory);
    if !database_path.is_file() || !source_path.is_dir() {
        return;
    }

    let started = Instant::now();
    let locators = w4dj::netease_cache::read_locators(std::path::Path::new(&cache))
        .expect("real lightweight locator cache should be readable");
    assert!(!locators.is_empty());
    let resolver = NeteaseMetadataResolver::from_locators(database_path, locators, None);
    let destination = tempdir().unwrap();
    let mut scan_cache = ScanCache::empty();
    let budget = Arc::new(GlobalConcurrencyBudget::new(4));
    let cancel = Arc::new(AtomicBool::new(false));
    let mut metadata_events = 0usize;
    let mut observer = |phase: w4dj::sync::ScanPhase, _: &std::path::Path| {
        if matches!(phase, w4dj::sync::ScanPhase::Metadata) {
            metadata_events += 1;
        }
        true
    };
    let preview = build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget_and_policy_and_resolver(
        &source_directory,
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Skip,
        FilenameRule::TitleArtist,
        Default::default(),
        FilenameNormalizationPolicy::PreserveSource,
        Some(&mut observer),
        &mut scan_cache,
        budget,
        cancel,
        &resolver,
    )
    .unwrap()
    .unwrap();

    assert_eq!(preview.candidates.len(), metadata_events);
    eprintln!(
        "lazy NetEase preview: {} candidates, {} metadata events, {:.2}s",
        preview.candidates.len(),
        metadata_events,
        started.elapsed().as_secs_f64(),
    );
}

fn duplicate_preview() -> SyncPreview {
    SyncPreview {
        source_directory: "/source".to_string(),
        destination_directory: "/destination".to_string(),
        new_count: 2,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        estimated_output_bytes: Some(2),
        candidates: vec![
            PreviewCandidate {
                name: "STONE KOLD - Skybreak, Subten".to_string(),
                source_path: "/source/stone-a.ncm".to_string(),
                destination_path: "/destination/STONE KOLD - Skybreak, Subten.mp3".to_string(),
                source_size_bytes: 1,
                estimated_output_bytes: Some(1),
                previous_destination_path: None,
                operation: PreviewOperation::Convert,
                netease_track_id: None,
                netease_album_id: None,
                album: None,
                netease_title: None,
                netease_artist: None,
                disambiguation_reason: None,
            },
            PreviewCandidate {
                name: "STONE KOLD - Skybreak, Subten".to_string(),
                source_path: "/source/stone-b.ncm".to_string(),
                destination_path: "/destination/STONE KOLD - Skybreak, Subten.mp3".to_string(),
                source_size_bytes: 1,
                estimated_output_bytes: Some(1),
                previous_destination_path: None,
                operation: PreviewOperation::Convert,
                netease_track_id: None,
                netease_album_id: None,
                album: None,
                netease_title: None,
                netease_artist: None,
                disambiguation_reason: None,
            },
        ],
        skipped: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        available_space_bytes: None,
        disk_space_sufficient: None,
        input_count: 2,
        output_duplicate_count: 0,
        action_kind: "convert".to_string(),
        action_count: 2,
        database_directory: None,
        detail_items: Vec::new(),
    }
}

#[test]
fn disambiguation_changes_only_duplicate_group_and_keeps_album_identity() {
    let mut preview = duplicate_preview();
    let identities = HashMap::from([
        (
            "/source/stone-a.ncm".to_string(),
            OutputTrackIdentity {
                track_id: Some("track-a".to_string()),
                album_id: Some("album-a".to_string()),
                title: "STONE KOLD".to_string(),
                artists: "Skybreak, Subten".to_string(),
                album: "HALF BLOOD".to_string(),
                source_path: PathBuf::from("/source/stone-a.ncm"),
            },
        ),
        (
            "/source/stone-b.ncm".to_string(),
            OutputTrackIdentity {
                track_id: Some("track-b".to_string()),
                album_id: Some("album-b".to_string()),
                title: "STONE KOLD".to_string(),
                artists: "Skybreak, Subten".to_string(),
                album: "OTHER ALBUM".to_string(),
                source_path: PathBuf::from("/source/stone-b.ncm"),
            },
        ),
    ]);

    disambiguate_duplicate_output_names(&mut preview, &identities);

    assert_eq!(preview.errors.len(), 0);
    assert_ne!(
        preview.candidates[0].destination_path,
        preview.candidates[1].destination_path
    );
    assert!(
        preview
            .candidates
            .iter()
            .any(|candidate| candidate.destination_path.contains("[HALF BLOOD]"))
    );
    assert!(
        preview
            .candidates
            .iter()
            .any(|candidate| candidate.destination_path.contains("[OTHER ALBUM]"))
    );
}

#[test]
fn disambiguation_leaves_non_duplicate_candidate_unchanged() {
    let mut preview = duplicate_preview();
    preview.candidates.truncate(1);
    preview.new_count = 1;
    let original = preview.candidates[0].clone();
    let identities = HashMap::from([(
        original.source_path.clone(),
        OutputTrackIdentity {
            track_id: Some("track-a".to_string()),
            album_id: Some("album-a".to_string()),
            title: "STONE KOLD".to_string(),
            artists: "Skybreak, Subten".to_string(),
            album: "HALF BLOOD".to_string(),
            source_path: PathBuf::from(&original.source_path),
        },
    )]);

    disambiguate_duplicate_output_names(&mut preview, &identities);

    assert_eq!(preview.candidates[0], original);
}

#[test]
fn disambiguation_keeps_existing_identity_in_preview_details() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let source_a = source.path().join("stone-a.ncm");
    let source_b = source.path().join("stone-b.ncm");
    write_file(&source_a, 1);
    write_file(&source_b, 1);
    let existing_path = destination.path().join("STONE KOLD - Skybreak, Subten.mp3");
    let mut tag = Tag::new();
    tag.set_title("STONE KOLD");
    tag.set_artist("Skybreak, Subten");
    tag.set_album("HALF BLOOD");
    write_file(&existing_path, 1);
    tag.write_to_path(&existing_path, Version::Id3v24).unwrap();

    let mut preview = SyncPreview {
        source_directory: source.path().display().to_string(),
        destination_directory: destination.path().display().to_string(),
        new_count: 2,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        estimated_output_bytes: Some(2),
        candidates: vec![
            PreviewCandidate {
                name: "STONE KOLD - Skybreak, Subten".to_string(),
                source_path: source_a.display().to_string(),
                destination_path: existing_path.display().to_string(),
                source_size_bytes: 1,
                estimated_output_bytes: Some(1),
                previous_destination_path: None,
                operation: PreviewOperation::Convert,
                netease_track_id: None,
                netease_album_id: None,
                album: None,
                netease_title: None,
                netease_artist: None,
                disambiguation_reason: None,
            },
            PreviewCandidate {
                name: "STONE KOLD - Skybreak, Subten".to_string(),
                source_path: source_b.display().to_string(),
                destination_path: existing_path.display().to_string(),
                source_size_bytes: 1,
                estimated_output_bytes: Some(1),
                previous_destination_path: None,
                operation: PreviewOperation::Convert,
                netease_track_id: None,
                netease_album_id: None,
                album: None,
                netease_title: None,
                netease_artist: None,
                disambiguation_reason: None,
            },
        ],
        skipped: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        available_space_bytes: None,
        disk_space_sufficient: None,
        input_count: 2,
        output_duplicate_count: 0,
        action_kind: "overwrite".to_string(),
        action_count: 0,
        database_directory: None,
        detail_items: Vec::new(),
    };
    let identities = HashMap::from([
        (
            source_a.display().to_string(),
            OutputTrackIdentity {
                track_id: Some("track-a".to_string()),
                album_id: Some("album-a".to_string()),
                title: "STONE KOLD".to_string(),
                artists: "Skybreak, Subten".to_string(),
                album: "HALF BLOOD".to_string(),
                source_path: source_a.clone(),
            },
        ),
        (
            source_b.display().to_string(),
            OutputTrackIdentity {
                track_id: Some("track-b".to_string()),
                album_id: Some("album-b".to_string()),
                title: "STONE KOLD".to_string(),
                artists: "Skybreak, Subten".to_string(),
                album: "OTHER ALBUM".to_string(),
                source_path: source_b.clone(),
            },
        ),
    ]);

    disambiguate_duplicate_output_names(&mut preview, &identities);

    assert_eq!(preview.candidates.len(), 1);
    let detail = preview
        .detail_items
        .iter()
        .find(|detail| detail.classification == "duplicate")
        .expect("existing duplicate should remain visible in preview details");
    assert_eq!(detail.name, "STONE KOLD - Skybreak, Subten");
    assert!(detail.existing_output);
    assert_eq!(
        detail.destination_path.as_deref(),
        Some(existing_path.to_str().unwrap())
    );
}

#[test]
fn output_identity_stable_key_prefers_track_id_and_falls_back_to_source() {
    let mut first = OutputTrackIdentity {
        track_id: Some("2714172644".into()),
        album_id: Some("album-a".into()),
        title: "STONE KOLD".into(),
        artists: "Skybreak, Subten".into(),
        album: "HALF BLOOD".into(),
        source_path: PathBuf::from("/source/a.ncm"),
    };
    let mut second = first.clone();
    second.album_id = Some("album-b".into());
    assert_eq!(first.stable_key(), second.stable_key());

    second.source_path = PathBuf::from("/source/another-copy.ncm");
    assert_eq!(first.stable_key(), second.stable_key());

    first.track_id = None;
    second.track_id = None;
    assert_ne!(first.stable_key(), second.stable_key());
}

#[test]
fn preview_separates_new_existing_and_estimated_bytes() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    write_file(source.path().join("new.mp3"), 120);
    write_file(source.path().join("existing.mp3"), 240);
    write_file(destination.path().join("existing.mp3"), 80);

    let preview = build_sync_preview(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 1);
    assert_eq!(preview.existing_count, 1);
    assert_eq!(preview.skipped_count, 1);
    assert_eq!(preview.error_count, 0);
    assert!(preview.skipped.is_empty());
    assert_eq!(preview.candidates[0].source_size_bytes, 120);
    assert_eq!(preview.estimated_output_bytes, Some(120));
}

#[test]
fn preview_reports_missing_source_and_invalid_destination() {
    let preview = build_sync_preview(
        "/path/that/does/not/exist",
        "/path/that/cannot/be/used",
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.error_count, 0);
    assert_eq!(preview.warnings.len(), 1);
    assert!(!preview.warnings[0].message.is_empty());
}

#[test]
fn preview_recovers_replaced_single_file_when_extension_changes() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let stale_source = source.path().join("Track.ncm");
    let replacement = source.path().join("Track.mp3");
    write_file(&replacement, 120);

    let preview = build_sync_preview(
        stale_source.to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.source_directory, replacement.display().to_string());
    assert_eq!(preview.new_count, 1);
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(
        preview.candidates[0].source_path,
        replacement.display().to_string()
    );
}

#[test]
fn preview_recovers_single_file_when_downloaded_name_changes() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let stale_source = source.path().join("Track-A1.ncm");
    let replacement = source.path().join("Track-A2.mp3");
    write_file(&replacement, 120);

    let preview = build_sync_preview(
        stale_source.to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.source_directory, replacement.display().to_string());
    assert_eq!(preview.new_count, 1);
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(
        preview.candidates[0].source_path,
        replacement.display().to_string()
    );
}

#[test]
fn preview_rechecks_current_folder_after_previous_input_and_output_are_removed() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let first_source = source.path().join("Track-A1.mp3");
    let first_output = destination.path().join("Track-A1.mp3");
    write_file(&first_source, 120);
    write_file(&first_output, 120);

    let first_preview = build_sync_preview(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();
    assert_eq!(first_preview.new_count, 0);
    assert_eq!(first_preview.existing_count, 1);
    assert_eq!(first_preview.skipped_count, 1);

    fs::remove_file(first_source).unwrap();
    fs::remove_file(first_output).unwrap();
    let replacement = source.path().join("Track-A2.mp3");
    write_file(&replacement, 120);

    let second_preview = build_sync_preview(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();
    assert_eq!(second_preview.new_count, 1);
    assert_eq!(second_preview.existing_count, 0);
    assert_eq!(second_preview.skipped_count, 0);
    assert_eq!(second_preview.candidates.len(), 1);
    assert_eq!(
        second_preview.candidates[0].source_path,
        replacement.display().to_string()
    );
}

#[test]
fn preview_does_not_guess_between_ambiguous_replaced_single_files() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let stale_source = source.path().join("Track.ncm");
    write_file(source.path().join("Track.mp3"), 120);
    write_file(source.path().join("Track.flac"), 120);

    let preview = build_sync_preview(
        stale_source.to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.source_directory, stale_source.display().to_string());
    assert!(preview.candidates.is_empty());
    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.warnings.len(), 1);
}

#[test]
fn preview_counts_unreadable_song_files_as_errors() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    write_file(source.path().join("empty.mp3"), 0);

    let preview = build_sync_preview(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.existing_count, 0);
    assert_eq!(preview.skipped_count, 0);
    assert_eq!(preview.error_count, 1);
    assert_eq!(preview.errors.len(), 1);
}

#[test]
fn preview_reports_an_unsupported_single_file_as_an_error() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let source_file = source.path().join("notes.txt");
    write_file(&source_file, 120);

    let preview = build_sync_preview(
        source_file.to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.error_count, 1);
    assert!(preview.candidates.is_empty());
    assert!(preview.errors[0].message.contains("不支持"));
}

#[test]
fn preview_reports_an_empty_single_audio_file_as_an_error() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let source_file = source.path().join("empty.mp3");
    write_file(&source_file, 0);

    let preview = build_sync_preview(
        source_file.to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.error_count, 1);
    assert!(preview.candidates.is_empty());
}

#[test]
fn preview_blocks_overwriting_a_single_source_file_in_place() {
    let source = tempdir().unwrap();
    let source_file = source.path().join("single.mp3");
    write_file(&source_file, 120);

    let preview = build_sync_preview_with_settings(
        source_file.to_str().unwrap(),
        source.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Overwrite,
        FilenameRule::TitleArtist,
    )
    .unwrap();

    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.error_count, 1);
    assert!(preview.candidates.is_empty());
    assert!(preview.errors[0].message.contains("源文件"));
}

#[test]
fn preview_blocks_updating_metadata_on_the_input_file_itself() {
    let source = tempdir().unwrap();
    let source_file = source.path().join("single.mp3");
    write_file(&source_file, 120);

    let preview = build_sync_preview_with_settings(
        source_file.to_str().unwrap(),
        source.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::UpdateMetadata,
        FilenameRule::TitleArtist,
    )
    .unwrap();

    assert_eq!(preview.new_count, 0);
    assert_eq!(preview.error_count, 1);
    assert!(preview.candidates.is_empty());
    assert!(preview.errors[0].message.contains("源文件"));
}

#[test]
fn single_track_in_output_folder_is_not_skipped_when_target_format_differs() {
    let source = tempdir().unwrap();
    let source_file = source.path().join("single.wav");
    write_file(&source_file, 120);

    let preview = build_sync_preview(
        source_file.to_str().unwrap(),
        source.path().to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 1);
    assert_eq!(preview.existing_count, 0);
    assert_eq!(preview.skipped_count, 0);
    assert!(
        preview.candidates[0]
            .destination_path
            .ends_with("single.mp3")
    );
}

#[test]
fn preview_does_not_count_destination_configuration_errors_as_song_files() {
    let source = tempdir().unwrap();
    let destination_parent = tempdir().unwrap();
    let destination = destination_parent.path().join("not-a-folder");
    write_file(source.path().join("new.mp3"), 120);
    write_file(&destination, 1);

    let preview = build_sync_preview(
        source.path().to_str().unwrap(),
        destination.to_str().unwrap(),
        Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(preview.new_count, 1);
    assert_eq!(preview.error_count, 0);
    assert_eq!(preview.warnings.len(), 1);
}

#[test]
fn conflict_strategies_produce_distinct_conversion_plans() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    write_file(source.path().join("Song.mp3"), 120);
    write_file(destination.path().join("Song.mp3"), 80);

    let preview = |strategy| {
        build_sync_preview_with_settings(
            source.path().to_str().unwrap(),
            destination.path().to_str().unwrap(),
            Mode::Compat,
            None,
            strategy,
            FilenameRule::TitleArtist,
        )
        .unwrap()
    };

    let skipped = preview(ConflictStrategy::Skip);
    assert_eq!(skipped.skipped_count, 1);
    assert!(skipped.candidates.is_empty());

    let overwritten = preview(ConflictStrategy::Overwrite);
    assert_eq!(overwritten.candidates[0].name, "Song");
    assert!(
        overwritten.candidates[0]
            .destination_path
            .ends_with("Song.mp3")
    );

    let renamed = preview(ConflictStrategy::Rename);
    assert_eq!(renamed.candidates[0].name, "Song (2)");
    assert!(
        renamed.candidates[0]
            .destination_path
            .ends_with("Song (2).mp3")
    );

    let metadata = preview(ConflictStrategy::UpdateMetadata);
    assert_eq!(
        metadata.candidates[0].operation,
        PreviewOperation::UpdateMetadata
    );
    assert_eq!(metadata.estimated_output_bytes, Some(0));
    assert!(
        metadata.candidates[0]
            .destination_path
            .ends_with("Song.mp3")
    );
}

#[test]
fn overwrite_remembers_old_path_when_filename_rule_changes() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let source_path = source.path().join("source.mp3");
    let old_output = destination.path().join("Artist - Song.mp3");
    write_file(&source_path, 120);
    write_file(&old_output, 120);

    for path in [&source_path, &old_output] {
        let mut tag = Tag::new();
        tag.set_title("Song");
        tag.set_artist("Artist");
        tag.write_to_path(path, Version::Id3v24).unwrap();
    }

    let preview = build_sync_preview_with_settings(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Overwrite,
        FilenameRule::TitleArtist,
    )
    .unwrap();

    assert_eq!(preview.candidates.len(), 1);
    assert!(
        preview.candidates[0]
            .destination_path
            .ends_with("Song - Artist.mp3")
    );
    assert_eq!(
        preview.candidates[0].previous_destination_path.as_deref(),
        Some(old_output.to_str().unwrap())
    );
}

#[test]
fn overwrite_cleanup_removes_old_output_but_protects_new_output_and_source() {
    let root = tempdir().unwrap();
    let previous = root.path().join("Artist - Song.mp3");
    let current = root.path().join("Song - Artist.mp3");
    let source = root.path().join("source.mp3");
    write_file(&previous, 1);
    write_file(&current, 1);
    write_file(&source, 1);

    assert!(remove_replaced_output(&previous, &current, &source).unwrap());
    assert!(!previous.exists());
    assert!(current.exists());
    assert!(source.exists());
}

#[test]
fn cleaned_name_collisions_keep_both_sources_and_are_explicit() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    write_file(source.path().join("A:B.mp3"), 120);
    write_file(source.path().join("A?B.mp3"), 121);

    let renamed = build_sync_preview_with_settings(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Rename,
        FilenameRule::Original,
    )
    .unwrap();
    assert_eq!(renamed.candidates.len(), 2);
    assert_ne!(
        renamed.candidates[0].destination_path,
        renamed.candidates[1].destination_path
    );

    let skipped = build_sync_preview_with_settings(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Skip,
        FilenameRule::Original,
    )
    .unwrap();
    // A same-target collision is now disambiguated before the selected
    // conflict strategy is applied, so both distinct source files remain
    // processable even when the strategy is Skip.
    assert_eq!(skipped.candidates.len(), 2);
    assert_eq!(skipped.skipped_count, 0);
    assert_ne!(
        skipped.candidates[0].destination_path,
        skipped.candidates[1].destination_path
    );
}

#[test]
fn auto_rename_reserves_names_across_the_whole_batch() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    write_file(source.path().join("Song.mp3"), 120);
    write_file(source.path().join("Song (2).mp3"), 120);
    write_file(destination.path().join("Song.mp3"), 80);

    let preview = build_sync_preview_with_settings(
        source.path().to_str().unwrap(),
        destination.path().to_str().unwrap(),
        Mode::Compat,
        None,
        ConflictStrategy::Rename,
        FilenameRule::TitleArtist,
    )
    .unwrap();

    let destinations = preview
        .candidates
        .iter()
        .map(|candidate| &candidate.destination_path)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(destinations.len(), preview.candidates.len());
}

#[test]
fn retry_preview_restores_pending_files_saved_before_app_exit() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let source_path = source.path().join("Pending.mp3");
    write_file(&source_path, 120);
    let destination_path = destination.path().join("Pending.mp3");
    let entry = HistoryEntry {
        id: "history-pending".into(),
        batch_id: "batch-pending".into(),
        operation_id: None,
        slot_index: 0,
        started_at: "1".into(),
        finished_at: "1".into(),
        duration_seconds: 0,
        source_directory: source.path().display().to_string(),
        destination_directory: destination.path().display().to_string(),
        mode: Mode::Compat,
        lossless_format: None,
        new_count: 1,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        completed_count: 0,
        failed_count: 0,
        failed_files: Vec::new(),
        pending_files: vec![PendingFile {
            name: "Pending".into(),
            source_path: source_path.display().to_string(),
            destination_path: destination_path.display().to_string(),
            source_size_bytes: 120,
            estimated_output_bytes: Some(120),
            previous_destination_path: None,
            operation: PreviewOperation::Convert,
        }],
        metadata_diagnostics: Vec::new(),
        logs: Vec::new(),
        status: HistoryStatus::Partial,
        retry_of: None,
        conflict_strategy: ConflictStrategy::Skip,
        filename_rule: FilenameRule::TitleArtist,
        netease_filename_format: Default::default(),
        report_path: None,
        analysis_reports: Vec::new(),
        runtime_session_dir: None,
    };

    let preview = build_retry_preview(&entry);
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].name, "Pending");
    assert_eq!(
        preview.candidates[0].destination_path,
        destination_path.display().to_string()
    );
}

#[test]
fn retry_preview_ignores_old_macos_appledouble_failures_and_pending_files() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let sidecar_path = source.path().join("._Song.flac");
    fs::write(&sidecar_path, b"\x00\x05\x16\x07macos-metadata").unwrap();
    let entry = HistoryEntry {
        id: "history-appledouble".into(),
        batch_id: "batch-appledouble".into(),
        operation_id: None,
        slot_index: 0,
        started_at: "1".into(),
        finished_at: "1".into(),
        duration_seconds: 0,
        source_directory: source.path().display().to_string(),
        destination_directory: destination.path().display().to_string(),
        mode: Mode::Compat,
        lossless_format: None,
        new_count: 1,
        existing_count: 0,
        skipped_count: 0,
        error_count: 1,
        completed_count: 0,
        failed_count: 1,
        failed_files: vec![FailedFile {
            name: ". Song".into(),
            source_path: sidecar_path.display().to_string(),
            destination_path: destination.path().join(". Song.mp3").display().to_string(),
            message: "FFmpeg 转换失败".into(),
            category: ErrorCategory::Ffmpeg,
        }],
        pending_files: vec![PendingFile {
            name: ". Song".into(),
            source_path: sidecar_path.display().to_string(),
            destination_path: destination
                .path()
                .join(". Song-pending.mp3")
                .display()
                .to_string(),
            source_size_bytes: 120,
            estimated_output_bytes: Some(120),
            previous_destination_path: None,
            operation: PreviewOperation::Convert,
        }],
        metadata_diagnostics: Vec::new(),
        logs: Vec::new(),
        status: HistoryStatus::Error,
        retry_of: None,
        conflict_strategy: ConflictStrategy::Skip,
        filename_rule: FilenameRule::TitleArtist,
        netease_filename_format: Default::default(),
        report_path: None,
        analysis_reports: Vec::new(),
        runtime_session_dir: None,
    };

    let preview = build_retry_preview(&entry);

    assert!(preview.candidates.is_empty());
    assert!(preview.errors.is_empty());
}
