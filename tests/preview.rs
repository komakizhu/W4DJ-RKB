use std::fs;

use tempfile::tempdir;
use w4dj::config::{ConflictStrategy, FilenameRule, Mode};
use w4dj::history::{HistoryEntry, HistoryStatus, PendingFile};
use w4dj::preview::{
    PreviewOperation, build_retry_preview, build_sync_preview, build_sync_preview_with_settings,
};

fn write_file(path: impl AsRef<std::path::Path>, size: usize) {
    fs::write(path, vec![b'x'; size]).unwrap();
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
            operation: PreviewOperation::Convert,
        }],
        status: HistoryStatus::Partial,
        retry_of: None,
        conflict_strategy: ConflictStrategy::Skip,
        filename_rule: FilenameRule::TitleArtist,
    };

    let preview = build_retry_preview(&entry);
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].name, "Pending");
    assert_eq!(
        preview.candidates[0].destination_path,
        destination_path.display().to_string()
    );
}
