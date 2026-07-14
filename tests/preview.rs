use std::fs;

use tempfile::tempdir;
use w4dj::config::Mode;
use w4dj::preview::build_sync_preview;

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
    assert_eq!(preview.errors.len(), 1);
    assert!(!preview.errors[0].message.is_empty());
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
    assert_eq!(preview.errors.len(), 1);
}
