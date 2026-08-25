use tempfile::tempdir;
use w4dj::config::{CandidateOperation, Mode};
use w4dj::history::{
    AnalysisReport, ErrorCategory, FailedFile, HistoryEntry, HistoryStatus, PendingFile,
    append_history, classify_error, clear_history, delete_history_entry, format_error_report,
    format_error_report_with_runtime, load_history, upsert_history,
};
use w4dj::netease::{NeteaseCoverSource, NeteaseRecordMatchMethod, NeteaseRecoveryDiagnostic};
use w4dj::sync::MetadataDiagnostic;

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
        metadata_diagnostics: Vec::new(),
        logs: Vec::new(),
        status: HistoryStatus::Completed,
        retry_of: None,
        conflict_strategy: Default::default(),
        filename_rule: Default::default(),
        netease_filename_format: Default::default(),
        report_path: None,
        analysis_reports: Vec::new(),
        runtime_session_dir: None,
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
fn runtime_report_exposes_cache_reuse_per_track() {
    let entry = test_entry(1);
    let runtime = serde_json::json!({
        "runtimeSession": {
            "files": {
                "events.jsonl": [{
                    "event": "analysis_candidate_finished",
                    "at": "2026-08-23 12:00:00 UTC",
                    "details": {
                        "name": "Song",
                        "source_path": "/music/in/song.mp3",
                        "destination_path": "/music/out/song.mp3",
                        "status": "completed",
                        "cached": true,
                        "elapsed_ms": 0
                    }
                }]
            }
        }
    });

    let report = format_error_report_with_runtime(&entry, Some(&runtime));
    assert!(report.contains("缓存复用：是"));
}

#[test]
fn conversion_report_includes_per_track_metadata_diagnostics() {
    let mut entry = test_entry(2);
    entry.metadata_diagnostics.push(MetadataDiagnostic {
        source_path: "/music/in/Artist - Song.mp3".into(),
        destination_path: "/music/out/Song - Artist.mp3".into(),
        source_filename: "Artist - Song".into(),
        source_extension: "mp3".into(),
        source_size_bytes: Some(100),
        output_size_bytes: Some(90),
        source_title: Some("Song".into()),
        source_artist: Some("Artist".into()),
        source_album: None,
        output_title: Some("Song".into()),
        output_artist: Some("Artist".into()),
        output_album: None,
        source_artwork: true,
        output_artwork: Some(true),
        detected_filename_layout: "歌手 - 标题".into(),
        decision: "最终标题：Song；最终歌手：Artist".into(),
        metadata_validation: "通过：标题、歌手和可用封面已校验".into(),
        validation_basis: Some("source_tags".into()),
        output_tags_match: Some(true),
        netease_recovery: None,
    });

    let report = format_error_report(&entry);
    assert!(report.contains("[逐曲元数据诊断]"));
    assert!(report.contains("输出封面：有（有效图片）"));
    assert!(report.contains("最终标题：Song；最终歌手：Artist"));
}

#[test]
fn conversion_report_explains_netease_database_and_cover_recovery() {
    let mut entry = test_entry(8);
    entry.metadata_diagnostics = vec![MetadataDiagnostic {
        source_path: "/music/in/Song - Artist.flac".into(),
        destination_path: "/music/out/Song - Artist.mp3".into(),
        source_filename: "Song - Artist".into(),
        source_extension: "flac".into(),
        source_size_bytes: Some(100),
        output_size_bytes: Some(90),
        source_title: Some("Song".into()),
        source_artist: Some("Artist".into()),
        source_album: Some("Album".into()),
        output_title: Some("Song".into()),
        output_artist: Some("Artist".into()),
        output_album: Some("Album".into()),
        source_artwork: false,
        output_artwork: Some(true),
        detected_filename_layout: "标题 - 歌手".into(),
        decision: "最终标题：Song；最终歌手：Artist".into(),
        metadata_validation: "通过".into(),
        validation_basis: Some("source_tags".into()),
        output_tags_match: Some(true),
        netease_recovery: Some(NeteaseRecoveryDiagnostic {
            database_path: Some("/music/sqlite_storage.sqlite3".into()),
            database_loaded: true,
            database_record_count: 89,
            matched: true,
            match_method: Some(NeteaseRecordMatchMethod::FileNameAndIdentity),
            track_id: Some("42".into()),
            album_id: Some("7".into()),
            cover_source: Some(NeteaseCoverSource::LocalCache),
            cover_bytes: Some(128),
            message: None,
        }),
    }];

    let report = format_error_report(&entry);
    assert!(report.contains("[网易云数据库与封面恢复]"));
    assert!(report.contains("加载记录数：89"));
    assert!(report.contains("本地封面成功：1"));
    assert!(report.contains("网易云匹配方式：fileNameAndIdentity"));
    assert!(report.contains("网易云封面来源：localCache"));
}

#[test]
fn conversion_report_is_generic_for_successful_tasks() {
    let report = format_error_report(&test_entry(2));
    assert!(report.starts_with("W4DJ RKB 转换报告\n"));
    assert!(report.contains("任务状态：已完成"));
    assert!(report.contains("错误文件：0"));
    assert!(report.contains("失败文件：0"));
    assert!(report.contains("[转换状态]"));
    assert!(report.contains("[增强分析总览]"));
}

#[test]
fn manual_report_lists_each_discogs_head_state_separately() {
    let mut entry = test_entry(3);
    let heads = serde_json::json!({
        "discogsEffnet": {"heads": {
            "moodTheme": {"status": "completed", "version": "v1", "labels": [{"label": "dark", "confidence": 0.82}], "frameCount": 4},
            "approachability": {"status": "model_missing", "version": "", "frameCount": 0, "reason": "missing"},
            "instrumentation": {"status": "failed", "version": "v1", "frameCount": 2, "reason": "inference"},
            "timbre": {"status": "timeout", "version": "v1", "frameCount": 1, "reason": "deadline"},
            "danceability": {"status": "cancelled", "version": "v1", "frameCount": 1, "reason": "user"}
        }}
    });
    entry.analysis_reports.push(AnalysisReport {
        source_path: "/music/in/song.mp3".into(),
        destination_path: "/music/out/song.mp3".into(),
        status: "completed".into(),
        message: None,
        drop_status: None,
        drop_loudness_lufs: None,
        model_status: Some("completed".into()),
        model_details: Some(serde_json::to_string(&heads).unwrap()),
        stage: Some("completed".into()),
        elapsed_ms: Some(1234),
        basic_status: Some("completed".into()),
        basic_danceability: Some(0.72),
        discogs_danceability_status: Some("cancelled".into()),
        discogs_danceability: None,
        discogs_completed_heads: Some(1),
        discogs_total_heads: Some(5),
        cached: Some(false),
    });

    let report = format_error_report(&entry);
    assert!(report.contains("[Discogs-EffNet 逐 head 状态]"));
    for state in [
        "completed",
        "model_missing",
        "failed",
        "timeout",
        "cancelled",
    ] {
        assert!(report.contains(state), "missing Discogs state {state}");
    }
    assert!(report.contains("moodTheme"));
    assert!(report.contains("dark 82.0%"));
    assert!(report.contains("1234 ms"));
}

#[test]
fn complete_error_report_contains_environment_settings_and_all_counts() {
    let entry = test_entry(1);

    let report = format_error_report(&entry);

    assert!(report.contains("W4DJ RKB 转换报告"));
    assert!(report.contains("报告格式版本：2"));
    assert!(report.contains(&format!("软件版本：{}", env!("CARGO_PKG_VERSION"))));
    assert!(report.contains("操作系统："));
    assert!(report.contains("CPU 架构："));
    assert!(report.contains("程序路径："));
    assert!(report.contains("FFmpeg 路径："));
    assert!(report.contains("任务 ID：history-1"));
    assert!(report.contains("批次 ID：batch-1"));
    assert!(report.contains("输出模式：兼容模式"));
    assert!(report.contains("冲突策略：跳过"));
    assert!(report.contains("文件名规则：标题 - 艺术家"));
    assert!(report.contains("新增文件：1"));
    assert!(report.contains("已存在文件：0"));
    assert!(report.contains("跳过文件：0"));
    assert!(report.contains("错误文件：0"));
    assert!(!report.contains("预检错误："));
    assert!(report.contains("完成文件：1"));
    assert!(report.contains("失败文件：0"));
    assert!(report.contains("待处理文件：0"));
}

#[test]
fn complete_error_report_lists_pending_files() {
    let mut entry = test_entry(1);
    entry.pending_files.push(PendingFile {
        name: "Pending Song".into(),
        source_path: "/music/in/pending.flac".into(),
        destination_path: "/music/out/pending.mp3".into(),
        source_size_bytes: 4_096,
        estimated_output_bytes: Some(2_048),
        previous_destination_path: None,
        operation: CandidateOperation::Convert,
    });

    let report = format_error_report(&entry);

    assert!(report.contains("待处理文件详情"));
    assert!(report.contains("Pending Song"));
    assert!(report.contains("/music/in/pending.flac"));
    assert!(report.contains("/music/out/pending.mp3"));
    assert!(report.contains("源文件大小：4096 bytes"));
}

#[test]
fn diagnostic_logs_survive_history_reload_and_appear_in_report() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("history.json");
    let mut serialized = serde_json::to_value(test_entry(1)).unwrap();
    serialized["logs"] = serde_json::json!([
        "Scanning source: /music/in",
        "Failed Song: FFmpeg conversion failed"
    ]);
    std::fs::write(&path, serde_json::to_vec_pretty(&vec![serialized]).unwrap()).unwrap();

    let entry = load_history(&path).unwrap().remove(0);
    let report = format_error_report(&entry);

    assert!(report.contains("运行日志"));
    assert!(report.contains("Scanning source: /music/in"));
    assert!(report.contains("Failed Song: FFmpeg conversion failed"));
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
