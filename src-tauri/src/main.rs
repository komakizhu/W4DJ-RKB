#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use base64::Engine as _;
use flate2::read::DeflateDecoder;
use ncmdump::Ncmdump;
use tauri::Manager;
use w4dj::analysis::{
    TrackAnalysis, TrackMetadata, analysis_file_path, build_rekordbox_xml, clear_analysis_file,
    load_analysis_file, merge_analysis_entries, read_track_metadata, save_analysis_file,
};
use w4dj::config::{
    ConflictStrategy, ConversionMode, FilenameRule, LosslessFormat, Mode, NeteaseFilenameFormat,
};
use w4dj::desktop::{DesktopController, DesktopState};
use w4dj::history::{
    AnalysisReport, FailedFile, HistoryEntry, HistoryStatus, PendingFile, append_analysis_reports,
    classify_error, clear_history, delete_history_entry, format_error_report,
    load_history as load_history_file, upsert_history,
};
use w4dj::library_catalog::{CatalogSourceRecord, LibraryCatalog};
use w4dj::library_query::{LibraryPage, LibraryQuery};
use w4dj::netease_library::{
    NeteaseDiscovery, build_catalog_snapshot_incremental, discover_netease_library,
};
use w4dj::preferences::{AppPreferences, load_preferences, save_preferences};
use w4dj::preview::{
    PreviewCandidate, PreviewIssue, PreviewOperation, SlotPreview, SyncPreview,
    build_retry_preview, build_sync_preview_with_settings_and_netease,
    build_sync_preview_with_settings_and_netease_observed_with_cache,
    is_recovered_single_source, resolve_missing_single_source_path,
};
use w4dj::scan_cache::{ScanCache, clear_scan_cache as clear_scan_cache_file, load_scan_cache, save_scan_cache_atomic};
use w4dj::sync::{
    apply_track_analysis_metadata, cleanup_temporary_outputs, compare_music_dicts,
    count_music_files_with_cancel,
    EmbeddedAnalysis, inspect_metadata_diagnostic,
    get_destination_music_dict, get_music_dict_with_scan_issues, is_supported_source_file,
    sync_music_library_transactional_with_observer, sync_music_library_with_observer,
    update_existing_metadata_transactionally, ScanPhase,
};

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
    scan_cache_path: Arc<Mutex<PathBuf>>,
    library_catalog_path: Arc<Mutex<PathBuf>>,
    history_write_lock: Arc<Mutex<()>>,
    destination_coordinator: DestinationCoordinator,
    scan_progress: Arc<Mutex<ScanProgress>>,
    scan_cancel: Arc<AtomicBool>,
    scan_result: Arc<Mutex<Option<Vec<SlotPreview>>>>,
    test_monitor_path: Arc<Mutex<PathBuf>>,
    test_monitors: Arc<Mutex<HashMap<String, Arc<TestMonitor>>>>,
}

// Debug-only delivery switch. Remove this block before creating a formal release build.
const DEBUG_TEST_MONITOR_ENABLED: bool = true;
const TEST_MONITOR_DIRECTORY: &str = "W4DJ-test-monitor";

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
    lock: Arc<Mutex<()>>,
    session: Arc<Mutex<TestMonitorSession>>,
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
            monitor: "W4DJ local debug test monitor",
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
            session_dir,
            lock: Arc::new(Mutex::new(())),
            session: Arc::new(Mutex::new(session)),
            remaining_jobs: Arc::new(AtomicUsize::new(job_count)),
        };
        monitor.write_session_file()?;
        fs::write(
            monitor.session_dir.join("candidates.json"),
            serde_json::to_string_pretty(previews).map_err(|error| {
                io::Error::other(format!("serialize monitor candidates: {error}"))
            })?,
        )?;
        fs::write(
            monitor.session_dir.join("README.md"),
            "# W4DJ 本地调试测试记录\n\n此目录由当前调试版自动生成，仅保存在本机下载目录，不会上传。\n\n- `candidates.json`：本次任务的输入与计划输出。\n- `events.jsonl`：按时间追加的任务和单曲结果。\n- `summary-slot-*.json`：每个任务结束后的完整转换历史、错误和元数据诊断。\n- `analysis-reports.json`：增强分析和分析元数据回写结果（增强模式生成）。\n- `session.json`：本次测试的设置、状态和任务汇总。\n\n路径和元数据诊断可能包含本机文件信息，请分享给开发者前自行确认。\n",
        )?;
        monitor.record_event("session_started", serde_json::json!({
            "session_directory": monitor.session_dir.display().to_string(),
            "candidate_count": previews.iter().map(|preview| preview.preview.candidates.len()).sum::<usize>(),
        }));
        Ok(monitor)
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
            eprintln!("Failed to save local test monitor summary: {error}");
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
                session.status = "completed".to_string();
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
        let path = self.session_dir.join("analysis-reports.json");
        if let Ok(contents) = serde_json::to_string_pretty(reports)
            && let Err(error) = fs::write(path, contents)
        {
            eprintln!("Failed to save local test monitor analysis reports: {error}");
        }
        self.record_event(
            "analysis_reports_updated",
            serde_json::json!({
                "report_count": reports.len(),
                "reports": reports,
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
}

fn write_json_file<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let contents = serde_json::to_vec_pretty(value)
        .map_err(|error| io::Error::other(format!("serialize json: {error}")))?;
    fs::write(path, contents)
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
}

#[derive(Debug, Clone, serde::Deserialize)]
struct AnalysisFailure {
    path: String,
    message: String,
}

struct ScanJob {
    conversion_mode: ConversionMode,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    scan_cache_path: PathBuf,
    tasks: Vec<(usize, String, String)>,
}

#[derive(serde::Serialize)]
struct AppInfo {
    version: &'static str,
    developer: &'static str,
    project_url: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryStatus {
    catalog_path: String,
    track_count: u64,
    netease: NeteaseDiscovery,
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

const ESSENTIA_MODEL_VERSION: &str = "essentia-musicnn-2022-v2";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelStatus {
    version: &'static str,
    embedding: bool,
    genre: bool,
    mood: bool,
    instrument: bool,
    downloading: bool,
}

#[derive(Debug, Clone, Copy)]
struct EssentiaModelSpec {
    id: &'static str,
    kind: &'static str,
    model_url: &'static str,
    weights_url: Option<&'static str>,
    classes: &'static [&'static str],
}

fn essentia_model_specs() -> Vec<EssentiaModelSpec> {
    vec![
        EssentiaModelSpec {
            id: "musicnn_embedding",
            kind: "embedding",
            model_url: "https://essentia.upf.edu/models/feature-extractors/musicnn/msd-musicnn-1-tfjs.zip",
            weights_url: None,
            classes: &[],
        },
        EssentiaModelSpec {
            id: "genre_rosamerica",
            kind: "genre",
            model_url: "https://essentia.upf.edu/models/classification-heads/genre_rosamerica/genre_rosamerica-msd-musicnn-1-tfjs/model.json",
            weights_url: Some("https://essentia.upf.edu/models/classification-heads/genre_rosamerica/genre_rosamerica-msd-musicnn-1-tfjs/group1-shard1of1.bin"),
            classes: &["cla", "dan", "hip", "jaz", "pop", "rhy", "roc", "spe"],
        },
        EssentiaModelSpec {
            id: "mood_aggressive",
            kind: "mood",
            model_url: "https://essentia.upf.edu/models/classification-heads/mood_aggressive/mood_aggressive-msd-musicnn-1-tfjs/model.json",
            weights_url: Some("https://essentia.upf.edu/models/classification-heads/mood_aggressive/mood_aggressive-msd-musicnn-1-tfjs/group1-shard1of1.bin"),
            classes: &["aggressive", "non_aggressive"],
        },
        EssentiaModelSpec {
            id: "mood_happy",
            kind: "mood",
            model_url: "https://essentia.upf.edu/models/classification-heads/mood_happy/mood_happy-msd-musicnn-1-tfjs/model.json",
            weights_url: Some("https://essentia.upf.edu/models/classification-heads/mood_happy/mood_happy-msd-musicnn-1-tfjs/group1-shard1of1.bin"),
            classes: &["happy", "non_happy"],
        },
        EssentiaModelSpec {
            id: "mood_relaxed",
            kind: "mood",
            model_url: "https://essentia.upf.edu/models/classification-heads/mood_relaxed/mood_relaxed-msd-musicnn-1-tfjs/model.json",
            weights_url: Some("https://essentia.upf.edu/models/classification-heads/mood_relaxed/mood_relaxed-msd-musicnn-1-tfjs/group1-shard1of1.bin"),
            classes: &["relaxed", "non_relaxed"],
        },
        EssentiaModelSpec {
            id: "mood_party",
            kind: "mood",
            model_url: "https://essentia.upf.edu/models/classification-heads/mood_party/mood_party-msd-musicnn-1-tfjs/model.json",
            weights_url: Some("https://essentia.upf.edu/models/classification-heads/mood_party/mood_party-msd-musicnn-1-tfjs/group1-shard1of1.bin"),
            classes: &["party", "non_party"],
        },
        EssentiaModelSpec {
            id: "mood_sad",
            kind: "mood",
            model_url: "https://essentia.upf.edu/models/classification-heads/mood_sad/mood_sad-msd-musicnn-1-tfjs/model.json",
            weights_url: Some("https://essentia.upf.edu/models/classification-heads/mood_sad/mood_sad-msd-musicnn-1-tfjs/group1-shard1of1.bin"),
            classes: &["sad", "non_sad"],
        },
        EssentiaModelSpec {
            id: "voice_instrumental",
            kind: "instrument",
            model_url: "https://essentia.upf.edu/models/classification-heads/voice_instrumental/voice_instrumental-msd-musicnn-1-tfjs/model.json",
            weights_url: Some("https://essentia.upf.edu/models/classification-heads/voice_instrumental/voice_instrumental-msd-musicnn-1-tfjs/group1-shard1of1.bin"),
            classes: &["instrumental", "voice"],
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
    let json_path = essentia_model_file_path(models_path, spec.id, "json");
    let weights_path = essentia_model_file_path(models_path, spec.id, "bin");
    if !json_path.is_file() || !weights_path.is_file() {
        return false;
    }
    let Ok(model_json) = fs::read_to_string(json_path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&model_json) else {
        return false;
    };
    parsed.get("modelTopology").is_some()
        && parsed
            .get("weightsManifest")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|manifests| !manifests.is_empty())
        && fs::metadata(weights_path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
}

fn essentia_embedding_is_installed(models_path: &Path) -> bool {
    essentia_model_is_installed(
        models_path,
        EssentiaModelSpec {
            id: "musicnn_embedding",
            kind: "embedding",
            model_url: "",
            weights_url: None,
            classes: &[],
        },
    )
}

fn essentia_model_status_for_path(models_path: &Path) -> EssentiaModelStatus {
    let specs = essentia_model_specs();
    let embedding = essentia_embedding_is_installed(models_path);
    EssentiaModelStatus {
        version: ESSENTIA_MODEL_VERSION,
        embedding,
        genre: specs
            .iter()
            .any(|spec| spec.kind == "genre" && essentia_model_is_installed(models_path, *spec))
            && embedding,
        mood: specs
            .iter()
            .filter(|spec| spec.kind == "mood")
            .all(|spec| essentia_model_is_installed(models_path, *spec))
            && embedding,
        instrument: specs
            .iter()
            .any(|spec| spec.kind == "instrument" && essentia_model_is_installed(models_path, *spec))
            && embedding,
        downloading: false,
    }
}

fn download_essentia_model_file(url: &str, destination: &Path) -> Result<(), String> {
    let response = ureq::get(url)
        .set("User-Agent", "W4DJ-RKB")
        .call()
        .map_err(|error| format!("下载 Essentia 模型失败：{error}"))?;
    if response.status() != 200 {
        return Err(format!("下载 Essentia 模型失败：HTTP {}", response.status()));
    }
    let temporary = destination.with_extension(format!(
        "{}.part",
        destination.extension().and_then(|value| value.to_str()).unwrap_or("bin")
    ));
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取 Essentia 模型失败：{error}"))?;
    if bytes.is_empty() {
        return Err("下载的 Essentia 模型为空".to_string());
    }
    fs::write(&temporary, bytes).map_err(|error| format!("保存 Essentia 模型失败：{error}"))?;
    fs::rename(&temporary, destination)
        .map_err(|error| format!("安装 Essentia 模型失败：{error}"))
}

fn download_essentia_embedding_model(url: &str, models_path: &Path) -> Result<(), String> {
    let response = ureq::get(url)
        .set("User-Agent", "W4DJ-RKB")
        .call()
        .map_err(|error| format!("下载 Essentia MusiCNN 模型失败：{error}"))?;
    if response.status() != 200 {
        return Err(format!("下载 Essentia MusiCNN 模型失败：HTTP {}", response.status()));
    }
    let mut archive_bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut archive_bytes)
        .map_err(|error| format!("读取 Essentia MusiCNN 模型失败：{error}"))?;
    let entries = extract_malformed_essentia_zip_entries(&archive_bytes)?;
    let model_json = entries
        .first()
        .cloned()
        .ok_or_else(|| "Essentia MusiCNN 模型包缺少 model.json".to_string())?;
    let weight_data = entries
        .get(1)
        .cloned()
        .ok_or_else(|| "Essentia MusiCNN 模型包缺少权重".to_string())?;
    if model_json.is_empty() || weight_data.is_empty() {
        return Err("Essentia MusiCNN 模型包缺少 model.json 或权重".to_string());
    }
    let json_path = essentia_model_file_path(models_path, "musicnn_embedding", "json");
    let bin_path = essentia_model_file_path(models_path, "musicnn_embedding", "bin");
    let json_temp = json_path.with_extension("json.part");
    let bin_temp = bin_path.with_extension("bin.part");
    fs::write(&json_temp, model_json).map_err(|error| format!("保存 Essentia 模型结构失败：{error}"))?;
    fs::write(&bin_temp, weight_data).map_err(|error| format!("保存 Essentia 模型权重失败：{error}"))?;
    fs::rename(&json_temp, &json_path).map_err(|error| format!("安装 Essentia 模型结构失败：{error}"))?;
    fs::rename(&bin_temp, &bin_path).map_err(|error| format!("安装 Essentia 模型权重失败：{error}"))
}

/// The official Essentia MusiCNN archive currently contains a malformed local
/// extra-field length. Its central directory is therefore rejected by the
/// standard ZIP reader even though both deflate streams are intact. Read the
/// two local entries defensively and locate each stream by validating its
/// expected decompressed size. This keeps the downloader limited to the
/// official archive format without accepting arbitrary filesystem paths.
fn extract_malformed_essentia_zip_entries(archive: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut entries = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = archive[cursor..]
        .windows(4)
        .position(|window| window == b"PK\x03\x04")
    {
        let offset = cursor + relative;
        if offset + 30 > archive.len() {
            break;
        }
        let compression = u16::from_le_bytes([archive[offset + 8], archive[offset + 9]]);
        let compressed_size = u32::from_le_bytes([
            archive[offset + 18],
            archive[offset + 19],
            archive[offset + 20],
            archive[offset + 21],
        ]) as usize;
        let uncompressed_size = u32::from_le_bytes([
            archive[offset + 22],
            archive[offset + 23],
            archive[offset + 24],
            archive[offset + 25],
        ]) as usize;
        let name_length = u16::from_le_bytes([archive[offset + 26], archive[offset + 27]]) as usize;
        let extra_length = u16::from_le_bytes([archive[offset + 28], archive[offset + 29]]) as usize;
        let data_floor = offset
            .checked_add(30)
            .and_then(|value| value.checked_add(name_length))
            .and_then(|value| value.checked_add(extra_length))
            .ok_or_else(|| "Essentia 模型包的条目头损坏".to_string())?;
        if compression != 8 || compressed_size == 0 || uncompressed_size == 0 {
            cursor = offset + 30 + name_length;
            continue;
        }

        let mut decoded = None;
        let search_end = data_floor
            .saturating_add(64)
            .min(archive.len().saturating_sub(compressed_size));
        for data_start in data_floor..=search_end {
            let data_end = data_start + compressed_size;
            let mut decoder = DeflateDecoder::new(&archive[data_start..data_end]);
            let mut output = Vec::with_capacity(uncompressed_size);
            if decoder.read_to_end(&mut output).is_ok() && output.len() == uncompressed_size {
                decoded = Some(output);
                break;
            }
        }
        if let Some(output) = decoded {
            entries.push(output);
            if entries.len() == 2 {
                return Ok(entries);
            }
        }
        cursor = data_floor.saturating_add(compressed_size).min(archive.len());
    }
    Err("Essentia MusiCNN 模型包缺少可读取的 model.json 或权重".to_string())
}

#[tauri::command]
fn get_essentia_model_status(state: tauri::State<'_, AppState>) -> Result<EssentiaModelStatus, String> {
    let path = essentia_models_path(&state);
    if path.as_os_str().is_empty() {
        return Err("Essentia 模型目录尚未准备好".to_string());
    }
    Ok(essentia_model_status_for_path(&path))
}

#[tauri::command]
fn download_essentia_models(state: tauri::State<'_, AppState>) -> Result<EssentiaModelStatus, String> {
    let path = essentia_models_path(&state);
    if path.as_os_str().is_empty() {
        return Err("Essentia 模型目录尚未准备好".to_string());
    }
    fs::create_dir_all(&path).map_err(|error| format!("创建 Essentia 模型目录失败：{error}"))?;
    for spec in essentia_model_specs() {
        let model_path = essentia_model_file_path(&path, spec.id, "json");
        let weights_path = essentia_model_file_path(&path, spec.id, "bin");
        if spec.kind == "embedding" {
            if !essentia_model_is_installed(&path, spec) {
                download_essentia_embedding_model(spec.model_url, &path)?;
            }
            continue;
        }
        if !model_path.is_file() {
            download_essentia_model_file(spec.model_url, &model_path)?;
        }
        if !weights_path.is_file() {
            download_essentia_model_file(
                spec.weights_url.ok_or_else(|| "Essentia 模型缺少权重地址".to_string())?,
                &weights_path,
            )?;
        }
    }
    Ok(essentia_model_status_for_path(&path))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EssentiaModelFile {
    id: String,
    model_json: String,
    weight_data: Vec<u8>,
    classes: Vec<String>,
    kind: String,
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
        return Err("Essentia 模型尚未下载".to_string());
    }
    let model_json = fs::read_to_string(model_path)
        .map_err(|error| format!("读取 Essentia 模型结构失败：{error}"))?;
    let weight_data = fs::read(weights_path)
        .map_err(|error| format!("读取 Essentia 模型权重失败：{error}"))?;
    Ok(EssentiaModelFile {
        id: spec.id.to_string(),
        model_json,
        weight_data,
        classes: spec.classes.iter().map(|value| (*value).to_string()).collect(),
        kind: spec.kind.to_string(),
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
    let merged = merge_analysis_entries(existing, entries);
    let count = merged.len();
    save_analysis_file(&path, &merged)?;
    Ok(count)
}

#[tauri::command]
fn clear_track_analyses(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let path = current_analysis_path(&state);
    let _guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    clear_analysis_file(&path)
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
fn start_sync(
    slot_index: usize,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let controller = Arc::clone(&state.controller);
    let destination_coordinator = state.destination_coordinator.clone();
    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        if controller.is_running(slot_index)? {
            return Ok(controller.state().clone());
        }

        controller.start_sync(slot_index, 0)?;
        controller.push_log(slot_index, "Scanning input source")?;
    }

    thread::spawn(move || run_sync_task(controller, destination_coordinator, slot_index));

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
        thread::spawn(move || run_sync_task(controller, destination_coordinator, slot_index));
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

    let mut previews = slots
        .into_iter()
        .map(|(slot_index, source, destination)| {
            let preview = build_sync_preview_with_settings_and_netease(
                &source,
                &destination,
                mode,
                lossless_format,
                conflict_strategy,
                filename_rule,
                netease_filename_format,
            )
            .map_err(|error| format!("预检失败：{error}"))?;
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
        if matches!(progress.status, ScanStatus::Running) {
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
        tasks,
    } = job;
    let mut scan_cache = load_scan_cache(&scan_cache_path).unwrap_or_else(|_| ScanCache::empty());
    let mut total = 0;
    let mut task_totals = HashMap::<usize, usize>::new();
    for (_, source, destination) in &tasks {
        let scan_source = resolve_missing_single_source_path(Path::new(source))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| source.clone());
        let (source_count, cancelled) = count_music_files_with_cancel(
            &scan_source,
            w4dj::sync::SUPPORTED_SOURCE_EXTENSIONS,
            || scan_cancel.load(Ordering::SeqCst),
        );
        let mut task_total = source_count;
        if cancelled {
            finish_scan_cancelled(&progress, &scan_result);
            return;
        }
        let (destination_count, cancelled) = count_music_files_with_cancel(
            destination,
            &["mp3", "wav", "aiff"],
            || scan_cancel.load(Ordering::SeqCst),
        );
        task_total += destination_count;
        total += task_total;
        if let Some((slot_index, _, _)) = tasks.iter().find(|(_, task_source, task_destination)| {
            task_source == source && task_destination == destination
        }) {
            task_totals.insert(*slot_index, task_total);
        }
        if cancelled {
            finish_scan_cancelled(&progress, &scan_result);
            return;
        }
    }
    update_scan_progress(&progress, |state| {
        state.total = total;
        state.message = "正在准备扫描".to_string();
        for task in &mut state.tasks {
            task.total = task_totals.get(&task.slot_index).copied().unwrap_or(0);
        }
    });
    let mut previews = Vec::with_capacity(tasks.len());
    for (slot_index, source, destination) in tasks {
        if scan_cancel.load(Ordering::SeqCst) {
            finish_scan_cancelled(&progress, &scan_result);
            return;
        }

        let mut observer = |phase: ScanPhase, path: &Path| {
            if scan_cancel.load(Ordering::SeqCst) {
                return false;
            }
            update_scan_progress(&progress, |state| {
                state.phase = match phase {
                    ScanPhase::Source => ScanProgressPhase::ScanningSource,
                    ScanPhase::Destination => ScanProgressPhase::ScanningDestination,
                };
                state.processed = state.processed.saturating_add(1);
                state.current_file = path.display().to_string();
                state.message = match phase {
                    ScanPhase::Source => "正在扫描输入目录".to_string(),
                    ScanPhase::Destination => "正在扫描输出目录".to_string(),
                };
                if let Some(task) = state
                    .tasks
                    .iter_mut()
                    .find(|task| task.slot_index == slot_index)
                {
                    task.phase = state.phase.clone();
                    task.processed = task.processed.saturating_add(1);
                    task.current_file = path.display().to_string();
                }
            });
            !scan_cancel.load(Ordering::SeqCst)
        };
        let preview = match build_sync_preview_with_settings_and_netease_observed_with_cache(
            &source,
            &destination,
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
            netease_filename_format,
            Some(&mut observer),
            &mut scan_cache,
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
            if let Some(task) = state
                .tasks
                .iter_mut()
                .find(|task| task.slot_index == slot_index)
            {
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
        state.message = "扫描完成".to_string();
    });
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
    let requested_analyses = analyses.unwrap_or_default();
    let requested_analysis_failures = analysis_failures.unwrap_or_default();
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
            let slot_index = slot_preview.slot_index;
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

    let test_monitor = if DEBUG_TEST_MONITOR_ENABLED && !jobs.is_empty() {
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
                eprintln!("Failed to initialize local test monitor: {error}");
                None
            }
        }
    } else {
        None
    };

    if monitor_needs_analysis && let Some(monitor) = test_monitor.as_ref() {
        state
            .test_monitors
            .lock()
            .expect("test monitor map lock poisoned")
            .insert(batch_id.clone(), Arc::clone(monitor));
    }

    for mut job in jobs {
        job.test_monitor = test_monitor.clone();
        let controller = Arc::clone(&state.controller);
        let destination_coordinator = destination_coordinator.clone();
        let history_path = history_path.clone();
        let history_write_lock = Arc::clone(&history_write_lock);
        thread::spawn(move || {
            run_confirmed_sync_task(
                controller,
                destination_coordinator,
                history_path,
                history_write_lock,
                job,
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

    let analysis_lookup = analyses
        .iter()
        .map(|analysis| (analysis.path.clone(), embedded_analysis_from_track(analysis)))
        .collect::<HashMap<_, _>>();
    let failure_lookup = analysis_failures
        .iter()
        .map(|failure| (failure.path.clone(), failure.message.clone()))
        .collect::<HashMap<_, _>>();
    let mut reports = Vec::new();

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
            };

            if let Some(message) = failure_lookup.get(&candidate.source_path) {
                let mut report = base_report();
                report.status = String::from("failed");
                report.message = Some(message.clone());
                reports.push(report);
                continue;
            }

            let Some(analysis) = analysis_lookup.get(&candidate.source_path) else {
                let mut report = base_report();
                report.status = String::from("failed");
                report.message = Some(String::from("未收到该歌曲的 Essentia 分析结果"));
                reports.push(report);
                continue;
            };

            let mut report = base_report();
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

            if !destination_path.is_file() {
                report.status = String::from("failed");
                report.message = Some(String::from("转换输出不存在，未执行分析元数据回写"));
                reports.push(report);
                continue;
            }

            let result = update_existing_metadata_transactionally(
                Path::new(&candidate.source_path),
                destination_path,
                slot_preview.netease_filename_format,
                |temporary_output| apply_track_analysis_metadata(temporary_output, analysis),
            );
            if let Err(error) = result {
                report.status = String::from("failed");
                report.message = Some(format!("分析元数据回写失败：{error}"));
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

    let monitor = if DEBUG_TEST_MONITOR_ENABLED {
        state
            .test_monitors
            .lock()
            .expect("test monitor map lock poisoned")
            .get(&batch_id)
            .cloned()
    } else {
        None
    };
    if let Some(monitor) = monitor {
        monitor.record_analysis_reports(&reports);
        state
            .test_monitors
            .lock()
            .expect("test monitor map lock poisoned")
            .remove(&batch_id);
    }

    if !reports.is_empty()
        && let Ok(entries) = load_history_file(&history_path)
    {
        for entry in entries.iter().filter(|entry| entry.batch_id == batch_id) {
                if let Some(report_path) = entry.report_path.as_deref()
                    && let Err(error) = fs::write(report_path, format_error_report(entry))
                {
                    eprintln!("Failed to update enhanced analysis report: {error}");
                }
            }
    }

    Ok(state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone())
}

#[tauri::command]
fn load_history(state: tauri::State<'_, AppState>) -> Result<Vec<HistoryEntry>, String> {
    let history_path = state
        .history_path
        .lock()
        .expect("history path lock poisoned")
        .clone();
    let _history_guard = state
        .history_write_lock
        .lock()
        .expect("history write lock poisoned");
    load_history_file(history_path).map_err(|error| format!("读取转换历史失败：{error}"))
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
    const PROJECT_URL: &str = "https://github.com/komakizhu/W4DJ-RKB";
    if url != PROJECT_URL && !url.starts_with("https://github.com/komakizhu/W4DJ-RKB/releases/") {
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

fn library_catalog_path(state: &tauri::State<'_, AppState>) -> PathBuf {
    state
        .library_catalog_path
        .lock()
        .expect("library catalog path lock poisoned")
        .clone()
}

fn open_library_catalog(path: &Path) -> Result<(LibraryCatalog, Option<PathBuf>), String> {
    LibraryCatalog::open_or_recover(path).map_err(|error| error.to_string())
}

#[tauri::command]
fn load_library_status(state: tauri::State<'_, AppState>) -> Result<LibraryStatus, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_library_catalog(&path)?;
    let netease = discover_netease_library();
    let track_count = catalog
        .count_tracks()
        .map_err(|error| error.to_string())?
        .max(0) as u64;
    Ok(LibraryStatus {
        catalog_path: path.display().to_string(),
        track_count,
        netease,
    })
}

#[tauri::command]
fn locate_netease_library() -> NeteaseDiscovery {
    discover_netease_library()
}

#[tauri::command]
fn refresh_library_catalog(
    state: tauri::State<'_, AppState>,
) -> Result<LibraryRefreshSummary, String> {
    let discovery = discover_netease_library();
    let database_path = discovery
        .database_path
        .as_deref()
        .ok_or_else(|| "未找到网易云音乐本地数据库，请手动选择歌曲来源".to_string())?;
    let path = library_catalog_path(&state);
    let (mut catalog, _) = open_library_catalog(&path)?;
    let previous_files = discovery
        .music_folder
        .as_deref()
        .map(|folder| {
            let mut files = Vec::new();
            let _ = collect_analyzable_audio_files(folder, &mut files);
            files
                .iter()
                .filter_map(|file| {
                    catalog
                        .local_file_by_path(Path::new(file))
                        .ok()
                        .flatten()
                })
                .count()
        })
        .unwrap_or_default();
    let snapshot = build_catalog_snapshot_incremental(
        database_path,
        discovery.music_folder.as_deref(),
        Some(&catalog),
    )
    .map_err(|error| error.to_string())?;
    let local_file_count = snapshot.local_files.len();
    let readable_file_count = snapshot
        .local_files
        .iter()
        .filter(|file| file.readable)
        .count();
    catalog
        .upsert_snapshot(&snapshot)
        .map_err(|error| error.to_string())?;

    let analysis_path = {
        let history_path = state
            .history_path
            .lock()
            .expect("history path lock poisoned")
            .clone();
        analysis_file_path(&history_path)
    };
    if let Ok(entries) = load_analysis_file(&analysis_path) {
        catalog
            .apply_analysis_entries(&entries)
            .map_err(|error| error.to_string())?;
    }
    let track_count = catalog
        .count_tracks()
        .map_err(|error| error.to_string())?
        .max(0) as u64;
    Ok(LibraryRefreshSummary {
        track_count,
        local_file_count,
        readable_file_count,
        reused_file_count: previous_files,
        database_path: database_path.display().to_string(),
        music_folder: discovery
            .music_folder
            .as_ref()
            .map(|path| path.display().to_string()),
    })
}

#[tauri::command]
fn query_library_catalog(
    query: LibraryQuery,
    state: tauri::State<'_, AppState>,
) -> Result<LibraryPage, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_library_catalog(&path)?;
    catalog.query(&query).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_library_track_detail(
    track_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<w4dj::library_catalog::CatalogTrack>, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_library_catalog(&path)?;
    catalog
        .track_detail(&track_key)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_library_track_source_records(
    track_key: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<CatalogSourceRecord>, String> {
    let path = library_catalog_path(&state);
    let (catalog, _) = open_library_catalog(&path)?;
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
    let (catalog, _) = open_library_catalog(&path)?;
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
    let entry = load_history_file(history_path)
        .map_err(|error| format!("无法读取转换历史：{error}"))?
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| String::from("找不到对应的转换历史"))?;

    fs::write(path, format_error_report(&entry))
        .map_err(|error| format!("错误报告保存失败：{error}"))
}

fn main() {
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
            scan_cache_path: Arc::new(Mutex::new(PathBuf::new())),
            library_catalog_path: Arc::new(Mutex::new(PathBuf::new())),
            history_write_lock: Arc::new(Mutex::new(())),
            destination_coordinator: DestinationCoordinator::default(),
            scan_progress: Arc::new(Mutex::new(ScanProgress::default())),
            scan_cancel: Arc::new(AtomicBool::new(false)),
            scan_result: Arc::new(Mutex::new(None)),
            test_monitor_path: Arc::new(Mutex::new(PathBuf::new())),
            test_monitors: Arc::new(Mutex::new(HashMap::new())),
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
            retry_history_failures,
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
            download_essentia_models,
            load_essentia_model,
            export_rekordbox_xml,
            load_library_status,
            locate_netease_library,
            refresh_library_catalog,
            query_library_catalog,
            get_library_track_detail,
            get_library_track_source_records,
            get_library_track_cover,
            clear_library_catalog_cache
        ])
        .setup(|app| {
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
            let scan_cache_path = preferences_path
                .parent()
                .expect("preferences path should have a parent")
                .join("scan-cache.json");
            let library_catalog_path = preferences_path
                .parent()
                .expect("preferences path should have a parent")
                .join("library-dashboard.sqlite3");
            let test_monitor_path = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| default_download_directory())
                .join(TEST_MONITOR_DIRECTORY);

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
                *path_guard = history_path;
            }

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .models_path
                    .lock()
                    .expect("models path lock poisoned");
                *path_guard = models_path;
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
                    .library_catalog_path
                    .lock()
                    .expect("library catalog path lock poisoned");
                *path_guard = library_catalog_path;
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
                let preferences = load_preferences(&preferences_path)
                    .unwrap_or_else(|_| AppPreferences::default());
                let state = app.state::<AppState>();
                let mut controller = state.controller.lock().expect("desktop lock poisoned");
                controller.apply_preferences(preferences);
            }

            #[cfg(target_os = "macos")]
            {
                let window = app
                    .get_webview_window("main")
                    .expect("main window should exist");

                apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::HudWindow,
                    Some(NSVisualEffectState::Active),
                    Some(18.0),
                )
                .expect("failed to apply macOS vibrancy");

                window.center().expect("failed to center main window");
                window.show().expect("failed to show main window");
                window.set_focus().expect("failed to focus main window");
            }

            #[cfg(not(target_os = "macos"))]
            {
                let window = app
                    .get_webview_window("main")
                    .expect("main window should exist");

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
    let preferences = {
        let controller = state.controller.lock().expect("desktop lock poisoned");
        controller.state().preferences()
    };

    let preferences_path = state
        .preferences_path
        .lock()
        .expect("preferences path lock poisoned")
        .clone();

    if preferences_path.as_os_str().is_empty() {
        return;
    }

    if let Err(error) = save_preferences(&preferences_path, &preferences) {
        eprintln!("Failed to save preferences: {}", error);
    }
}

fn apply_analysis_for_candidate_to_path(
    candidate: &PreviewCandidate,
    output_path: &Path,
    analyses: &HashMap<String, EmbeddedAnalysis>,
) -> io::Result<()> {
    let Some(analysis) = analyses.get(&candidate.source_path) else {
        return Ok(());
    };
    apply_track_analysis_metadata(output_path, analysis)
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
    }));
    persist_recovery_entry(&history_path, &history_write_lock, &recovery_entry);

    let mut setup_error: Option<String> = None;
    if let Err(error) = validate_source_input(&job.source) {
        setup_error = Some(error);
    } else if let Err(error) = fs::create_dir_all(&job.destination) {
        setup_error = Some(format!("无法创建输出目录：{error}"));
    }

    if setup_error.is_none() {
        let destination_lock = destination_coordinator.lock_for(Path::new(&job.destination));
        let _destination_guard = destination_lock
            .lock()
            .expect("destination sync lock poisoned");

        if let Err(error) = cleanup_temporary_outputs(&job.destination) {
            setup_error = Some(format!("无法清理临时文件：{error}"));
        } else {
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

                let result = update_existing_metadata_transactionally(
                    Path::new(&candidate.source_path),
                    Path::new(&candidate.destination_path),
                    job.netease_filename_format,
                    |temporary_output| {
                        apply_analysis_for_candidate_to_path(
                            candidate,
                            temporary_output,
                            &analysis_lookup,
                        )
                    },
                );
                let mut controller_guard = controller.lock().expect("desktop lock poisoned");
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
                record_metadata_diagnostic(&recovery_entry, candidate);
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
                sync_music_library_transactional_with_observer(
                    &queued_files,
                    &job.destination,
                    &job.mode,
                    job.lossless_format,
                    job.netease_filename_format,
                    &task_controller,
                    |name, temporary_output| {
                        let Some(candidate) = candidate_lookup.get(name) else {
                            return Ok(());
                        };
                        apply_analysis_for_candidate_to_path(
                            candidate,
                            temporary_output,
                            &analysis_lookup,
                        )
                    },
                    |name, task, error| {
                        let candidate = candidate_lookup.get(name);
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
                            record_metadata_diagnostic(&recovery_entry, candidate);
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
                )
            };

            let mut controller_guard = controller.lock().expect("desktop lock poisoned");
            match sync_result {
                Ok(snapshot) => controller_guard
                    .finish_sync(job.slot_index, snapshot)
                    .expect("confirmed slot index should be valid"),
                Err(error) => controller_guard
                    .fail_sync(job.slot_index, format!("导出失败：{error}"))
                    .expect("confirmed slot index should be valid"),
            }
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
        fail_sync(&controller, job.slot_index, error);
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
    let mut history_entry = HistoryEntry {
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
    };

    let report_path = automatic_report_path(&history_path, &history_entry);
    history_entry.report_path = Some(report_path.display().to_string());
    if let Err(error) = fs::create_dir_all(
        report_path
            .parent()
            .expect("automatic report path should have a parent"),
    ) {
        history_entry.report_path = None;
        eprintln!("Failed to create automatic report directory: {error}");
    } else if let Err(error) = fs::write(&report_path, format_error_report(&history_entry)) {
        history_entry.report_path = None;
        eprintln!("Failed to save automatic conversion report: {error}");
    }

    if let Some(monitor) = job.test_monitor.as_ref() {
        monitor.record_task_finished(&history_entry);
    }

    let _history_guard = history_write_lock
        .lock()
        .expect("history write lock poisoned");
    if let Err(error) = upsert_history(history_path, history_entry) {
        eprintln!("Failed to save conversion history: {}", error);
    }
}

fn record_metadata_diagnostic(
    recovery_entry: &Arc<Mutex<HistoryEntry>>,
    candidate: &PreviewCandidate,
) {
    let diagnostic = inspect_metadata_diagnostic(
        Path::new(&candidate.source_path),
        Path::new(&candidate.destination_path),
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

fn automatic_report_path(history_path: &Path, entry: &HistoryEntry) -> PathBuf {
    let report_directory = history_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("conversion-reports");
    let safe_id: String = entry
        .id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    report_directory.join(format!(
        "W4DJ-RKB-{safe_id}-task-{}.txt",
        entry.slot_index + 1
    ))
}

fn pending_file_from_candidate(candidate: &PreviewCandidate) -> PendingFile {
    PendingFile {
        name: candidate.name.clone(),
        source_path: candidate.source_path.clone(),
        destination_path: candidate.destination_path.clone(),
        source_size_bytes: candidate.source_size_bytes,
        estimated_output_bytes: candidate.estimated_output_bytes,
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
    slot_index: usize,
) {
    let (source, destination, using_fallback, mode, lossless_format, task_controller) = {
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
    let _destination_guard = destination_lock
        .lock()
        .expect("destination sync lock poisoned");

    if let Err(error) = cleanup_temporary_outputs(&destination) {
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
    let (mut source_files, scan_issues) = get_music_dict_with_scan_issues(&source);
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
    let destination_files = get_destination_music_dict(&destination);
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
    let result = sync_music_library_with_observer(
        &queued_files,
        &destination,
        &mode,
        lossless_format,
        &task_controller,
        |name, task, error| {
            if error.is_some() {
                failed_files += 1;
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
    );

    let mut controller = controller.lock().expect("desktop lock poisoned");
    if failed_files > 0 {
        controller
            .push_log(
                slot_index,
                format!("Failed {} file(s) during sync", failed_files),
            )
            .expect("sync slot index validated before worker start");
    }
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
    use super::TestMonitor;
    use super::DestinationCoordinator;
    use super::apply_preflight_summary;
    use super::collect_processable_previews;
    use super::deduplicate_cross_slot_candidates;
    use super::history_status_for;
    use super::validate_destination_directory;
    use super::validate_scan_previews;
    use super::validate_source_input;
    use super::validate_unique_planned_outputs;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use w4dj::config::Mode;
    use w4dj::desktop::{DesktopController, DesktopState};
    use w4dj::history::{FailedFile, HistoryStatus};
    use w4dj::preferences::{AppPreferences, SyncSlotPreferences};
    use w4dj::preview::{PreviewCandidate, PreviewIssue, SlotPreview, SyncPreview};
    use w4dj::task::TaskController;

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
                        operation: Default::default(),
                    }]
                } else {
                    Vec::new()
                },
                skipped: Vec::new(),
                errors: Vec::new(),
                warnings: Vec::new(),
                available_space_bytes: None,
                disk_space_sufficient: None,
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

        let event_text = fs::read_to_string(monitor.session_dir.join("events.jsonl")).unwrap();
        assert!(event_text.contains("candidate_result"));
        assert!(event_text.contains("/music/in/song.mp3"));

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
}
