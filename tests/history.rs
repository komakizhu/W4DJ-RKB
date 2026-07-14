use tempfile::tempdir;
use w4dj::config::Mode;
use w4dj::history::{
    FailedFile, HistoryEntry, HistoryStatus, append_history, format_error_report, load_history,
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
        status: HistoryStatus::Completed,
        retry_of: None,
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
    });

    let report = format_error_report(&entry);

    assert!(report.contains("/music/in/song.flac"));
    assert!(report.contains("FFmpeg failed"));
}
