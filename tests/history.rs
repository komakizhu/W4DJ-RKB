use tempfile::tempdir;
use w4dj::config::Mode;
use w4dj::history::{
    ErrorCategory, FailedFile, HistoryEntry, HistoryStatus, append_history, classify_error,
    clear_history, delete_history_entry, format_error_report, load_history, upsert_history,
};

fn test_entry(index: usize) -> HistoryEntry {
    HistoryEntry {
        id: format!("history-{index}"),
        batch_id: format!("batch-{index}"),
        slot_index: 0,
        started_at: format!("2026-07-14T00:{index:02}:00Z"),
        finished_at: format!("2026-07-14T00:{index:02}:01Z"),
        duration_seconds: 1,
        source_directory: "/music/in".into(),
        destination_directory: "/music/out".into(),
        mode: Mode::Compat,
        lossless_format: None,
        new_count: 1,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        completed_count: 1,
        failed_count: 0,
        failed_files: Vec::new(),
        pending_files: Vec::new(),
        status: HistoryStatus::Completed,
        retry_of: None,
        conflict_strategy: Default::default(),
        filename_rule: Default::default(),
    }
}

#[test]
fn history_keeps_newest_fifty_entries() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("history.json");

    for index in 0..51 {
        append_history(&path, test_entry(index)).unwrap();
    }

    let loaded = load_history(&path).unwrap();
    assert_eq!(loaded.len(), 50);
    assert_eq!(loaded[0].batch_id, "batch-50");
    assert_eq!(loaded[49].batch_id, "batch-1");
}

#[test]
fn error_report_contains_failed_path_and_reason() {
    let mut entry = test_entry(1);
    entry.failed_count = 1;
    entry.status = HistoryStatus::Partial;
    entry.failed_files.push(FailedFile {
        name: "Song".into(),
        source_path: "/music/in/song.flac".into(),
        destination_path: "/music/out/song.mp3".into(),
        message: "FFmpeg failed".into(),
        category: Default::default(),
    });

    let report = format_error_report(&entry);

    assert!(report.contains("/music/in/song.flac"));
    assert!(report.contains("FFmpeg failed"));
}

#[test]
fn history_entries_can_be_updated_deleted_and_cleared_without_touching_outputs() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("history.json");
    let mut entry = test_entry(1);
    append_history(&path, entry.clone()).unwrap();

    entry.completed_count = 2;
    upsert_history(&path, entry).unwrap();
    let loaded = load_history(&path).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].completed_count, 2);

    assert!(delete_history_entry(&path, "history-1").unwrap());
    assert!(load_history(&path).unwrap().is_empty());

    append_history(&path, test_entry(2)).unwrap();
    clear_history(&path).unwrap();
    assert!(load_history(&path).unwrap().is_empty());
}

#[test]
fn errors_are_classified_for_user_facing_reports() {
    assert_eq!(
        classify_error("FFmpeg conversion failed"),
        ErrorCategory::Ffmpeg
    );
    assert_eq!(
        classify_error("No space left on device"),
        ErrorCategory::DiskSpace
    );
    assert_eq!(
        classify_error("Permission denied while writing output"),
        ErrorCategory::OutputPermission
    );
    assert_eq!(
        classify_error("unsupported audio format"),
        ErrorCategory::UnsupportedFormat
    );
    assert_eq!(
        classify_error("invalid filename"),
        ErrorCategory::InvalidFilename
    );
    assert_eq!(classify_error("无法读取源文件"), ErrorCategory::FileDamaged);
}
