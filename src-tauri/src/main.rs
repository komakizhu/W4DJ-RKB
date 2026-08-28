#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use base64::Engine as _;
use ncmdump::Ncmdump;
use tauri::utils::config::BackgroundThrottlingPolicy;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use w4dj::analysis::{
    REQUIRED_DISCOGS_HEAD_IDS, TrackAnalysis, TrackMetadata, analysis_file_path,
    build_rekordbox_xml, clear_analysis_file, is_basic_analysis_complete,
    is_complete_analysis, load_analysis_file,
    merge_analysis_entries, read_track_metadata, save_analysis_file,
};
use w4dj::config::{
    ConflictStrategy, ConversionMode, FilenameNormalizationPolicy, FilenameRule, LosslessFormat,
    Mode, NeteaseFilenameFormat,
};
use w4dj::concurrency::GlobalConcurrencyBudget;
use w4dj::dj_playlist::{
    ImportedDjPlaylist, ImportedDjPlaylistSummary, parse_w4dj_playlist, serialize_w4dj_playlist,
};
use w4dj::dj_playlist_match::DjPlaylistMatchReport;
use w4dj::filename_policy::sanitize_filename_component;
use w4dj::m3u8::{
    M3u8ExportSummary, ResolvedDjPlaylistTrack, build_relative_m3u8_with_summary,
    write_relative_m3u8_atomic,
};
use w4dj::desktop::{DesktopController, DesktopState};
use w4dj::history::{
    AnalysisReport, FailedFile, HistoryEntry, HistoryStatus, PendingFile, append_analysis_reports,
    classify_error, clear_history, delete_history_entry, format_error_report_with_runtime,
    load_history as load_history_file, upsert_history,
};
use w4dj::library_catalog::{CatalogLocalFile, CatalogSourceRecord, LibraryCatalog};
use w4dj::library_query::{LibraryPage, LibraryQuery};
use w4dj::w4dj_library::{
    EmotionEvaluationManifest, W4djLibrary, write_emotion_evaluation_manifest,
};
use w4dj::netease_library::{
    CatalogBuildError, NeteaseDiscovery,
    build_catalog_snapshot_incremental_observed, discover_netease_library,
    discover_netease_library_for_refresh, discover_netease_library_from_database,
    discover_netease_library_from_database_for_refresh, count_audio_files,
    discover_netease_library_from_database_observed, discover_netease_library_observed,
    count_audio_files_observed,
};
use w4dj::netease::{
    NeteaseMetadataResolver, database_fingerprint_view,
    load_locators_from_db_observed, locate_supported_database,
};
use w4dj::netease_cache::{self, CacheState};
use w4dj::preferences::{AppPreferences, load_preferences, save_preferences};
use w4dj::preview::{
    PreviewCandidate, PreviewIssue, PreviewOperation, SlotPreview, SyncPreview,
    attach_netease_identities,
    build_retry_preview,
    build_sync_preview_with_settings_and_netease_observed_with_policy_and_resolver,
    build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget_and_policy_and_resolver,
    is_recovered_single_source,
};
use w4dj::scan_cache::{ScanCache, clear_scan_cache as clear_scan_cache_file, load_scan_cache, save_scan_cache_atomic};
use w4dj::sync::{
    apply_track_analysis_metadata_with_context,
    cleanup_temporary_outputs, compare_music_dicts,
    EmbeddedAnalysis, inspect_metadata_diagnostic_with_resolver,
    is_ignored_music_file,
    is_supported_source_file,
    get_music_dict_with_scan_issues_with_settings_and_observer_with_budget_and_policy,
    sync_music_library_transactional_with_observer_and_budget_and_context_with_policy,
    ConversionMetadataContext, update_analysis_metadata_transactionally,
    update_existing_metadata_transactionally_with_context_and_policy,
    remove_replaced_output,
    planned_output_path_with_policy,
    validate_track_analysis_metadata,
    ScanEnumerationError, ScanPhase, enumerate_music_files_observed,
};

mod essentia_model_import;

// Enhanced analysis remains implemented and callable for later debugging,
// but every normal app launch starts with it disabled. The frontend uses the
// matching single visibility switch before exposing its controls again.
const ENHANCED_ANALYSIS_DEFAULT_ENABLED: bool = false;

#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
#[cfg(target_os = "macos")]
use window_vibrancy::{NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy};

struct AppState {
    controller: Arc<Mutex<DesktopController>>,
    preferences_path: Arc<Mutex<PathBuf>>,
    history_path: Arc<Mutex<PathBuf>>,
    models_path: Arc<Mutex<PathBuf>>,
    bundled_models_path: Arc<Mutex<PathBuf>>,
    scan_cache_path: Arc<Mutex<PathBuf>>,
    library: Arc<LibraryState>,
    history_write_lock: Arc<Mutex<()>>,
    models_write_lock: Arc<Mutex<()>>,
    destination_coordinator: DestinationCoordinator,
    scan_progress: Arc<Mutex<ScanProgress>>,
    scan_cancel: Arc<AtomicBool>,
    scan_result: Arc<Mutex<Option<Vec<SlotPreview>>>>,
    test_monitor_path: Arc<Mutex<PathBuf>>,
    test_monitors: Arc<Mutex<HashMap<String, Arc<TestMonitor>>>>,
    concurrency_budget: Arc<Mutex<Arc<GlobalConcurrencyBudget>>>,
    ffmpeg_registry: Arc<w4dj::sync::ActiveFfmpegRegistry>,
    headless_config: Option<HeadlessAcceptanceConfig>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HeadlessAcceptanceConfig {
    scenario: String,
    exercise_cancel_resume: bool,
    input_path: Option<String>,
    output_path: Option<String>,
    database_path: Option<String>,
    report_path: String,
}

struct LibraryState {
    catalog_path: Mutex<PathBuf>,
    manual_database_path: Mutex<Option<PathBuf>>,
    metadata_cache: Mutex<NeteaseMetadataCacheProgress>,
    metadata_cache_cancel: AtomicBool,
    metadata_cache_build_lock: Mutex<()>,
    metadata_cache_worker: Mutex<Option<thread::JoinHandle<()>>>,
    refresh: Mutex<LibraryRefreshProgress>,
    cancel: AtomicBool,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    invalid_scan: Mutex<InvalidScanProgress>,
    invalid_scan_cancel: AtomicBool,
    invalid_scan_worker: Mutex<Option<thread::JoinHandle<()>>>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseMetadataCacheProgress {
    status: String,
    stage: String,
    processed: usize,
    total: Option<usize>,
    current_item: String,
    message: String,
    error: Option<String>,
    database_path: Option<String>,
    cached_record_count: usize,
}

impl Default for NeteaseMetadataCacheProgress {
    fn default() -> Self {
        Self {
            status: CacheState::Idle.as_str().to_string(),
            stage: "idle".to_string(),
            processed: 0,
            total: None,
            current_item: String::new(),
            message: String::new(),
            error: None,
            database_path: None,
            cached_record_count: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NeteaseDiscoveryProgressEvent {
    status: String,
    stage: String,
    processed: usize,
    total: Option<usize>,
    current_item: String,
    message: String,
    suggestion: Option<NeteaseDiscovery>,
    error: Option<String>,
}

// Runtime session recording is intentionally local-only. It gives the user a
// complete, exportable account of a conversion without sending diagnostics
// anywhere or blocking the conversion path.
const RUNTIME_SESSION_RECORDING_ENABLED: bool = true;
const RUNTIME_SESSION_DIRECTORY: &str = "W4DJ-runtime-sessions";

#[derive(Debug, Clone, serde::Serialize)]
struct TestMonitorSession {
    schema_version: u32,
    monitor: &'static str,
    app_version: &'static str,
    session_id: String,
    batch_id: String,
    started_at: String,
    updated_at: String,
    finished_at: Option<String>,
    status: String,
    settings: serde_json::Value,
    tasks: serde_json::Value,
    task_results: Vec<serde_json::Value>,
}

#[derive(Clone)]
struct TestMonitor {
    session_dir: PathBuf,
    session_path: PathBuf,
    events_path: PathBuf,
    analysis_state_path: PathBuf,
    lock: Arc<Mutex<()>>,
    session: Arc<Mutex<TestMonitorSession>>,
    analysis_reports: Arc<Mutex<Vec<AnalysisReport>>>,
    remaining_jobs: Arc<AtomicUsize>,
}

impl TestMonitor {
    fn new(
        root: &Path,
        batch_id: &str,
        settings: serde_json::Value,
        previews: &[SlotPreview],
        job_count: usize,
    ) -> io::Result<Self> {
        let session_id = format!(
            "{}-{}",
            unique_timestamp(),
            monitor_safe_component(batch_id)
        );
        let session_dir = root.join(format!("session-{session_id}"));
        fs::create_dir_all(&session_dir)?;

        let tasks = serde_json::to_value(previews)
            .map_err(|error| io::Error::other(format!("serialize monitor tasks: {error}")))?;
        let now = timestamp_string();
        let session = TestMonitorSession {
            schema_version: 1,
            monitor: "W4DJ runtime session recorder",
            app_version: env!("CARGO_PKG_VERSION"),
            session_id: session_id.clone(),
            batch_id: batch_id.to_string(),
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
            status: "running".to_string(),
            settings,
            tasks,
            task_results: Vec::new(),
        };

        let monitor = Self {
            session_path: session_dir.join("session.json"),
            events_path: session_dir.join("events.jsonl"),
            analysis_state_path: session_dir.join("analysis-state.json"),
            session_dir,
            lock: Arc::new(Mutex::new(())),
            session: Arc::new(Mutex::new(session)),
            analysis_reports: Arc::new(Mutex::new(Vec::new())),
            remaining_jobs: Arc::new(AtomicUsize::new(job_count)),
        };
        monitor.write_session_file()?;
        let total = previews
            .iter()
            .map(|preview| preview.preview.candidates.len())
            .sum::<usize>();
        let mut tracks = serde_json::Map::new();
        for preview in previews {
            for candidate in &preview.preview.candidates {
                tracks.insert(
                    candidate.source_path.clone(),
                    serde_json::json!({
                        "name": candidate.name,
                        "sourcePath": candidate.source_path,
                        "destinationPath": candidate.destination_path,
                        "status": "pending",
                        "stage": "pending",
                    }),
                );
            }
        }
        write_json_file(
            &monitor.analysis_state_path,
            &serde_json::json!({
                "schemaVersion": 1,
                "batchId": batch_id,
                "status": "notRequested",
                "total": total,
                "completed": 0,
                "failed": 0,
                "timedOut": 0,
                "pending": total,
                "tracks": tracks,
            }),
        )?;
        fs::write(
            monitor.session_dir.join("candidates.json"),
            serde_json::to_string_pretty(previews).map_err(|error| {
                io::Error::other(format!("serialize monitor candidates: {error}"))
            })?,
        )?;
        fs::write(
            monitor.session_dir.join("README.md"),
            "# W4DJ 运行会话记录\n\n此目录由应用自动生成，仅保存在本机下载目录，不会上传。它用于保留本次任务的内部运行轨迹，不会自动导出错误报告。\n\n- `candidates.json`：本次任务的输入与计划输出。\n- `events.jsonl`：按时间追加的预检、转换、分析和回写事件。\n- `summary-slot-*.json`：每个任务结束后的完整转换历史、错误和元数据诊断。\n- `analysis-reports.json`：增强分析和分析元数据回写结果。\n- `session.json`：本次运行的设置、状态和任务汇总。\n\n需要报告时，请在转换历史中手动点击“导出错误报告”，选择保存位置后生成 UTF-8 文本文件。路径和元数据诊断可能包含本机文件信息，请分享给开发者前自行确认。\n",
        )?;
        monitor.record_event("session_started", serde_json::json!({
            "session_directory": monitor.session_dir.display().to_string(),
            "candidate_count": previews.iter().map(|preview| preview.preview.candidates.len()).sum::<usize>(),
        }));
        Ok(monitor)
    }

    /// Re-open a session created by a previous application process.  The
    /// durable event/state files are the source of truth after a WebView or
    /// backend restart; the in-memory conversion session is intentionally not
    /// reconstructed because conversion jobs have already finished.
    fn from_existing(session_dir: PathBuf) -> io::Result<Self> {
        if !session_dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "运行会话目录不存在",
            ));
        }
        let session_id = session_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("recovered-session")
            .to_string();
        let now = timestamp_string();
        Ok(Self {
            session_path: session_dir.join("session.json"),
            events_path: session_dir.join("events.jsonl"),
            analysis_state_path: session_dir.join("analysis-state.json"),
            session_dir,
            lock: Arc::new(Mutex::new(())),
            session: Arc::new(Mutex::new(TestMonitorSession {
                schema_version: 1,
                monitor: "W4DJ runtime session recorder",
                app_version: env!("CARGO_PKG_VERSION"),
                session_id,
                batch_id: String::new(),
                started_at: now.clone(),
                updated_at: now,
                finished_at: None,
                status: "running".to_string(),
                settings: serde_json::Value::Null,
                tasks: serde_json::Value::Array(Vec::new()),
                task_results: Vec::new(),
            })),
            analysis_reports: Arc::new(Mutex::new(Vec::new())),
            remaining_jobs: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn record_event(&self, event: &str, details: serde_json::Value) {
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        let record = serde_json::json!({
            "at": timestamp_string(),
            "event": event,
            "details": details,
        });
            if let Ok(line) = serde_json::to_string(&record)
            && let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.events_path)
            {
                let _ = writeln!(file, "{line}");
            }
        self.update_analysis_state(&record);
    }

    /// Keep a small, atomically replaced state snapshot alongside the append
    /// only event log. This is deliberately derived from events so a crash or
    /// force quit still leaves the latest known song/stage available on the
    /// next launch.
    fn update_analysis_state(&self, record: &serde_json::Value) {
        let Ok(mut state) = fs::read_to_string(&self.analysis_state_path)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
            .or_else(|| Some(serde_json::json!({
                "schemaVersion": 1,
                "status": "notRequested",
                "total": 0,
                "completed": 0,
                "failed": 0,
                "timedOut": 0,
                "pending": 0,
                "tracks": {},
            })))
            .ok_or(()) else {
            return;
        };
        let Some(object) = state.as_object_mut() else {
            return;
        };
        let event = record.get("event").and_then(serde_json::Value::as_str).unwrap_or_default();
        let details = record.get("details").unwrap_or(&serde_json::Value::Null);
        let at = record.get("at").cloned().unwrap_or(serde_json::Value::Null);
        let mut track_map = object
            .remove("tracks")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let source_path = details.get("source_path").and_then(serde_json::Value::as_str);
        if let Some(path) = source_path {
            let track = track_map
                .entry(path.to_string())
                .or_insert_with(|| serde_json::json!({"sourcePath": path}));
            if let Some(track_object) = track.as_object_mut() {
                if let Some(value) = details.get("name") {
                    track_object.insert("name".into(), value.clone());
                }
                if let Some(value) = details.get("destination_path") {
                    track_object.insert("destinationPath".into(), value.clone());
                }
                if let Some(value) = details.get("worker_job_id") {
                    track_object.insert("workerJobId".into(), value.clone());
                }
                if let Some(value) = details.get("stage") {
                    track_object.insert("stage".into(), value.clone());
                }
                if let Some(value) = details.get("elapsed_ms") {
                    track_object.insert("elapsedMs".into(), value.clone());
                }
                if let Some(value) = details.get("processed") {
                    track_object.insert("processed".into(), value.clone());
                }
                if let Some(value) = details.get("total") {
                    track_object.insert("total".into(), value.clone());
                }
                if let Some(value) = details.get("stage_started_at") {
                    track_object.insert("stageStartedAt".into(), value.clone());
                }
                if let Some(value) = details.get("backend") {
                    track_object.insert("backend".into(), value.clone());
                }
                if let Some(value) = details.get("patch_count") {
                    track_object.insert("patchCount".into(), value.clone());
                }
                if let Some(value) = details.get("tf_memory") {
                    track_object.insert("tfMemory".into(), value.clone());
                }
                match event {
                    "analysis_candidate_started" | "analysis_candidate_progress" => {
                        track_object.insert("status".into(), serde_json::json!("running"));
                        if event == "analysis_candidate_started" {
                            track_object.insert("startedAt".into(), at.clone());
                        }
                    }
                    "analysis_candidate_finished" | "analysis_candidate_persisted" => {
                        if let Some(value) = details.get("status") {
                            track_object.insert("status".into(), value.clone());
                        } else {
                            track_object.insert("status".into(), serde_json::json!("completed"));
                        }
                        track_object.insert("finishedAt".into(), at.clone());
                        if let Some(value) = details.get("error").or_else(|| details.get("message")) {
                            track_object.insert("terminationReason".into(), value.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        match event {
            "analysis_requested" => {
                object.insert("status".into(), serde_json::json!("pending"));
                object.insert("requestedAt".into(), at.clone());
                if let Some(value) = details.get("attempt_id") {
                    object.insert("attemptId".into(), value.clone());
                }
                if let Some(value) = details.get("candidate_count") {
                    object.insert("total".into(), value.clone());
                }
            }
            "analysis_started" => {
                object.insert("status".into(), serde_json::json!("running"));
                object.insert("startedAt".into(), at.clone());
                if let Some(value) = details.get("attempt_id") {
                    object.insert("attemptId".into(), value.clone());
                }
            }
            "analysis_candidate_progress" => {
                object.insert("status".into(), serde_json::json!("running"));
                object.insert("lastHeartbeatAt".into(), at.clone());
                for (key, field) in [("name", "currentItem"), ("stage", "currentStage"), ("worker_job_id", "workerJobId")] {
                    if let Some(value) = details.get(key) {
                        object.insert(field.into(), value.clone());
                    }
                }
                for (key, field) in [("processed", "stageProcessed"), ("total", "stageTotal")] {
                    if let Some(value) = details.get(key) {
                        object.insert(field.into(), value.clone());
                    }
                }
            }
            "analysis_cancelled" | "analysis_error" => {
                object.insert("status".into(), serde_json::json!(if event == "analysis_cancelled" { "cancelled" } else { "error" }));
                object.insert("finishedAt".into(), at.clone());
                if let Some(value) = details.get("reason").or_else(|| details.get("message")) {
                    object.insert("terminationReason".into(), value.clone());
                }
                for track in track_map.values_mut() {
                    if let Some(track_object) = track.as_object_mut()
                        && matches!(track_object.get("status").and_then(serde_json::Value::as_str), Some("pending" | "running"))
                    {
                        track_object.insert("status".into(), serde_json::json!(if event == "analysis_cancelled" { "cancelled" } else { "failed" }));
                        track_object.insert("finishedAt".into(), at.clone());
                    }
                }
            }
            "analysis_completed" => {
                object.insert("finishedAt".into(), at.clone());
            }
            _ => {}
        }

        object.insert("lastHeartbeatAt".into(), at);
        object.insert("lastHeartbeatEpochMs".into(), serde_json::json!(unix_timestamp_ms()));

        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut timed_out = 0usize;
        let mut terminal = 0usize;
        for track in track_map.values() {
            match track.get("status").and_then(serde_json::Value::as_str) {
                Some("completed") => { completed += 1; terminal += 1; }
                Some("failed") => { failed += 1; terminal += 1; }
                Some("timeout") => { timed_out += 1; terminal += 1; }
                Some("cancelled") => { terminal += 1; }
                _ => {}
            }
        }
        let total = object.get("total").and_then(serde_json::Value::as_u64).unwrap_or(track_map.len() as u64) as usize;
        object.insert("completed".into(), serde_json::json!(completed));
        object.insert("failed".into(), serde_json::json!(failed));
        object.insert("timedOut".into(), serde_json::json!(timed_out));
        object.insert("pending".into(), serde_json::json!(total.saturating_sub(terminal)));
        if event == "analysis_completed" {
            object.insert(
                "status".into(),
                serde_json::json!(if failed > 0 || timed_out > 0 || terminal < total {
                    "partial"
                } else {
                    "completed"
                }),
            );
        }
        object.insert("tracks".into(), serde_json::Value::Object(track_map));
        let _ = write_json_file(&self.analysis_state_path, &state);
    }

    fn record_task_started(&self, slot_index: usize, source: &str, destination: &str, count: usize) {
        self.record_event(
            "task_started",
            serde_json::json!({
                "slot_index": slot_index,
                "source_directory": source,
                "destination_directory": destination,
                "candidate_count": count,
            }),
        );
    }

    fn record_candidate_result(
        &self,
        candidate: &PreviewCandidate,
        status: &str,
        error: Option<&str>,
    ) {
        let input_snapshot = monitor_file_snapshot(Path::new(&candidate.source_path));
        let output_snapshot = monitor_file_snapshot(Path::new(&candidate.destination_path));
        self.record_event(
            "candidate_result",
            serde_json::json!({
                "status": status,
                "error": error,
                "input_snapshot": input_snapshot,
                "output_snapshot": output_snapshot,
                "input": {
                    "path": candidate.source_path,
                    "name": candidate.name,
                    "size_bytes": candidate.source_size_bytes,
                },
                "output": {
                    "path": candidate.destination_path,
                    "estimated_size_bytes": candidate.estimated_output_bytes,
                    "operation": candidate.operation,
                },
            }),
        );
    }

    fn record_task_finished(&self, entry: &HistoryEntry) {
        let summary_path = self
            .session_dir
            .join(format!("summary-slot-{}.json", entry.slot_index + 1));
        if let Ok(contents) = serde_json::to_string_pretty(entry)
            && let Err(error) = fs::write(summary_path, contents)
        {
            eprintln!("Failed to save runtime session summary: {error}");
        }

        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        if let Ok(mut session) = self.session.lock() {
            session.updated_at = timestamp_string();
            session.task_results.push(serde_json::json!({
                "slot_index": entry.slot_index,
                "history_id": entry.id,
                "status": entry.status,
                "completed_count": entry.completed_count,
                "failed_count": entry.failed_count,
                "report_path": entry.report_path,
            }));
            if self.remaining_jobs.fetch_sub(1, Ordering::SeqCst) == 1 {
                session.status = if session
                    .task_results
                    .iter()
                    .all(|result| result.get("status").and_then(serde_json::Value::as_str) == Some("completed"))
                {
                    "completed"
                } else if session
                    .task_results
                    .iter()
                    .any(|result| result.get("status").and_then(serde_json::Value::as_str) == Some("cancelled"))
                {
                    "cancelled"
                } else if session
                    .task_results
                    .iter()
                    .any(|result| result.get("status").and_then(serde_json::Value::as_str) == Some("error"))
                {
                    "error"
                } else {
                    "partial"
                }
                .to_string();
                session.finished_at = Some(timestamp_string());
            }
            let _ = write_json_file(&self.session_path, &*session);
        }
        drop(_guard);
        self.record_event(
            "task_finished",
            serde_json::json!({
                "slot_index": entry.slot_index,
                "history_id": entry.id,
                "status": entry.status,
                "completed_count": entry.completed_count,
                "failed_count": entry.failed_count,
                "report_path": entry.report_path,
            }),
        );
    }

    fn record_analysis_reports(&self, reports: &[AnalysisReport]) {
        let snapshot = {
            let Ok(mut all_reports) = self.analysis_reports.lock() else {
                return;
            };
            for report in reports {
                all_reports.retain(|existing| existing.source_path != report.source_path);
                all_reports.push(report.clone());
            }
            all_reports.clone()
        };
        let path = self.session_dir.join("analysis-reports.json");
        if let Ok(contents) = serde_json::to_string_pretty(&snapshot)
            && let Err(error) = fs::write(path, contents)
        {
            eprintln!("Failed to save runtime session analysis reports: {error}");
        }
        self.record_event(
            "analysis_reports_updated",
            serde_json::json!({
                "report_count": snapshot.len(),
                "reports": snapshot,
            }),
        );
    }

    fn write_session_file(&self) -> io::Result<()> {
        let session = self
            .session
            .lock()
            .map_err(|_| io::Error::other("test monitor session lock poisoned"))?;
        write_json_file(&self.session_path, &*session)
    }

    fn claim_analysis_run(&self, attempt_id: &str) -> Result<(), String> {
        if attempt_id.trim().is_empty() {
            return Err("增强分析缺少 attemptId".to_string());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "运行会话锁损坏".to_string())?;
        let mut state = fs::read_to_string(&self.analysis_state_path)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
            .unwrap_or_else(|| serde_json::json!({"schemaVersion": 1, "status": "notRequested"}));
        let object = state
            .as_object_mut()
            .ok_or_else(|| "分析状态文件格式无效".to_string())?;
        let now_ms = unix_timestamp_ms();
        if object.get("status").and_then(serde_json::Value::as_str) == Some("running") {
            let current_attempt = object
                .get("attemptId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let heartbeat = object
                .get("lastHeartbeatEpochMs")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if current_attempt != attempt_id && now_ms.saturating_sub(heartbeat) <= 15_000 {
                return Err("该批次增强分析已有运行中的尝试".to_string());
            }
            if current_attempt != attempt_id {
                object.insert("status".into(), serde_json::json!("interrupted"));
                object.insert("interruptionReason".into(), serde_json::json!("超过 15 秒未收到心跳，已允许接管"));
            }
        }
        object.insert("attemptId".into(), serde_json::json!(attempt_id));
        object.insert("status".into(), serde_json::json!("running"));
        object.insert("startedAt".into(), serde_json::json!(timestamp_string()));
        object.insert("lastHeartbeatEpochMs".into(), serde_json::json!(now_ms));
        write_json_file(&self.analysis_state_path, &state)
            .map_err(|error| format!("保存增强分析状态失败：{error}"))
    }
}

fn write_json_file<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let contents = serde_json::to_vec_pretty(value)
        .map_err(|error| io::Error::other(format!("serialize json: {error}")))?;
    let temporary_path = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|extension| extension.to_str()).unwrap_or("json")
    ));
    fs::write(&temporary_path, contents)?;
    fs::rename(temporary_path, path)
}

fn monitor_safe_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "batch".to_string()
    } else {
        sanitized
    }
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn monitor_file_snapshot(path: &Path) -> serde_json::Value {
    match fs::metadata(path) {
        Ok(metadata) => serde_json::json!({
            "path": path,
            "exists": true,
            "is_file": metadata.is_file(),
            "size_bytes": metadata.len(),
            "modified_at": metadata.modified().ok().and_then(|modified| {
                modified.duration_since(UNIX_EPOCH).ok().map(|duration| duration.as_secs())
            }),
        }),
        Err(error) => serde_json::json!({
            "path": path,
            "exists": false,
            "error": error.to_string(),
        }),
    }
}

fn default_download_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| home.join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("Downloads"))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ScanStatus {
    Idle,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ScanProgressPhase {
    Preparing,
    ScanningSource,
    ScanningDestination,
    MatchingMetadata,
    Checking,
    Completed,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ScanProgress {
    status: ScanStatus,
    phase: ScanProgressPhase,
    processed: usize,
    total: usize,
    current_file: String,
    message: String,
    tasks: Vec<ScanTaskProgress>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
struct ScanTaskProgress {
    slot_index: usize,
    phase: ScanProgressPhase,
    processed: usize,
    total: usize,
    source_processed: usize,
    source_total: Option<usize>,
    destination_processed: usize,
    destination_total: Option<usize>,
    metadata_processed: usize,
    metadata_total: Option<usize>,
    current_file: String,
}

impl Default for ScanProgress {
    fn default() -> Self {
        Self {
            status: ScanStatus::Idle,
            phase: ScanProgressPhase::Preparing,
            processed: 0,
            total: 0,
            current_file: String::new(),
            message: String::new(),
            tasks: Vec::new(),
        }
    }
}

struct ConfirmedSyncJob {
    batch_id: String,
    slot_index: usize,
    source: String,
    destination: String,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    candidates: Vec<PreviewCandidate>,
    analyses: Vec<EmbeddedAnalysis>,
    analysis_failures: Vec<AnalysisFailure>,
    preview: SyncPreview,
    retry_of: Option<String>,
    test_monitor: Option<Arc<TestMonitor>>,
    w4dj_path: PathBuf,
    metadata_context: Arc<ConversionMetadataContext>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct AnalysisFailure {
    path: String,
    message: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    stage: Option<String>,
    #[serde(rename = "elapsedMs", default)]
    elapsed_ms: Option<u64>,
}

struct ScanJob {
    conversion_mode: ConversionMode,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    scan_cache_path: PathBuf,
    concurrency_budget: Arc<GlobalConcurrencyBudget>,
    metadata_resolver: Arc<NeteaseMetadataResolver>,
    tasks: Vec<(usize, String, String)>,
}

#[derive(serde::Serialize)]
struct AppInfo {
    version: &'static str,
    developer: &'static str,
    project_url: &'static str,
}

#[derive(Debug, Clone, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AnalysisSessionSummary {
    status: String,
    total: usize,
    completed: usize,
    failed: usize,
    timed_out: usize,
    pending: usize,
    current_item: Option<String>,
    current_stage: Option<String>,
    worker_job_id: Option<String>,
    requested_at: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    termination_reason: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct HistoryEntryView {
    #[serde(flatten)]
    entry: HistoryEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    analysis: Option<AnalysisSessionSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteAnalysisRun {
    batch_id: String,
    previews: Vec<SlotPreview>,
    analysis: AnalysisSessionSummary,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryStatus {
    catalog_path: String,
    track_count: u64,
    analyzed_track_count: u64,
    /// Empty compatibility data for Task 1 discovery; Dashboard queries the
    /// W4DJ analysis projection instead of this field.
    netease: NeteaseDiscovery,
    manual_database_path: Option<String>,
    refresh: LibraryRefreshProgress,
    database_warning: Option<String>,
    total_track_count: u64,
    available_track_count: u64,
    invalid_track_count: u64,
    not_analyzed_count: u64,
    analysis_failed_count: u64,
    analysis_completed_count: u64,
    invalid_scan: InvalidScanProgress,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum NeteaseMetadataDatabaseSource {
    Manual,
    Automatic,
    Unavailable,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct NeteaseMetadataDatabaseStatus {
    manual_path: Option<String>,
    effective_path: Option<String>,
    source: NeteaseMetadataDatabaseSource,
    loaded: bool,
    record_count: usize,
    warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cached_record_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    database_changed: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InvalidScanProgress {
    scan_id: String,
    status: String,
    processed: usize,
    total: usize,
    current_item: String,
    message: String,
    error: Option<String>,
}

impl Default for InvalidScanProgress {
    fn default() -> Self {
        Self {
            scan_id: String::new(),
            status: "idle".to_string(),
            processed: 0,
            total: 0,
            current_item: String::new(),
            message: String::new(),
            error: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryRefreshSummary {
    track_count: u64,
    local_file_count: usize,
    readable_file_count: usize,
    reused_file_count: usize,
    database_path: String,
    music_folder: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum LibraryRefreshStatus {
    Idle,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum LibraryRefreshStage {
    LocatingDatabase,
    ReadingRecords,
    CheckingLocalFiles,
    ProbingLocalFiles,
    ImportingAnalysis,
    Committing,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryRefreshProgress {
    refresh_id: String,
    status: LibraryRefreshStatus,
    stage: LibraryRefreshStage,
    processed: usize,
    total: Option<usize>,
    current_item: String,
    message: String,
    summary: Option<LibraryRefreshSummary>,
    error: Option<String>,
}

impl Default for LibraryRefreshProgress {
    fn default() -> Self {
        Self {
            refresh_id: String::new(),
            status: LibraryRefreshStatus::Idle,
            stage: LibraryRefreshStage::LocatingDatabase,
            processed: 0,
            total: None,
            current_item: String::new(),
            message: String::new(),
            summary: None,
            error: None,
        }
    }
}

impl LibraryState {
    fn new() -> Self {
        Self {
            catalog_path: Mutex::new(PathBuf::new()),
            manual_database_path: Mutex::new(None),
            metadata_cache: Mutex::new(NeteaseMetadataCacheProgress::default()),
            metadata_cache_cancel: AtomicBool::new(false),
            metadata_cache_build_lock: Mutex::new(()),
            metadata_cache_worker: Mutex::new(None),
            refresh: Mutex::new(LibraryRefreshProgress::default()),
            cancel: AtomicBool::new(false),
            worker: Mutex::new(None),
            invalid_scan: Mutex::new(InvalidScanProgress::default()),
            invalid_scan_cancel: AtomicBool::new(false),
            invalid_scan_worker: Mutex::new(None),
        }
    }
}

const ESSENTIA_MODEL_VERSION: &str = "essentia-musicnn-2022-v2";
const ESSENTIA_MODELS_URL: &str = "https://essentia.upf.edu/models/";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelStatus {
    version: &'static str,
    embedding: bool,
    genre: bool,
    mood: bool,
    instrument: bool,
    installing: bool,
    emotion_continuous: bool,
    emotion_cluster: bool,
    discogs_effnet: Option<DiscogsEffnetModelStatus>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscogsEffnetModelStatus {
    embedding: bool,
    mood_theme: bool,
    approachability: bool,
    instrumentation: bool,
    timbre: bool,
    danceability: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelImportIssueDto {
    file_name: String,
    reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelImportResult {
    installed_ids: Vec<String>,
    issues: Vec<EssentiaModelImportIssueDto>,
    missing_ids: Vec<String>,
    status: EssentiaModelStatus,
    message: String,
}

#[derive(Debug, Clone, Copy)]
struct EssentiaModelSpec {
    id: &'static str,
    kind: &'static str,
    classes: &'static [&'static str],
    output_units: Option<u64>,
    output_name: &'static str,
    embedding_family: Option<&'static str>,
    input_shape: Option<&'static [u64]>,
    input_width: Option<u64>,
}

const DISCOGS_EFFNET_INPUT_SHAPE: &[u64] = &[64, 128, 96];
const DISCOGS_MOOD_THEME_CLASSES: &[&str] = &[
    "action", "adventure", "advertising", "background", "ballad", "calm", "children",
    "christmas", "commercial", "cool", "corporate", "dark", "deep", "documentary",
    "drama", "dramatic", "dream", "emotional", "energetic", "epic", "fast", "film", "fun",
    "funny", "game", "groovy", "happy", "heavy", "holiday", "hopeful", "inspiring", "love",
    "meditative", "melancholic", "melodic", "motivational", "movie", "nature", "party",
    "positive", "powerful", "relaxing", "retro", "romantic", "sad", "sexy", "slow", "soft",
    "soundscape", "space", "sport", "summer", "trailer", "travel", "upbeat", "uplifting",
];
const DISCOGS_INSTRUMENTATION_CLASSES: &[&str] = &[
    "accordion", "acousticbassguitar", "acousticguitar", "bass", "beat", "bell", "bongo",
    "brass", "cello", "clarinet", "classicalguitar", "computer", "doublebass", "drummachine",
    "drums", "electricguitar", "electricpiano", "flute", "guitar", "harmonica", "harp", "horn",
    "keyboard", "oboe", "orchestra", "organ", "pad", "percussion", "piano", "pipeorgan", "rhodes",
    "sampler", "saxophone", "strings", "synthesizer", "trombone", "trumpet", "viola", "violin", "voice",
];
const DISCOGS_APPROACHABILITY_CLASSES: &[&str] = &["not approachable", "approachable"];
const DISCOGS_TIMBRE_CLASSES: &[&str] = &["bright", "dark"];
const DISCOGS_DANCEABILITY_CLASSES: &[&str] = &["danceable", "not_danceable"];

fn essentia_model_specs() -> Vec<EssentiaModelSpec> {
    vec![
        EssentiaModelSpec {
            id: "musicnn_embedding",
            kind: "embedding",
            classes: &[],
            output_units: None,
            output_name: "model/dense/Relu",
            embedding_family: Some("musicnn"), input_shape: None, input_width: None,
        },
        EssentiaModelSpec {
            id: "mood_aggressive",
            kind: "mood",
            classes: &["aggressive", "non_aggressive"],
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "mood_happy",
            kind: "mood",
            classes: &["happy", "non_happy"],
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "mood_relaxed",
            kind: "mood",
            classes: &["relaxed", "non_relaxed"],
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "mood_party",
            kind: "mood",
            classes: &["party", "non_party"],
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "mood_sad",
            kind: "mood",
            classes: &["sad", "non_sad"],
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "voice_instrumental",
            kind: "instrument",
            classes: &["instrumental", "voice"],
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "emomusic",
            kind: "emotionContinuous",
            classes: &["valence", "arousal"],
            output_units: Some(2),
            output_name: "model/Identity",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "muse",
            kind: "emotionContinuous",
            classes: &["valence", "arousal"],
            output_units: Some(2),
            output_name: "model/Identity",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "mirex",
            kind: "emotionCluster",
            classes: &["passionate", "rollicking", "literate", "humorous", "aggressive"],
            output_units: Some(5),
            output_name: "PartitionedCall",
            embedding_family: Some("musicnn"), input_shape: None, input_width: Some(200),
        },
        EssentiaModelSpec {
            id: "discogs_effnet",
            kind: "genreEmbedding",
            classes: &[],
            output_units: Some(1280),
            output_name: "discogs_embedding",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: Some(DISCOGS_EFFNET_INPUT_SHAPE), input_width: None,
        },
        EssentiaModelSpec {
            id: "discogs_effnet_embedding",
            kind: "discogsEffnetEmbedding",
            classes: &[],
            output_units: Some(1280),
            output_name: "discogs_embedding",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: Some(DISCOGS_EFFNET_INPUT_SHAPE), input_width: None,
        },
        EssentiaModelSpec {
            id: "genre_discogs400",
            kind: "genre",
            classes: &[],
            output_units: Some(400),
            output_name: "discogs_genre",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: None, input_width: Some(1280),
        },
        EssentiaModelSpec {
            id: "discogs_mood_theme",
            kind: "discogsEffnetHead",
            classes: DISCOGS_MOOD_THEME_CLASSES,
            output_units: Some(56),
            output_name: "model/Sigmoid",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: None, input_width: Some(1280),
        },
        EssentiaModelSpec {
            id: "discogs_approachability",
            kind: "discogsEffnetHead",
            classes: DISCOGS_APPROACHABILITY_CLASSES,
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: None, input_width: Some(1280),
        },
        EssentiaModelSpec {
            id: "discogs_instrumentation",
            kind: "discogsEffnetHead",
            classes: DISCOGS_INSTRUMENTATION_CLASSES,
            output_units: Some(40),
            output_name: "model/Sigmoid",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: None, input_width: Some(1280),
        },
        EssentiaModelSpec {
            id: "discogs_timbre",
            kind: "discogsEffnetHead",
            classes: DISCOGS_TIMBRE_CLASSES,
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: None, input_width: Some(1280),
        },
        EssentiaModelSpec {
            id: "discogs_danceability",
            kind: "discogsEffnetHead",
            classes: DISCOGS_DANCEABILITY_CLASSES,
            output_units: Some(2),
            output_name: "model/Softmax",
            embedding_family: Some("discogs-effnet-bs64-1"), input_shape: None, input_width: Some(1280),
        },
    ]
}

fn essentia_models_path(state: &tauri::State<'_, AppState>) -> PathBuf {
    state
        .models_path
        .lock()
        .expect("models path lock poisoned")
        .clone()
}

fn essentia_model_file_path(models_path: &Path, id: &str, extension: &str) -> PathBuf {
    models_path.join(format!("{id}.{extension}"))
}

fn essentia_model_is_installed(models_path: &Path, spec: EssentiaModelSpec) -> bool {
    essentia_model_import::installed_model_pair_is_valid(models_path, spec.id)
}

fn essentia_embedding_is_installed(models_path: &Path) -> bool {
    essentia_model_specs()
        .into_iter()
        .find(|spec| spec.id == "musicnn_embedding")
        .is_some_and(|spec| essentia_model_is_installed(models_path, spec))
}

fn essentia_model_status_for_path(models_path: &Path) -> EssentiaModelStatus {
    let specs = essentia_model_specs();
    let embedding = essentia_embedding_is_installed(models_path);
    let genre_embedding = specs
        .iter()
        .find(|spec| spec.id == "discogs_effnet_embedding")
        .is_some_and(|spec| essentia_model_is_installed(models_path, *spec))
        || specs
            .iter()
            .find(|spec| spec.id == "discogs_effnet")
            .is_some_and(|spec| essentia_model_is_installed(models_path, *spec));
    let genre_head = specs
        .iter()
        .find(|spec| spec.id == "genre_discogs400")
        .is_some_and(|spec| essentia_model_is_installed(models_path, *spec));
    let discogs_status = |id: &str| {
        specs
            .iter()
            .find(|spec| spec.id == id)
            .is_some_and(|spec| essentia_model_is_installed(models_path, *spec))
    };
    EssentiaModelStatus {
        version: ESSENTIA_MODEL_VERSION,
        embedding,
        genre: embedding && genre_embedding && genre_head,
        mood: specs
            .iter()
            .filter(|spec| spec.kind == "mood")
            .all(|spec| essentia_model_is_installed(models_path, *spec))
            && embedding,
        instrument: specs
            .iter()
            .any(|spec| spec.kind == "instrument" && essentia_model_is_installed(models_path, *spec))
            && embedding,
        installing: false,
        emotion_continuous: specs
            .iter()
            .filter(|spec| spec.kind == "emotionContinuous")
            .all(|spec| essentia_model_is_installed(models_path, *spec))
            && embedding,
        emotion_cluster: specs
            .iter()
            .any(|spec| spec.kind == "emotionCluster" && essentia_model_is_installed(models_path, *spec))
            && embedding,
        discogs_effnet: Some(DiscogsEffnetModelStatus {
            embedding: specs
                .iter()
                .find(|spec| spec.id == "discogs_effnet_embedding")
                .is_some_and(|spec| essentia_model_is_installed(models_path, *spec))
                || genre_embedding,
            mood_theme: discogs_status("discogs_mood_theme"),
            approachability: discogs_status("discogs_approachability"),
            instrumentation: discogs_status("discogs_instrumentation"),
            timbre: discogs_status("discogs_timbre"),
            danceability: discogs_status("discogs_danceability"),
        }),
    }
}

#[tauri::command]
fn get_essentia_model_status(state: tauri::State<'_, AppState>) -> Result<EssentiaModelStatus, String> {
    let path = essentia_models_path(&state);
    if path.as_os_str().is_empty() {
        return Err("Essentia 模型目录尚未准备好".to_string());
    }
    Ok(essentia_model_status_for_path(&path))
}

/// Validate and install the bundled model files on demand.
///
/// Model files are deliberately not touched during application startup. The
/// first enhanced-analysis request calls this command, so ordinary conversion
/// and the initial Dashboard render do not pay the model verification/copy
/// cost. The write lock keeps this operation safe if two analysis entry points
/// race to initialize the model directory.
#[tauri::command]
fn ensure_essentia_models(state: tauri::State<'_, AppState>) -> Result<EssentiaModelStatus, String> {
    let _models_write_guard = state
        .models_write_lock
        .lock()
        .expect("Essentia model write lock poisoned");
    let models_path = essentia_models_path(&state);
    if models_path.as_os_str().is_empty() {
        return Err("Essentia 模型目录尚未准备好".to_string());
    }
    let bundled_models_path = state
        .bundled_models_path
        .lock()
        .expect("bundled models path lock poisoned")
        .clone();
    if bundled_models_path.as_os_str().is_empty() {
        return Err("内置 Essentia 模型目录尚未准备好".to_string());
    }
    essentia_model_import::install_bundled_model_set(
        &bundled_models_path,
        &models_path,
        false,
    )?;
    Ok(essentia_model_status_for_path(&models_path))
}

#[tauri::command]
fn import_essentia_models(
    paths: Vec<String>,
    state: tauri::State<'_, AppState>,
) -> Result<EssentiaModelImportResult, String> {
    let _models_write_guard = state
        .models_write_lock
        .lock()
        .expect("Essentia model write lock poisoned");
    let models_path = essentia_models_path(&state);
    if models_path.as_os_str().is_empty() {
        return Err("Essentia 模型目录尚未准备好".to_string());
    }
    let input_paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    let report = essentia_model_import::import_model_paths(&input_paths, &models_path)?;
    let status = essentia_model_status_for_path(&models_path);
    let importable_ids = essentia_model_import::known_import_model_ids();
    let mut missing_ids = essentia_model_specs()
        .into_iter()
        .filter(|spec| importable_ids.contains(&spec.id))
        .filter(|spec| !essentia_model_is_installed(&models_path, *spec))
        .map(|spec| spec.id.to_string())
        .collect::<Vec<_>>();
    missing_ids.sort();
    missing_ids.dedup();
    let installed_count = report.installed_ids.len();
    let issue_count = report.issues.len();
    let message = if missing_ids.is_empty() && issue_count == 0 {
        format!("已导入 {installed_count} 个 Essentia 模型。")
    } else if installed_count > 0 {
        format!(
            "已导入 {installed_count} 个模型，仍缺少 {} 个模型{}。",
            missing_ids.len(),
            if issue_count > 0 {
                format!("；{issue_count} 个文件未导入")
            } else {
                String::new()
            }
        )
    } else {
        "没有可安装的 Essentia 模型；请检查文件格式和官方模型目录。".to_string()
    };
    Ok(EssentiaModelImportResult {
        installed_ids: report.installed_ids,
        issues: report
            .issues
            .into_iter()
            .map(|issue| EssentiaModelImportIssueDto {
                file_name: issue.file_name,
                reason: issue.reason,
            })
            .collect(),
        missing_ids,
        status,
        message,
    })
}

#[tauri::command]
fn restore_bundled_essentia_models(
    state: tauri::State<'_, AppState>,
) -> Result<EssentiaModelStatus, String> {
    let _models_write_guard = state
        .models_write_lock
        .lock()
        .expect("Essentia model write lock poisoned");
    let models_path = essentia_models_path(&state);
    if models_path.as_os_str().is_empty() {
        return Err("Essentia 模型目录尚未准备好".to_string());
    }
    let bundled_models_path = state
        .bundled_models_path
        .lock()
        .expect("bundled models path lock poisoned")
        .clone();
    if bundled_models_path.as_os_str().is_empty() {
        return Err("内置 Essentia 模型目录尚未准备好".to_string());
    }
    essentia_model_import::install_bundled_model_set(
        &bundled_models_path,
        &models_path,
        true,
    )?;
    Ok(essentia_model_status_for_path(&models_path))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelFile {
    id: String,
    model_json: String,
    /// Sending model weights as a JSON `Vec<u8>` makes WebKit materialize one
    /// JavaScript number for every byte before the promise can resolve.  The
    /// largest bundled model is ~18 MB, so that representation can block the
    /// UI thread for minutes.  Keep the command payload compact and decode it
    /// to a Uint8Array in the frontend before transferring it to the Worker.
    weight_data_base64: String,
    classes: Vec<String>,
    kind: String,
    output_name: String,
    output_units: Option<u64>,
    embedding_family: Option<&'static str>,
    input_shape: Option<Vec<u64>>,
    input_width: Option<u64>,
    version: &'static str,
}

#[tauri::command]
fn load_essentia_model(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<EssentiaModelFile, String> {
    let spec = essentia_model_specs()
        .into_iter()
        .find(|spec| spec.id == id)
        .ok_or_else(|| "未知的 Essentia 模型".to_string())?;
    let path = essentia_models_path(&state);
    let model_path = essentia_model_file_path(&path, spec.id, "json");
    let weights_path = essentia_model_file_path(&path, spec.id, "bin");
    if !essentia_model_is_installed(&path, spec) {
        return Err("Essentia 模型尚未安装或已损坏".to_string());
    }
    let model_json = fs::read_to_string(model_path)
        .map_err(|error| format!("读取 Essentia 模型结构失败：{error}"))?;
    let weight_data = fs::read(weights_path)
        .map_err(|error| format!("读取 Essentia 模型权重失败：{error}"))?;
    let classes = if spec.id == "genre_discogs400" {
        serde_json::from_str::<Vec<String>>(include_str!("../resources/essentia-models/genre_discogs400.labels.json"))
            .map_err(|error| format!("读取 Discogs Genre 标签失败：{error}"))?
    } else {
        spec.classes.iter().map(|value| (*value).to_string()).collect()
    };
    Ok(EssentiaModelFile {
        id: spec.id.to_string(),
        model_json,
        weight_data_base64: base64::engine::general_purpose::STANDARD.encode(weight_data),
        classes,
        kind: spec.kind.to_string(),
        output_name: spec.output_name.to_string(),
        output_units: spec.output_units,
        embedding_family: spec.embedding_family,
        input_shape: spec.input_shape.map(|shape| shape.to_vec()),
        input_width: spec.input_width,
        version: ESSENTIA_MODEL_VERSION,
    })
}

#[derive(serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    name: Option<String>,
}

#[derive(serde::Serialize)]
struct UpdateCheckResult {
    current_version: &'static str,
    latest_version: String,
    update_available: bool,
    release_url: String,
    release_name: String,
}

#[derive(Clone, Default)]
struct DestinationCoordinator {
    locks: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>,
}

struct InstanceLock {
    _file: fs::File,
}

impl DestinationCoordinator {
    fn lock_for(&self, destination: &Path) -> Arc<Mutex<()>> {
        let key = fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
        let mut locks = self.locks.lock().expect("destination lock map poisoned");
        Arc::clone(locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
    }
}

fn acquire_single_instance_lock() -> io::Result<Option<InstanceLock>> {
    let lock_path = std::env::temp_dir().join("w4dj-rkb.desktop.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;

        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(error);
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::mem::zeroed;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
        };
        use windows_sys::Win32::System::IO::OVERLAPPED;

        let mut overlapped = unsafe { zeroed::<OVERLAPPED>() };
        let locked = unsafe {
            LockFileEx(
                file.as_raw_handle() as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                u32::MAX,
                u32::MAX,
                &mut overlapped,
            )
        };

        if locked == 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(error);
        }
    }

    let _ = writeln!(&file, "{}", std::process::id());
    Ok(Some(InstanceLock { _file: file }))
}

#[tauri::command]
fn load_desktop_state(state: tauri::State<'_, AppState>) -> DesktopState {
    state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone()
}

fn concurrency_budget_snapshot(state: &AppState) -> Arc<GlobalConcurrencyBudget> {
    state
        .concurrency_budget
        .lock()
        .expect("concurrency budget lock poisoned")
        .clone()
}

#[tauri::command]
fn select_source_directory(
    slot_index: usize,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.select_source_directory(slot_index, path)?;
        controller.state().clone()
    };
    persist_preferences(&state);
    Ok(snapshot)
}

#[tauri::command]
fn select_destination_directory(
    slot_index: usize,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.select_destination_directory(slot_index, path)?;
        controller.state().clone()
    };
    persist_preferences(&state);
    Ok(snapshot)
}

fn validate_destination_directory(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err(String::from("输出目录为空"));
    }
    if !path.is_dir() {
        return Err(format!("输出目录不存在或不是文件夹：{}", path.display()));
    }
    Ok(())
}

#[tauri::command]
fn open_destination(path: String) -> Result<(), String> {
    let destination = PathBuf::from(path.trim());
    validate_destination_directory(&destination)?;

    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer");
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = Command::new("xdg-open");

    command
        .arg(&destination)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开输出目录：{error}"))
}

#[tauri::command]
fn open_source(path: String) -> Result<(), String> {
    let source = PathBuf::from(path.trim());
    if source.as_os_str().is_empty() {
        return Err(String::from("输入来源为空"));
    }
    if !source.exists() {
        return Err(format!("输入来源不存在：{}", source.display()));
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        if source.is_file() {
            command.arg("-R");
        }
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        if source.is_file() {
            command.arg("/select,");
        }
        command
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = Command::new("xdg-open");

    command
        .arg(&source)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开输入来源：{error}"))
}

fn is_analyzable_audio_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "mp3" | "flac" | "ncm" | "wav" | "aiff" | "aif"
                )
            })
}

fn collect_analyzable_audio_files(path: &Path, output: &mut Vec<String>) -> io::Result<()> {
    if is_analyzable_audio_file(path) {
        output.push(path.to_string_lossy().into_owned());
        return Ok(());
    }

    if !path.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        collect_analyzable_audio_files(&entry_path, output)?;
    }

    Ok(())
}

#[tauri::command]
fn list_audio_files(path: String) -> Result<Vec<String>, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(String::from("音乐目录为空"));
    }
    let path = PathBuf::from(trimmed);
    if !path.exists() {
        return Err(format!("音乐目录不存在：{}", path.display()));
    }

    let mut files = Vec::new();
    collect_analyzable_audio_files(&path, &mut files)
        .map_err(|error| format!("扫描音乐目录失败：{error}"))?;
    files.sort();
    Ok(files)
}

#[tauri::command]
fn read_audio_file(path: String) -> Result<Vec<u8>, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(String::from("音频路径为空"));
    }
    let path = PathBuf::from(trimmed);
    if !is_analyzable_audio_file(&path) {
        return Err(format!("暂不支持分析此音频文件：{}", path.display()));
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ncm"))
    {
        let file = fs::File::open(&path).map_err(|error| format!("读取 NCM 文件失败：{error}"))?;
        let mut ncm = Ncmdump::from_reader(file).map_err(|error| format!("解析 NCM 文件失败：{error}"))?;
        return ncm
            .get_data()
            .map_err(|error| format!("提取 NCM 音频数据失败：{error}"));
    }
    fs::read(&path).map_err(|error| format!("读取音频文件失败：{error}"))
}

#[tauri::command]
fn read_audio_metadata(path: String) -> Result<TrackMetadata, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(String::from("音频路径为空"));
    }
    let path = PathBuf::from(trimmed);
    if !is_analyzable_audio_file(&path) {
        return Err(format!("暂不支持读取此音频元数据：{}", path.display()));
    }
    Ok(read_track_metadata(&path))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioFileFingerprint {
    size_bytes: u64,
    modified_at: Option<u64>,
}

#[tauri::command]
fn get_audio_file_fingerprint(path: String) -> Result<AudioFileFingerprint, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(String::from("音频路径为空"));
    }
    let metadata = fs::metadata(trimmed).map_err(|error| format!("读取音频文件信息失败：{error}"))?;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64);
    Ok(AudioFileFingerprint {
        size_bytes: metadata.len(),
        modified_at,
    })
}

fn current_analysis_path(state: &tauri::State<'_, AppState>) -> PathBuf {
    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    analysis_file_path(&history_path)
}

#[tauri::command]
fn load_track_analyses(state: tauri::State<'_, AppState>) -> Result<Vec<TrackAnalysis>, String> {
    let path = current_analysis_path(&state);
    let _guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    load_analysis_file(&path)
}

#[tauri::command]
fn save_track_analyses(
    entries: Vec<TrackAnalysis>,
    state: tauri::State<'_, AppState>,
) -> Result<usize, String> {
    let path = current_analysis_path(&state);
    let _guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    let existing = load_analysis_file(&path)?;
    let merged = merge_analysis_entries(existing.clone(), entries);
    let count = merged.len();
    save_analysis_file(&path, &merged)?;
    // This command persists the reusable analysis cache only. The output
    // owned W4DJ projection is updated by apply_track_analysis_results after
    // the destination file has received and passed a metadata read-back
    // check. Marking it completed here would make a cache-only result appear
    // as if the current MP3 had already been updated after a reload/cancel.
    Ok(count)
}

#[tauri::command]
fn clear_track_analyses(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let path = current_analysis_path(&state);
    let _guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    let existing = load_analysis_file(&path)?;
    clear_analysis_file(&path)?;
    let catalog_path = library_catalog_path(&state);
    let (mut catalog, _) = match open_w4dj_library(&catalog_path) {
        Ok(value) => value,
        Err(error) => {
            let _ = save_analysis_file(&path, &existing);
            return Err(format!("打开 W4DJ 分析歌曲库失败：{error}"));
        }
    };
    if let Err(error) = catalog.clear_analyses() {
        let _ = save_analysis_file(&path, &existing);
        return Err(format!("清除 W4DJ 分析歌曲库失败：{error}"));
    }
    Ok(())
}

#[tauri::command]
fn export_rekordbox_xml(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(String::from("请指定 Rekordbox XML 保存位置"));
    }
    let output_path = PathBuf::from(trimmed);
    let analysis_path = current_analysis_path(&state);

    let _guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    let entries = load_analysis_file(&analysis_path)?;
    if entries.is_empty() {
        return Err(String::from("还没有音乐分析结果，请先分析音乐库"));
    }
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建 XML 保存目录失败：{error}"))?;
    }
    let xml = build_rekordbox_xml(&entries, env!("CARGO_PKG_VERSION"));
    fs::write(output_path, xml).map_err(|error| format!("写入 Rekordbox XML 失败：{error}"))
}

#[tauri::command]
fn choose_mode(mode: Mode, state: tauri::State<'_, AppState>) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_mode(mode);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_lossless_format(
    format: Option<LosslessFormat>,
    state: tauri::State<'_, AppState>,
) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_lossless_format(format);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_conversion_mode(
    mode: ConversionMode,
    state: tauri::State<'_, AppState>,
) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_conversion_mode(mode);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_enhanced_mode(enabled: bool, state: tauri::State<'_, AppState>) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_enhanced_mode(enabled);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_conflict_strategy(
    strategy: ConflictStrategy,
    state: tauri::State<'_, AppState>,
) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_conflict_strategy(strategy);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_filename_rule(rule: FilenameRule, state: tauri::State<'_, AppState>) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_filename_rule(rule);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_netease_filename_format(
    format: NeteaseFilenameFormat,
    state: tauri::State<'_, AppState>,
) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_netease_filename_format(format);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_concurrency_limit(
    value: String,
    state: tauri::State<'_, AppState>,
) -> DesktopState {
    let fallback = {
        state
            .controller
            .lock()
            .expect("desktop lock poisoned")
            .state()
            .concurrency_limit
    };
    let parsed = value.trim().parse::<f64>().unwrap_or(f64::NAN);
    let normalized = w4dj::preferences::normalize_concurrency_limit(parsed, fallback);
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_concurrency_limit(normalized);
        controller.state().clone()
    };
    // The budget is swapped only for the next task.  Existing tasks retain
    // their Arc snapshot and therefore their original worker limit.
    *state
        .concurrency_budget
        .lock()
        .expect("concurrency budget lock poisoned") =
        Arc::new(GlobalConcurrencyBudget::new(normalized as usize));
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn start_sync(
    slot_index: usize,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let controller = Arc::clone(&state.controller);
    let destination_coordinator = state.destination_coordinator.clone();
    let concurrency_budget = concurrency_budget_snapshot(&state);
    let ffmpeg_registry = Arc::clone(&state.ffmpeg_registry);
    let metadata_context = conversion_metadata_context(&state);
    let w4dj_path = state
        .library
        .catalog_path
        .lock()
        .expect("library catalog path lock poisoned")
        .clone();
    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        if controller.is_running(slot_index)? {
            return Ok(controller.state().clone());
        }

        controller.start_sync(slot_index, 0)?;
        controller.push_log(slot_index, "Scanning input source")?;
    }

    thread::spawn(move || {
        run_sync_task(
            controller,
            destination_coordinator,
            w4dj_path,
            slot_index,
            concurrency_budget,
            ffmpeg_registry,
            metadata_context,
        )
    });

    Ok(state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone())
}

#[tauri::command]
fn pause_sync(
    slot_index: usize,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.pause_sync(slot_index)?;
    Ok(controller.state().clone())
}

#[tauri::command]
fn cancel_sync(
    slot_index: usize,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.cancel_sync(slot_index)?;
    Ok(controller.state().clone())
}

#[tauri::command]
fn start_all_sync(state: tauri::State<'_, AppState>) -> Result<DesktopState, String> {
    let controller = Arc::clone(&state.controller);
    let destination_coordinator = state.destination_coordinator.clone();
    let concurrency_budget = concurrency_budget_snapshot(&state);
    let ffmpeg_registry = Arc::clone(&state.ffmpeg_registry);
    let metadata_context = conversion_metadata_context(&state);
    let w4dj_path = state
        .library
        .catalog_path
        .lock()
        .expect("library catalog path lock poisoned")
        .clone();
    let slot_indexes = {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        let slot_indexes = controller.startable_slot_indexes();

        if slot_indexes.is_empty() {
            if controller.state().slots.iter().any(|slot| {
                !slot.source_directory.trim().is_empty()
                    && matches!(slot.status, w4dj::desktop::DesktopStatus::Running)
            }) {
                return Ok(controller.state().clone());
            }
            return Err(String::from("请至少选择一个歌曲文件夹或单曲"));
        }

        for &slot_index in &slot_indexes {
            controller.start_sync(slot_index, 0)?;
            controller.push_log(slot_index, "Scanning input source")?;
        }

        slot_indexes
    };

    for slot_index in slot_indexes {
        let controller = Arc::clone(&controller);
        let destination_coordinator = destination_coordinator.clone();
        let w4dj_path = w4dj_path.clone();
        let concurrency_budget = concurrency_budget.clone();
        let ffmpeg_registry = ffmpeg_registry.clone();
        let metadata_context = metadata_context.clone();
        thread::spawn(move || {
            run_sync_task(
                controller,
                destination_coordinator,
                w4dj_path,
                slot_index,
                concurrency_budget,
                ffmpeg_registry,
                metadata_context,
            )
        });
    }

    Ok(state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone())
}

#[tauri::command]
fn pause_all_sync(state: tauri::State<'_, AppState>) -> Result<DesktopState, String> {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.pause_all_running()?;
    Ok(controller.state().clone())
}

#[tauri::command]
fn cancel_all_sync(state: tauri::State<'_, AppState>) -> Result<DesktopState, String> {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.cancel_all_running()?;
    Ok(controller.state().clone())
}

#[tauri::command]
fn preview_all_sync(state: tauri::State<'_, AppState>) -> Result<Vec<SlotPreview>, String> {
    let (slot_indexes, mode, lossless_format, conflict_strategy, filename_rule, netease_filename_format, slots) = {
        let controller = state.controller.lock().expect("desktop lock poisoned");
        let slot_indexes = controller.startable_slot_indexes();
        let mode = controller.state().mode;
        let lossless_format = controller.state().lossless_format;
        let conflict_strategy = controller.state().conflict_strategy;
        let filename_rule = controller.state().filename_rule;
        let netease_filename_format = controller.state().netease_filename_format;
        let slots = slot_indexes
            .iter()
            .map(|slot_index| {
                let slot = &controller.state().slots[*slot_index];
                let destination = controller
                    .effective_destination(*slot_index)
                    .map_err(|error| error.to_string())?
                    .unwrap_or_default();
                Ok((*slot_index, slot.source_directory.clone(), destination))
            })
            .collect::<Result<Vec<_>, String>>()?;
        (
            slot_indexes,
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
            netease_filename_format,
            slots,
        )
    };

    if slot_indexes.is_empty() {
        return Err(String::from("请至少选择一个歌曲文件夹或单曲"));
    }

    let metadata_context = conversion_metadata_context(state.inner());
    let mut previews = slots
        .into_iter()
        .map(|(slot_index, source, destination)| {
            let mut preview = build_sync_preview_with_settings_and_netease_observed_with_policy_and_resolver(
                &source,
                &destination,
                mode,
                lossless_format,
                conflict_strategy,
                filename_rule,
                netease_filename_format,
                filename_normalization_policy_for_slot(slot_index),
                None,
                metadata_context.netease.as_ref(),
            )
            .map_err(|error| format!("预检失败：{error}"))?
            .ok_or_else(|| "预检被取消".to_string())?;
        if matches!(
            filename_normalization_policy_for_slot(slot_index),
            FilenameNormalizationPolicy::PreserveSource
        ) {
            attach_netease_identities(&mut preview, metadata_context.netease.as_ref());
        }
            Ok(SlotPreview {
                slot_index,
                mode,
                lossless_format,
                conflict_strategy,
                filename_rule,
                netease_filename_format,
                preview,
                retry_of: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    deduplicate_cross_slot_candidates(&mut previews);
    Ok(previews)
}

fn filename_normalization_policy_for_slot(slot_index: usize) -> FilenameNormalizationPolicy {
    if slot_index == 0 {
        FilenameNormalizationPolicy::PreserveSource
    } else {
        FilenameNormalizationPolicy::SoundCloud
    }
}

fn scan_progress_snapshot(progress: &Arc<Mutex<ScanProgress>>) -> ScanProgress {
    progress
        .lock()
        .expect("scan progress lock poisoned")
        .clone()
}

fn update_scan_progress(
    progress: &Arc<Mutex<ScanProgress>>,
    update: impl FnOnce(&mut ScanProgress),
) {
    let mut guard = progress.lock().expect("scan progress lock poisoned");
    update(&mut guard);
}

#[tauri::command]
fn load_scan_state(state: tauri::State<'_, AppState>) -> ScanProgress {
    scan_progress_snapshot(&state.scan_progress)
}

#[tauri::command]
fn load_scan_result(state: tauri::State<'_, AppState>) -> Result<Vec<SlotPreview>, String> {
    state
        .scan_result
        .lock()
        .expect("scan result lock poisoned")
        .clone()
        .ok_or_else(|| String::from("扫描结果尚未准备好"))
}

#[tauri::command]
fn cancel_scan(state: tauri::State<'_, AppState>) -> ScanProgress {
    state.scan_cancel.store(true, Ordering::SeqCst);
    update_scan_progress(&state.scan_progress, |progress| {
        if matches!(progress.status, ScanStatus::Running | ScanStatus::Cancelling) {
            progress.status = ScanStatus::Cancelling;
            progress.message = "正在取消扫描".to_string();
        }
    });
    scan_progress_snapshot(&state.scan_progress)
}

#[tauri::command]
fn clear_scan_cache(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let path = state
        .scan_cache_path
        .lock()
        .expect("scan cache path lock poisoned")
        .clone();
    clear_scan_cache_file(&path).map_err(|error| format!("清除扫描缓存失败：{error}"))
}

#[tauri::command]
fn start_scan(state: tauri::State<'_, AppState>) -> Result<ScanProgress, String> {
    {
        let progress = state
            .scan_progress
            .lock()
            .expect("scan progress lock poisoned");
        if matches!(progress.status, ScanStatus::Running | ScanStatus::Cancelling) {
            return Ok(progress.clone());
        }
    }

    let (
        conversion_mode,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        scan_cache_path,
        tasks,
    ) = {
        let controller = state.controller.lock().expect("desktop lock poisoned");
        let slot_indexes = controller.startable_slot_indexes();
        if slot_indexes.is_empty() {
            return Err(String::from("请至少选择一个歌曲文件夹或单曲"));
        }

        let tasks = slot_indexes
            .iter()
            .map(|slot_index| {
                let slot = &controller.state().slots[*slot_index];
                let destination = controller
                    .effective_destination(*slot_index)
                    .map_err(|error| error.to_string())?
                    .unwrap_or_default();
                Ok((*slot_index, slot.source_directory.clone(), destination))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let scan_cache_path = state
            .scan_cache_path
            .lock()
            .expect("scan cache path lock poisoned")
            .clone();
        (
            controller.state().conversion_mode,
            controller.state().mode,
            controller.state().lossless_format,
            controller.state().conflict_strategy,
            controller.state().filename_rule,
            controller.state().netease_filename_format,
            scan_cache_path,
            tasks,
        )
    };
    let concurrency_budget = concurrency_budget_snapshot(&state);
    let metadata_resolver = Arc::clone(&conversion_metadata_context(state.inner()).netease);

    state.scan_cancel.store(false, Ordering::SeqCst);
    {
        let mut result = state.scan_result.lock().expect("scan result lock poisoned");
        *result = None;
    }
    update_scan_progress(&state.scan_progress, |progress| {
        *progress = ScanProgress {
            status: ScanStatus::Running,
            phase: ScanProgressPhase::Preparing,
            processed: 0,
            total: 0,
            current_file: String::new(),
            message: "正在准备扫描".to_string(),
            tasks: tasks
                .iter()
                .map(|(slot_index, _, _)| ScanTaskProgress {
                    slot_index: *slot_index,
                    phase: ScanProgressPhase::Preparing,
                    processed: 0,
                    total: 0,
                    source_processed: 0,
                    source_total: None,
                    destination_processed: 0,
                    destination_total: None,
                    metadata_processed: 0,
                    metadata_total: None,
                    current_file: String::new(),
                })
                .collect(),
        };
    });

    let progress = Arc::clone(&state.scan_progress);
    let scan_cancel = Arc::clone(&state.scan_cancel);
    let scan_result = Arc::clone(&state.scan_result);
    let job = ScanJob {
        conversion_mode,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        scan_cache_path,
        concurrency_budget,
        metadata_resolver,
        tasks,
    };
    thread::spawn(move || run_scan_task(progress, scan_cancel, scan_result, job));

    Ok(scan_progress_snapshot(&state.scan_progress))
}

fn run_scan_task(
    progress: Arc<Mutex<ScanProgress>>,
    scan_cancel: Arc<AtomicBool>,
    scan_result: Arc<Mutex<Option<Vec<SlotPreview>>>>,
    job: ScanJob,
) {
    let ScanJob {
        conversion_mode,
        mode,
        lossless_format,
        conflict_strategy,
        filename_rule,
        netease_filename_format,
        scan_cache_path,
        concurrency_budget,
        metadata_resolver,
        tasks,
    } = job;
    let mut scan_cache = load_scan_cache(&scan_cache_path).unwrap_or_else(|_| ScanCache::empty());
    if scan_cancel.load(Ordering::SeqCst) {
        finish_scan_cancelled(&progress, &scan_result);
        return;
    }
    update_scan_progress(&progress, |state| {
        state.total = 0;
        state.processed = 0;
        state.message = "正在枚举输入和输出文件".to_string();
    });
    let mut previews = Vec::with_capacity(tasks.len());
    for (slot_index, source, destination) in tasks {
        if scan_cancel.load(Ordering::SeqCst) {
            finish_scan_cancelled(&progress, &scan_result);
            return;
        }

        // Establish denominators before worker processing starts. The walk
        // checks the shared cancellation flag on every entry, so this
        // preflight remains interruptible while the UI can render `x/total`
        // from the first processed file.
        let source_total = match count_scan_files(
            &source,
            w4dj::sync::SUPPORTED_SOURCE_EXTENSIONS,
            &scan_cancel,
        ) {
            Ok(total) => Some(total),
            Err(ScanEnumerationError::Cancelled) => {
                finish_scan_cancelled(&progress, &scan_result);
                return;
            }
            Err(ScanEnumerationError::Failed(_)) => None,
        };
        let destination_total = if destination.trim().is_empty() {
            Some(0)
        } else {
            match count_scan_files(&destination, &["mp3", "wav", "aiff"], &scan_cancel) {
                Ok(total) => Some(total),
                Err(ScanEnumerationError::Cancelled) => {
                    finish_scan_cancelled(&progress, &scan_result);
                    return;
                }
                Err(ScanEnumerationError::Failed(_)) => None,
            }
        };
        update_scan_progress(&progress, |state| {
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| task.slot_index == slot_index)
            {
                task.source_total = source_total;
                task.destination_total = destination_total;
                // The task progress bar follows the active phase.  Its
                // denominator must not combine input and output entries;
                // otherwise an empty destination or a stale output count can
                // leak into the final input ratio.
                task.total = source_total.unwrap_or(0);
            }
        });

        let mut observer = |phase: ScanPhase, path: &Path| {
            if scan_cancel.load(Ordering::SeqCst) {
                return false;
            }
            update_scan_progress(&progress, |state| {
                state.phase = match phase {
                    ScanPhase::Source => ScanProgressPhase::ScanningSource,
                    ScanPhase::Destination => ScanProgressPhase::ScanningDestination,
                    ScanPhase::Metadata => ScanProgressPhase::MatchingMetadata,
                };
                state.current_file = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                state.message = match phase {
                    ScanPhase::Source => "正在扫描输入目录".to_string(),
                    ScanPhase::Destination => "正在扫描输出目录".to_string(),
                    ScanPhase::Metadata => "正在匹配网易云元数据".to_string(),
                };
                if let Some(task) = state
                    .tasks
                    .iter_mut()
                    .find(|task| task.slot_index == slot_index)
                {
                    task.phase = state.phase.clone();
                    match phase {
                        ScanPhase::Source => {
                            task.source_processed = task.source_processed.saturating_add(1);
                        }
                        ScanPhase::Destination => {
                            task.destination_processed =
                                task.destination_processed.saturating_add(1);
                        }
                        ScanPhase::Metadata => {
                            if task.metadata_total.is_none() {
                                task.metadata_total = task.source_total.or(Some(task.source_processed));
                            }
                            task.metadata_processed = task.metadata_processed.saturating_add(1);
                        }
                    }
                    let (processed, total) = match phase {
                        ScanPhase::Source => (task.source_processed, task.source_total),
                        ScanPhase::Destination => {
                            (task.destination_processed, task.destination_total)
                        }
                        ScanPhase::Metadata => (task.metadata_processed, task.metadata_total),
                    };
                    task.processed = processed;
                    task.total = total.unwrap_or(0);
                    task.current_file = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                }
                state.processed = state
                    .tasks
                    .iter()
                    .map(|task| task.processed)
                    .sum::<usize>();
                state.total = state
                    .tasks
                    .iter()
                    .map(|task| match task.phase {
                        ScanProgressPhase::ScanningSource => task.source_total,
                        ScanProgressPhase::ScanningDestination => task.destination_total,
                        ScanProgressPhase::MatchingMetadata => task.metadata_total,
                        ScanProgressPhase::Completed => task.source_total,
                        _ => Some(task.total),
                    }
                    .unwrap_or(0))
                    .sum::<usize>();
            });
            !scan_cancel.load(Ordering::SeqCst)
        };
        let preview = match build_sync_preview_with_settings_and_netease_observed_with_cache_and_budget_and_policy_and_resolver(
            &source,
            &destination,
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
            netease_filename_format,
            filename_normalization_policy_for_slot(slot_index),
            Some(&mut observer),
            &mut scan_cache,
            Arc::clone(&concurrency_budget),
            Arc::clone(&scan_cancel),
            metadata_resolver.as_ref(),
        ) {
            Ok(Some(preview)) => preview,
            Ok(None) => {
                finish_scan_cancelled(&progress, &scan_result);
                return;
            }
            Err(error) => {
                finish_scan_error(&progress, &scan_result, format!("扫描失败：{error}"));
                return;
            }
        };
        previews.push(SlotPreview {
            slot_index,
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
            netease_filename_format,
            preview,
            retry_of: None,
        });
        update_scan_progress(&progress, |state| {
            let input_processed = state
                .tasks
                .iter()
                .map(|task| task.source_processed)
                .sum::<usize>();
            let input_total = state
                .tasks
                .iter()
                .map(|task| task.source_total.unwrap_or(task.source_processed))
                .sum::<usize>();
            state.processed = input_processed;
            state.total = input_total;
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| task.slot_index == slot_index)
            {
                task.total = task.source_total.unwrap_or(task.source_processed);
                task.metadata_total = task.metadata_total.or(Some(task.metadata_processed));
                task.phase = ScanProgressPhase::Completed;
                task.current_file.clear();
            }
        });
        if let Err(error) = save_scan_cache_atomic(&scan_cache_path, &scan_cache) {
            finish_scan_error(
                &progress,
                &scan_result,
                format!("扫描缓存保存失败：{error}"),
            );
            return;
        }
    }

    if scan_cancel.load(Ordering::SeqCst) {
        finish_scan_cancelled(&progress, &scan_result);
        return;
    }

    update_scan_progress(&progress, |state| {
        state.phase = ScanProgressPhase::Checking;
        state.current_file.clear();
        state.message = "正在检查转换条件".to_string();
    });
    deduplicate_cross_slot_candidates(&mut previews);

    if matches!(conversion_mode, ConversionMode::Direct)
        && let Err(error) = validate_scan_previews(&previews)
    {
        finish_scan_error(&progress, &scan_result, error);
        return;
    }

    {
        let mut result = scan_result.lock().expect("scan result lock poisoned");
        *result = Some(previews);
    }
    update_scan_progress(&progress, |state| {
        state.status = ScanStatus::Completed;
        state.phase = ScanProgressPhase::Completed;
        state.current_file.clear();
        state.message = "扫描成功".to_string();
    });
}

fn count_scan_files(
    folder: &str,
    allowed_extensions: &[&str],
    cancel: &AtomicBool,
) -> Result<usize, ScanEnumerationError> {
    enumerate_music_files_observed(folder, allowed_extensions, cancel, |_, _, _| {})
        .map(|result| result.paths.len())
}

fn validate_scan_previews(previews: &[SlotPreview]) -> Result<(), String> {
    let processable = previews
        .iter()
        .filter(|preview| !preview.preview.candidates.is_empty())
        .collect::<Vec<_>>();

    if processable.is_empty() {
        return Err(String::from("没有可处理的转换任务"));
    }

    for preview in &processable {
        validate_source_input(&preview.preview.source_directory)?;
        let source_path = Path::new(&preview.preview.source_directory);
        if source_path.is_file() {
            fs::File::open(source_path)
                .map_err(|error| format!("无法读取输入文件：{error}"))?;
        } else {
            fs::read_dir(source_path)
                .map_err(|error| format!("无法读取输入目录：{error}"))?;
        }
        validate_destination_directory(Path::new(&preview.preview.destination_directory))?;
        if let Some(issue) = preview.preview.errors.first() {
            return Err(format!("输入文件检查失败：{}", issue.message));
        }
        if matches!(preview.preview.disk_space_sufficient, Some(false)) {
            return Err(format!("输出目录磁盘空间不足：{}", preview.preview.destination_directory));
        }
    }

    let processable = processable.into_iter().cloned().collect::<Vec<_>>();
    validate_unique_planned_outputs(&processable)
}

fn finish_scan_cancelled(
    progress: &Arc<Mutex<ScanProgress>>,
    result: &Arc<Mutex<Option<Vec<SlotPreview>>>>,
) {
    *result.lock().expect("scan result lock poisoned") = None;
    update_scan_progress(progress, |state| {
        state.status = ScanStatus::Cancelled;
        state.phase = ScanProgressPhase::Cancelled;
        state.current_file.clear();
        state.message = "扫描已取消".to_string();
    });
}

fn finish_scan_error(
    progress: &Arc<Mutex<ScanProgress>>,
    result: &Arc<Mutex<Option<Vec<SlotPreview>>>>,
    message: String,
) {
    *result.lock().expect("scan result lock poisoned") = None;
    update_scan_progress(progress, |state| {
        state.status = ScanStatus::Error;
        state.phase = ScanProgressPhase::Error;
        state.current_file.clear();
        state.message = message;
    });
}

fn deduplicate_cross_slot_candidates(previews: &mut [SlotPreview]) {
    let mut planned_outputs = HashMap::<String, usize>::new();

    for slot_preview in previews {
        let mut retained = Vec::with_capacity(slot_preview.preview.candidates.len());
        for candidate in std::mem::take(&mut slot_preview.preview.candidates) {
            let key = planned_output_key(&candidate.destination_path);
            if let Some(owner_slot) = planned_outputs.get(&key) {
                slot_preview.preview.new_count = slot_preview.preview.new_count.saturating_sub(1);
                slot_preview.preview.skipped_count += 1;
                slot_preview.preview.estimated_output_bytes = match (
                    slot_preview.preview.estimated_output_bytes,
                    candidate.estimated_output_bytes,
                ) {
                    (Some(total), Some(candidate_bytes)) => {
                        Some(total.saturating_sub(candidate_bytes))
                    }
                    _ => None,
                };
                let issue = PreviewIssue {
                    path: candidate.source_path,
                    message: format!(
                        "与任务 {} 的输出文件重复，已交由任务 {} 处理",
                        owner_slot + 1,
                        owner_slot + 1
                    ),
                };
                slot_preview.preview.skipped.push(issue.clone());
                slot_preview.preview.warnings.push(issue);
                continue;
            }

            planned_outputs.insert(key, slot_preview.slot_index);
            retained.push(candidate);
        }
        slot_preview.preview.candidates = retained;
    }
}

fn validate_unique_planned_outputs(previews: &[SlotPreview]) -> Result<(), String> {
    let mut planned_outputs = HashSet::new();
    for preview in previews {
        for candidate in &preview.preview.candidates {
            if !planned_outputs.insert(planned_output_key(&candidate.destination_path)) {
                return Err(String::from(
                    "两个任务包含相同的输出文件，请重新预检后再开始",
                ));
            }
        }
    }
    Ok(())
}

fn planned_output_key(path: &str) -> String {
    let path = Path::new(path);
    let normalized = path
        .parent()
        .and_then(|parent| fs::canonicalize(parent).ok())
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.to_path_buf());
    let key = normalized.to_string_lossy().into_owned();

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return key.to_lowercase();

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    key
}

fn collect_processable_previews(
    previews: Vec<SlotPreview>,
    allow_error_only_retry: bool,
) -> Result<Vec<SlotPreview>, String> {
    let processable = previews
        .into_iter()
        .filter(|preview| {
            !preview.preview.candidates.is_empty()
                || (allow_error_only_retry
                    && preview.retry_of.is_some()
                    && !preview.preview.errors.is_empty())
        })
        .collect::<Vec<_>>();

    if processable.is_empty() {
        return Err(String::from("没有可处理的转换任务"));
    }

    Ok(processable)
}

#[tauri::command]
fn start_confirmed_sync(
    previews: Vec<SlotPreview>,
    retry_of: Option<String>,
    analyses: Option<Vec<TrackAnalysis>>,
    analysis_failures: Option<Vec<AnalysisFailure>>,
    batch_id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    if previews.is_empty() {
        return Err(String::from("没有可处理的转换任务"));
    }

    let batch_id = batch_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("batch-{}", unique_timestamp()));
    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let history_write_lock = Arc::clone(&state.history_write_lock);
    let destination_coordinator = state.destination_coordinator.clone();
    let w4dj_path = state
        .library
        .catalog_path
        .lock()
        .expect("library catalog path lock poisoned")
        .clone();
    let requested_analyses = analyses.unwrap_or_default();
    let requested_analysis_failures = analysis_failures.unwrap_or_default();
    let metadata_context = conversion_metadata_context(&state);
    let mut jobs = Vec::with_capacity(previews.len());
    let mut seen_slots = Vec::with_capacity(previews.len());
    let monitor_previews;
    let monitor_settings;
    let monitor_needs_analysis;
    let mut recovered_source_updated = false;

    {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        let state_mode = controller.state().mode;
        let state_lossless_format = controller.state().lossless_format;
        let enhanced_mode = controller.state().enhanced_mode;
        monitor_needs_analysis = enhanced_mode;
        let state_conflict_strategy = controller.state().conflict_strategy;
        let state_filename_rule = controller.state().filename_rule;
        let state_netease_filename_format = controller.state().netease_filename_format;
        let mut validated_previews = Vec::with_capacity(previews.len());

        for slot_preview in previews {
            let mut slot_preview = slot_preview;
            let slot_index = slot_preview.slot_index;
            if matches!(
                filename_normalization_policy_for_slot(slot_index),
                FilenameNormalizationPolicy::PreserveSource
            ) {
                attach_netease_identities(
                    &mut slot_preview.preview,
                    metadata_context.netease.as_ref(),
                );
            }
            if seen_slots.contains(&slot_index) {
                return Err(format!("重复的同步任务槽位：{slot_index}"));
            }
            seen_slots.push(slot_index);

            let slot = controller
                .state()
                .slots
                .get(slot_index)
                .ok_or_else(|| format!("无效的同步任务槽位：{slot_index}"))?;
            if matches!(slot.status, w4dj::desktop::DesktopStatus::Running) {
                return Err(format!("任务 {} 正在运行", slot_index + 1));
            }
            let is_history_retry = retry_of.is_some() || slot_preview.retry_of.is_some();
            let source_matches = slot_preview.preview.source_directory == slot.source_directory
                || is_recovered_single_source(
                    &slot.source_directory,
                    &slot_preview.preview.source_directory,
                );
            if !is_history_retry
                && (slot_preview.mode != state_mode
                    || slot_preview.lossless_format != state_lossless_format
                    || slot_preview.conflict_strategy != state_conflict_strategy
                    || slot_preview.filename_rule != state_filename_rule
                    || slot_preview.netease_filename_format != state_netease_filename_format
                    || !source_matches
                    || slot_preview.preview.destination_directory
                        != controller
                            .effective_destination(slot_index)?
                            .unwrap_or_default())
            {
                return Err(String::from("任务设置在预检后发生变化，请重新扫描"));
            }
            if matches!(slot_preview.preview.disk_space_sufficient, Some(false)) {
                return Err(format!("任务 {} 的输出磁盘空间不足", slot_index + 1));
            }
            validated_previews.push(slot_preview);
        }

        let recovered_source_updates = validated_previews
            .iter()
            .filter_map(|slot_preview| {
                let original_source = controller
                    .state()
                    .slots
                    .get(slot_preview.slot_index)?
                    .source_directory
                    .clone();
                let resolved_source = slot_preview.preview.source_directory.clone();
                is_recovered_single_source(&original_source, &resolved_source)
                    .then_some((slot_preview.slot_index, resolved_source))
            })
            .collect::<Vec<_>>();
        for (slot_index, source) in recovered_source_updates {
            controller.select_source_directory(slot_index, source)?;
            recovered_source_updated = true;
        }

        validate_unique_planned_outputs(&validated_previews)?;

        let allow_error_only_retry = retry_of.is_some()
            || validated_previews
                .iter()
                .any(|preview| preview.retry_of.is_some());
        let processable_previews =
            collect_processable_previews(validated_previews.clone(), allow_error_only_retry)?;
        monitor_previews = validated_previews.clone();
        monitor_settings = serde_json::to_value(controller.state())
            .unwrap_or_else(|_| serde_json::json!({"serialization": "failed"}));

        for slot_preview in processable_previews {
            let slot_index = slot_preview.slot_index;
            let candidate_paths = slot_preview
                .preview
                .candidates
                .iter()
                .map(|candidate| candidate.source_path.as_str())
                .collect::<HashSet<_>>();
            jobs.push(ConfirmedSyncJob {
                batch_id: batch_id.clone(),
                slot_index,
                source: slot_preview.preview.source_directory.clone(),
                destination: slot_preview.preview.destination_directory.clone(),
                mode: slot_preview.mode,
                lossless_format: slot_preview.lossless_format,
                conflict_strategy: slot_preview.conflict_strategy,
                filename_rule: slot_preview.filename_rule,
                netease_filename_format: slot_preview.netease_filename_format,
                candidates: slot_preview.preview.candidates.clone(),
                analyses: if enhanced_mode {
                    requested_analyses
                        .iter()
                        .filter(|analysis| candidate_paths.contains(analysis.path.as_str()))
                        .map(embedded_analysis_from_track)
                        .collect()
                } else {
                    Vec::new()
                },
                analysis_failures: if enhanced_mode {
                    requested_analysis_failures
                        .iter()
                        .filter(|failure| candidate_paths.contains(failure.path.as_str()))
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                },
                preview: slot_preview.preview,
                retry_of: retry_of.clone().or(slot_preview.retry_of),
                test_monitor: None,
                w4dj_path: w4dj_path.clone(),
                metadata_context: metadata_context.clone(),
            });
        }

        for slot_preview in &validated_previews {
            if slot_preview.preview.candidates.is_empty()
                && jobs
                    .iter()
                    .all(|job| job.slot_index != slot_preview.slot_index)
            {
                apply_preflight_summary(
                    &mut controller,
                    slot_preview.slot_index,
                    &slot_preview.preview,
                )?;
            }
        }

        for job in &jobs {
            controller.start_confirmed_sync(job.slot_index, job.candidates.len())?;
            apply_preflight_summary(&mut controller, job.slot_index, &job.preview)?;
            controller.push_log(job.slot_index, "Confirmed preflight; conversion started")?;
            for failure in &job.analysis_failures {
                controller.push_log(
                    job.slot_index,
                    format!(
                        "Essentia analysis failed for {}: {}",
                        failure.path, failure.message
                    ),
                )?;
            }
        }
    }

    if recovered_source_updated {
        persist_preferences(&state);
    }

    let test_monitor = if RUNTIME_SESSION_RECORDING_ENABLED && !jobs.is_empty() {
        let root = state
            .test_monitor_path
            .lock()
            .expect("test monitor path lock poisoned")
            .clone();
        match TestMonitor::new(
            &root,
            &batch_id,
            monitor_settings,
            &monitor_previews,
            jobs.len(),
        ) {
            Ok(monitor) => Some(Arc::new(monitor)),
            Err(error) => {
                eprintln!("Failed to initialize runtime session recorder: {error}");
                None
            }
        }
    } else {
        None
    };

    if RUNTIME_SESSION_RECORDING_ENABLED
        && monitor_needs_analysis
        && let Some(monitor) = test_monitor.as_ref()
    {
        state
            .test_monitors
            .lock()
            .expect("test monitor map lock poisoned")
            .insert(batch_id.clone(), Arc::clone(monitor));
    }

    let task_concurrency_budget = concurrency_budget_snapshot(&state);
    let task_ffmpeg_registry = Arc::clone(&state.ffmpeg_registry);
    for mut job in jobs {
        job.test_monitor = test_monitor.clone();
        let controller = Arc::clone(&state.controller);
        let destination_coordinator = destination_coordinator.clone();
        let history_path = history_path.clone();
        let history_write_lock = Arc::clone(&history_write_lock);
        let concurrency_budget = Arc::clone(&task_concurrency_budget);
        let ffmpeg_registry = Arc::clone(&task_ffmpeg_registry);
        thread::spawn(move || {
            run_confirmed_sync_task(
                controller,
                destination_coordinator,
                history_path,
                history_write_lock,
                job,
                concurrency_budget,
                ffmpeg_registry,
            )
        });
    }

    Ok(state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone())
}

#[tauri::command]
fn apply_track_analysis_results(
    batch_id: String,
    previews: Vec<SlotPreview>,
    analyses: Vec<TrackAnalysis>,
    analysis_failures: Vec<AnalysisFailure>,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    if batch_id.trim().is_empty() {
        return Err(String::from("增强分析缺少批次 ID"));
    }

    // Analysis writeback is a separate operation from conversion, but it
    // must use the same read-only database snapshot for every candidate in
    // this batch.  Resolve it once before entering the per-track loop.
    let metadata_context = conversion_metadata_context(state.inner());

    let track_analysis_lookup = analyses
        .iter()
        .map(|analysis| (analysis.path.clone(), analysis))
        .collect::<HashMap<_, _>>();
    let failure_lookup = analysis_failures
        .iter()
        .map(|failure| (failure.path.clone(), failure))
        .collect::<HashMap<_, _>>();
    let mut reports = Vec::new();
    let w4dj_path = state
        .library
        .catalog_path
        .lock()
        .expect("library catalog path lock poisoned")
        .clone();
    let mut w4dj_library = match W4djLibrary::open(&w4dj_path) {
        Ok(library) => Some(library),
        Err(error) => {
            eprintln!("W4DJ analysis library unavailable: {error}");
            None
        }
    };
    let mut w4dj_projection_count = 0usize;

    for slot_preview in &previews {
        for candidate in &slot_preview.preview.candidates {
            let destination_path = Path::new(&candidate.destination_path);
            let base_report = || AnalysisReport {
                source_path: candidate.source_path.clone(),
                destination_path: candidate.destination_path.clone(),
                status: String::from("completed"),
                message: None,
                drop_status: None,
                drop_loudness_lufs: None,
                model_status: None,
                model_details: None,
                stage: None,
                elapsed_ms: None,
                basic_status: None,
                basic_danceability: None,
                discogs_danceability_status: None,
                discogs_danceability: None,
                discogs_completed_heads: None,
                discogs_total_heads: None,
                cached: None,
            };

            let Some(analysis) = track_analysis_lookup.get(&candidate.source_path) else {
                let mut report = base_report();
                if let Some(failure) = failure_lookup.get(&candidate.source_path) {
                    report.status = failure
                        .status
                        .clone()
                        .unwrap_or_else(|| String::from("failed"));
                    report.message = Some(failure.message.clone());
                    report.stage = failure.stage.clone();
                    report.elapsed_ms = failure.elapsed_ms;
                } else {
                    report.status = String::from("failed");
                    report.stage = Some(String::from("pending"));
                    report.message = Some(String::from("未收到该歌曲的 Essentia 分析结果"));
                }
                if let Some(library) = w4dj_library.as_mut() {
                    let _ = library.mark_analysis_failed_for_destination(
                        destination_path,
                        report.message.as_deref().unwrap_or("未收到分析结果"),
                    );
                }
                reports.push(report);
                continue;
            };

            let mut report = base_report();
            let complete = is_complete_analysis(analysis);
            report.status = if complete {
                String::from("completed")
            } else {
                String::from("failed")
            };
            report.basic_status = Some(if is_basic_analysis_complete(analysis) {
                String::from("completed")
            } else {
                String::from("failed")
            });
            report.basic_danceability = analysis
                .danceability
                .filter(|value| value.is_finite());
            report.drop_status = analysis
                .drop_analysis
                .as_ref()
                .map(|value| value.status.clone());
            report.drop_loudness_lufs = analysis
                .drop_loudness_lufs
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.2}"));
            report.model_status = analysis.high_level.as_ref().map(|value| value.status.clone());
            report.model_details = analysis
                .high_level
                .as_ref()
                .and_then(|value| serde_json::to_string(value).ok());
            if let Some(discogs) = analysis
                .high_level
                .as_ref()
                .and_then(|value| value.discogs_effnet.as_ref())
            {
                report.discogs_total_heads = Some(REQUIRED_DISCOGS_HEAD_IDS.len());
                report.discogs_completed_heads = Some(
                    REQUIRED_DISCOGS_HEAD_IDS
                        .iter()
                        .filter(|id| {
                            discogs
                                .heads
                                .get(**id)
                                .is_some_and(|head| head.status == "completed")
                        })
                        .count(),
                );
                if let Some(head) = discogs.heads.get("danceability") {
                    report.discogs_danceability_status = Some(head.status.clone());
                    report.discogs_danceability = head
                        .selected_confidence
                        .filter(|value| value.is_finite());
                }
            }
            if let Some(failure) = failure_lookup.get(&candidate.source_path) {
                report.status = failure
                    .status
                    .clone()
                    .unwrap_or_else(|| String::from("failed"));
                report.message = Some(failure.message.clone());
                report.stage = failure.stage.clone();
                report.elapsed_ms = failure.elapsed_ms;
            }

            if !destination_path.is_file() {
                report.status = String::from("failed");
                report.message = Some(String::from("转换输出不存在，未执行分析元数据回写"));
                reports.push(report);
                continue;
            }

            let embedded_analysis = embedded_analysis_from_track(analysis);
            let result = update_analysis_metadata_transactionally(
                destination_path,
                |temporary_output| {
                    apply_track_analysis_metadata_with_context(
                        temporary_output,
                        &embedded_analysis,
                        metadata_context.as_ref(),
                    )
                },
            );
            if let Err(error) = result {
                report.status = String::from("failed");
                report.message = Some(format!("分析元数据回写失败：{error}"));
                if let Some(library) = w4dj_library.as_mut() {
                    let _ = library.mark_analysis_failed_for_destination(
                        destination_path,
                        report.message.as_deref().unwrap_or("分析元数据回写失败"),
                    );
                }
            } else if let Err(error) = validate_track_analysis_metadata(destination_path, &embedded_analysis) {
                report.status = String::from("failed");
                report.message = Some(format!("分析元数据回读校验失败：{error}"));
                if let Some(library) = w4dj_library.as_mut() {
                    let _ = library.mark_analysis_failed_for_destination(
                        destination_path,
                        report.message.as_deref().unwrap_or("分析元数据回读校验失败"),
                    );
                }
            } else if let Some(library) = w4dj_library.as_mut() {
                match library.apply_analysis_for_destination(destination_path, analysis) {
                    Ok(true) => {
                        w4dj_projection_count += 1;
                    }
                    Ok(false) => {
                        report.status = String::from("failed");
                        report.message = Some(String::from("W4DJ 分析库未找到对应输出记录"));
                    }
                    Err(error) => {
                        report.status = String::from("failed");
                        report.message = Some(format!("写入 W4DJ 分析库失败：{error}"));
                    }
                }
            } else {
                report.status = String::from("failed");
                report.message = Some(String::from("W4DJ 分析库不可用，未完成最终投影"));
            }
            reports.push(report);
        }
    }

    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let _history_guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    append_analysis_reports(&history_path, &batch_id, reports.clone())
        .map_err(|error| format!("保存增强分析报告失败：{error}"))?;

    let monitor = runtime_monitor_for_batch(&state, &batch_id);
    if let Some(monitor) = monitor {
        monitor.record_event(
            "analysis_persisted",
            serde_json::json!({
                "report_count": reports.len(),
                "completed_count": reports.iter().filter(|report| report.status == "completed").count(),
                "failed_count": reports.iter().filter(|report| report.status == "failed").count(),
                "metadata_writeback_count": reports.iter().filter(|report| report.status == "completed").count(),
                "w4dj_library_projection_count": w4dj_projection_count,
            }),
        );
        monitor.record_analysis_reports(&reports);
    }

    Ok(state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone())
}

#[tauri::command]
fn load_history(state: tauri::State<'_, AppState>) -> Result<Vec<HistoryEntryView>, String> {
    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let _history_guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    let entries = load_history_file(history_path).map_err(|error| format!("读取转换历史失败：{error}"))?;
    let monitor_root = state
        .test_monitor_path
        .lock()
        .expect("runtime session path lock poisoned")
        .clone();
    Ok(entries
        .into_iter()
        .map(|entry| {
            let session_dir = resolve_runtime_session_dir(&monitor_root, &entry);
            let analysis = runtime_session_analysis_summary(session_dir.as_deref());
            HistoryEntryView { entry, analysis }
        })
        .collect())
}

#[tauri::command]
fn load_incomplete_analysis_run(
    state: tauri::State<'_, AppState>,
) -> Result<Option<IncompleteAnalysisRun>, String> {
    let root = state
        .test_monitor_path
        .lock()
        .expect("runtime session path lock poisoned")
        .clone();
    let Ok(mut candidates) = fs::read_dir(&root) else {
        return Ok(None);
    };
    let mut best: Option<(SystemTime, IncompleteAnalysisRun)> = None;
    while let Some(Ok(entry)) = candidates.next() {
        let session_dir = entry.path();
        if !session_dir.is_dir() {
            continue;
        }
        let Some(state_value) = fs::read_to_string(session_dir.join("analysis-state.json"))
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        else {
            continue;
        };
        let status = state_value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("notRequested");
        if !matches!(status, "pending" | "running" | "partial" | "error" | "interrupted") {
            continue;
        }
        let Some(batch_id) = state_value
            .get("batchId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                fs::read_to_string(session_dir.join("session.json"))
                    .ok()
                    .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
                    .and_then(|session| session.get("batch_id").and_then(serde_json::Value::as_str).map(str::to_owned))
            })
        else {
            continue;
        };
        let Some(previews) = fs::read_to_string(session_dir.join("candidates.json"))
            .ok()
            .and_then(|contents| serde_json::from_str::<Vec<SlotPreview>>(&contents).ok())
        else {
            continue;
        };
        let analysis = runtime_session_analysis_summary(Some(&session_dir)).unwrap_or_default();
        let modified = fs::metadata(session_dir.join("analysis-state.json"))
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        let run = IncompleteAnalysisRun { batch_id, previews, analysis };
        if best
            .as_ref()
            .map(|(current, _)| modified > *current)
            .unwrap_or(true)
        {
            best = Some((modified, run));
        }
    }
    Ok(best.map(|(_, run)| run))
}

#[tauri::command]
fn retry_history_failures(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<SlotPreview, String> {
    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let _history_guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    let entry = load_history_file(history_path)
        .map_err(|error| format!("无法读取转换历史：{error}"))?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| String::from("找不到对应的转换历史"))?;

    let preview = build_retry_preview(&entry);
    Ok(SlotPreview {
        slot_index: entry.slot_index,
        mode: entry.mode,
        lossless_format: entry.lossless_format,
        conflict_strategy: entry.conflict_strategy,
        filename_rule: entry.filename_rule,
        netease_filename_format: entry.netease_filename_format,
        preview,
        retry_of: Some(entry.id),
    })
}

#[tauri::command]
fn delete_history_entry_command(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let _history_guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    let removed = delete_history_entry(history_path, &id)
        .map_err(|error| format!("删除历史记录失败：{error}"))?;
    if !removed {
        return Err(String::from("找不到对应的转换历史"));
    }
    Ok(())
}

#[tauri::command]
fn clear_history_command(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let _history_guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    clear_history(history_path).map_err(|error| format!("清空历史记录失败：{error}"))
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        developer: "komakizhu",
        project_url: "https://github.com/komakizhu/W4DJ-RKB",
    }
}

fn parse_release_version(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.trim().trim_start_matches(['v', 'V']);
    let mut parts = value.split(['-', '+']).next()?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

#[tauri::command]
fn check_for_updates() -> Result<UpdateCheckResult, String> {
    let response = ureq::get("https://api.github.com/repos/komakizhu/W4DJ-RKB/releases/latest")
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "W4DJ-RKB")
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|error| format!("无法连接 GitHub：{error}"))?;
    let release: GitHubRelease = response
        .into_json()
        .map_err(|error| format!("GitHub 更新信息格式错误：{error}"))?;
    let current_version = env!("CARGO_PKG_VERSION");
    let current = parse_release_version(current_version)
        .ok_or_else(|| format!("当前版本号无法识别：{current_version}"))?;
    let latest = parse_release_version(&release.tag_name)
        .ok_or_else(|| format!("GitHub Release 版本号无法识别：{}", release.tag_name))?;
    Ok(UpdateCheckResult {
        current_version,
        latest_version: release.tag_name,
        update_available: latest > current,
        release_url: release.html_url,
        release_name: release.name.unwrap_or_default(),
    })
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    if !external_url_is_allowed(&url) {
        return Err("不允许打开此外部地址".to_string());
    }

    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(url).status();

    #[cfg(target_os = "windows")]
    let status = Command::new("cmd").args(["/C", "start", "", &url]).status();

    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(url).status();

    status
        .map_err(|error| format!("无法打开链接：{error}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!("无法打开链接（退出码 {:?}）", status.code()))
            }
        })
}

fn external_url_is_allowed(url: &str) -> bool {
    const PROJECT_URL: &str = "https://github.com/komakizhu/W4DJ-RKB";
    const DJ_CRATE_DIGGER_URL: &str = "https://github.com/komakizhu/dj-crate-digger-skill";
    url == PROJECT_URL
        || url == DJ_CRATE_DIGGER_URL
        || url.starts_with("https://github.com/komakizhu/W4DJ-RKB/releases/")
        || url == ESSENTIA_MODELS_URL
}

fn library_catalog_path(state: &tauri::State<'_, AppState>) -> PathBuf {
    state
        .library
        .catalog_path
        .lock()
        .expect("library catalog path lock poisoned")
        .clone()
}

/// The old catalog remains a metadata staging area for the compatibility
/// NetEase refresh commands.  It must never share the output-owned W4DJ
/// database used by Dashboard queries.
fn legacy_library_catalog_path(state: &tauri::State<'_, AppState>) -> PathBuf {
    library_catalog_path(state).with_file_name("library-dashboard.sqlite3")
}

fn validated_w4dj_import_path(path: &str) -> Result<PathBuf, String> {
    let input = PathBuf::from(path);
    if !input.is_absolute() {
        return Err("DJ 歌单路径必须是绝对路径".to_string());
    }
    if input
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("w4dj"))
    {
        return Err("请选择 .w4dj 歌单文件".to_string());
    }
    let metadata = fs::symlink_metadata(&input).map_err(|error| format!("无法读取 DJ 歌单：{error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("DJ 歌单必须是普通文件，不能是目录或符号链接".to_string());
    }
    fs::canonicalize(&input).map_err(|error| format!("无法定位 DJ 歌单：{error}"))
}

#[tauri::command]
fn import_w4dj_playlist(path: String, state: tauri::State<'_, AppState>) -> Result<ImportedDjPlaylist, String> {
    let canonical = validated_w4dj_import_path(&path)?;
    let metadata = fs::metadata(&canonical).map_err(|error| format!("无法读取 DJ 歌单：{error}"))?;
    if metadata.len() > w4dj::dj_playlist::W4DJ_PLAYLIST_MAX_BYTES as u64 {
        return Err(format!("DJ 歌单文件超过 {} MiB 上限", w4dj::dj_playlist::W4DJ_PLAYLIST_MAX_BYTES / (1024 * 1024)));
    }
    let bytes = fs::read(&canonical).map_err(|error| format!("无法读取 DJ 歌单：{error}"))?;
    let playlist = parse_w4dj_playlist(&bytes, Some(&canonical)).map_err(|error| error.to_string())?;
    let path = library_catalog_path(&state);
    let mut library = W4djLibrary::open(&path).map_err(|error| error.to_string())?;
    library
        .upsert_imported_dj_playlist(&playlist)
        .map_err(|error| error.to_string())?;
    library
        .get_imported_dj_playlist(&playlist.playlist_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "导入后的 DJ 歌单无法重新读取".to_string())
}

#[tauri::command]
fn list_imported_dj_playlists(state: tauri::State<'_, AppState>) -> Result<Vec<ImportedDjPlaylistSummary>, String> {
    let path = library_catalog_path(&state);
    W4djLibrary::open(&path)
        .and_then(|library| library.list_imported_dj_playlists())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn load_imported_dj_playlist(
    playlist_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ImportedDjPlaylist, String> {
    if playlist_id.trim().is_empty() {
        return Err("DJ 歌单 ID 不能为空".to_string());
    }
    let path = library_catalog_path(&state);
    W4djLibrary::open(&path)
        .and_then(|library| library.get_imported_dj_playlist(&playlist_id))
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到指定的 DJ 歌单".to_string())
}

#[tauri::command]
fn export_imported_dj_playlist_w4dj(
    playlist_id: String,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if playlist_id.trim().is_empty() {
        return Err("DJ 歌单 ID 不能为空".to_string());
    }
    let output = PathBuf::from(path);
    if !output.is_absolute() {
        return Err("W4DJ 导出路径必须是绝对路径".to_string());
    }
    if output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("w4dj"))
    {
        return Err("W4DJ 导出路径必须使用 .w4dj 扩展名".to_string());
    }
    if output.parent().is_none_or(|parent| !parent.is_dir()) {
        return Err("W4DJ 导出目录不存在".to_string());
    }
    let library_path = library_catalog_path(&state);
    let library = W4djLibrary::open(&library_path).map_err(|error| error.to_string())?;
    let playlist = library
        .get_imported_dj_playlist(&playlist_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到指定的 DJ 歌单".to_string())?;
    let bytes = serialize_w4dj_playlist(&playlist).map_err(|error| error.to_string())?;
    fs::write(&output, bytes).map_err(|error| format!("写入 W4DJ 歌单失败：{error}"))
}

#[tauri::command]
fn export_netease_playlist_text(path: String, text: String) -> Result<(), String> {
    let output = PathBuf::from(path);
    if !output.is_absolute() {
        return Err("TXT 导出路径必须是绝对路径".to_string());
    }
    if output.extension().and_then(|extension| extension.to_str()).is_none_or(|extension| !extension.eq_ignore_ascii_case("txt")) {
        return Err("TXT 导出路径必须使用 .txt 扩展名".to_string());
    }
    if let Some(parent) = output.parent()
        && !parent.is_dir()
    {
        return Err("TXT 导出目录不存在".to_string());
    }
    fs::write(&output, text.as_bytes()).map_err(|error| format!("写入 TXT 失败：{error}"))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DjPlaylistM3u8ExportResult {
    path: String,
    export_directory: String,
    matched_count: usize,
    total: usize,
    copied_count: usize,
    copy_audio: bool,
    portable: bool,
    omitted: Vec<w4dj::m3u8::M3u8OmittedTrack>,
}

fn playlist_export_paths(
    selected_path: &Path,
    playlist_name: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let parent = selected_path
        .parent()
        .ok_or_else(|| "无法确定歌单导出目录".to_string())?;
    let sanitized_name = sanitize_filename_component(playlist_name);
    let folder_name = if sanitized_name.is_empty() {
        "W4DJ 歌单".to_string()
    } else {
        sanitized_name
    };
    let export_directory = if parent
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(&folder_name))
    {
        parent.to_path_buf()
    } else {
        parent.join(&folder_name)
    };
    let playlist_path = export_directory.join(format!("{folder_name}.m3u8"));
    Ok((export_directory, playlist_path))
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .ok()
            .zip(fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

fn unique_playlist_audio_target(
    directory: &Path,
    source_name: &std::ffi::OsStr,
    occupied: &HashSet<PathBuf>,
) -> PathBuf {
    let initial = directory.join(source_name);
    if !initial.exists() && !occupied.contains(&initial) {
        return initial;
    }
    let stem = Path::new(source_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("track");
    let extension = Path::new(source_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let mut index = 1usize;
    loop {
        let candidate = directory.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() && !occupied.contains(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

#[cfg(unix)]
fn set_portable_export_permissions(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("读取导出权限失败（{}）：{error}", path.display()))?
        .permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("设置跨账户导出权限失败（{}）：{error}", path.display()))
}

#[cfg(not(unix))]
fn set_portable_export_permissions(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

fn validate_portable_playlist_export(
    resolved: &[ResolvedDjPlaylistTrack],
    export_directory: &Path,
    contents: &str,
) -> Result<(), String> {
    for track in resolved {
        if track.destination_path.parent() != Some(export_directory) {
            return Err(format!(
                "复制模式产生了歌单文件夹外的音频路径：{}",
                track.destination_path.display()
            ));
        }
        let metadata = fs::metadata(&track.destination_path)
            .map_err(|error| format!("复制后的音频不可读（{}）：{error}", track.destination_path.display()))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(format!(
                "复制后的音频不存在或为空：{}",
                track.destination_path.display()
            ));
        }
        fs::File::open(&track.destination_path)
            .map_err(|error| format!("复制后的音频不可打开（{}）：{error}", track.destination_path.display()))?;
    }

    for line in contents.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let path = Path::new(line);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(format!("复制模式的 M3U8 包含歌单文件夹外路径：{line}"));
        }
    }
    Ok(())
}

fn copy_playlist_audio_to_directory(
    resolved: &mut [ResolvedDjPlaylistTrack],
    directory: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut occupied = HashSet::new();
    let mut created = Vec::new();
    for track in resolved {
        let source = track.destination_path.clone();
        let source_name = source
            .file_name()
            .ok_or_else(|| format!("输出文件没有有效文件名：{}", source.display()))?;
        let default_target = directory.join(source_name);
        let target = if paths_refer_to_same_file(&source, &default_target) {
            default_target
        } else {
            unique_playlist_audio_target(directory, source_name, &occupied)
        };
        if !paths_refer_to_same_file(&source, &target) {
            if let Err(error) = fs::copy(&source, &target) {
                for created_path in &created {
                    let _ = fs::remove_file(created_path);
                }
                return Err(format!("复制音频失败（{}）：{error}", source.display()));
            }
            created.push(target.clone());
        }
        if let Err(error) = set_portable_export_permissions(&target, 0o644) {
            for created_path in &created {
                let _ = fs::remove_file(created_path);
            }
            return Err(error);
        }
        occupied.insert(target.clone());
        track.destination_path = target;
    }
    Ok(created)
}

#[tauri::command]
fn match_imported_dj_playlist(
    playlist_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DjPlaylistMatchReport, String> {
    if playlist_id.trim().is_empty() {
        return Err("DJ 歌单 ID 不能为空".to_string());
    }
    let path = library_catalog_path(&state);
    let mut library = W4djLibrary::open(&path).map_err(|error| error.to_string())?;
    let report = library
        .compute_imported_dj_playlist_matches(&playlist_id)
        .map_err(|error| error.to_string())?;
    library
        .replace_imported_dj_playlist_matches(&playlist_id, &report)
        .map_err(|error| error.to_string())?;
    library
        .get_imported_dj_playlist_match_report(&playlist_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn load_imported_dj_playlist_matches(
    playlist_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<DjPlaylistMatchReport, String> {
    if playlist_id.trim().is_empty() {
        return Err("DJ 歌单 ID 不能为空".to_string());
    }
    let path = library_catalog_path(&state);
    W4djLibrary::open(&path)
        .and_then(|library| library.get_imported_dj_playlist_match_report(&playlist_id))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_imported_dj_playlist_match(
    playlist_id: String,
    position: u64,
    track_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<DjPlaylistMatchReport, String> {
    if playlist_id.trim().is_empty() || track_key.trim().is_empty() {
        return Err("DJ 歌单 ID 和输出 trackKey 不能为空".to_string());
    }
    let path = library_catalog_path(&state);
    let mut library = W4djLibrary::open(&path).map_err(|error| error.to_string())?;
    library
        .set_imported_dj_playlist_match(&playlist_id, position, &track_key)
        .map_err(|error| error.to_string())?;
    library
        .get_imported_dj_playlist_match_report(&playlist_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn clear_imported_dj_playlist_match(
    playlist_id: String,
    position: u64,
    state: tauri::State<'_, AppState>,
) -> Result<DjPlaylistMatchReport, String> {
    if playlist_id.trim().is_empty() {
        return Err("DJ 歌单 ID 不能为空".to_string());
    }
    let path = library_catalog_path(&state);
    let mut library = W4djLibrary::open(&path).map_err(|error| error.to_string())?;
    library
        .clear_imported_dj_playlist_match(&playlist_id, position)
        .map_err(|error| error.to_string())?;
    library
        .get_imported_dj_playlist_match_report(&playlist_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn export_imported_dj_playlist_m3u8(
    playlist_id: String,
    path: String,
    allow_partial: bool,
    copy_audio: Option<bool>,
    state: tauri::State<'_, AppState>,
) -> Result<DjPlaylistM3u8ExportResult, String> {
    if playlist_id.trim().is_empty() {
        return Err("DJ 歌单 ID 不能为空".to_string());
    }
    let selected_path = PathBuf::from(path);
    if !selected_path.is_absolute() {
        return Err("M3U8 导出路径必须是绝对路径".to_string());
    }
    if selected_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("m3u8"))
    {
        return Err("M3U8 导出路径必须使用 .m3u8 扩展名".to_string());
    }
    let library_path = library_catalog_path(&state);
    let library = W4djLibrary::open(&library_path).map_err(|error| error.to_string())?;
    let playlist = library
        .get_imported_dj_playlist(&playlist_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "找不到指定的 DJ 歌单".to_string())?;
    let selected_parent = selected_path
        .parent()
        .ok_or_else(|| "无法确定歌单导出目录".to_string())?;
    if !selected_parent.is_dir() {
        return Err("歌单导出目录不存在".to_string());
    }
    let (export_directory, output) = playlist_export_paths(&selected_path, &playlist.name)?;
    let report = library
        .get_imported_dj_playlist_match_report(&playlist_id)
        .map_err(|error| error.to_string())?;
    let candidates = library
        .available_dj_output_candidates()
        .map_err(|error| error.to_string())?;
    let resolved = report
        .matches
        .iter()
        .filter_map(|row| {
            let track_key = row.track_key.as_deref()?;
            let candidate = candidates.iter().find(|candidate| candidate.track_key == track_key)?;
            Some(ResolvedDjPlaylistTrack {
                position: row.position,
                title: row.title.clone(),
                artist_display: row.artist_display.clone(),
                duration_seconds: candidate.duration_seconds,
                destination_path: candidate.destination_path.clone(),
            })
        })
        .collect::<Vec<_>>();
    let mut resolved = resolved;
    let copy_audio = copy_audio.unwrap_or(false);
    let created_export_directory = !export_directory.exists();
    if created_export_directory {
        fs::create_dir(&export_directory)
            .map_err(|error| format!("创建歌单文件夹失败：{error}"))?;
    } else if !export_directory.is_dir() {
        return Err(format!(
            "歌单文件夹路径已被文件占用：{}",
            export_directory.display()
        ));
    }
    if let Err(error) = set_portable_export_permissions(&export_directory, 0o755) {
        if created_export_directory {
            let _ = fs::remove_dir(&export_directory);
        }
        return Err(error);
    }
    let created_audio = if copy_audio {
        match copy_playlist_audio_to_directory(&mut resolved, &export_directory) {
            Ok(paths) => paths,
            Err(error) => {
                if created_export_directory {
                    let _ = fs::remove_dir(&export_directory);
                }
                return Err(error);
            }
        }
    } else {
        Vec::new()
    };
    let (contents, summary): (String, M3u8ExportSummary) =
        match build_relative_m3u8_with_summary(&playlist, &resolved, &output, allow_partial) {
            Ok(result) => result,
            Err(error) => {
                for created_path in &created_audio {
                    let _ = fs::remove_file(created_path);
                }
                if created_export_directory {
                    let _ = fs::remove_dir(&export_directory);
                }
                return Err(error.to_string());
            }
        };
    if copy_audio
        && let Err(error) =
            validate_portable_playlist_export(&resolved, &export_directory, &contents)
    {
        for created_path in &created_audio {
            let _ = fs::remove_file(created_path);
        }
        if created_export_directory {
            let _ = fs::remove_dir(&export_directory);
        }
        return Err(error);
    }
    if let Err(error) = write_relative_m3u8_atomic(&output, &contents) {
        for created_path in &created_audio {
            let _ = fs::remove_file(created_path);
        }
        if created_export_directory {
            let _ = fs::remove_dir(&export_directory);
        }
        return Err(error.to_string());
    }
    Ok(DjPlaylistM3u8ExportResult {
        path: output.to_string_lossy().into_owned(),
        export_directory: export_directory.to_string_lossy().into_owned(),
        matched_count: summary.matched_count,
        total: summary.total,
        copied_count: if copy_audio { resolved.len() } else { 0 },
        copy_audio,
        portable: copy_audio,
        omitted: summary.omitted,
    })
}

fn open_library_catalog(path: &Path) -> Result<(LibraryCatalog, Option<PathBuf>), String> {
    LibraryCatalog::open_or_recover(path).map_err(|error| error.to_string())
}

fn open_w4dj_library(path: &Path) -> Result<(W4djLibrary, Option<PathBuf>), String> {
    // Opening the private catalog is intentionally side-effect free.  In
    // particular, do not clean old temporary rows or inspect their paths on
    // a status/query call: startup must only open SQLite and render the UI.
    W4djLibrary::open_or_recover(path).map_err(|error| error.to_string())
}

fn manual_netease_database_path(state: &AppState) -> Option<PathBuf> {
    state
        .library
        .manual_database_path
        .lock()
        .expect("manual database path lock poisoned")
        .clone()
}

fn netease_metadata_cache_path(state: &AppState) -> PathBuf {
    state
        .library
        .catalog_path
        .lock()
        .expect("library catalog path lock poisoned")
        .with_file_name("library-dashboard.sqlite3")
}

fn emit_netease_metadata_cache_progress(
    app: &tauri::AppHandle,
    progress: &NeteaseMetadataCacheProgress,
) {
    let _ = app.emit("netease-metadata-cache-progress", progress.clone());
}

fn metadata_cache_snapshot(state: &AppState) -> NeteaseMetadataCacheProgress {
    state
        .library
        .metadata_cache
        .lock()
        .expect("metadata cache progress lock poisoned")
        .clone()
}

fn update_metadata_cache_progress<F>(state: &LibraryState, update: F) -> NeteaseMetadataCacheProgress
where
    F: FnOnce(&mut NeteaseMetadataCacheProgress),
{
    let mut progress = state
        .metadata_cache
        .lock()
        .expect("metadata cache progress lock poisoned");
    update(&mut progress);
    progress.clone()
}

fn build_metadata_cache_blocking(
    library: &LibraryState,
    cache_path: &Path,
    database_path: &Path,
) -> Result<usize, String> {
    let _build_guard = library
        .metadata_cache_build_lock
        .lock()
        .map_err(|_| "metadata cache build lock poisoned".to_string())?;
    let fingerprint = database_fingerprint_view(database_path);
    netease_cache::mark_state(cache_path, CacheState::Building, None)
        .map_err(|error| format!("初始化网易云轻量索引失败：{error}"))?;
    let locators = load_locators_from_db_observed(database_path, |table, processed, total| {
        if library.metadata_cache_cancel.load(Ordering::SeqCst) {
            return false;
        }
        let _ = update_metadata_cache_progress(library, |progress| {
            progress.status = CacheState::Building.as_str().to_string();
            progress.stage = "readingLocators".to_string();
            progress.processed = processed;
            progress.total = Some(total);
            progress.current_item = table.to_string();
            progress.message = "正在建立网易云轻量索引".to_string();
        });
        true
    })
    .map_err(|error| format!("读取网易云轻量索引失败：{error}"))?;
    if library.metadata_cache_cancel.load(Ordering::SeqCst) {
        let _ = netease_cache::mark_state(cache_path, CacheState::Cancelled, None);
        return Err("cancelled".to_string());
    }
    let after = database_fingerprint_view(database_path);
    if after != fingerprint {
        let _ = netease_cache::mark_state(cache_path, CacheState::Stale, Some("源数据库在建立索引期间发生变化"));
        return Err("网易云数据库在建立索引期间发生变化，请重试".to_string());
    }
    netease_cache::replace_locators(cache_path, database_path, &fingerprint, &locators)
        .map_err(|error| format!("提交网易云轻量索引失败：{error}"))?;
    Ok(locators.len())
}

fn ensure_metadata_cache_ready(state: &AppState) -> Result<(), String> {
    let Some(database_path) = locate_supported_database(manual_netease_database_path(state).as_deref()) else {
        return Ok(());
    };
    let cache_path = netease_metadata_cache_path(state);
    let fingerprint = database_fingerprint_view(&database_path);
    let summary = netease_cache::read_summary(&cache_path, Some(&database_path), Some(&fingerprint))
        .map_err(|error| format!("读取网易云轻量索引状态失败：{error}"))?;
    if summary.state == CacheState::Ready {
        return Ok(());
    }
    let count = build_metadata_cache_blocking(&state.library, &cache_path, &database_path)?;
    let _ = update_metadata_cache_progress(&state.library, |progress| {
        progress.status = CacheState::Ready.as_str().to_string();
        progress.stage = "completed".to_string();
        progress.processed = count;
        progress.total = Some(count);
        progress.current_item.clear();
        progress.message = "网易云轻量索引已就绪".to_string();
        progress.database_path = Some(database_path.to_string_lossy().into_owned());
        progress.cached_record_count = count;
        progress.error = None;
    });
    Ok(())
}

fn resolve_netease_metadata_database_status(
    manual_path: Option<&Path>,
    cache_path: &Path,
) -> Result<(NeteaseMetadataDatabaseStatus, NeteaseMetadataResolver), String> {
    let mut warning = None;
    if let Some(manual) = manual_path
        && (!manual.is_file()
            || !w4dj::netease::probe_netease_database(manual)
                .map(|summary| summary.supported)
                .unwrap_or(false))
    {
        warning = Some(format!(
            "保存的网易云数据库不可用或 schema 不受支持：{}，已尝试自动定位",
            manual.display()
        ));
    }
    let effective_path = locate_supported_database(manual_path);
    let cache_summary = if let Some(path) = effective_path.as_deref() {
        let fingerprint = database_fingerprint_view(path);
        netease_cache::read_summary(cache_path, Some(path), Some(&fingerprint))
            .map_err(|error| format!("网易云轻量索引状态读取失败：{error}"))?
    } else {
        netease_cache::CacheSummary::default()
    };
    let (resolver, cache_status, cached_record_count, database_changed) =
        if let Some(path) = effective_path.as_deref()
            && cache_summary.state == CacheState::Ready
        {
            let locators = netease_cache::read_locators(cache_path)
                .map_err(|error| format!("网易云轻量索引读取失败：{error}"))?;
            (
                NeteaseMetadataResolver::from_locators(path, locators, warning.clone()),
                Some(CacheState::Ready.as_str().to_string()),
                Some(cache_summary.record_count),
                Some(false),
            )
        } else {
            (
                NeteaseMetadataResolver::default(),
                Some(cache_summary.state.as_str().to_string()),
                Some(cache_summary.record_count),
                Some(cache_summary.state == CacheState::Stale),
            )
        };
    let source = if manual_path.is_some()
        && effective_path.as_deref() == manual_path
    {
        NeteaseMetadataDatabaseSource::Manual
    } else if effective_path.is_some() {
        NeteaseMetadataDatabaseSource::Automatic
    } else {
        NeteaseMetadataDatabaseSource::Unavailable
    };
    let status = NeteaseMetadataDatabaseStatus {
        manual_path: manual_path.map(|path| path.display().to_string()),
        effective_path: effective_path
            .as_ref()
            .map(|path| path.display().to_string()),
        source,
        loaded: effective_path.is_some() && cache_summary.state == CacheState::Ready,
        record_count: if let Some(path) = effective_path.as_deref() {
            w4dj::netease::probe_netease_database(path)
                .map(|summary| summary.record_count)
                .unwrap_or_default()
        } else {
            0
        },
        warning: warning.or_else(|| {
            (effective_path.is_some() && cache_summary.state != CacheState::Ready)
                .then(|| "网易云轻量索引未就绪，转换前会按需准备".to_string())
        }),
        cache_status,
        cached_record_count,
        database_changed,
    };
    Ok((status, resolver))
}

fn persist_preferences_checked(state: &AppState) -> Result<(), String> {
    let mut preferences = {
        let controller = state.controller.lock().map_err(|_| "desktop lock poisoned".to_string())?;
        controller.state().preferences()
    };
    preferences.netease_database_path = state
        .library
        .manual_database_path
        .lock()
        .map_err(|_| "manual database path lock poisoned".to_string())?
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());

    let preferences_path = state
        .preferences_path
        .lock()
        .map_err(|_| "preferences path lock poisoned".to_string())?
        .clone();
    if preferences_path.as_os_str().is_empty() {
        return Ok(());
    }
    save_preferences(&preferences_path, &preferences)
        .map_err(|error| format!("保存偏好失败：{error}"))
}

fn set_manual_netease_database_path(
    state: &AppState,
    path: Option<PathBuf>,
) -> Result<(), String> {
    let previous = {
        let mut manual = state
            .library
            .manual_database_path
            .lock()
            .map_err(|_| "manual database path lock poisoned".to_string())?;
        let previous = manual.clone();
        *manual = path;
        previous
    };
    if let Err(error) = persist_preferences_checked(state) {
        *state
            .library
            .manual_database_path
            .lock()
            .map_err(|_| "manual database path lock poisoned".to_string())? = previous;
        return Err(error);
    }
    Ok(())
}

fn ensure_netease_database_not_busy(state: &AppState) -> Result<(), String> {
    if matches!(
        library_progress_snapshot(state).status,
        LibraryRefreshStatus::Running | LibraryRefreshStatus::Cancelling
    ) {
        return Err("歌曲库正在更新，暂时不能更换数据库".to_string());
    }
    Ok(())
}

fn conversion_metadata_context(state: &AppState) -> Arc<ConversionMetadataContext> {
    let preferred = manual_netease_database_path(state);
    // This is the first conversion/scan boundary, not app startup. Prepare
    // the small locator snapshot here if the user has not explicitly done so;
    // complete source rows are still fetched one song at a time by recover().
    if let Err(error) = ensure_metadata_cache_ready(state)
        && error != "cancelled"
    {
        eprintln!("Netease metadata cache warning: {error}");
    }
    let cache_path = netease_metadata_cache_path(state);
    let (resolver, warning) = NeteaseMetadataResolver::load_lazy_with_warning(
        preferred.as_deref(),
        &cache_path,
    )
        .unwrap_or_else(|error| {
            (
                NeteaseMetadataResolver::default(),
                Some(format!("网易云数据库加载失败：{error}")),
            )
        });
    if let Some(warning) = warning {
        eprintln!("Netease metadata resolver warning: {warning}");
    }
    Arc::new(ConversionMetadataContext {
        netease: Arc::new(resolver),
    })
}

fn task_one_music_directory(state: &AppState) -> Option<PathBuf> {
    state
        .controller
        .lock()
        .ok()
        .and_then(|controller| controller.state().slots.first().cloned())
        .map(|slot| PathBuf::from(slot.source_directory))
        .filter(|path| path.is_dir())
}

fn resolve_netease_library_inputs(
    manual_path: Option<&Path>,
    task_one_source: Option<&Path>,
    for_refresh: bool,
) -> (NeteaseDiscovery, Option<String>) {
    let mut warning = None;
    let mut discovery = manual_path
        .and_then(|path| match if for_refresh {
            discover_netease_library_from_database_for_refresh(path)
        } else {
            discover_netease_library_from_database(path)
        } {
            Ok(discovery) => Some(discovery),
            Err(error) => {
                warning = Some(format!("保存的网易云数据库不可用：{error}，已尝试自动定位"));
                None
            }
        })
        .unwrap_or_else(|| {
            if for_refresh {
                discover_netease_library_for_refresh()
            } else {
                discover_netease_library()
            }
        });

    if task_one_source.is_some() {
        discovery.music_folder = task_one_source.map(Path::to_path_buf);
        if !for_refresh {
            discovery.local_file_count = discovery
                .music_folder
                .as_deref()
                .map(count_audio_files)
                .unwrap_or_default();
        }
    }
    (discovery, warning)
}

fn library_progress_snapshot(state: &AppState) -> LibraryRefreshProgress {
    state
        .library
        .refresh
        .lock()
        .expect("library refresh lock poisoned")
        .clone()
}

fn invalid_scan_is_active(state: &AppState) -> bool {
    matches!(
        state
            .library
            .invalid_scan
            .lock()
            .expect("invalid scan lock poisoned")
            .status
            .as_str(),
        "running" | "cancelling"
    )
}

fn update_library_progress(
    state: &LibraryState,
    update: impl FnOnce(&mut LibraryRefreshProgress),
) -> LibraryRefreshProgress {
    let mut progress = state
        .refresh
        .lock()
        .expect("library refresh lock poisoned");
    update(&mut progress);
    progress.clone()
}

fn emit_library_progress(app: &tauri::AppHandle, progress: &LibraryRefreshProgress) {
    let _ = app.emit("library-refresh-progress", progress);
}

fn set_library_stage(
    app: &tauri::AppHandle,
    state: &LibraryState,
    stage: LibraryRefreshStage,
    message: impl Into<String>,
) {
    let progress = update_library_progress(state, |progress| {
        progress.stage = stage;
        progress.message = message.into();
        progress.processed = 0;
        progress.total = None;
        progress.current_item.clear();
    });
    emit_library_progress(app, &progress);
}

fn finish_library_refresh(
    app: &tauri::AppHandle,
    state: &LibraryState,
    status: LibraryRefreshStatus,
    stage: Option<LibraryRefreshStage>,
    message: impl Into<String>,
    summary: Option<LibraryRefreshSummary>,
    error: Option<String>,
) {
    let message = message.into();
    let progress = update_library_progress(state, |progress| {
        apply_library_refresh_terminal(progress, status, stage, message, summary, error);
    });
    emit_library_progress(app, &progress);
}

fn apply_library_refresh_terminal(
    progress: &mut LibraryRefreshProgress,
    status: LibraryRefreshStatus,
    stage: Option<LibraryRefreshStage>,
    message: String,
    summary: Option<LibraryRefreshSummary>,
    error: Option<String>,
) {
    progress.status = status;
    if let Some(stage) = stage {
        progress.stage = stage;
    }
    progress.message = message;
    progress.error = error;
    progress.current_item.clear();
    if matches!(progress.status, LibraryRefreshStatus::Completed)
        && let Some(summary) = summary.as_ref()
    {
        let track_count = usize::try_from(summary.track_count).unwrap_or(usize::MAX);
        progress.processed = track_count;
        progress.total = Some(track_count);
    }
    progress.summary = summary;
    if progress.total.is_none() {
        progress.total = Some(progress.processed);
    }
}

#[derive(Debug)]
enum LibraryRefreshRunError {
    Cancelled,
    Failed(String),
}

fn run_library_refresh(
    app: &tauri::AppHandle,
    state: &LibraryState,
    catalog_path: &Path,
    manual_database_path: Option<&Path>,
    task_one_source: Option<&Path>,
    analysis_path: &Path,
) -> Result<LibraryRefreshSummary, LibraryRefreshRunError> {
    set_library_stage(app, state, LibraryRefreshStage::LocatingDatabase, "正在定位网易云数据库");
    if state.cancel.load(Ordering::SeqCst) {
        return Err(LibraryRefreshRunError::Cancelled);
    }

    let (discovery, warning) =
        resolve_netease_library_inputs(manual_database_path, task_one_source, true);
    if let Some(warning) = warning {
        let progress = update_library_progress(state, |progress| {
            progress.message = warning;
        });
        emit_library_progress(app, &progress);
    }
    if state.cancel.load(Ordering::SeqCst) {
        return Err(LibraryRefreshRunError::Cancelled);
    }
    let database_path = discovery
        .database_path
        .as_deref()
        .ok_or_else(|| {
            LibraryRefreshRunError::Failed(
                "未找到网易云音乐本地数据库，请手动选择歌曲来源".to_string(),
            )
        })?;
    let music_folder = discovery.music_folder.as_deref();

    let (old_catalog, _) = open_library_catalog(catalog_path)
        .map_err(LibraryRefreshRunError::Failed)?;
    let mut last_snapshot_emit = Instant::now() - Duration::from_millis(100);
    let mut last_snapshot_stage = "";
    let snapshot = build_catalog_snapshot_incremental_observed(
        database_path,
        music_folder,
        Some(&old_catalog),
        || state.cancel.load(Ordering::SeqCst),
        |update| {
            let stage = match update.stage {
                "readingRecords" => LibraryRefreshStage::ReadingRecords,
                "checkingLocalFiles" => LibraryRefreshStage::CheckingLocalFiles,
                "probingLocalFiles" => LibraryRefreshStage::ProbingLocalFiles,
                _ => LibraryRefreshStage::ReadingRecords,
            };
            let progress = update_library_progress(state, |progress| {
                progress.stage = stage;
                progress.processed = update.processed;
                progress.total = update.total;
                progress.current_item = update.current_item;
                progress.message = "正在建立歌曲库快照".to_string();
            });
            let should_emit = last_snapshot_stage != update.stage
                || last_snapshot_emit.elapsed() >= Duration::from_millis(100)
                || update.total == Some(update.processed);
            if should_emit {
                last_snapshot_stage = update.stage;
                last_snapshot_emit = Instant::now();
                emit_library_progress(app, &progress);
            }
        },
    )
    .map_err(|error| match error {
        CatalogBuildError::Cancelled => LibraryRefreshRunError::Cancelled,
        CatalogBuildError::Failed(message) => LibraryRefreshRunError::Failed(message),
    })?;

    if state.cancel.load(Ordering::SeqCst) {
        return Err(LibraryRefreshRunError::Cancelled);
    }
    set_library_stage(app, state, LibraryRefreshStage::ImportingAnalysis, "正在导入已有分析结果");
    let analysis_entries = load_analysis_file(analysis_path)
        .map_err(|error| LibraryRefreshRunError::Failed(format!("无法读取分析缓存：{error}")))?;
    let local_file_count = snapshot.local_files.len();
    let readable_file_count = snapshot.local_files.iter().filter(|file| file.readable).count();
    let reused_file_count = snapshot
        .local_files
        .iter()
        .filter(|file| old_catalog.local_file_by_path(Path::new(&file.path)).ok().flatten().is_some())
        .count();

    set_library_stage(app, state, LibraryRefreshStage::Committing, "正在提交歌曲库更新");
    let mut catalog = open_library_catalog(catalog_path)
        .map_err(LibraryRefreshRunError::Failed)?
        .0;
    let mut last_commit_emit = Instant::now() - Duration::from_millis(100);
    catalog
        .upsert_snapshot_with_analysis(
            &snapshot,
            &analysis_entries,
            || state.cancel.load(Ordering::SeqCst),
            |processed, total, item| {
                let progress = update_library_progress(state, |progress| {
                    progress.stage = LibraryRefreshStage::Committing;
                    progress.processed = processed;
                    progress.total = Some(total);
                    progress.current_item = Path::new(item)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| item.to_string());
                });
                if last_commit_emit.elapsed() >= Duration::from_millis(100) || processed == total {
                    last_commit_emit = Instant::now();
                    emit_library_progress(app, &progress);
                }
            },
        )
        .map_err(|error| {
            if error.to_string().contains("刷新已取消") {
                LibraryRefreshRunError::Cancelled
            } else {
                LibraryRefreshRunError::Failed(error.to_string())
            }
        })?;
    let track_count = catalog
        .count_tracks()
        .map_err(|error| LibraryRefreshRunError::Failed(error.to_string()))?
        .max(0) as u64;
    Ok(LibraryRefreshSummary {
        track_count,
        local_file_count,
        readable_file_count,
        reused_file_count,
        database_path: database_path.display().to_string(),
        music_folder: music_folder.map(|path| path.display().to_string()),
    })
}

#[tauri::command]
fn load_library_status(state: tauri::State<'_, AppState>) -> Result<LibraryStatus, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_w4dj_library(&path)?;
    let stats = catalog.stats().map_err(|error| error.to_string())?;
    let manual_path = manual_netease_database_path(&state);
    let database_warning = manual_path.as_deref().and_then(|path| {
        (!path.is_file()).then(|| {
            format!(
                "保存的网易云数据库不可用：{}；兼容刷新时将尝试自动定位",
                path.display()
            )
        })
    });
    Ok(LibraryStatus {
        catalog_path: path.display().to_string(),
        track_count: stats.total,
        analyzed_track_count: stats.analysis_completed,
        // Loading Dashboard state must never probe or open the NetEase DB.
        netease: NeteaseDiscovery {
            database_path: None,
            music_folder: None,
            record_count: 0,
            local_file_count: 0,
        },
        manual_database_path: manual_path.map(|path| path.display().to_string()),
        refresh: library_progress_snapshot(&state),
        database_warning,
        total_track_count: stats.total,
        available_track_count: stats.available,
        invalid_track_count: stats.invalid,
        not_analyzed_count: stats.not_analyzed,
        analysis_failed_count: stats.analysis_failed,
        analysis_completed_count: stats.analysis_completed,
        invalid_scan: state
            .library
            .invalid_scan
            .lock()
            .expect("invalid scan lock poisoned")
            .clone(),
    })
}

#[tauri::command]
fn export_emotion_evaluation_manifest(
    output_path: String,
    count: Option<usize>,
    seed: Option<u64>,
    state: tauri::State<'_, AppState>,
) -> Result<EmotionEvaluationManifest, String> {
    let output_path = output_path.trim();
    if output_path.is_empty() {
        return Err("情绪验收 manifest 缺少输出路径".to_string());
    }
    let path = library_catalog_path(&state);
    let library = open_w4dj_library(&path)?.0;
    let manifest = library
        .emotion_evaluation_manifest(count.unwrap_or(100), seed.unwrap_or(1))
        .map_err(|error| error.to_string())?;
    write_emotion_evaluation_manifest(Path::new(output_path), &manifest)
        .map_err(|error| error.to_string())?;
    Ok(manifest)
}

#[tauri::command]
fn locate_netease_library(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    force: Option<bool>,
) -> NeteaseDiscovery {
    let force = force.unwrap_or(false);
    let emit = |event: NeteaseDiscoveryProgressEvent| {
        let _ = app.emit("netease-discovery-progress", event);
    };
    emit(NeteaseDiscoveryProgressEvent {
        status: "running".to_string(),
        stage: "locatingDatabase".to_string(),
        processed: 0,
        total: None,
        current_item: String::new(),
        message: "正在查找网易云数据库".to_string(),
        suggestion: None,
        error: None,
    });
    let manual_path = manual_netease_database_path(&state);
    let task_one_source = (!force)
        .then(|| task_one_music_directory(&state))
        .flatten();
    let (mut discovery, warning) = if let Some(path) = manual_path.as_deref() {
        match discover_netease_library_from_database_observed(path, false, |progress| {
            emit(NeteaseDiscoveryProgressEvent {
                status: "running".to_string(),
                stage: progress.stage.to_string(),
                processed: progress.processed,
                total: progress.total,
                current_item: progress.current_item,
                message: progress.message,
                suggestion: None,
                error: None,
            });
        }) {
            Ok(discovery) => (discovery, None),
            Err(error) => (
                discover_netease_library_observed(false, |progress| {
                    emit(NeteaseDiscoveryProgressEvent {
                        status: "running".to_string(),
                        stage: progress.stage.to_string(),
                        processed: progress.processed,
                        total: progress.total,
                        current_item: progress.current_item,
                        message: progress.message,
                        suggestion: None,
                        error: None,
                    });
                }),
                Some(format!("保存的网易云数据库不可用：{error}，已尝试自动定位")),
            ),
        }
    } else {
        (
            discover_netease_library_observed(false, |progress| {
                emit(NeteaseDiscoveryProgressEvent {
                    status: "running".to_string(),
                    stage: progress.stage.to_string(),
                    processed: progress.processed,
                    total: progress.total,
                    current_item: progress.current_item,
                    message: progress.message,
                    suggestion: None,
                    error: None,
                });
            }),
            None,
        )
    };
    if let Some(task_one_source) = task_one_source.clone() {
        discovery.music_folder = Some(task_one_source);
    }
    let discovery_ok = discovery.database_path.is_some();
    if !discovery_ok {
        emit(NeteaseDiscoveryProgressEvent {
            status: "error".to_string(),
            stage: "checkingMusicFolder".to_string(),
            processed: 0,
            total: None,
            current_item: String::new(),
            message: warning.clone().unwrap_or_else(|| {
                "未找到网易云本地数据库，请选择来源或在歌曲库中手动选择数据库".to_string()
            }),
            suggestion: Some(discovery.clone()),
            error: Some(warning.unwrap_or_else(|| "未找到网易云本地数据库".to_string())),
        });
        return discovery;
    }

    let should_count = discovery.music_folder.as_deref().is_some_and(|folder| {
        if let Some(current) = task_one_source.as_deref() {
            w4dj::scan_cache::normalize_path(folder) != w4dj::scan_cache::normalize_path(current)
        } else {
            true
        }
    });

    if !should_count {
        emit(NeteaseDiscoveryProgressEvent {
            status: "completed".to_string(),
            stage: "checkingMusicFolder".to_string(),
            processed: 0,
            total: Some(0),
            current_item: String::new(),
            message: warning.unwrap_or_else(|| "网易云库发现完成".to_string()),
            suggestion: Some(discovery.clone()),
            error: None,
        });
        return discovery;
    }

    emit(NeteaseDiscoveryProgressEvent {
        status: "running".to_string(),
        stage: "checkingMusicFolder".to_string(),
        processed: 0,
        total: None,
        current_item: String::new(),
        message: "正在检查音乐目录".to_string(),
        suggestion: Some(discovery.clone()),
        error: None,
    });
    if let Some(folder) = discovery.music_folder.clone() {
        let app = app.clone();
        let warning = warning.clone();
        let suggestion = discovery.clone();
        thread::spawn(move || {
            let count = count_audio_files_observed(&folder, |processed, path| {
                let _ = app.emit(
                    "netease-discovery-progress",
                    NeteaseDiscoveryProgressEvent {
                        status: "running".to_string(),
                        stage: "checkingMusicFolder".to_string(),
                        processed,
                        total: None,
                        current_item: path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        message: "正在检查音乐目录".to_string(),
                        suggestion: None,
                        error: None,
                    },
                );
            });
            let mut completed = suggestion;
            completed.local_file_count = count;
            let _ = app.emit(
                "netease-discovery-progress",
                NeteaseDiscoveryProgressEvent {
                    status: "completed".to_string(),
                    stage: "checkingMusicFolder".to_string(),
                    processed: count,
                    total: Some(count),
                    current_item: String::new(),
                    message: warning.unwrap_or_else(|| "网易云库发现完成".to_string()),
                    suggestion: Some(completed),
                    error: None,
                },
            );
        });
    }
    discovery
}

fn try_start_library_refresh(
    state: &LibraryState,
    refresh_id: String,
) -> Result<LibraryRefreshProgress, String> {
    let mut progress = state
        .refresh
        .lock()
        .expect("library refresh lock poisoned");
    if matches!(
        progress.status,
        LibraryRefreshStatus::Running | LibraryRefreshStatus::Cancelling
    ) {
        return Err("歌曲库正在更新".to_string());
    }
    state.cancel.store(false, Ordering::SeqCst);
    *progress = LibraryRefreshProgress {
        refresh_id,
        status: LibraryRefreshStatus::Running,
        stage: LibraryRefreshStage::LocatingDatabase,
        processed: 0,
        total: None,
        current_item: String::new(),
        message: "正在定位网易云数据库".to_string(),
        summary: None,
        error: None,
    };
    Ok(progress.clone())
}

fn request_library_refresh_cancel(state: &LibraryState) -> LibraryRefreshProgress {
    let mut progress = state
        .refresh
        .lock()
        .expect("library refresh lock poisoned");
    if matches!(progress.status, LibraryRefreshStatus::Running) {
        state.cancel.store(true, Ordering::SeqCst);
        progress.status = LibraryRefreshStatus::Cancelling;
        progress.message = "正在取消歌曲库更新".to_string();
    }
    progress.clone()
}

#[tauri::command]
fn refresh_library_catalog(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<LibraryRefreshProgress, String> {
    if invalid_scan_is_active(&state) {
        return Err("失效歌曲正在扫描，暂时不能刷新歌曲库".to_string());
    }
    let path = legacy_library_catalog_path(&state);
    let manual_database_path = manual_netease_database_path(&state);
    let task_one_source = task_one_music_directory(&state);
    let analysis_path = {
        let history_path = state
            .history_path
            .lock()
            .expect("history path lock poisoned")
            .clone();
        analysis_file_path(&history_path)
    };
    let refresh_id = format!("library-{}", unique_timestamp());
    let initial = try_start_library_refresh(&state.library, refresh_id.clone())?;
    emit_library_progress(&app, &initial);
    if let Some(worker) = state
        .library
        .worker
        .lock()
        .expect("library worker lock poisoned")
        .take()
    {
        let _ = worker.join();
    }
    let library = Arc::clone(&state.library);
    let worker_app = app.clone();
    let worker = thread::Builder::new()
        .name("library-refresh".to_string())
        .spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_library_refresh(
                &worker_app,
                &library,
                &path,
                manual_database_path.as_deref(),
                task_one_source.as_deref(),
                &analysis_path,
            )
        }));
        match result {
            Ok(Ok(summary)) => finish_library_refresh(
                &worker_app,
                &library,
                LibraryRefreshStatus::Completed,
                Some(LibraryRefreshStage::Committing),
                "歌曲库更新完成",
                Some(summary),
                None,
            ),
            Ok(Err(LibraryRefreshRunError::Cancelled)) => finish_library_refresh(
                &worker_app,
                &library,
                LibraryRefreshStatus::Cancelled,
                None,
                "歌曲库更新已取消",
                None,
                None,
            ),
            Ok(Err(LibraryRefreshRunError::Failed(error))) => finish_library_refresh(
                &worker_app,
                &library,
                LibraryRefreshStatus::Error,
                None,
                "歌曲库更新失败",
                None,
                Some(error),
            ),
            Err(_) => finish_library_refresh(
                &worker_app,
                &library,
                LibraryRefreshStatus::Error,
                None,
                "歌曲库更新失败",
                None,
                Some("歌曲库后台任务异常退出".to_string()),
            ),
        }
        })
        .map_err(|error| {
            finish_library_refresh(
                &app,
                &state.library,
                LibraryRefreshStatus::Error,
                None,
                "歌曲库更新失败",
                None,
                Some(format!("无法启动歌曲库后台任务：{error}")),
            );
            error.to_string()
        })?;
    *state.library.worker.lock().expect("library worker lock poisoned") = Some(worker);
    Ok(initial)
}

#[tauri::command]
fn cancel_library_refresh(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> LibraryRefreshProgress {
    let snapshot = request_library_refresh_cancel(&state.library);
    emit_library_progress(&app, &snapshot);
    snapshot
}

#[tauri::command]
fn select_netease_database_fallback(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<LibraryStatus, String> {
    ensure_netease_database_not_busy(state.inner())?;
    let database_path = PathBuf::from(path.trim());
    NeteaseMetadataResolver::load_exact(&database_path)
        .map_err(|error| format!("所选网易云数据库无效：{error}"))?;
    set_manual_netease_database_path(state.inner(), Some(database_path))?;
    load_library_status(state)
}

#[tauri::command]
fn clear_netease_database_fallback(
    state: tauri::State<'_, AppState>,
) -> Result<LibraryStatus, String> {
    ensure_netease_database_not_busy(state.inner())?;
    set_manual_netease_database_path(state.inner(), None)?;
    load_library_status(state)
}

#[tauri::command]
fn load_netease_metadata_database_status(
    state: tauri::State<'_, AppState>,
) -> Result<NeteaseMetadataDatabaseStatus, String> {
    let manual_path = manual_netease_database_path(state.inner());
    let cache_path = netease_metadata_cache_path(state.inner());
    resolve_netease_metadata_database_status(manual_path.as_deref(), &cache_path)
        .map(|(status, _)| status)
}

#[tauri::command]
fn select_netease_metadata_database(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<NeteaseMetadataDatabaseStatus, String> {
    ensure_netease_database_not_busy(state.inner())?;
    let database_path = PathBuf::from(path.trim());
    let supported = database_path.is_file()
        && w4dj::netease::probe_netease_database(&database_path)
            .map(|summary| summary.supported)
            .unwrap_or(false);
    if !supported {
        return Err("所选网易云数据库无效：schema 不受支持".to_string());
    }
    set_manual_netease_database_path(state.inner(), Some(database_path))?;
    load_netease_metadata_database_status(state)
}

#[tauri::command]
fn clear_netease_metadata_database(
    state: tauri::State<'_, AppState>,
) -> Result<NeteaseMetadataDatabaseStatus, String> {
    ensure_netease_database_not_busy(state.inner())?;
    set_manual_netease_database_path(state.inner(), None)?;
    load_netease_metadata_database_status(state)
}

#[tauri::command]
fn load_netease_metadata_cache_status(
    state: tauri::State<'_, AppState>,
) -> NeteaseMetadataCacheProgress {
    metadata_cache_snapshot(state.inner())
}

#[tauri::command]
fn cancel_netease_metadata_cache(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> NeteaseMetadataCacheProgress {
    state
        .library
        .metadata_cache_cancel
        .store(true, Ordering::SeqCst);
    let progress = update_metadata_cache_progress(&state.library, |progress| {
        if progress.status == CacheState::Building.as_str() {
            progress.status = CacheState::Cancelling.as_str().to_string();
            progress.message = "正在取消网易云轻量索引".to_string();
        }
    });
    emit_netease_metadata_cache_progress(&app, &progress);
    progress
}

#[tauri::command]
fn prepare_netease_metadata_cache(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<NeteaseMetadataCacheProgress, String> {
    {
        let progress = state
            .library
            .metadata_cache
            .lock()
            .map_err(|_| "metadata cache progress lock poisoned".to_string())?;
        if progress.status == CacheState::Building.as_str()
            || progress.status == CacheState::Cancelling.as_str()
        {
            return Ok(progress.clone());
        }
    }
    let database_path = locate_supported_database(manual_netease_database_path(state.inner()).as_deref())
        .ok_or_else(|| "未找到可用的网易云数据库".to_string())?;
    let cache_path = netease_metadata_cache_path(state.inner());
    let fingerprint = database_fingerprint_view(&database_path);
    let summary = netease_cache::read_summary(&cache_path, Some(&database_path), Some(&fingerprint))
        .map_err(|error| format!("读取网易云轻量索引状态失败：{error}"))?;
    if summary.state == CacheState::Ready {
        let progress = update_metadata_cache_progress(&state.library, |progress| {
            progress.status = CacheState::Ready.as_str().to_string();
            progress.stage = "completed".to_string();
            progress.processed = summary.record_count;
            progress.total = Some(summary.record_count);
            progress.database_path = Some(database_path.to_string_lossy().into_owned());
            progress.cached_record_count = summary.record_count;
            progress.message = "网易云轻量索引已就绪".to_string();
            progress.error = None;
        });
        emit_netease_metadata_cache_progress(&app, &progress);
        return Ok(progress);
    }
    state
        .library
        .metadata_cache_cancel
        .store(false, Ordering::SeqCst);
    let initial = update_metadata_cache_progress(&state.library, |progress| {
        progress.status = CacheState::Building.as_str().to_string();
        progress.stage = "readingLocators".to_string();
        progress.processed = 0;
        progress.total = None;
        progress.current_item.clear();
        progress.database_path = Some(database_path.to_string_lossy().into_owned());
        progress.cached_record_count = summary.record_count;
        progress.message = "正在建立网易云轻量索引".to_string();
        progress.error = None;
    });
    emit_netease_metadata_cache_progress(&app, &initial);
    if let Some(worker) = state
        .library
        .metadata_cache_worker
        .lock()
        .map_err(|_| "metadata cache worker lock poisoned".to_string())?
        .take()
    {
        let _ = worker.join();
    }
    let library = Arc::clone(&state.library);
    let worker_app = app.clone();
    let worker = thread::Builder::new()
        .name("netease-metadata-cache".to_string())
        .spawn(move || {
            let result = build_metadata_cache_blocking(&library, &cache_path, &database_path);
            let progress = match result {
                Ok(count) => update_metadata_cache_progress(&library, |progress| {
                    progress.status = CacheState::Ready.as_str().to_string();
                    progress.stage = "completed".to_string();
                    progress.processed = count;
                    progress.total = Some(count);
                    progress.current_item.clear();
                    progress.cached_record_count = count;
                    progress.message = "网易云轻量索引已就绪".to_string();
                    progress.error = None;
                }),
                Err(error) if error == "cancelled" => update_metadata_cache_progress(&library, |progress| {
                    progress.status = CacheState::Cancelled.as_str().to_string();
                    progress.stage = "cancelled".to_string();
                    progress.message = "网易云轻量索引已取消".to_string();
                }),
                Err(error) => update_metadata_cache_progress(&library, |progress| {
                    progress.status = CacheState::Error.as_str().to_string();
                    progress.stage = "error".to_string();
                    progress.error = Some(error.clone());
                    progress.message = "网易云轻量索引失败".to_string();
                }),
            };
            emit_netease_metadata_cache_progress(&worker_app, &progress);
        })
        .map_err(|error| format!("启动网易云轻量索引失败：{error}"))?;
    *state
        .library
        .metadata_cache_worker
        .lock()
        .map_err(|_| "metadata cache worker lock poisoned".to_string())? = Some(worker);
    Ok(initial)
}

#[tauri::command]
fn query_library_catalog(
    query: LibraryQuery,
    state: tauri::State<'_, AppState>,
) -> Result<LibraryPage, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_w4dj_library(&path)?;
    catalog.query(&query).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_library_track_detail(
    track_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<w4dj::library_catalog::CatalogTrack>, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_w4dj_library(&path)?;
    catalog
        .track_detail(&track_key)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn relocate_library_track(
    track_key: String,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if matches!(
        library_progress_snapshot(&state).status,
        LibraryRefreshStatus::Running | LibraryRefreshStatus::Cancelling
    ) {
        return Err("歌曲库正在更新，暂时不能重新定位文件".to_string());
    }
    if invalid_scan_is_active(&state) {
        return Err("失效歌曲正在扫描，暂时不能重新定位文件".to_string());
    }
    let catalog_path = library_catalog_path(&state);
    let replacement_path = Path::new(path.trim());
    if !is_analyzable_audio_file(replacement_path) {
        return Err("所选文件不是支持的音频文件".to_string());
    }
    let (mut catalog, _) = open_w4dj_library(&catalog_path)?;
    catalog
        .relocate_analyzed_track(&track_key, replacement_path)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn remove_library_track(
    track_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    if matches!(
        library_progress_snapshot(&state).status,
        LibraryRefreshStatus::Running | LibraryRefreshStatus::Cancelling
    ) {
        return Err("歌曲库正在更新，暂时不能移除记录".to_string());
    }
    if invalid_scan_is_active(&state) {
        return Err("失效歌曲正在扫描，暂时不能移除记录".to_string());
    }
    let catalog_path = library_catalog_path(&state);
    let (mut catalog, _) = open_w4dj_library(&catalog_path)?;
    catalog
        .remove_analyzed_track(&track_key)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn clear_invalid_library_tracks(state: tauri::State<'_, AppState>) -> Result<u64, String> {
    if matches!(
        library_progress_snapshot(&state).status,
        LibraryRefreshStatus::Running | LibraryRefreshStatus::Cancelling
    ) {
        return Err("歌曲库正在更新，暂时不能清除失效文件".to_string());
    }
    if invalid_scan_is_active(&state) {
        return Err("失效歌曲正在扫描，暂时不能清除失效文件".to_string());
    }
    let catalog_path = library_catalog_path(&state);
    let (mut catalog, _) = open_w4dj_library(&catalog_path)?;
    catalog.remove_invalid().map_err(|error| error.to_string())
}

fn invalid_scan_snapshot(state: &AppState) -> InvalidScanProgress {
    state
        .library
        .invalid_scan
        .lock()
        .expect("invalid scan lock poisoned")
        .clone()
}

fn update_invalid_scan(
    state: &LibraryState,
    update: impl FnOnce(&mut InvalidScanProgress),
) -> InvalidScanProgress {
    let mut progress = state
        .invalid_scan
        .lock()
        .expect("invalid scan lock poisoned");
    update(&mut progress);
    progress.clone()
}

fn emit_invalid_scan_progress(app: &tauri::AppHandle, progress: &InvalidScanProgress) {
    let _ = app.emit("library-invalid-scan-progress", progress);
}

#[tauri::command]
fn find_invalid_library_tracks(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<InvalidScanProgress, String> {
    if matches!(
        library_progress_snapshot(&state).status,
        LibraryRefreshStatus::Running | LibraryRefreshStatus::Cancelling
    ) {
        return Err("歌曲库正在更新，暂时不能扫描失效歌曲".to_string());
    }
    {
        let mut progress = state
            .library
            .invalid_scan
            .lock()
            .expect("invalid scan lock poisoned");
        if matches!(progress.status.as_str(), "running" | "cancelling") {
            return Err("失效歌曲正在扫描".to_string());
        }
        state.library.invalid_scan_cancel.store(false, Ordering::SeqCst);
        progress.scan_id = format!("invalid-{}", unique_timestamp());
        progress.status = "running".to_string();
        progress.processed = 0;
        progress.total = 0;
        progress.current_item.clear();
        progress.message = "正在检查已登记歌曲".to_string();
        progress.error = None;
    }
    let initial = invalid_scan_snapshot(&state);
    emit_invalid_scan_progress(&app, &initial);
    if let Some(worker) = state
        .library
        .invalid_scan_worker
        .lock()
        .expect("invalid scan worker lock poisoned")
        .take()
    {
        let _ = worker.join();
    }
    let library = Arc::clone(&state.library);
    let path = library_catalog_path(&state);
    let worker_app = app.clone();
    let worker = thread::Builder::new()
        .name("library-invalid-scan".to_string())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let (mut database, _) = open_w4dj_library(&path)
                    .map_err(|error| error.to_string())?;
                let mut last_emit = Instant::now() - Duration::from_millis(100);
                database
                    .scan_invalid(
                        || library.invalid_scan_cancel.load(Ordering::SeqCst),
                        |processed, total, current_item| {
                            let progress = update_invalid_scan(&library, |value| {
                                value.processed = processed;
                                value.total = total;
                                value.current_item = current_item.to_string();
                                value.message = "正在检查已登记歌曲".to_string();
                            });
                            if last_emit.elapsed() >= Duration::from_millis(100)
                                || processed == total
                            {
                                last_emit = Instant::now();
                                emit_invalid_scan_progress(&worker_app, &progress);
                            }
                        },
                    )
                    .map_err(|error| error.to_string())
            }));
            let (status, message, error) = match result {
                Ok(Ok(stats)) => (
                    "completed",
                    format!(
                        "失效歌曲扫描完成：{} 首可用，{} 首失效",
                        stats.available, stats.invalid
                    ),
                    None,
                ),
                Ok(Err(error)) if error.contains("已取消") => {
                    ("cancelled", "失效歌曲扫描已取消".to_string(), None)
                }
                Ok(Err(error)) => ("error", "失效歌曲扫描失败".to_string(), Some(error)),
                Err(_) => (
                    "error",
                    "失效歌曲扫描失败".to_string(),
                    Some("后台扫描任务异常退出".to_string()),
                ),
            };
            let progress = update_invalid_scan(&library, |value| {
                value.status = status.to_string();
                value.message = message;
                value.error = error;
                value.current_item.clear();
                if value.total == 0 {
                    value.total = value.processed;
                }
            });
            emit_invalid_scan_progress(&worker_app, &progress);
        })
        .map_err(|error| {
            let progress = update_invalid_scan(&state.library, |value| {
                value.status = "error".to_string();
                value.message = "失效歌曲扫描无法启动".to_string();
                value.error = Some(error.to_string());
            });
            emit_invalid_scan_progress(&app, &progress);
            error.to_string()
        })?;
    *state
        .library
        .invalid_scan_worker
        .lock()
        .expect("invalid scan worker lock poisoned") = Some(worker);
    Ok(initial)
}

#[tauri::command]
fn cancel_invalid_library_scan(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> InvalidScanProgress {
    let progress = update_invalid_scan(&state.library, |value| {
        if value.status == "running" {
            state
                .library
                .invalid_scan_cancel
                .store(true, Ordering::SeqCst);
            value.status = "cancelling".to_string();
            value.message = "正在取消失效歌曲扫描".to_string();
        }
    });
    emit_invalid_scan_progress(&app, &progress);
    progress
}

#[tauri::command]
fn get_library_track_source_records(
    track_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CatalogSourceRecord>, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_w4dj_library(&path)?;
    catalog
        .source_records_for_track(&track_key)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_library_track_cover(
    track_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_w4dj_library(&path)?;
    let Some(track) = catalog
        .track_detail(&track_key)
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let source_path = catalog
        .local_files_for_track(&track_key)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|file| file.readable)
        .map(|file| file.path)
        .or_else(|| {
            track
                .cover_path
                .as_deref()
                .map(PathBuf::from)
                .filter(|path| path.is_file())
        });
    let Some(source_path) = source_path else {
        return Ok(None);
    };
    let bytes = w4dj::netease::recover_local_cover(&source_path)
        .or_else(|| fs::read(&source_path).ok().filter(|bytes| is_image_bytes(bytes)));
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let Some(mime) = image_mime_type(&bytes) else {
        return Ok(None);
    };
    Ok(Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryAnalysisCandidate {
    path: String,
    name: String,
    size_bytes: u64,
    slot_index: Option<usize>,
}

fn is_supported_library_analysis_file(path: &Path) -> bool {
    !is_ignored_music_file(path)
        && path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "mp3" | "flac" | "wav" | "aif" | "aiff"
                )
            })
}

fn build_library_analysis_candidates(
    files: Vec<CatalogLocalFile>,
    output_roots: &[PathBuf],
) -> Vec<LibraryAnalysisCandidate> {
    files
        .into_iter()
        .filter(|file| is_supported_library_analysis_file(&file.path))
        .map(|file| {
            let slot_index = output_roots
                .iter()
                .position(|root| file.path.starts_with(root));
            LibraryAnalysisCandidate {
                name: file
                    .path
                    .file_name()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.path.to_string_lossy().into_owned()),
                path: file.path.to_string_lossy().into_owned(),
                size_bytes: file.size_bytes.max(0) as u64,
                slot_index,
            }
        })
        .collect()
}

#[tauri::command]
fn list_library_analysis_candidates(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<LibraryAnalysisCandidate>, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_w4dj_library(&path)?;
    let output_roots = state
        .controller
        .lock()
        .ok()
        .map(|controller| {
            controller
                .state()
                .slots
                .iter()
                .map(|slot| PathBuf::from(&slot.destination_directory))
                .filter(|root| !root.as_os_str().is_empty())
                .map(|root| fs::canonicalize(&root).unwrap_or(root))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    catalog
        .readable_local_files()
        .map(|files| build_library_analysis_candidates(files, &output_roots))
        .map_err(|error| error.to_string())
}

fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

fn is_image_bytes(bytes: &[u8]) -> bool {
    image_mime_type(bytes).is_some()
}

#[tauri::command]
fn clear_library_catalog_cache(state: tauri::State<'_, AppState>) -> Result<(), String> {
    if matches!(library_progress_snapshot(&state).status, LibraryRefreshStatus::Running | LibraryRefreshStatus::Cancelling) {
        return Err("歌曲库正在更新，暂时不能清除索引".to_string());
    }
    if invalid_scan_is_active(&state) {
        return Err("失效歌曲正在扫描，暂时不能清除索引".to_string());
    }
    let path = library_catalog_path(&state);
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    for candidate in [
        path.clone(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("清除歌曲库缓存失败：{error}")),
        }
    }
    Ok(())
}

fn find_runtime_session_dir(root: &Path, batch_id: &str) -> Option<PathBuf> {
    let mut matches = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let session_path = path.join("session.json");
            let contents = fs::read_to_string(&session_path).ok()?;
            let session = serde_json::from_str::<serde_json::Value>(&contents).ok()?;
            (session.get("batch_id").and_then(serde_json::Value::as_str) == Some(batch_id))
                .then(|| {
                    let modified = fs::metadata(&session_path)
                        .and_then(|metadata| metadata.modified())
                        .unwrap_or(UNIX_EPOCH);
                    (modified, path)
                })
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(modified, _)| *modified);
    matches.pop().map(|(_, path)| path)
}

/// Resolve a history entry to the exact runtime session that was created for
/// it. New entries carry this path explicitly; the batch-id scan remains only
/// as a backwards-compatible fallback for history written before the field
/// existed. Paths are accepted only below the configured runtime-session root.
fn resolve_runtime_session_dir(root: &Path, entry: &HistoryEntry) -> Option<PathBuf> {
    if let Some(saved) = entry.runtime_session_dir.as_deref() {
        let candidate = PathBuf::from(saved);
        if candidate.starts_with(root)
            && candidate.is_dir()
            && fs::read_to_string(candidate.join("session.json"))
                .ok()
                .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
                .and_then(|session| session.get("batch_id").and_then(serde_json::Value::as_str).map(|batch| batch == entry.batch_id))
                .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    find_runtime_session_dir(root, &entry.batch_id)
}

fn runtime_event_string(details: &serde_json::Value, key: &str) -> Option<String> {
    details.get(key).and_then(serde_json::Value::as_str).map(str::to_owned)
}

fn runtime_session_analysis_summary(session_dir: Option<&Path>) -> Option<AnalysisSessionSummary> {
    let session_dir = session_dir?;
    let candidates = fs::read_to_string(session_dir.join("candidates.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());
    let total_from_candidates = candidates
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .map(|slots| {
            slots
                .iter()
                .filter_map(|slot| slot.get("preview"))
                .filter_map(|preview| preview.get("candidates"))
                .filter_map(serde_json::Value::as_array)
                .map(Vec::len)
                .sum::<usize>()
        })
        .unwrap_or(0);
    let events = fs::read_to_string(session_dir.join("events.jsonl"))
        .ok()
        .map(|contents| {
            contents
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let analysis_reports = fs::read_to_string(session_dir.join("analysis-reports.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let analysis_state = fs::read_to_string(session_dir.join("analysis-state.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());
    if events.is_empty()
        && total_from_candidates == 0
        && analysis_reports.is_empty()
        && analysis_state.is_none()
    {
        return None;
    }

    let mut statuses = std::collections::HashMap::<String, String>::new();
    let mut names = std::collections::HashMap::<String, String>::new();
    let mut current_item = None;
    let mut current_stage = None;
    let mut worker_job_id = None;
    let mut requested_at = None;
    let mut started_at = None;
    let mut finished_at = None;
    let mut termination_reason = None;
    let mut requested = !analysis_reports.is_empty();
    let mut terminal_status = None;

    let mut stale_running_state = false;
    if let Some(state) = analysis_state.as_ref() {
        let state_status = state
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("notRequested");
        requested |= state_status != "notRequested";
        if let Some(tracks) = state.get("tracks").and_then(serde_json::Value::as_object) {
            for (path, track) in tracks {
                statuses.insert(
                    path.clone(),
                    track
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("pending")
                        .to_string(),
                );
                if let Some(name) = track.get("name").and_then(serde_json::Value::as_str) {
                    names.insert(path.clone(), name.to_string());
                }
            }
        }
        current_item = state
            .get("currentItem")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        current_stage = state
            .get("currentStage")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        worker_job_id = state
            .get("workerJobId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        requested_at = state
            .get("requestedAt")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        started_at = state
            .get("startedAt")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        finished_at = state
            .get("finishedAt")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        termination_reason = state
            .get("terminationReason")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        terminal_status = match state_status {
            "cancelled" | "error" | "completed" | "partial" | "interrupted" => Some(state_status.to_string()),
            _ => None,
        };
        let heartbeat = state
            .get("lastHeartbeatEpochMs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if state_status == "running"
            && heartbeat > 0
            && unix_timestamp_ms().saturating_sub(heartbeat) > 15_000
        {
            stale_running_state = true;
            terminal_status = Some(String::from("interrupted"));
            for status in statuses.values_mut() {
                if status == "running" {
                    *status = String::from("interrupted");
                }
            }
        }
    }

    for report in &analysis_reports {
        let Some(path) = report.get("source_path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        statuses.insert(
            path.to_string(),
            report
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("failed")
                .to_string(),
        );
    }

    for event in &events {
        let event_name = event.get("event").and_then(serde_json::Value::as_str).unwrap_or_default();
        let at = event.get("at").and_then(serde_json::Value::as_str).map(str::to_owned);
        let details = event.get("details").unwrap_or(&serde_json::Value::Null);
        match event_name {
            "analysis_requested" => {
                requested = true;
                requested_at = at;
            }
            "analysis_started" => {
                requested = true;
                if requested_at.is_none() {
                    requested_at = at.clone();
                }
                started_at = at;
            }
            "analysis_candidate_started" => {
                if let Some(path) = runtime_event_string(details, "source_path") {
                    statuses.insert(path.clone(), String::from("running"));
                    if let Some(name) = runtime_event_string(details, "name") {
                        names.insert(path.clone(), name);
                    }
                    current_item = names.get(&path).cloned();
                }
                worker_job_id = runtime_event_string(details, "worker_job_id").or(worker_job_id);
                current_stage = Some(String::from("preparing"));
            }
            "analysis_candidate_progress" => {
                if let Some(path) = runtime_event_string(details, "source_path") {
                    statuses.entry(path.clone()).or_insert_with(|| String::from("running"));
                    current_item = runtime_event_string(details, "name").or_else(|| names.get(&path).cloned());
                }
                current_stage = runtime_event_string(details, "stage").or(current_stage);
                worker_job_id = runtime_event_string(details, "worker_job_id").or(worker_job_id);
            }
            "analysis_candidate_finished" => {
                if let Some(path) = runtime_event_string(details, "source_path") {
                    statuses.insert(
                        path,
                        runtime_event_string(details, "status").unwrap_or_else(|| String::from("failed")),
                    );
                }
                current_stage = runtime_event_string(details, "stage").or(current_stage);
                worker_job_id = runtime_event_string(details, "worker_job_id").or(worker_job_id);
            }
            "analysis_candidate_persisted" => {
                if let Some(path) = runtime_event_string(details, "source_path") {
                    statuses.entry(path).or_insert_with(|| String::from("completed"));
                }
            }
            "analysis_cancelled" => {
                terminal_status = Some(String::from("cancelled"));
                termination_reason = runtime_event_string(details, "reason")
                    .or_else(|| runtime_event_string(details, "message"));
                finished_at = at;
            }
            "analysis_error" => {
                terminal_status = Some(String::from("error"));
                termination_reason = runtime_event_string(details, "message");
                finished_at = at;
            }
            "analysis_completed" => {
                terminal_status = Some(String::from("completed"));
                finished_at = at;
            }
            _ => {}
        }
    }
    // Events are append-only and can predate the final durable heartbeat
    // snapshot. Replaying them above is useful for names/stages, but a stale
    // heartbeat must still win over old running events for per-track status.
    if stale_running_state {
        for status in statuses.values_mut() {
            if status == "running" {
                *status = String::from("interrupted");
            }
        }
        terminal_status = Some(String::from("interrupted"));
    }
    if !requested {
        return Some(AnalysisSessionSummary {
            status: String::from("notRequested"),
            total: total_from_candidates,
            ..AnalysisSessionSummary::default()
        });
    }
    let total = total_from_candidates.max(statuses.len());
    let completed = statuses.values().filter(|status| status.as_str() == "completed").count();
    let timed_out = statuses.values().filter(|status| status.as_str() == "timeout").count();
    let failed = statuses.values().filter(|status| status.as_str() == "failed").count();
    let pending = total.saturating_sub(completed + timed_out + failed);
    let status = match terminal_status.as_deref() {
        Some("cancelled") => String::from("cancelled"),
        Some("interrupted") => String::from("interrupted"),
        Some("error") => String::from("error"),
        Some("completed") if failed > 0 || timed_out > 0 || pending > 0 => {
            String::from("partial")
        }
        Some(value) => value.to_string(),
        None if pending > 0 => String::from("running"),
        None if failed > 0 || timed_out > 0 => String::from("partial"),
        None => String::from("completed"),
    };
    Some(AnalysisSessionSummary {
        status,
        total,
        completed,
        failed,
        timed_out,
        pending,
        current_item,
        current_stage,
        worker_job_id,
        requested_at,
        started_at,
        finished_at,
        termination_reason,
    })
}

fn read_runtime_session_artifacts(session_dir: &Path) -> serde_json::Value {
    let mut artifacts = serde_json::Map::new();
    let Ok(entries) = fs::read_dir(session_dir) else {
        return serde_json::json!({
            "available": false,
            "error": "运行会话目录不存在或无法读取",
        });
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let known_artifact = matches!(
            name,
            "session.json"
                | "candidates.json"
                | "events.jsonl"
                | "analysis-reports.json"
                | "analysis-state.json"
                | "README.md"
        ) || name.starts_with("summary-slot-");
        if !known_artifact {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            artifacts.insert(
                name.to_string(),
                serde_json::json!({"error": "文件无法读取"}),
            );
            continue;
        };
        let value = if name.ends_with(".json") {
            serde_json::from_str(&contents)
                .unwrap_or_else(|_| serde_json::json!({"raw": contents}))
        } else if name.ends_with(".jsonl") {
            let events = contents
                .lines()
                .map(|line| {
                    serde_json::from_str::<serde_json::Value>(line).unwrap_or_else(|error| {
                        serde_json::json!({
                            "raw": line,
                            "parseError": error.to_string(),
                        })
                    })
                })
                .collect::<Vec<_>>();
            serde_json::Value::Array(events)
        } else {
            serde_json::Value::String(contents)
        };
        artifacts.insert(name.to_string(), value);
    }

    serde_json::json!({
        "available": true,
        "directory": session_dir.display().to_string(),
        "files": artifacts,
    })
}

fn build_runtime_session_export(
    entry: &HistoryEntry,
    session_dir: Option<&Path>,
) -> serde_json::Value {
    let runtime_session = session_dir
        .map(read_runtime_session_artifacts)
        .unwrap_or_else(|| {
            serde_json::json!({
                "available": false,
                "error": "找不到该任务的运行会话记录（可能是旧版本任务）",
            })
        });
    let readable_summary = format_error_report_with_runtime(
        entry,
        Some(&serde_json::json!({"runtimeSession": runtime_session.clone()})),
    );
    serde_json::json!({
        "schemaVersion": 1,
        "exportedAt": timestamp_string(),
        "appVersion": env!("CARGO_PKG_VERSION"),
        "privacy": "包含本地路径、文件元数据、运行日志和分析状态；仅在确认后分享。",
        "history": entry,
        "readableSummary": readable_summary,
        "runtimeSession": runtime_session,
    })
}

/// Return the in-memory monitor for a batch, reopening its durable session
/// after an application restart when necessary. Keeping the recovered monitor
/// in the map also serializes concurrent event appends and state snapshots.
fn runtime_monitor_for_batch(
    state: &AppState,
    batch_id: &str,
) -> Option<Arc<TestMonitor>> {
    if !RUNTIME_SESSION_RECORDING_ENABLED || batch_id.trim().is_empty() {
        return None;
    }
    let mut monitors = state
        .test_monitors
        .lock()
        .expect("runtime session map lock poisoned");
    if let Some(monitor) = monitors.get(batch_id).cloned() {
        return Some(monitor);
    }
    let root = state
        .test_monitor_path
        .lock()
        .expect("runtime session path lock poisoned")
        .clone();
    let monitor = find_runtime_session_dir(&root, batch_id)
        .and_then(|directory| TestMonitor::from_existing(directory).ok())
        .map(Arc::new);
    if let Some(monitor) = monitor.as_ref() {
        monitors.insert(batch_id.to_string(), Arc::clone(monitor));
    }
    monitor
}

#[tauri::command]
fn record_runtime_session_event(
    batch_id: String,
    event: String,
    details: serde_json::Value,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if batch_id.trim().is_empty() || event.trim().is_empty() {
        return Ok(());
    }
    let monitor = runtime_monitor_for_batch(&state, &batch_id);
    if let Some(monitor) = monitor {
        monitor.record_event(&event, details);
    }
    Ok(())
}

#[tauri::command]
fn claim_analysis_run(
    batch_id: String,
    attempt_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let monitor = runtime_monitor_for_batch(&state, &batch_id);
    match monitor {
        Some(monitor) => monitor.claim_analysis_run(&attempt_id).map(|()| true),
        // Library-only analysis has no conversion monitor. It still uses the
        // same per-song Worker lifecycle, but there is no shared run to claim.
        None => Ok(true),
    }
}

#[tauri::command]
fn finalize_analysis_session(
    batch_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if batch_id.trim().is_empty() {
        return Ok(());
    }
    state
        .test_monitors
        .lock()
        .expect("runtime session map lock poisoned")
        .remove(&batch_id);
    Ok(())
}

#[tauri::command]
fn export_runtime_session(
    id: String,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(String::from("请指定运行会话记录保存位置"));
    }

    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let entries = {
        let _history_guard = state
            .history_write_lock
            .lock()
            .expect("history write lock poisoned");
        load_history_file(&history_path)
            .map_err(|error| format!("无法读取转换历史：{error}"))?
    };
    let entry = entries
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| String::from("找不到对应的转换历史"))?;
    let monitor_root = state
        .test_monitor_path
        .lock()
        .expect("runtime session path lock poisoned")
        .clone();
    let session_dir = resolve_runtime_session_dir(&monitor_root, &entry);
    let payload = build_runtime_session_export(&entry, session_dir.as_deref());
    let contents = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("运行会话记录序列化失败：{error}"))?;
    fs::write(path, contents).map_err(|error| format!("运行会话记录保存失败：{error}"))
}

#[tauri::command]
fn export_history_error_report(
    id: String,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(String::from("请指定错误报告保存位置"));
    }

    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let _history_guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    let monitor_root = state
        .test_monitor_path
        .lock()
        .expect("runtime session path lock poisoned")
        .clone();
    let entry = load_history_file(&history_path)
        .map_err(|error| format!("无法读取转换历史：{error}"))?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| String::from("找不到对应的转换历史"))?;
    let session_dir = resolve_runtime_session_dir(&monitor_root, &entry);
    export_history_error_report_to_path_with_runtime(
        &history_path,
        &id,
        Path::new(&path),
        session_dir.as_deref(),
    )
}

#[allow(dead_code)]
fn export_history_error_report_to_path(
    history_path: &Path,
    id: &str,
    output_path: &Path,
) -> Result<(), String> {
    export_history_error_report_to_path_with_runtime(history_path, id, output_path, None)
}

fn export_history_error_report_to_path_with_runtime(
    history_path: &Path,
    id: &str,
    output_path: &Path,
    session_dir: Option<&Path>,
) -> Result<(), String> {
    let mut entries = load_history_file(history_path)
        .map_err(|error| format!("无法读取转换历史：{error}"))?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| String::from("找不到对应的转换历史"))?;
    entry.report_path = Some(output_path.display().to_string());
    let runtime = session_dir.map(|path| {
        serde_json::json!({
            "runtimeSession": read_runtime_session_artifacts(path),
        })
    });
    let report = format_error_report_with_runtime(entry, runtime.as_ref());

    fs::write(output_path, report).map_err(|error| format!("错误报告保存失败：{error}"))?;
    upsert_history(history_path, entry.clone())
        .map_err(|error| format!("错误报告已保存，但历史记录更新失败：{error}"))
}

const HEADLESS_ACCEPTANCE_SCENARIOS: &[&str] = &[
    "libraryAnalysis",
    "neteaseMetadata",
    "flacCoverRecovery",
    "energyDashboard",
    "emotionManifest",
    "externalFormats",
    "bundleSmoke",
];

fn parse_headless_acceptance_args(args: &[String]) -> Result<Option<HeadlessAcceptanceConfig>, String> {
    let mut scenario = None;
    let mut report_path = None;
    let mut exercise_cancel_resume = false;
    let mut input_path = None;
    let mut output_path = None;
    let mut database_path = None;
    let mut index = 0;

    while index < args.len() {
        let argument = args[index].as_str();
        let next_value = |index: &mut usize, flag: &str| -> Result<String, String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .filter(|value| !value.starts_with('-'))
                .ok_or_else(|| format!("{flag} 缺少参数"))
        };
        match argument {
            "--headless-acceptance" => scenario = Some(next_value(&mut index, argument)?),
            "--acceptance-report" => report_path = Some(next_value(&mut index, argument)?),
            "--exercise-cancel-resume" => exercise_cancel_resume = true,
            "--input" => input_path = Some(next_value(&mut index, argument)?),
            "--output" => output_path = Some(next_value(&mut index, argument)?),
            "--database" => database_path = Some(next_value(&mut index, argument)?),
            value if value.starts_with("--") => {
                return Err(format!("未知验收参数：{value}"));
            }
            _ => {}
        }
        index += 1;
    }

    let Some(scenario) = scenario else {
        return Ok(None);
    };
    if !HEADLESS_ACCEPTANCE_SCENARIOS.contains(&scenario.as_str()) {
        return Err(format!("未知验收场景：{scenario}"));
    }
    let report_path = report_path.ok_or_else(|| String::from("--acceptance-report 必须提供绝对路径"))?;
    if !Path::new(&report_path).is_absolute() {
        return Err(String::from("--acceptance-report 必须是绝对路径"));
    }
    for (flag, value) in [
        ("--input", input_path.as_deref()),
        ("--output", output_path.as_deref()),
        ("--database", database_path.as_deref()),
    ] {
        if let Some(value) = value
            && !Path::new(value).is_absolute()
        {
            return Err(format!("{flag} 必须是绝对路径"));
        }
    }

    Ok(Some(HeadlessAcceptanceConfig {
        scenario,
        exercise_cancel_resume,
        input_path,
        output_path,
        database_path,
        report_path,
    }))
}

#[tauri::command]
fn load_headless_acceptance_config(
    state: tauri::State<'_, AppState>,
) -> Result<HeadlessAcceptanceConfig, String> {
    state
        .headless_config
        .clone()
        .ok_or_else(|| String::from("当前不是隐藏验收运行时"))
}

#[tauri::command]
fn write_headless_acceptance_event(
    report_path: String,
    event: serde_json::Value,
) -> Result<(), String> {
    let path = PathBuf::from(report_path.trim());
    if !path.is_absolute() {
        return Err(String::from("验收报告路径必须是绝对路径"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建验收报告目录失败：{error}"))?;
    }
    let line = serde_json::to_string(&event).map_err(|error| format!("序列化验收事件失败：{error}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("打开验收报告失败：{error}"))?;
    writeln!(file, "{line}").map_err(|error| format!("写入验收报告失败：{error}"))
}

#[tauri::command]
fn finish_headless_acceptance(
    _app: tauri::AppHandle,
    code: i32,
) -> Result<(), String> {
    if !matches!(code, 0 | 2 | 3 | 4) {
        return Err(String::from("无效的验收退出码"));
    }
    // `AppHandle::exit` requests a Tauri run-loop exit, but the generated
    // desktop entry point would otherwise return success after the loop
    // stops.  The explicit delayed exit preserves the documented CLI exit
    // codes while allowing the IPC response and final JSONL write to flush.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        std::process::exit(code);
    });
    Ok(())
}

fn main() {
    let command_line_args = std::env::args().skip(1).collect::<Vec<_>>();
    let headless_config = match parse_headless_acceptance_args(&command_line_args) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Headless acceptance argument error: {error}");
            std::process::exit(4);
        }
    };
    let headless_mode = headless_config.is_some();
    let Some(_instance_lock) = acquire_single_instance_lock()
        .unwrap_or_else(|error| panic!("failed to acquire single-instance lock: {}", error))
    else {
        return;
    };

    let controller =
        DesktopController::new(DesktopState::from_preferences(AppPreferences::default()));

    tauri::Builder::default()
        .manage(AppState {
            controller: Arc::new(Mutex::new(controller)),
            preferences_path: Arc::new(Mutex::new(PathBuf::new())),
            history_path: Arc::new(Mutex::new(PathBuf::new())),
            models_path: Arc::new(Mutex::new(PathBuf::new())),
            bundled_models_path: Arc::new(Mutex::new(PathBuf::new())),
            scan_cache_path: Arc::new(Mutex::new(PathBuf::new())),
            library: Arc::new(LibraryState::new()),
            history_write_lock: Arc::new(Mutex::new(())),
            models_write_lock: Arc::new(Mutex::new(())),
            destination_coordinator: DestinationCoordinator::default(),
            scan_progress: Arc::new(Mutex::new(ScanProgress::default())),
            scan_cancel: Arc::new(AtomicBool::new(false)),
            scan_result: Arc::new(Mutex::new(None)),
            test_monitor_path: Arc::new(Mutex::new(PathBuf::new())),
            test_monitors: Arc::new(Mutex::new(HashMap::new())),
            concurrency_budget: Arc::new(Mutex::new(Arc::new(
                GlobalConcurrencyBudget::new(w4dj::concurrency::DEFAULT_CONCURRENCY_LIMIT),
            ))),
            ffmpeg_registry: Arc::new(w4dj::sync::ActiveFfmpegRegistry::new()),
            headless_config: headless_config.clone(),
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_desktop_state,
            pick_source_path,
            select_source_directory,
            select_destination_directory,
            choose_mode,
            choose_lossless_format,
            choose_conversion_mode,
            choose_enhanced_mode,
            choose_conflict_strategy,
            choose_filename_rule,
            choose_netease_filename_format,
            choose_concurrency_limit,
            start_sync,
            pause_sync,
            cancel_sync,
            start_all_sync,
            pause_all_sync,
            cancel_all_sync,
            preview_all_sync,
            load_scan_state,
            load_scan_result,
            start_scan,
            cancel_scan,
            clear_scan_cache,
            start_confirmed_sync,
            apply_track_analysis_results,
            load_history,
            load_incomplete_analysis_run,
            retry_history_failures,
            record_runtime_session_event,
            claim_analysis_run,
            finalize_analysis_session,
            export_runtime_session,
            export_history_error_report,
            delete_history_entry_command,
            clear_history_command,
            app_info,
            check_for_updates,
            open_external_url,
            open_destination,
            open_source,
            list_audio_files,
            read_audio_file,
            read_audio_metadata,
            get_audio_file_fingerprint,
            load_track_analyses,
            save_track_analyses,
            clear_track_analyses,
            get_essentia_model_status,
            ensure_essentia_models,
            restore_bundled_essentia_models,
            import_essentia_models,
            load_essentia_model,
            export_rekordbox_xml,
            load_library_status,
            locate_netease_library,
            refresh_library_catalog,
            cancel_library_refresh,
            select_netease_database_fallback,
            clear_netease_database_fallback,
            load_netease_metadata_database_status,
            select_netease_metadata_database,
            clear_netease_metadata_database,
            prepare_netease_metadata_cache,
            cancel_netease_metadata_cache,
            load_netease_metadata_cache_status,
            query_library_catalog,
            get_library_track_detail,
            relocate_library_track,
            remove_library_track,
            clear_invalid_library_tracks,
            find_invalid_library_tracks,
            cancel_invalid_library_scan,
            export_emotion_evaluation_manifest,
            get_library_track_source_records,
            get_library_track_cover,
            list_library_analysis_candidates,
            clear_library_catalog_cache,
            import_w4dj_playlist,
            list_imported_dj_playlists,
            load_imported_dj_playlist,
            export_imported_dj_playlist_w4dj,
            export_netease_playlist_text,
            match_imported_dj_playlist,
            load_imported_dj_playlist_matches,
            set_imported_dj_playlist_match,
            clear_imported_dj_playlist_match,
            export_imported_dj_playlist_m3u8,
            load_headless_acceptance_config,
            write_headless_acceptance_event,
            finish_headless_acceptance
        ])
        .setup(move |app| {
            let headless_url = if headless_mode {
                WebviewUrl::App("headless.html".into())
            } else {
                WebviewUrl::App("index.html".into())
            };
            let mut window_builder = WebviewWindowBuilder::new(app, "main", headless_url)
                .title("W4DJ RKB")
                .inner_size(1120.0, 760.0)
                .min_inner_size(760.0, 560.0)
                .resizable(true)
                .visible(!headless_mode);
            if headless_mode {
                // macOS otherwise suspends hidden WebViews after they are
                // detached from the foreground.  That would pause Web
                // Audio, WASM and Worker execution while the CLI run is
                // still alive, producing a false analysis stall.
                window_builder = window_builder
                    .background_throttling(BackgroundThrottlingPolicy::Disabled);
            }
            let window = window_builder.build()?;
            let preferences_path = app
                .path()
                .app_config_dir()
                .expect("failed to resolve app config directory")
                .join("preferences.json");
            let history_path = preferences_path
                .parent()
                .expect("preferences path should have a parent")
                .join("history.json");
            let models_path = preferences_path
                .parent()
                .expect("preferences path should have a parent")
                .join("essentia-models");
            let bundled_models_path = app
                .path()
                .resource_dir()
                .expect("failed to resolve bundled resource directory")
                .join("essentia-models");
            let scan_cache_path = preferences_path
                .parent()
                .expect("preferences path should have a parent")
                .join("scan-cache.json");
            let w4dj_library_path = preferences_path
                .parent()
                .expect("preferences path should have a parent")
                .join("w4dj.sqlite3");
            let startup_library_path = w4dj_library_path.clone();
            let test_monitor_path = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| default_download_directory())
                .join(RUNTIME_SESSION_DIRECTORY);

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .preferences_path
                    .lock()
                    .expect("preferences path lock poisoned");
                *path_guard = preferences_path.clone();
            }

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .history_path
                    .lock()
                    .expect("history path lock poisoned");
                *path_guard = history_path.clone();
            }

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .models_path
                    .lock()
                    .expect("models path lock poisoned");
                *path_guard = models_path.clone();
            }

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .bundled_models_path
                    .lock()
                    .expect("bundled models path lock poisoned");
                *path_guard = bundled_models_path.clone();
            }

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .scan_cache_path
                    .lock()
                    .expect("scan cache path lock poisoned");
                *path_guard = scan_cache_path;
            }

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .library
                    .catalog_path
                    .lock()
                    .expect("library catalog path lock poisoned");
                *path_guard = w4dj_library_path;
            }

            // Opening the private SQLite catalog is the only library work
            // performed during setup.  Historical imports, output traversal,
            // file probes and NetEase reads all wait for an explicit command
            // or a successful conversion callback.
            if let Err(error) = W4djLibrary::open_or_recover(&startup_library_path) {
                eprintln!("Failed to open W4DJ output library at startup: {error}");
            }

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .test_monitor_path
                    .lock()
                    .expect("test monitor path lock poisoned");
                *path_guard = test_monitor_path;
            }

            {
                let mut preferences = load_preferences(&preferences_path)
                    .unwrap_or_else(|_| AppPreferences::default());
                preferences.enhanced_mode = ENHANCED_ANALYSIS_DEFAULT_ENABLED;
                let state = app.state::<AppState>();
                *state
                    .library
                    .manual_database_path
                    .lock()
                    .expect("manual database path lock poisoned") = preferences
                    .netease_database_path
                    .clone()
                    .map(PathBuf::from);
                let mut controller = state.controller.lock().expect("desktop lock poisoned");
                controller.apply_preferences(preferences);
                let limit = controller.state().concurrency_limit as usize;
                *state
                    .concurrency_budget
                    .lock()
                    .expect("concurrency budget lock poisoned") =
                    Arc::new(GlobalConcurrencyBudget::new(limit));
            }

            if !headless_mode {
                #[cfg(target_os = "macos")]
                {
                    apply_vibrancy(
                        &window,
                        NSVisualEffectMaterial::HudWindow,
                        Some(NSVisualEffectState::Active),
                        Some(18.0),
                    )
                    .expect("failed to apply macOS vibrancy");
                }
                window.center().expect("failed to center main window");
                window.show().expect("failed to show main window");
                window.set_focus().expect("failed to focus main window");
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run W4DJ desktop shell");
}

fn persist_preferences(state: &tauri::State<'_, AppState>) {
    if let Err(error) = persist_preferences_checked(state.inner()) {
        eprintln!("Failed to save preferences: {error}");
    }
}

fn apply_analysis_for_candidate_to_path(
    candidate: &PreviewCandidate,
    output_path: &Path,
    analyses: &HashMap<String, EmbeddedAnalysis>,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<()> {
    let Some(analysis) = analyses.get(&candidate.source_path) else {
        return Ok(());
    };
    apply_track_analysis_metadata_with_context(output_path, analysis, metadata_context)?;
    validate_track_analysis_metadata(output_path, analysis)
}

fn register_committed_output(
    library: &mut Option<W4djLibrary>,
    slot_index: usize,
    _destination_root: &Path,
    candidate: &PreviewCandidate,
    _analyses: &HashMap<String, EmbeddedAnalysis>,
) -> Option<String> {
    let Some(library) = library else {
        return None;
    };
    let title = candidate.netease_title.as_deref().unwrap_or_default();
    let artist = candidate.netease_artist.as_deref().unwrap_or_default();
    if let Err(error) = library.upsert_lightweight_output(
        slot_index,
        Some(Path::new(&candidate.source_path)),
        Path::new(&candidate.destination_path),
        candidate.netease_track_id.as_deref(),
        candidate.netease_album_id.as_deref(),
        title,
        artist,
    ) {
        eprintln!(
            "W4DJ output registration warning for {}: {}",
            candidate.destination_path, error
        );
        return Some(format!(
            "歌曲库轻量登记警告：{}（音频已保留，可稍后重试登记）",
            error
        ));
    }
    None
}

/// Replaced-output cleanup is only valid for a rename inside the active
/// destination root. A successful conversion to a different root updates
/// the lightweight index but must never inspect or remove the old root.
fn replacement_is_inside_destination_root(previous_path: &Path, destination_root: &Path) -> bool {
    previous_path.starts_with(destination_root)
}

fn embedded_analysis_from_track(analysis: &TrackAnalysis) -> EmbeddedAnalysis {
    EmbeddedAnalysis {
        path: analysis.path.clone(),
        title: analysis.title.clone(),
        artist: analysis.artist.clone(),
        album: analysis.album.clone(),
        genre: analysis.genre.clone(),
        bpm: analysis.bpm,
        key: analysis.key.clone(),
        scale: analysis.scale.clone(),
        key_strength: analysis.key_strength,
        integrated_loudness_lufs: analysis.integrated_loudness_lufs,
        loudness_range_lu: analysis.loudness_range_lu,
        energy: analysis.energy,
        danceability: analysis.danceability,
        beat_positions: analysis.beat_positions.clone(),
        analyzer: analysis.analyzer.clone(),
        analysis_version: analysis.analysis_version.clone(),
        drop_analysis: analysis.drop_analysis.clone(),
        drop_loudness_lufs: analysis.drop_loudness_lufs,
        high_level: analysis.high_level.clone(),
    }
}

fn run_confirmed_sync_task(
    controller: Arc<Mutex<DesktopController>>,
    destination_coordinator: DestinationCoordinator,
    history_path: PathBuf,
    history_write_lock: Arc<Mutex<()>>,
    job: ConfirmedSyncJob,
    concurrency_budget: Arc<GlobalConcurrencyBudget>,
    ffmpeg_registry: Arc<w4dj::sync::ActiveFfmpegRegistry>,
) {
    let started_at = timestamp_string();
    let started = Instant::now();
    let history_id = format!("{}-slot{}", job.batch_id, job.slot_index + 1);
    if let Some(monitor) = job.test_monitor.as_ref() {
        monitor.record_task_started(
            job.slot_index,
            &job.source,
            &job.destination,
            job.candidates.len(),
        );
        let resolver = job.metadata_context.netease.as_ref();
        monitor.record_event(
            "netease_resolver",
            serde_json::json!({
                "database_path": resolver.database_path().map(|path| path.display().to_string()),
                "database_loaded": resolver.database_loaded(),
                "record_count": resolver.record_count(),
                "warning": resolver.warning(),
            }),
        );
        monitor.record_event(
            "task_candidates",
            serde_json::json!({
                "slot_index": job.slot_index,
                "candidates": job.candidates,
            }),
        );
    }
    let task_controller = {
        let controller_guard = controller.lock().expect("desktop lock poisoned");
        controller_guard
            .task_controller(job.slot_index)
            .expect("confirmed slot index should be valid")
    };
    let (initial_failed_files, initial_logs) = {
        let controller_guard = controller.lock().expect("desktop lock poisoned");
        let slot = &controller_guard.state().slots[job.slot_index];
        (slot.failed_files.clone(), slot.logs.clone())
    };
    let recovery_entry = Arc::new(Mutex::new(HistoryEntry {
        id: history_id,
        batch_id: job.batch_id.clone(),
        slot_index: job.slot_index,
        started_at: started_at.clone(),
        finished_at: started_at.clone(),
        duration_seconds: 0,
        source_directory: job.source.clone(),
        destination_directory: job.destination.clone(),
        mode: job.mode,
        lossless_format: job.lossless_format,
        new_count: job.preview.new_count,
        existing_count: job.preview.existing_count,
        skipped_count: job.preview.skipped_count,
        error_count: initial_failed_files.len(),
        completed_count: 0,
        failed_count: initial_failed_files.len(),
        failed_files: initial_failed_files,
        pending_files: job
            .candidates
            .iter()
            .map(pending_file_from_candidate)
            .collect(),
        metadata_diagnostics: Vec::new(),
        logs: initial_logs,
        status: HistoryStatus::Partial,
        retry_of: job.retry_of.clone(),
        conflict_strategy: job.conflict_strategy,
        filename_rule: job.filename_rule,
        netease_filename_format: job.netease_filename_format,
        report_path: None,
        analysis_reports: Vec::new(),
        runtime_session_dir: job
            .test_monitor
            .as_ref()
            .map(|monitor| monitor.session_dir.display().to_string()),
    }));
    {
        let resolver = job.metadata_context.netease.as_ref();
        let database = resolver
            .database_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| String::from("未加载"));
        let mut entry = recovery_entry.lock().expect("recovery history lock poisoned");
        entry.logs.push(format!(
            "网易云元数据解析器：数据库={}，已加载={}，记录数={}{}",
            database,
            if resolver.database_loaded() { "是" } else { "否" },
            resolver.record_count(),
            resolver
                .warning()
                .map(|warning| format!("，警告={warning}"))
                .unwrap_or_default(),
        ));
    }
    persist_recovery_entry(&history_path, &history_write_lock, &recovery_entry);

    let mut setup_error: Option<String> = None;
    // Keep the conversion outcome separate from the UI terminal state.  The
    // history entry is the durable record of a completed conversion, so it
    // must be written before the slot is exposed as completed/error to the
    // frontend.  Otherwise the frontend can observe the terminal state and
    // reload history during the small window in which the worker has not yet
    // persisted the entry.
    let mut task_result: Result<w4dj::task::TaskSnapshot, String> =
        Ok(task_controller.snapshot());
    if let Err(error) = validate_source_input(&job.source) {
        setup_error = Some(error);
    } else if let Err(error) = fs::create_dir_all(&job.destination) {
        setup_error = Some(format!("无法创建输出目录：{error}"));
    }

    if setup_error.is_none() {
        let destination_lock = destination_coordinator.lock_for(Path::new(&job.destination));
        let cleanup_result = {
            let _destination_guard = destination_lock
                .lock()
                .expect("destination sync lock poisoned");
            cleanup_temporary_outputs(&job.destination)
        };
        let mut w4dj_library = match W4djLibrary::open(&job.w4dj_path) {
            Ok(library) => Some(library),
            Err(error) => {
                eprintln!("W4DJ output library unavailable; conversion will continue: {error}");
                None
            }
        };

        if let Err(error) = cleanup_result {
            setup_error = Some(format!("无法清理临时文件：{error}"));
        }
        if setup_error.is_none() {
            let mut candidate_lookup = HashMap::new();
            let analysis_lookup = job
                .analyses
                .into_iter()
                .map(|analysis| (analysis.path.clone(), analysis))
                .collect::<HashMap<String, EmbeddedAnalysis>>();
            let mut source_files: HashMap<String, (String, PathBuf)> = HashMap::new();

            for candidate in &job.candidates {
                candidate_lookup.insert(candidate.name.clone(), candidate.clone());
                if matches!(candidate.operation, PreviewOperation::UpdateMetadata) {
                    continue;
                }
                let source_path = PathBuf::from(&candidate.source_path);
                if source_path.exists() {
                    source_files.insert(
                        candidate.name.clone(),
                        (candidate.source_size_bytes.to_string(), source_path),
                    );
                } else {
                    let message = "源文件在开始转换前已不存在";
                    record_failed_candidate(
                        &controller,
                        job.slot_index,
                        &task_controller,
                        candidate,
                        message,
                    );
                    mark_recovery_processed(
                        &history_path,
                        &history_write_lock,
                        &recovery_entry,
                        &candidate.name,
                        task_controller.snapshot().completed,
                        Some(FailedFile {
                            name: candidate.name.clone(),
                            source_path: candidate.source_path.clone(),
                            destination_path: candidate.destination_path.clone(),
                            message: message.to_string(),
                            category: classify_error(message),
                        }),
                    );
                    if let Some(monitor) = job.test_monitor.as_ref() {
                        monitor.record_candidate_result(candidate, "failed", Some(message));
                    }
                }
            }

            for candidate in job
                .candidates
                .iter()
                .filter(|candidate| matches!(candidate.operation, PreviewOperation::UpdateMetadata))
            {
                if task_controller.is_cancelled() {
                    break;
                }
                if !task_controller.should_start_next_file() {
                    break;
                }

                let result = update_existing_metadata_transactionally_with_context_and_policy(
                    Path::new(&candidate.source_path),
                    Path::new(&candidate.destination_path),
                    job.netease_filename_format,
                    |temporary_output| {
                        apply_analysis_for_candidate_to_path(
                            candidate,
                            temporary_output,
                            &analysis_lookup,
                            job.metadata_context.as_ref(),
                        )
                    },
                    job.metadata_context.as_ref(),
                    filename_normalization_policy_for_slot(job.slot_index),
                );
                let registration_warning = if result.is_ok()
                    && let Some(candidate) = candidate_lookup.get(&candidate.name)
                {
                    register_committed_output(
                        &mut w4dj_library,
                        job.slot_index,
                        Path::new(&job.destination),
                        candidate,
                        &analysis_lookup,
                    )
                } else {
                    None
                };
                let mut controller_guard = controller.lock().expect("desktop lock poisoned");
                if let Some(warning) = registration_warning {
                    controller_guard
                        .push_log(job.slot_index, warning)
                        .expect("confirmed slot index should be valid");
                }
                let failed_file = match result {
                    Ok(()) => {
                        task_controller.complete_current_file();
                        controller_guard
                            .record_file_result(
                                job.slot_index,
                                &candidate.name,
                                task_controller.snapshot(),
                                None,
                            )
                            .expect("confirmed slot index should be valid");
                        None
                    }
                    Err(error) => {
                        let message = error.to_string();
                        let failed_file = FailedFile {
                            name: candidate.name.clone(),
                            source_path: candidate.source_path.clone(),
                            destination_path: candidate.destination_path.clone(),
                            category: classify_error(&message),
                            message,
                        };
                        controller_guard
                            .record_file_failed(
                                job.slot_index,
                                failed_file.clone(),
                                task_controller.snapshot(),
                            )
                            .expect("confirmed slot index should be valid");
                        Some(failed_file)
                    }
                };
                drop(controller_guard);
                if let Some(monitor) = job.test_monitor.as_ref() {
                    monitor.record_candidate_result(
                        candidate,
                        if failed_file.is_some() { "failed" } else { "completed" },
                        failed_file.as_ref().map(|file| file.message.as_str()),
                    );
                }
                record_metadata_diagnostic(
                    &recovery_entry,
                    candidate,
                    job.metadata_context.as_ref(),
                );
                mark_recovery_processed(
                    &history_path,
                    &history_write_lock,
                    &recovery_entry,
                    &candidate.name,
                    task_controller.snapshot().completed,
                    failed_file,
                );
            }

            let queued_files = source_files.iter().collect::<HashMap<_, _>>();
            let sync_result = if queued_files.is_empty() {
                Ok(task_controller.snapshot())
            } else {
                let finalize_candidates = Arc::new(candidate_lookup.clone());
                let finalize_analyses = Arc::new(analysis_lookup.clone());
                let finalize_metadata_context = Arc::clone(&job.metadata_context);
                sync_music_library_transactional_with_observer_and_budget_and_context_with_policy(
                    &queued_files,
                    &job.destination,
                    &job.mode,
                    job.lossless_format,
                    job.netease_filename_format,
                    filename_normalization_policy_for_slot(job.slot_index),
                    &task_controller,
                    {
                        let candidates = Arc::clone(&finalize_candidates);
                        let analyses = Arc::clone(&finalize_analyses);
                        let metadata_context = Arc::clone(&finalize_metadata_context);
                        move |name: &str, temporary_output: &Path| {
                        let Some(candidate) = candidates.get(name) else {
                            return Ok(());
                        };
                        apply_analysis_for_candidate_to_path(
                            candidate,
                            temporary_output,
                            &analyses,
                            metadata_context.as_ref(),
                        )
                        }
                    },
                    |name, task, error| {
                        if error.is_some_and(|error| {
                            error.kind() == io::ErrorKind::Interrupted && task.is_cancelled()
                        }) {
                            return;
                        }
                        let candidate = candidate_lookup.get(name);
                        let replacement_warning = if error.is_none()
                            && matches!(job.conflict_strategy, ConflictStrategy::Overwrite)
                        {
                            candidate
                                .and_then(|candidate| candidate.previous_destination_path.as_deref())
                                .and_then(|previous_path| {
                                    if !replacement_is_inside_destination_root(
                                        Path::new(previous_path),
                                        Path::new(&job.destination),
                                    ) {
                                        return None;
                                    }
                                    candidate.and_then(|candidate| {
                                        match remove_replaced_output(
                                            Path::new(previous_path),
                                            Path::new(&candidate.destination_path),
                                            Path::new(&candidate.source_path),
                                        ) {
                                            Ok(true) | Ok(false) => None,
                                            Err(error) => Some(format!(
                                                "覆盖已生成新文件，但旧输出未能删除 {}：{}",
                                                previous_path, error
                                            )),
                                        }
                                    })
                                })
                        } else {
                            None
                        };
                        let registration_warning = if error.is_none() {
                            candidate.and_then(|candidate| {
                                register_committed_output(
                                    &mut w4dj_library,
                                    job.slot_index,
                                    Path::new(&job.destination),
                                    candidate,
                                    &analysis_lookup,
                                )
                            })
                        } else {
                            None
                        };
                        let failed_file = if let Some(error) = error {
                            let failed_file = FailedFile {
                                name: name.to_string(),
                                source_path: candidate
                                    .map(|candidate| candidate.source_path.clone())
                                    .unwrap_or_default(),
                                destination_path: candidate
                                    .map(|candidate| candidate.destination_path.clone())
                                    .unwrap_or_default(),
                                category: classify_error(&error.to_string()),
                                message: error.to_string(),
                            };
                            let mut controller_guard =
                                controller.lock().expect("desktop lock poisoned");
                            controller_guard
                                .record_file_failed(
                                    job.slot_index,
                                    failed_file.clone(),
                                    task.snapshot(),
                                )
                                .expect("confirmed slot index should be valid");
                            Some(failed_file)
                        } else {
                            let mut controller_guard =
                                controller.lock().expect("desktop lock poisoned");
                            if let Some(warning) = replacement_warning {
                                controller_guard
                                    .push_log(job.slot_index, warning)
                                    .expect("confirmed slot index should be valid");
                            }
                            if let Some(warning) = registration_warning {
                                controller_guard
                                    .push_log(job.slot_index, warning)
                                    .expect("confirmed slot index should be valid");
                            }
                            controller_guard
                                .record_file_result(job.slot_index, name, task.snapshot(), None)
                                .expect("confirmed slot index should be valid");
                            None
                        };
                        if let Some(candidate) = candidate {
                            if let Some(monitor) = job.test_monitor.as_ref() {
                                monitor.record_candidate_result(
                                    candidate,
                                    if failed_file.is_some() {
                                        "failed"
                                    } else {
                                        "completed"
                                    },
                                    failed_file.as_ref().map(|file| file.message.as_str()),
                                );
                            }
                            record_metadata_diagnostic(
                                &recovery_entry,
                                candidate,
                                job.metadata_context.as_ref(),
                            );
                        }
                        mark_recovery_processed(
                            &history_path,
                            &history_write_lock,
                            &recovery_entry,
                            name,
                            task.snapshot().completed,
                            failed_file,
                        );
                    },
                    Arc::clone(&concurrency_budget),
                    Arc::clone(&ffmpeg_registry),
                    job.metadata_context.as_ref(),
                )
            };

            task_result = sync_result.map_err(|error| format!("导出失败：{error}"));
        }
    }

    if let Some(error) = setup_error {
        for candidate in &job.candidates {
            if let Some(monitor) = job.test_monitor.as_ref() {
                monitor.record_candidate_result(candidate, "failed", Some(&error));
            }
            record_failed_candidate(
                &controller,
                job.slot_index,
                &task_controller,
                candidate,
                &error,
            );
        }
        task_result = Err(error);
    }

    let finished_at = timestamp_string();
    let (snapshot, slot) = {
        let controller_guard = controller.lock().expect("desktop lock poisoned");
        (
            task_controller.snapshot(),
            controller_guard.state().slots[job.slot_index].clone(),
        )
    };
    let error_count = slot.error_tracks;
    let failed_files = slot.failed_files;
    let status = history_status_for(&snapshot, &failed_files);
    let pending_files = if snapshot.cancelled || snapshot.paused {
        recovery_entry
            .lock()
            .expect("recovery history lock poisoned")
            .pending_files
            .clone()
    } else {
        Vec::new()
    };
    let history_entry = HistoryEntry {
        id: format!("{}-slot{}", job.batch_id, job.slot_index + 1),
        batch_id: job.batch_id,
        slot_index: job.slot_index,
        started_at,
        finished_at,
        duration_seconds: started.elapsed().as_secs(),
        source_directory: job.source,
        destination_directory: job.destination,
        mode: job.mode,
        lossless_format: job.lossless_format,
        new_count: job.preview.new_count,
        existing_count: job.preview.existing_count,
        skipped_count: job.preview.skipped_count,
        error_count,
        completed_count: snapshot.completed,
        failed_count: failed_files.len(),
        failed_files,
        pending_files,
        metadata_diagnostics: recovery_entry
            .lock()
            .expect("recovery history lock poisoned")
            .metadata_diagnostics
            .clone(),
        logs: slot.logs,
        status,
        retry_of: job.retry_of,
        conflict_strategy: job.conflict_strategy,
        filename_rule: job.filename_rule,
        netease_filename_format: job.netease_filename_format,
        report_path: None,
        analysis_reports: Vec::new(),
        runtime_session_dir: job
            .test_monitor
            .as_ref()
            .map(|monitor| monitor.session_dir.display().to_string()),
    };

    if let Some(monitor) = job.test_monitor.as_ref() {
        monitor.record_task_finished(&history_entry);
    }

    if let Err(error) = persist_history_before_terminal_state(
        &history_path,
        &history_write_lock,
        &controller,
        job.slot_index,
        history_entry,
        task_result,
    ) {
        eprintln!("Failed to save conversion history: {error}");
    }
}

/// Persist the final conversion record before publishing the slot's terminal
/// state.  The frontend polls that state and reloads history when it changes;
/// keeping this order makes a completed conversion and its history record one
/// observable transition instead of a race between two independent writes.
fn persist_history_before_terminal_state(
    history_path: &Path,
    history_write_lock: &Arc<Mutex<()>>,
    controller: &Arc<Mutex<DesktopController>>,
    slot_index: usize,
    history_entry: HistoryEntry,
    task_result: Result<w4dj::task::TaskSnapshot, String>,
) -> Result<(), String> {
    let history_result = {
        let _history_guard = history_write_lock
            .lock()
            .map_err(|_| String::from("history write lock poisoned"))?;
        upsert_history(history_path, history_entry)
            .map_err(|error| format!("保存转换历史失败：{error}"))
    };

    let mut controller_guard = controller.lock().expect("desktop lock poisoned");
    if let Err(error) = &history_result {
        controller_guard
            .push_log(
                slot_index,
                format!("转换完成，但保存转换历史失败：{error}"),
            )
            .expect("confirmed slot index should be valid");
    }
    match task_result {
        Ok(snapshot) => controller_guard
            .finish_sync(slot_index, snapshot)
            .expect("confirmed slot index should be valid"),
        Err(error) => controller_guard
            .fail_sync(slot_index, error)
            .expect("confirmed slot index should be valid"),
    }

    history_result
}

fn record_metadata_diagnostic(
    recovery_entry: &Arc<Mutex<HistoryEntry>>,
    candidate: &PreviewCandidate,
    metadata_context: &ConversionMetadataContext,
) {
    let diagnostic = inspect_metadata_diagnostic_with_resolver(
        Path::new(&candidate.source_path),
        Path::new(&candidate.destination_path),
        metadata_context.netease.as_ref(),
    );
    let mut entry = recovery_entry.lock().expect("recovery history lock poisoned");
    if entry
        .metadata_diagnostics
        .iter()
        .all(|existing| existing.source_path != diagnostic.source_path)
    {
        entry.metadata_diagnostics.push(diagnostic);
    }
}

fn pending_file_from_candidate(candidate: &PreviewCandidate) -> PendingFile {
    PendingFile {
        name: candidate.name.clone(),
        source_path: candidate.source_path.clone(),
        destination_path: candidate.destination_path.clone(),
        source_size_bytes: candidate.source_size_bytes,
        estimated_output_bytes: candidate.estimated_output_bytes,
        previous_destination_path: candidate.previous_destination_path.clone(),
        operation: candidate.operation,
    }
}

fn persist_recovery_entry(
    history_path: &Path,
    history_write_lock: &Arc<Mutex<()>>,
    recovery_entry: &Arc<Mutex<HistoryEntry>>,
) {
    let entry = recovery_entry
        .lock()
        .expect("recovery history lock poisoned")
        .clone();
    let _history_guard = history_write_lock
        .lock()
        .expect("history write lock poisoned");
    if let Err(error) = upsert_history(history_path, entry) {
        eprintln!("Failed to save resumable conversion state: {error}");
    }
}

fn mark_recovery_processed(
    history_path: &Path,
    history_write_lock: &Arc<Mutex<()>>,
    recovery_entry: &Arc<Mutex<HistoryEntry>>,
    name: &str,
    completed_count: usize,
    failed_file: Option<FailedFile>,
) {
    {
        let mut entry = recovery_entry
            .lock()
            .expect("recovery history lock poisoned");
        entry
            .pending_files
            .retain(|candidate| candidate.name != name);
        entry.completed_count = completed_count;
        entry.finished_at = timestamp_string();
        match failed_file.as_ref() {
            Some(failed_file) => entry.logs.push(format!(
                "Failed {}: {}",
                failed_file.name, failed_file.message
            )),
            None => entry.logs.push(format!("Processed {name}")),
        }
        if let Some(failed_file) = failed_file
            && !entry
                .failed_files
                .iter()
                .any(|existing| existing.name == failed_file.name)
        {
            entry.failed_files.push(failed_file);
        }
        entry.failed_count = entry.failed_files.len();
        entry.error_count = entry.failed_count;
    }
    persist_recovery_entry(history_path, history_write_lock, recovery_entry);
}

fn record_preflight_issues(
    controller: &mut DesktopController,
    slot_index: usize,
    issues: &[PreviewIssue],
) -> Result<(), String> {
    let task_controller = controller.task_controller(slot_index)?;
    for issue in issues {
        controller.record_file_failed(
            slot_index,
            FailedFile {
                name: Path::new(&issue.path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(&issue.path)
                    .to_string(),
                source_path: issue.path.clone(),
                destination_path: String::new(),
                category: classify_error(&issue.message),
                message: issue.message.clone(),
            },
            task_controller.snapshot(),
        )?;
    }
    Ok(())
}

fn apply_preflight_summary(
    controller: &mut DesktopController,
    slot_index: usize,
    preview: &SyncPreview,
) -> Result<(), String> {
    // Preserve the failed-file details first, then set the authoritative preview counts.
    // Otherwise record_file_failed would add the same preflight errors a second time.
    record_preflight_issues(controller, slot_index, &preview.errors)?;
    controller.set_preflight_summary(
        slot_index,
        preview.new_count,
        preview.existing_count,
        preview.skipped_count,
        preview.error_count,
        preview.estimated_output_bytes,
    )
}

fn record_failed_candidate(
    controller: &Arc<Mutex<DesktopController>>,
    slot_index: usize,
    task_controller: &w4dj::task::TaskController,
    candidate: &PreviewCandidate,
    message: &str,
) {
    let mut controller_guard = controller.lock().expect("desktop lock poisoned");
    let already_recorded = controller_guard.state().slots[slot_index]
        .failed_files
        .iter()
        .any(|failed_file| failed_file.name == candidate.name);
    if already_recorded {
        return;
    }

    controller_guard
        .record_file_failed(
            slot_index,
            FailedFile {
                name: candidate.name.clone(),
                source_path: candidate.source_path.clone(),
                destination_path: candidate.destination_path.clone(),
                category: classify_error(message),
                message: message.to_string(),
            },
            task_controller.snapshot(),
        )
        .expect("confirmed slot index should be valid");
}

fn history_status_for(
    snapshot: &w4dj::task::TaskSnapshot,
    failed_files: &[FailedFile],
) -> HistoryStatus {
    if snapshot.cancelled {
        HistoryStatus::Cancelled
    } else if snapshot.paused || !failed_files.is_empty() && snapshot.completed > 0 {
        HistoryStatus::Partial
    } else if !failed_files.is_empty() {
        HistoryStatus::Error
    } else {
        HistoryStatus::Completed
    }
}

fn unique_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn timestamp_string() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format_unix_timestamp(seconds)
}

fn format_unix_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let hour = seconds_of_day / 3_600;
    let minute = seconds_of_day % 3_600 / 60;
    let second = seconds_of_day % 60;

    // Convert days since the Unix epoch to a Gregorian calendar date.
    let shifted_days = days + 719_468;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

fn run_sync_task(
    controller: Arc<Mutex<DesktopController>>,
    destination_coordinator: DestinationCoordinator,
    w4dj_path: PathBuf,
    slot_index: usize,
    concurrency_budget: Arc<GlobalConcurrencyBudget>,
    ffmpeg_registry: Arc<w4dj::sync::ActiveFfmpegRegistry>,
    metadata_context: Arc<ConversionMetadataContext>,
) {
    let (
        source,
        destination,
        using_fallback,
        mode,
        lossless_format,
        filename_rule,
        netease_filename_format,
        task_controller,
    ) = {
        let controller = controller.lock().expect("desktop lock poisoned");
        let state = controller.state();
        let slot = &state.slots[slot_index];
        let destination = controller
            .effective_destination(slot_index)
            .expect("sync slot index validated before worker start")
            .unwrap_or_default();
        (
            slot.source_directory.clone(),
            destination.clone(),
            slot_index == 1
                && slot.destination_directory.trim().is_empty()
                && !destination.trim().is_empty(),
            state.mode,
            state.lossless_format,
            state.filename_rule,
            state.netease_filename_format,
            controller
                .task_controller(slot_index)
                .expect("sync slot index validated before worker start"),
        )
    };

    if destination.trim().is_empty() {
        fail_sync(&controller, slot_index, "请选择输出目录");
        return;
    }

    if let Err(error) = validate_source_input(&source) {
        fail_sync(&controller, slot_index, error);
        return;
    }

    if let Err(error) = fs::create_dir_all(&destination) {
        fail_sync(
            &controller,
            slot_index,
            format!("无法创建输出目录：{}", error),
        );
        return;
    }

    let destination_lock = destination_coordinator.lock_for(Path::new(&destination));
    let cleanup_result = {
        let _destination_guard = destination_lock
            .lock()
            .expect("destination sync lock poisoned");
        cleanup_temporary_outputs(&destination)
    };
    if let Err(error) = cleanup_result {
        fail_sync(
            &controller,
            slot_index,
            format!("无法清理临时文件：{}", error),
        );
        return;
    }

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        if using_fallback {
            controller
                .push_log(
                    slot_index,
                    format!("Using output directory 1 fallback: {}", destination),
                )
                .expect("sync slot index validated before worker start");
        }
        controller
            .push_log(slot_index, format!("Scanning source: {}", source))
            .expect("sync slot index validated before worker start");
    }
    let cancel_signal = task_controller.cancellation_flag();
    let mut source_observer = |_: ScanPhase, path: &Path| {
        if cancel_signal.load(Ordering::SeqCst) {
            return false;
        }
        let mut guard = controller.lock().expect("desktop lock poisoned");
        let _ = guard.record_file_started(
            slot_index,
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
        );
        true
    };
    let (mut source_files, scan_issues, scan_cancelled) =
        get_music_dict_with_scan_issues_with_settings_and_observer_with_budget_and_policy(
            &source,
            w4dj::sync::SUPPORTED_SOURCE_EXTENSIONS,
            filename_rule,
            netease_filename_format,
            filename_normalization_policy_for_slot(slot_index),
            Arc::clone(&concurrency_budget),
            Arc::clone(&cancel_signal),
            ScanPhase::Source,
            &mut source_observer,
        );
    if scan_cancelled || task_controller.is_cancelled() {
        let mut guard = controller.lock().expect("desktop lock poisoned");
        let _ = guard.push_log(slot_index, "扫描已取消");
        let _ = guard.finish_sync(slot_index, task_controller.snapshot());
        return;
    }
    let missing_sources = source_files
        .iter()
        .filter(|(_, (_, path))| !path.exists())
        .map(|(name, (_, path))| (name.clone(), path.display().to_string()))
        .collect::<Vec<(String, String)>>();

    if !missing_sources.is_empty() || !scan_issues.is_empty() {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        for (name, path) in &missing_sources {
            controller
                .push_log(
                    slot_index,
                    format!("Failed to read source before sync: {} ({})", name, path),
                )
                .expect("sync slot index validated before worker start");
        }
        for issue in &scan_issues {
            controller
                .push_log(
                    slot_index,
                    format!(
                        "Failed to scan source before sync: {} ({})",
                        issue.path.display(),
                        issue.message
                    ),
                )
                .expect("sync slot index validated before worker start");
        }
    }

    source_files.retain(|_, (_, path)| path.exists());

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller
            .push_log(slot_index, format!("Scanning destination: {}", destination))
            .expect("sync slot index validated before worker start");
    }
    let mut destination_observer = |_: ScanPhase, path: &Path| {
        if cancel_signal.load(Ordering::SeqCst) {
            return false;
        }
        let mut guard = controller.lock().expect("desktop lock poisoned");
        let _ = guard.record_file_started(
            slot_index,
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
        );
        true
    };
    let (destination_files, _destination_issues, destination_cancelled) =
        get_music_dict_with_scan_issues_with_settings_and_observer_with_budget_and_policy(
            &destination,
            &["mp3", "wav", "aiff"],
            filename_rule,
            netease_filename_format,
            filename_normalization_policy_for_slot(slot_index),
            Arc::clone(&concurrency_budget),
            Arc::clone(&cancel_signal),
            ScanPhase::Destination,
            &mut destination_observer,
        );
    if destination_cancelled || task_controller.is_cancelled() {
        let mut guard = controller.lock().expect("desktop lock poisoned");
        let _ = guard.push_log(slot_index, "扫描已取消");
        let _ = guard.finish_sync(slot_index, task_controller.snapshot());
        return;
    }
    let queued_files =
        compare_music_dicts(&source_files, &destination_files, &mode, lossless_format);
    let existing_files = source_files.len().saturating_sub(queued_files.len());

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller
            .set_progress_total(slot_index, queued_files.len())
            .expect("sync slot index validated before worker start");
        controller
            .set_preflight_summary(
                slot_index,
                queued_files.len(),
                existing_files,
                existing_files,
                missing_sources.len() + scan_issues.len(),
                None,
            )
            .expect("sync slot index validated before worker start");
        controller
            .push_log(
                slot_index,
                format!("Found {} songs to sync", queued_files.len()),
            )
            .expect("sync slot index validated before worker start");

        if queued_files.is_empty() {
            controller
                .finish_sync(slot_index, task_controller.snapshot())
                .expect("sync slot index validated before worker start");
            return;
        }
    }

    let mut failed_files = 0usize;
    let mut w4dj_library = match W4djLibrary::open(&w4dj_path) {
        Ok(library) => Some(library),
        Err(error) => {
            eprintln!("W4DJ output library unavailable; conversion will continue: {error}");
            None
        }
    };
    let result = sync_music_library_transactional_with_observer_and_budget_and_context_with_policy(
        &queued_files,
        &destination,
        &mode,
        lossless_format,
        netease_filename_format,
        filename_normalization_policy_for_slot(slot_index),
        &task_controller,
        |_, _| Ok(()),
        |name, task, error| {
            if error.is_some_and(|error| {
                error.kind() == io::ErrorKind::Interrupted && task.is_cancelled()
            }) {
                return;
            }
            if error.is_some() {
                failed_files += 1;
            }

            if error.is_none()
                && let Some((_, source_path)) = source_files
                    .get(name)
                    .or_else(|| {
                        source_files
                            .iter()
                            .find(|(key, _)| w4dj::sync::source_entry_name(key) == name)
                            .map(|(_, value)| value)
                    })
                && let Ok(destination_path) = planned_output_path_with_policy(
                    &destination,
                    w4dj::sync::source_entry_name(name),
                    source_path,
                    mode,
                    lossless_format,
                    filename_normalization_policy_for_slot(slot_index),
                )
                && let Some(library) = w4dj_library.as_mut()
                && let Err(registration_error) = {
                    let identity = metadata_context.netease.track_identity(source_path);
                    library.upsert_lightweight_output(
                        slot_index,
                        Some(source_path),
                        &destination_path,
                        identity.as_ref().and_then(|value| value.track_id.as_deref()),
                        identity.as_ref().and_then(|value| value.album_id.as_deref()),
                        identity.as_ref().map_or("", |value| value.title.as_str()),
                        identity.as_ref().map_or("", |value| value.artists.as_str()),
                    )
                }
            {
                eprintln!(
                    "W4DJ lightweight output registration warning for {}: {}",
                    destination_path.display(),
                    registration_error
                );
            }

            let mut controller = controller.lock().expect("desktop lock poisoned");
            controller
                .record_file_result(
                    slot_index,
                    name,
                    task.snapshot(),
                    error.map(|err| err.to_string()),
                )
                .expect("sync slot index validated before worker start");
        },
        Arc::clone(&concurrency_budget),
        Arc::clone(&ffmpeg_registry),
        metadata_context.as_ref(),
    );

    if failed_files > 0 {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller
            .push_log(
                slot_index,
                format!("Failed {} file(s) during sync", failed_files),
            )
            .expect("sync slot index validated before worker start");
    }
    let mut controller = controller.lock().expect("desktop lock poisoned");
    match result {
        Ok(snapshot) => controller
            .finish_sync(slot_index, snapshot)
            .expect("sync slot index validated before worker start"),
        Err(error) => controller
            .fail_sync(slot_index, format!("导出失败：{}", error))
            .expect("sync slot index validated before worker start"),
    }
}

fn validate_source_input(source: &str) -> Result<(), String> {
    if source.trim().is_empty() {
        return Err(String::from("请选择歌曲文件夹或单曲"));
    }

    let path = Path::new(source);
    if !path.exists() {
        return Err(format!("输入来源不存在：{source}"));
    }
    if path.is_file() && !is_supported_source_file(path) {
        return Err(String::from(
            "不支持的单曲格式；请选择 MP3、FLAC、NCM、WAV 或 AIFF 文件",
        ));
    }
    if !path.is_dir() && !path.is_file() {
        return Err(String::from("输入来源不是文件夹或音频文件"));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn source_picker_result(confirmed: bool, path: Option<String>) -> Result<Option<String>, String> {
    if !confirmed {
        return Ok(None);
    }

    path.map(Some)
        .ok_or_else(|| String::from("the selected source has no readable path"))
}

#[cfg(target_os = "macos")]
fn selected_source_path_from_open_panel(title: &str) -> Result<Option<String>, String> {
    let marker = MainThreadMarker::new()
        .ok_or_else(|| String::from("source picker must run on the macOS main thread"))?;
    let panel = NSOpenPanel::openPanel(marker);
    let title = NSString::from_str(title);

    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(false);
    panel.setTitle(Some(&title));

    let confirmed = panel.runModal() == NSModalResponseOK;
    let path = if confirmed {
        panel
            .URL()
            .and_then(|url| url.path())
            .map(|path| path.to_string())
    } else {
        None
    };

    source_picker_result(confirmed, path)
}

#[tauri::command]
async fn pick_source_path(window: tauri::Window, title: String) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        window
            .run_on_main_thread(move || {
                let result = selected_source_path_from_open_panel(&title);
                let _ = sender.send(result);
            })
            .map_err(|error| format!("failed to open source picker: {error}"))?;

        receiver
            .recv()
            .map_err(|error| format!("source picker did not return a result: {error}"))?
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, title);
        Err(String::from(
            "unified source picker is only available on macOS",
        ))
    }
}

fn fail_sync(
    controller: &Arc<Mutex<DesktopController>>,
    slot_index: usize,
    message: impl Into<String>,
) {
    let mut controller = controller.lock().expect("desktop lock poisoned");
    controller
        .fail_sync(slot_index, message)
        .expect("sync slot index validated before worker start");
}

#[cfg(test)]
mod tests {
    use super::{parse_headless_acceptance_args, HeadlessAcceptanceConfig};
    use super::essentia_model_import::known_import_model_ids;
    use super::essentia_model_specs;
    use super::LibraryRefreshProgress;
    use super::LibraryRefreshSummary;
    use super::LibraryRefreshStage;
    use super::LibraryRefreshStatus;
    use super::LibraryState;
    use super::InvalidScanProgress;
    use super::ScanProgress;
    use super::ScanProgressPhase;
    use super::ScanStatus;
    use super::ScanTaskProgress;
    use super::NeteaseMetadataDatabaseSource;
    use super::NeteaseMetadataDatabaseStatus;
    use super::NeteaseMetadataCacheProgress;
    use super::TestMonitor;
    use super::DestinationCoordinator;
    use super::CatalogLocalFile;
    use super::apply_preflight_summary;
    use super::collect_processable_previews;
    use super::deduplicate_cross_slot_candidates;
    use super::history_status_for;
    use super::persist_history_before_terminal_state;
    use super::validate_destination_directory;
    use super::validate_scan_previews;
    use super::validate_source_input;
    use super::validate_unique_planned_outputs;
    use super::{
        apply_library_refresh_terminal, request_library_refresh_cancel,
        try_start_library_refresh,
    };
    use std::fs;
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use w4dj::config::Mode;
    use w4dj::desktop::{DesktopController, DesktopState};
    use w4dj::history::{AnalysisReport, FailedFile, HistoryEntry, HistoryStatus};
    use w4dj::m3u8::ResolvedDjPlaylistTrack;
    use w4dj::preferences::{AppPreferences, SyncSlotPreferences};
    use w4dj::preview::{PreviewCandidate, PreviewIssue, SlotPreview, SyncPreview};
    use w4dj::task::TaskController;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn headless_acceptance_requires_absolute_report_path() {
        let args = vec![
            "--headless-acceptance".into(),
            "libraryAnalysis".into(),
            "--acceptance-report".into(),
            "relative.jsonl".into(),
        ];
        assert!(parse_headless_acceptance_args(&args).is_err());
    }

    #[test]
    fn headless_acceptance_parses_known_scenario_and_optional_paths() {
        let args = vec![
            "--headless-acceptance".into(),
            "libraryAnalysis".into(),
            "--exercise-cancel-resume".into(),
            "--acceptance-report".into(),
            "/private/tmp/w4dj-headless.jsonl".into(),
            "--input".into(),
            "/tmp/input".into(),
        ];
        let config = parse_headless_acceptance_args(&args)
            .expect("arguments should parse")
            .expect("headless mode should be enabled");
        assert_eq!(config.scenario, "libraryAnalysis");
        assert!(config.exercise_cancel_resume);
        assert_eq!(config.input_path.as_deref(), Some("/tmp/input"));
        assert_eq!(config.report_path, "/private/tmp/w4dj-headless.jsonl");
    }

    #[test]
    fn normal_arguments_do_not_enable_headless_mode() {
        let args: Vec<String> = Vec::new();
        assert!(parse_headless_acceptance_args(&args)
            .expect("normal arguments should parse")
            .is_none());
    }

    #[test]
    fn opening_startup_catalog_does_not_import_history_or_probe_old_outputs() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "w4dj-startup-test-{}-{suffix}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let stale_output = directory.join("old-output/song.mp3");
        fs::create_dir_all(stale_output.parent().unwrap()).unwrap();
        fs::write(&stale_output, b"legacy output").unwrap();
        // A malformed history file proves the startup opener does not even
        // attempt to parse it.  Historical import remains an explicit,
        // compatibility-only API and is never part of Tauri setup.
        fs::write(directory.join("history.json"), b"not-json").unwrap();
        let database_path = directory.join("w4dj.sqlite3");

        let (library, backup) = super::open_w4dj_library(&database_path)
            .expect("startup catalog should open without historical inputs");
        assert!(backup.is_none());
        assert_eq!(library.stats().unwrap().total, 0);
        assert!(!library.is_initial_import_done().unwrap());
        assert!(stale_output.is_file());
        drop(library);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replaced_output_cleanup_is_scoped_to_the_active_destination_root() {
        assert!(super::replacement_is_inside_destination_root(
            Path::new("/music/B/old.mp3"),
            Path::new("/music/B"),
        ));
        assert!(!super::replacement_is_inside_destination_root(
            Path::new("/music/A/old.mp3"),
            Path::new("/music/B"),
        ));
    }

    #[test]
    fn headless_config_uses_camel_case_wire_fields() {
        let config = HeadlessAcceptanceConfig {
            scenario: "libraryAnalysis".into(),
            exercise_cancel_resume: true,
            input_path: None,
            output_path: None,
            database_path: None,
            report_path: "/tmp/report.jsonl".into(),
        };
        let value = serde_json::to_value(config).expect("config should serialize");
        assert_eq!(value["exerciseCancelResume"], true);
        assert_eq!(value["reportPath"], "/tmp/report.jsonl");
    }

    #[test]
    fn library_analysis_candidates_include_available_files_outside_configured_roots() {
        let candidates = super::build_library_analysis_candidates(
            vec![
                CatalogLocalFile {
                    id: None,
                    track_key: "track:test-root".into(),
                    path: PathBuf::from("/music/test/inside.mp3"),
                    size_bytes: 11,
                    modified_at_ms: None,
                    measured_format: Some("mp3".into()),
                    measured_bitrate_bps: None,
                    measured_duration_seconds: None,
                    sample_rate_hz: None,
                    channels: None,
                    readable: true,
                    probe_error: None,
                },
                CatalogLocalFile {
                    id: None,
                    track_key: "track:other-root".into(),
                    path: PathBuf::from("/music/testtttt/outside.flac"),
                    size_bytes: 22,
                    modified_at_ms: None,
                    measured_format: Some("flac".into()),
                    measured_bitrate_bps: None,
                    measured_duration_seconds: None,
                    sample_rate_hz: None,
                    channels: None,
                    readable: true,
                    probe_error: None,
                },
                CatalogLocalFile {
                    id: None,
                    track_key: "track:ignored".into(),
                    path: PathBuf::from("/music/test/.DS_Store"),
                    size_bytes: 1,
                    modified_at_ms: None,
                    measured_format: None,
                    measured_bitrate_bps: None,
                    measured_duration_seconds: None,
                    sample_rate_hz: None,
                    channels: None,
                    readable: true,
                    probe_error: None,
                },
            ],
            &[PathBuf::from("/music/test")],
        );

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].name, "inside.mp3");
        assert_eq!(candidates[0].slot_index, Some(0));
        assert_eq!(candidates[1].name, "outside.flac");
        assert_eq!(candidates[1].slot_index, None);
    }

    #[test]
    fn library_refresh_progress_uses_camel_case_payloads() {
        let progress = LibraryRefreshProgress {
            refresh_id: "library-1".into(),
            status: LibraryRefreshStatus::Running,
            stage: LibraryRefreshStage::ProbingLocalFiles,
            processed: 2,
            total: Some(3),
            current_item: "Song.mp3".into(),
            message: "正在探测本地文件".into(),
            summary: None,
            error: None,
        };
        let value = serde_json::to_value(progress).unwrap();
        assert_eq!(value["refreshId"], "library-1");
        assert_eq!(value["currentItem"], "Song.mp3");
        assert_eq!(value["status"], "running");
        assert_eq!(value["stage"], "probingLocalFiles");
    }

    #[test]
    fn scan_progress_serializes_metadata_phase_and_cancelling_status() {
        let progress = ScanProgress {
            status: ScanStatus::Cancelling,
            phase: ScanProgressPhase::MatchingMetadata,
            processed: 7,
            total: 1088,
            current_file: "Song.mp3".into(),
            message: "正在取消扫描".into(),
            tasks: vec![ScanTaskProgress {
                slot_index: 0,
                phase: ScanProgressPhase::MatchingMetadata,
                processed: 1088,
                total: 1088,
                source_processed: 1088,
                source_total: Some(1088),
                destination_processed: 0,
                destination_total: None,
                metadata_processed: 7,
                metadata_total: Some(1088),
                current_file: "Song.mp3".into(),
            }],
        };

        let value = serde_json::to_value(progress).unwrap();
        assert_eq!(value["status"], "cancelling");
        assert_eq!(value["phase"], "matching_metadata");
        assert_eq!(value["tasks"][0]["metadata_processed"], 7);
        assert_eq!(value["tasks"][0]["metadata_total"], 1088);
    }

    #[test]
    fn netease_metadata_database_status_uses_camel_case() {
        let value = serde_json::to_value(NeteaseMetadataDatabaseStatus {
            manual_path: Some("/music/db.sqlite3".into()),
            effective_path: Some("/music/db.sqlite3".into()),
            source: NeteaseMetadataDatabaseSource::Manual,
            loaded: true,
            record_count: 42,
            warning: None,
            cache_status: Some("ready".into()),
            cached_record_count: Some(42),
            database_changed: Some(false),
        })
        .unwrap();
        assert_eq!(value["source"], "manual");
        assert_eq!(value["recordCount"], 42);
        assert!(value.get("manual_path").is_none());
    }

    #[test]
    fn netease_metadata_database_status_reports_invalid_manual_path_as_fallback() {
        let missing = Path::new("/definitely/missing/sqlite_storage.sqlite3");
        let (status, resolver) =
            super::resolve_netease_metadata_database_status(
                Some(missing),
                Path::new("/definitely/missing/library-dashboard.sqlite3"),
            )
            .unwrap();
        assert_ne!(status.source, NeteaseMetadataDatabaseSource::Manual);
        assert!(status.warning.is_some());
        assert_eq!(
            status.manual_path.as_deref(),
            Some("/definitely/missing/sqlite_storage.sqlite3")
        );
        assert_eq!(status.loaded, resolver.database_loaded());
    }

    #[test]
    fn external_url_allowlist_includes_project_dj_skill_and_official_essentia_pages() {
        assert!(super::external_url_is_allowed(
            "https://github.com/komakizhu/dj-crate-digger-skill"
        ));
        assert!(!super::external_url_is_allowed(
            "https://github.com/komakizhu/dj-crate-digger-skill/evil"
        ));
        assert!(super::external_url_is_allowed("https://essentia.upf.edu/models/"));
        assert!(!super::external_url_is_allowed(
            "https://essentia.upf.edu.evil.example/models/"
        ));
        assert!(!super::external_url_is_allowed("http://essentia.upf.edu/models/"));
    }

    #[test]
    fn essentia_import_result_uses_camel_case() {
        let value = serde_json::to_value(super::EssentiaModelImportResult {
            installed_ids: vec!["musicnn_embedding".into()],
            issues: vec![super::EssentiaModelImportIssueDto {
                file_name: "model.json".into(),
                reason: "缺少权重".into(),
            }],
            missing_ids: vec!["mood_happy".into()],
            status: super::EssentiaModelStatus {
                version: super::ESSENTIA_MODEL_VERSION,
                embedding: true,
                genre: true,
                mood: false,
                instrument: false,
                installing: false,
                emotion_continuous: false,
                emotion_cluster: false,
                discogs_effnet: None,
            },
            message: "部分导入完成".into(),
        })
        .unwrap();
        assert!(value.get("installedIds").is_some());
        assert!(value.get("missingIds").is_some());
        assert!(value["status"].get("emotionContinuous").is_some());
        assert!(value["status"].get("emotionCluster").is_some());
        assert_eq!(value["issues"][0]["fileName"], "model.json");
    }

    #[test]
    fn importer_identity_table_matches_runtime_model_specs() {
        let runtime = essentia_model_specs()
            .into_iter()
            .map(|spec| spec.id)
            .collect::<BTreeSet<_>>();
        let importable = known_import_model_ids().iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(runtime, importable);
    }

    #[test]
    fn checked_in_musicnn_classifier_outputs_match_runtime_specs() {
        let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("essentia-models");
        let specs = essentia_model_specs();
        for id in [
            "mood_aggressive",
            "mood_happy",
            "mood_relaxed",
            "mood_party",
            "mood_sad",
            "voice_instrumental",
        ] {
            let spec = specs
                .iter()
                .find(|candidate| candidate.id == id)
                .expect("runtime model spec is registered");
            let value: serde_json::Value = serde_json::from_slice(
                &fs::read(resources.join(format!("{id}.json"))).expect("bundled model exists"),
            )
            .expect("bundled model JSON is valid");
            let nodes = value
                .pointer("/modelTopology/node")
                .and_then(serde_json::Value::as_array)
                .expect("bundled model has graph nodes");
            assert!(
                nodes.iter().any(|node| {
                    node.get("name").and_then(serde_json::Value::as_str)
                        == Some(spec.output_name)
                }),
                "{id} runtime output {} is absent from the checked-in graph",
                spec.output_name,
            );
        }
    }

    #[test]
    fn completed_library_refresh_reports_unique_track_count() {
        let mut progress = LibraryRefreshProgress {
            refresh_id: "library-1".into(),
            status: LibraryRefreshStatus::Running,
            stage: LibraryRefreshStage::Committing,
            processed: 68,
            total: Some(68),
            current_item: "metadata-row".into(),
            message: "正在提交歌曲库更新".into(),
            summary: None,
            error: None,
        };
        let summary = LibraryRefreshSummary {
            track_count: 24,
            local_file_count: 24,
            readable_file_count: 24,
            reused_file_count: 0,
            database_path: "/music/library.db".into(),
            music_folder: Some("/music/netease".into()),
        };

        apply_library_refresh_terminal(
            &mut progress,
            LibraryRefreshStatus::Completed,
            Some(LibraryRefreshStage::Committing),
            "歌曲库更新完成".into(),
            Some(summary),
            None,
        );

        assert_eq!(progress.processed, 24);
        assert_eq!(progress.total, Some(24));
        assert_eq!(progress.summary.as_ref().map(|summary| summary.track_count), Some(24));
    }

    #[test]
    fn library_refresh_is_singleton_and_cancel_is_idempotent() {
        let state = LibraryState {
            catalog_path: Mutex::new(PathBuf::new()),
            manual_database_path: Mutex::new(None),
            metadata_cache: Mutex::new(NeteaseMetadataCacheProgress::default()),
            metadata_cache_cancel: AtomicBool::new(false),
            metadata_cache_build_lock: Mutex::new(()),
            metadata_cache_worker: Mutex::new(None),
            refresh: Mutex::new(LibraryRefreshProgress {
                refresh_id: String::new(),
                status: LibraryRefreshStatus::Idle,
                stage: LibraryRefreshStage::LocatingDatabase,
                processed: 0,
                total: None,
                current_item: String::new(),
                message: String::new(),
                summary: None,
                error: None,
            }),
            cancel: AtomicBool::new(false),
            worker: Mutex::new(None),
            invalid_scan: Mutex::new(InvalidScanProgress::default()),
            invalid_scan_cancel: AtomicBool::new(false),
            invalid_scan_worker: Mutex::new(None),
        };

        let initial = try_start_library_refresh(&state, "library-test".to_string()).unwrap();
        assert!(matches!(initial.status, LibraryRefreshStatus::Running));
        assert!(try_start_library_refresh(&state, "library-test-2".to_string()).is_err());

        let cancelling = request_library_refresh_cancel(&state);
        assert!(matches!(cancelling.status, LibraryRefreshStatus::Cancelling));
        assert!(state.cancel.load(std::sync::atomic::Ordering::SeqCst));
        let repeated = request_library_refresh_cancel(&state);
        assert!(matches!(repeated.status, LibraryRefreshStatus::Cancelling));
        assert_eq!(repeated.refresh_id, "library-test");
    }

    fn sample_preview(slot_index: usize, has_candidate: bool) -> SlotPreview {
        SlotPreview {
            slot_index,
            mode: Mode::Compat,
            lossless_format: None,
            conflict_strategy: Default::default(),
            filename_rule: Default::default(),
            netease_filename_format: Default::default(),
            retry_of: None,
            preview: SyncPreview {
                source_directory: format!("/music/in-{slot_index}"),
                destination_directory: format!("/music/out-{slot_index}"),
                new_count: usize::from(has_candidate),
                existing_count: usize::from(!has_candidate),
                skipped_count: usize::from(!has_candidate),
                error_count: 0,
                estimated_output_bytes: has_candidate.then_some(1024),
                candidates: if has_candidate {
                    vec![PreviewCandidate {
                        name: "song".into(),
                        source_path: "/music/in/song.mp3".into(),
                        destination_path: "/music/out/song.mp3".into(),
                        source_size_bytes: 1024,
                        estimated_output_bytes: Some(1024),
                        previous_destination_path: None,
                        operation: Default::default(),
                        netease_track_id: None,
                        netease_album_id: None,
                        album: None,
                        netease_title: None,
                        netease_artist: None,
                        disambiguation_reason: None,
                    }]
                } else {
                    Vec::new()
                },
                skipped: Vec::new(),
                errors: Vec::new(),
                warnings: Vec::new(),
                available_space_bytes: None,
                disk_space_sufficient: None,
                input_count: 0,
                output_duplicate_count: 0,
                action_kind: String::new(),
                action_count: 0,
                database_directory: None,
                detail_items: Vec::new(),
            },
        }
    }

    #[test]
    fn local_test_monitor_writes_input_output_and_summary_files() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-test-monitor-{}",
            super::unique_timestamp()
        ));
        let preview = sample_preview(0, true);
        let monitor = TestMonitor::new(
            &root,
            "batch/test",
            serde_json::json!({"enhanced_mode": true}),
            std::slice::from_ref(&preview),
            1,
        )
        .unwrap();

        let candidate = &preview.preview.candidates[0];
        monitor.record_candidate_result(candidate, "completed", None);

        let session = serde_json::from_slice::<serde_json::Value>(
            &fs::read(monitor.session_dir.join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(session["status"], "running");
        assert!(monitor.session_dir.join("candidates.json").is_file());
        assert!(monitor.session_dir.join("events.jsonl").is_file());
        assert!(monitor.session_dir.join("README.md").is_file());
        assert!(monitor.session_dir.join("analysis-state.json").is_file());

        let event_text = fs::read_to_string(monitor.session_dir.join("events.jsonl")).unwrap();
        assert!(event_text.contains("candidate_result"));
        assert!(event_text.contains("/music/in/song.mp3"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn analysis_run_claim_rejects_a_live_attempt_and_allows_stale_takeover() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-analysis-claim-{}",
            super::unique_timestamp()
        ));
        let preview = sample_preview(0, true);
        let monitor = TestMonitor::new(
            &root,
            "batch/claim",
            serde_json::json!({"enhanced_mode": true}),
            std::slice::from_ref(&preview),
            1,
        )
        .unwrap();
        monitor.claim_analysis_run("attempt-a").unwrap();
        assert!(monitor.claim_analysis_run("attempt-b").is_err());
        let state_path = monitor.session_dir.join("analysis-state.json");
        let mut state = serde_json::from_slice::<serde_json::Value>(&fs::read(&state_path).unwrap()).unwrap();
        state["lastHeartbeatEpochMs"] = serde_json::json!(0);
        fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
        monitor.claim_analysis_run("attempt-b").unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&state_path).unwrap()).unwrap()["attemptId"],
            "attempt-b"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovered_test_monitor_continues_persisting_analysis_events() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-analysis-recovered-monitor-{}",
            super::unique_timestamp()
        ));
        let preview = sample_preview(0, true);
        let monitor = TestMonitor::new(
            &root,
            "batch/recovered",
            serde_json::json!({"enhanced_mode": true}),
            std::slice::from_ref(&preview),
            1,
        )
        .unwrap();
        monitor.claim_analysis_run("attempt-before-restart").unwrap();
        let recovered = TestMonitor::from_existing(monitor.session_dir.clone()).unwrap();
        recovered.record_event(
            "analysis_candidate_started",
            serde_json::json!({
                "source_path": "/music/in/song.mp3",
                "destination_path": "/music/out/song.mp3",
                "name": "song.mp3",
                "worker_job_id": "worker-after-restart",
            }),
        );
        let state = serde_json::from_slice::<serde_json::Value>(
            &fs::read(monitor.session_dir.join("analysis-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(state["tracks"]["/music/in/song.mp3"]["status"], "running");
        assert_eq!(state["tracks"]["/music/in/song.mp3"]["workerJobId"], "worker-after-restart");
        assert!(fs::read_to_string(monitor.session_dir.join("events.jsonl"))
            .unwrap()
            .contains("worker-after-restart"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_test_monitor_accumulates_incremental_analysis_reports() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-test-monitor-analysis-{}",
            super::unique_timestamp()
        ));
        let preview = sample_preview(0, true);
        let monitor = TestMonitor::new(
            &root,
            "batch/analysis",
            serde_json::json!({"enhanced_mode": true}),
            std::slice::from_ref(&preview),
            1,
        )
        .unwrap();
        let report = |source_path: &str| AnalysisReport {
            source_path: source_path.into(),
            destination_path: format!("/music/out/{}", source_path.rsplit('/').next().unwrap()),
            status: "completed".into(),
            message: None,
            drop_status: Some("completed".into()),
            drop_loudness_lufs: None,
            model_status: Some("completed".into()),
            model_details: None,
            stage: None,
            elapsed_ms: None,
            basic_status: None,
            basic_danceability: None,
            discogs_danceability_status: None,
            discogs_danceability: None,
            discogs_completed_heads: None,
            discogs_total_heads: None,
            cached: None,
        };

        monitor.record_analysis_reports(&[report("/music/in/one.mp3")]);
        monitor.record_analysis_reports(&[report("/music/in/two.mp3")]);

        let reports = serde_json::from_slice::<Vec<AnalysisReport>>(
            &fs::read(monitor.session_dir.join("analysis-reports.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].source_path, "/music/in/one.mp3");
        assert_eq!(reports[1].source_path, "/music/in/two.mp3");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_session_export_contains_history_and_local_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-runtime-session-export-{}",
            super::unique_timestamp()
        ));
        let preview = sample_preview(0, true);
        let monitor = TestMonitor::new(
            &root,
            "batch/export",
            serde_json::json!({"enhanced_mode": true}),
            std::slice::from_ref(&preview),
            1,
        )
        .unwrap();
        monitor.record_event(
            "analysis_completed",
            serde_json::json!({"result_count": 1}),
        );
        let entry = HistoryEntry {
            id: "history-export".into(),
            batch_id: "batch/export".into(),
            slot_index: 0,
            started_at: "2026-08-20 00:00:00 UTC".into(),
            finished_at: "2026-08-20 00:00:01 UTC".into(),
            duration_seconds: 1,
            source_directory: "/music/in-0".into(),
            destination_directory: "/music/out-0".into(),
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
            logs: vec!["conversion complete".into()],
            status: HistoryStatus::Completed,
            retry_of: None,
            conflict_strategy: Default::default(),
            filename_rule: Default::default(),
            netease_filename_format: Default::default(),
            report_path: None,
            analysis_reports: Vec::new(),
            runtime_session_dir: None,
        };

        let payload = super::build_runtime_session_export(&entry, Some(&monitor.session_dir));
        assert_eq!(payload["schemaVersion"], 1);
        assert_eq!(payload["history"]["id"], "history-export");
        let mut slot_entry = entry.clone();
        slot_entry.id = "batch-export-slot2".into();
        assert_eq!(
            super::resolve_runtime_session_dir(&root, &slot_entry),
            Some(monitor.session_dir.clone())
        );
        assert_eq!(payload["runtimeSession"]["available"], true);
        assert!(payload["runtimeSession"]["files"]["session.json"].is_object());
        assert!(payload["runtimeSession"]["files"]["events.jsonl"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["event"] == "analysis_completed")));
        assert!(payload["readableSummary"].as_str().is_some());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_analysis_summary_distinguishes_timeout_and_pending_tracks() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-runtime-analysis-summary-{}",
            super::unique_timestamp()
        ));
        fs::create_dir_all(&root).unwrap();
        let candidates = serde_json::json!([
            { "preview": { "candidates": [
                { "name": "completed.mp3", "source_path": "/music/in/completed.mp3", "destination_path": "/music/out/completed.mp3" },
                { "name": "timeout.mp3", "source_path": "/music/in/timeout.mp3", "destination_path": "/music/out/timeout.mp3" },
                { "name": "pending.mp3", "source_path": "/music/in/pending.mp3", "destination_path": "/music/out/pending.mp3" }
            ] } }
        ]);
        fs::write(
            root.join("candidates.json"),
            serde_json::to_vec(&candidates).unwrap(),
        )
        .unwrap();
        let events = [
            serde_json::json!({ "event": "analysis_requested", "at": "2026-08-22 00:00:00 UTC", "details": {} }),
            serde_json::json!({ "event": "analysis_started", "at": "2026-08-22 00:00:01 UTC", "details": {} }),
            serde_json::json!({ "event": "analysis_candidate_finished", "at": "2026-08-22 00:01:00 UTC", "details": {
                "source_path": "/music/in/completed.mp3", "name": "completed.mp3", "status": "completed", "worker_job_id": "worker-1"
            } }),
            serde_json::json!({ "event": "analysis_candidate_finished", "at": "2026-08-22 00:02:00 UTC", "details": {
                "source_path": "/music/in/timeout.mp3", "name": "timeout.mp3", "status": "timeout", "stage": "analyzingHighLevel", "worker_job_id": "worker-2", "elapsed_ms": 300000
            } }),
            serde_json::json!({ "event": "analysis_completed", "at": "2026-08-22 00:03:00 UTC", "details": {} }),
        ];
        let contents = events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.join("events.jsonl"), contents).unwrap();

        let summary = super::runtime_session_analysis_summary(Some(&root)).unwrap();
        assert_eq!(summary.status, "partial");
        assert_eq!(summary.total, 3);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.timed_out, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.pending, 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manual_error_report_export_writes_file_and_records_path() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-error-report-export-{}",
            super::unique_timestamp()
        ));
        fs::create_dir_all(&root).unwrap();
        let history_path = root.join("history.json");
        let output_path = root.join("manual-report.txt");
        let entry = HistoryEntry {
            id: "history-manual-report".into(),
            batch_id: "batch-manual-report".into(),
            slot_index: 0,
            started_at: "2026-08-20 00:00:00 UTC".into(),
            finished_at: "2026-08-20 00:00:01 UTC".into(),
            duration_seconds: 1,
            source_directory: "/music/in".into(),
            destination_directory: "/music/out".into(),
            mode: Mode::Compat,
            lossless_format: None,
            new_count: 1,
            existing_count: 0,
            skipped_count: 0,
            error_count: 1,
            completed_count: 0,
            failed_count: 1,
            failed_files: vec![FailedFile {
                name: "broken.mp3".into(),
                source_path: "/music/in/broken.mp3".into(),
                destination_path: "/music/out/broken.mp3".into(),
                message: "conversion failed".into(),
                category: Default::default(),
            }],
            pending_files: Vec::new(),
            metadata_diagnostics: Vec::new(),
            logs: vec!["conversion failed".into()],
            status: HistoryStatus::Error,
            retry_of: None,
            conflict_strategy: Default::default(),
            filename_rule: Default::default(),
            netease_filename_format: Default::default(),
            report_path: None,
            analysis_reports: Vec::new(),
            runtime_session_dir: None,
        };
        w4dj::history::upsert_history(&history_path, entry).unwrap();

        super::export_history_error_report_to_path(
            &history_path,
            "history-manual-report",
            &output_path,
        )
        .unwrap();

        let report = fs::read_to_string(&output_path).unwrap();
        assert!(report.contains("任务 ID：history-manual-report"));
        assert!(report.contains("导出保存位置："));
        let entries = w4dj::history::load_history(&history_path).unwrap();
        assert_eq!(entries[0].report_path.as_deref(), Some(output_path.to_str().unwrap()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_validation_accepts_a_single_audio_file() {
        let source = std::env::temp_dir().join(format!(
            "w4dj-single-source-{}-{}.mp3",
            std::process::id(),
            super::unique_timestamp()
        ));
        fs::write(&source, b"single-track").unwrap();

        let result = validate_source_input(source.to_str().unwrap());
        let _ = fs::remove_file(&source);

        assert!(result.is_ok());
    }

    #[test]
    fn source_validation_rejects_an_unsupported_single_file() {
        let source = std::env::temp_dir().join(format!(
            "w4dj-single-source-{}-{}.txt",
            std::process::id(),
            super::unique_timestamp()
        ));
        fs::write(&source, b"not-a-track").unwrap();

        let result = validate_source_input(source.to_str().unwrap());
        let _ = fs::remove_file(&source);

        assert!(result.is_err());
    }

    #[test]
    fn history_timestamps_are_human_readable_utc() {
        assert_eq!(super::format_unix_timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            super::format_unix_timestamp(1_784_210_712),
            "2026-07-16 14:05:12 UTC"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn source_picker_helper_returns_none_when_cancelled() {
        assert_eq!(super::source_picker_result(false, None), Ok(None));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn source_picker_helper_returns_selected_path() {
        let path = String::from("/music/single-track.flac");

        assert_eq!(
            super::source_picker_result(true, Some(path.clone())),
            Ok(Some(path)),
        );
        assert!(super::source_picker_result(true, None).is_err());
    }

    #[test]
    fn duplicate_outputs_across_slots_are_only_planned_once() {
        let mut previews = vec![sample_preview(0, true), sample_preview(1, true)];

        assert!(validate_unique_planned_outputs(&previews).is_err());
        deduplicate_cross_slot_candidates(&mut previews);

        assert_eq!(previews[0].preview.candidates.len(), 1);
        assert!(previews[1].preview.candidates.is_empty());
        assert_eq!(previews[1].preview.new_count, 0);
        assert_eq!(previews[1].preview.skipped_count, 1);
        assert!(validate_unique_planned_outputs(&previews).is_ok());
    }

    #[test]
    fn processable_previews_ignore_slots_without_new_files() {
        let processable = collect_processable_previews(
            vec![sample_preview(0, false), sample_preview(1, true)],
            false,
        )
        .expect("a slot with candidates should start even when another slot is already complete");

        assert_eq!(processable.len(), 1);
        assert_eq!(processable[0].slot_index, 1);
    }

    #[test]
    fn direct_validation_allows_a_valid_slot_when_another_slot_is_empty() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-direct-validation-{}-{}",
            std::process::id(),
            super::unique_timestamp()
        ));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();

        let mut valid = sample_preview(0, true);
        valid.preview.source_directory = source.to_string_lossy().into_owned();
        valid.preview.destination_directory = destination.to_string_lossy().into_owned();

        let mut empty = sample_preview(1, false);
        empty.preview.source_directory = source.to_string_lossy().into_owned();
        empty.preview.destination_directory = destination.to_string_lossy().into_owned();

        let result = validate_scan_previews(&[valid, empty]);
        let _ = fs::remove_dir_all(root);

        assert!(result.is_ok());
    }

    #[test]
    fn retry_previews_with_only_missing_files_can_be_recorded() {
        let mut preview = sample_preview(0, false);
        preview.retry_of = Some(String::from("history-1"));
        preview.preview.error_count = 1;
        preview.preview.errors.push(PreviewIssue {
            path: String::from("/music/in/missing.mp3"),
            message: String::from("重试时找不到源文件"),
        });

        let processable = collect_processable_previews(vec![preview], true)
            .expect("a retry should preserve a missing file as a failed task");

        assert_eq!(processable.len(), 1);
        assert!(processable[0].preview.candidates.is_empty());
    }

    #[test]
    fn preflight_file_errors_are_recorded_for_history() {
        let mut controller =
            DesktopController::new(DesktopState::from_preferences(AppPreferences {
                slots: [
                    SyncSlotPreferences::new("/music/in-1", "/music/out-1"),
                    SyncSlotPreferences::new("/music/in-2", "/music/out-2"),
                ],
                mode: Mode::Compat,
                lossless_format: None,
                ..AppPreferences::default()
            }));
        controller.start_confirmed_sync(0, 1).unwrap();
        let mut preview = sample_preview(0, true).preview;
        preview.error_count = 1;
        preview.errors = vec![PreviewIssue {
            path: String::from("/music/in-1/unreadable.mp3"),
            message: String::from("无法读取源文件"),
        }];

        apply_preflight_summary(&mut controller, 0, &preview).unwrap();

        assert_eq!(controller.state().slots[0].error_tracks, 1);
        assert_eq!(controller.state().slots[0].failed_files.len(), 1);
    }

    #[test]
    fn coordinator_reuses_a_lock_for_the_same_destination() {
        let coordinator = DestinationCoordinator::default();

        let first = coordinator.lock_for(Path::new("/music/output-a"));
        let second = coordinator.lock_for(Path::new("/music/output-a"));
        let other = coordinator.lock_for(Path::new("/music/output-b"));

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn validates_output_directories_before_opening_them() {
        let directory = std::env::temp_dir().join(format!(
            "w4dj-open-destination-{}",
            super::unique_timestamp()
        ));
        fs::create_dir_all(&directory).unwrap();
        assert!(validate_destination_directory(&directory).is_ok());

        let missing = directory.join("missing");
        assert!(validate_destination_directory(&missing).is_err());
        assert!(validate_destination_directory(Path::new("")).is_err());
        let _ = fs::remove_dir(&directory);
    }

    #[test]
    fn history_status_distinguishes_partial_and_failed_runs() {
        let task = TaskController::running(2);
        let failed_file = FailedFile {
            name: "song".into(),
            source_path: "/in/song.flac".into(),
            destination_path: "/out/song.mp3".into(),
            message: "failed".into(),
            category: Default::default(),
        };

        assert_eq!(
            history_status_for(&task.snapshot(), std::slice::from_ref(&failed_file)),
            HistoryStatus::Error
        );
        task.complete_current_file();
        assert_eq!(
            history_status_for(&task.snapshot(), &[failed_file]),
            HistoryStatus::Partial
        );
    }

    #[test]
    fn confirmed_sync_persists_history_before_publishing_terminal_state() {
        let directory = std::env::temp_dir().join(format!(
            "w4dj-history-order-{}-{}",
            std::process::id(),
            super::unique_timestamp()
        ));
        fs::create_dir_all(&directory).unwrap();
        let history_path = directory.join("history.json");
        let controller = Arc::new(Mutex::new(DesktopController::new(
            DesktopState::from_preferences(AppPreferences {
                slots: [
                    SyncSlotPreferences::new("/music/in", "/music/out"),
                    SyncSlotPreferences::new("", ""),
                ],
                mode: Mode::Compat,
                lossless_format: None,
                ..AppPreferences::default()
            }),
        )));
        let task_controller = {
            let mut guard = controller.lock().expect("desktop lock should not be poisoned");
            guard.start_confirmed_sync(0, 1).unwrap();
            guard.task_controller(0).unwrap()
        };
        task_controller.complete_current_file();

        let entry = HistoryEntry {
            id: "batch-history-order-slot1".into(),
            batch_id: "batch-history-order".into(),
            slot_index: 0,
            started_at: "2026-08-20 06:00:00 UTC".into(),
            finished_at: "2026-08-20 06:00:01 UTC".into(),
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
        };

        let history_lock = Arc::new(Mutex::new(()));
        persist_history_before_terminal_state(
            &history_path,
            &history_lock,
            &controller,
            0,
            entry,
            Ok(task_controller.snapshot()),
        )
        .expect("final history should be persisted");

        let entries = w4dj::history::load_history(&history_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].batch_id, "batch-history-order");
        assert_eq!(
            controller.lock().unwrap().state().slots[0].status,
            w4dj::desktop::DesktopStatus::Completed
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn copying_playlist_audio_keeps_duplicate_names_and_updates_m3u8_paths() {
        let root = std::env::temp_dir().join(format!("w4dj-copy-audio-{}", super::unique_timestamp()));
        let source_a = root.join("a");
        let source_b = root.join("b");
        let output = root.join("out");
        fs::create_dir_all(&source_a).unwrap();
        fs::create_dir_all(&source_b).unwrap();
        fs::create_dir_all(&output).unwrap();
        let source_a_path = source_a.join("Same.mp3");
        let source_b_path = source_b.join("Same.mp3");
        fs::write(&source_a_path, b"audio-a").unwrap();
        fs::write(&source_b_path, b"audio-b").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&source_a_path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::set_permissions(&source_b_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let mut resolved = vec![
            ResolvedDjPlaylistTrack {
                position: 1,
                title: "A".into(),
                artist_display: "Artist".into(),
                duration_seconds: None,
                destination_path: source_a_path.clone(),
            },
            ResolvedDjPlaylistTrack {
                position: 2,
                title: "B".into(),
                artist_display: "Artist".into(),
                duration_seconds: None,
                destination_path: source_b_path.clone(),
            },
        ];

        let created = super::copy_playlist_audio_to_directory(&mut resolved, &output).unwrap();

        assert_eq!(created.len(), 2);
        assert_eq!(resolved[0].destination_path.file_name().unwrap(), "Same.mp3");
        assert_eq!(resolved[1].destination_path.file_name().unwrap(), "Same (1).mp3");
        assert_eq!(fs::read(&resolved[0].destination_path).unwrap(), b"audio-a");
        assert_eq!(fs::read(&resolved[1].destination_path).unwrap(), b"audio-b");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&resolved[0].destination_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
            assert_eq!(
                fs::metadata(&resolved[1].destination_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
            assert_eq!(
                fs::metadata(&source_a_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&source_b_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copied_playlist_paths_are_self_contained() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-portable-playlist-{}",
            super::unique_timestamp()
        ));
        let export_directory = root.join("Playlist");
        fs::create_dir_all(&export_directory).unwrap();
        let audio = export_directory.join("Track.mp3");
        fs::write(&audio, b"audio").unwrap();
        let resolved = vec![ResolvedDjPlaylistTrack {
            position: 1,
            title: "Track".into(),
            artist_display: "Artist".into(),
            duration_seconds: None,
            destination_path: audio,
        }];

        assert!(super::validate_portable_playlist_export(
            &resolved,
            &export_directory,
            "#EXTM3U\n#EXTINF:-1,Artist - Track\nTrack.mp3"
        )
        .is_ok());
        assert!(super::validate_portable_playlist_export(
            &resolved,
            &export_directory,
            "#EXTM3U\n#EXTINF:-1,Artist - Track\n../Track.mp3"
        )
        .is_err());
        assert!(super::validate_portable_playlist_export(
            &resolved,
            &export_directory,
            "#EXTM3U\n#EXTINF:-1,Artist - Track\n/Users/mac/Music/Track.mp3"
        )
        .is_err());
        let outside_audio = root.join("Outside.mp3");
        fs::write(&outside_audio, b"outside").unwrap();
        let outside = vec![ResolvedDjPlaylistTrack {
            position: 1,
            title: "Outside".into(),
            artist_display: "Artist".into(),
            duration_seconds: None,
            destination_path: outside_audio,
        }];
        assert!(super::validate_portable_playlist_export(
            &outside,
            &export_directory,
            "#EXTM3U\n#EXTINF:-1,Artist - Outside\nOutside.mp3"
        )
        .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn portable_playlist_directory_is_cross_account_readable() {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "w4dj-portable-directory-{}",
            super::unique_timestamp()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();

        super::set_portable_export_permissions(&directory, 0o755).unwrap();

        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let _ = fs::remove_dir(&directory);
    }

    #[test]
    fn playlist_export_paths_use_the_playlist_name_as_the_folder_and_file_name() {
        let root = std::env::temp_dir().join(format!(
            "w4dj-playlist-export-paths-{}",
            super::unique_timestamp()
        ));
        let selected = root.join("ignored-name.m3u8");

        let (directory, playlist_path) =
            super::playlist_export_paths(&selected, "模拟 UK Bass 歌单").unwrap();

        assert_eq!(directory, root.join("模拟 UK Bass 歌单"));
        assert_eq!(
            playlist_path,
            root.join("模拟 UK Bass 歌单").join("模拟 UK Bass 歌单.m3u8")
        );

        let selected_inside_folder = playlist_path.clone();
        let (same_directory, same_playlist_path) =
            super::playlist_export_paths(&selected_inside_folder, "模拟 UK Bass 歌单").unwrap();
        assert_eq!(same_directory, directory);
        assert_eq!(same_playlist_path, playlist_path);
    }
}
