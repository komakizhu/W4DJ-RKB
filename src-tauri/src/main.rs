#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
    FailedFile, HistoryEntry, HistoryStatus, PendingFile, classify_error, clear_history,
    delete_history_entry, format_error_report, load_history as load_history_file, upsert_history,
};
use w4dj::preferences::{AppPreferences, load_preferences, save_preferences};
use w4dj::preview::{
    PreviewCandidate, PreviewIssue, PreviewOperation, SlotPreview, SyncPreview,
    build_retry_preview, build_sync_preview_with_settings_and_netease,
    build_sync_preview_with_settings_and_netease_observed,
};
use w4dj::sync::{
    apply_track_analysis_metadata, cleanup_temporary_outputs, compare_music_dicts,
    count_music_files_with_cancel,
    EmbeddedAnalysis, inspect_metadata_diagnostic,
    get_destination_music_dict, get_music_dict_with_scan_issues, is_supported_source_file,
    sync_music_library_with_observer, update_existing_metadata, ScanPhase,
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
    history_write_lock: Arc<Mutex<()>>,
    destination_coordinator: DestinationCoordinator,
    scan_progress: Arc<Mutex<ScanProgress>>,
    scan_cancel: Arc<AtomicBool>,
    scan_result: Arc<Mutex<Option<Vec<SlotPreview>>>>,
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
    candidates: Vec<PreviewCandidate>,
    analyses: Vec<EmbeddedAnalysis>,
    analysis_failures: Vec<AnalysisFailure>,
    preview: SyncPreview,
    retry_of: Option<String>,
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
    tasks: Vec<(usize, String, String)>,
}

#[derive(serde::Serialize)]
struct AppInfo {
    version: &'static str,
    developer: &'static str,
    project_url: &'static str,
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
        (
            controller.state().conversion_mode,
            controller.state().mode,
            controller.state().lossless_format,
            controller.state().conflict_strategy,
            controller.state().filename_rule,
            controller.state().netease_filename_format,
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
        tasks,
    } = job;
    let mut total = 0;
    for (_, source, destination) in &tasks {
        let (source_count, cancelled) = count_music_files_with_cancel(
            source,
            w4dj::sync::SUPPORTED_SOURCE_EXTENSIONS,
            || scan_cancel.load(Ordering::SeqCst),
        );
        total += source_count;
        if cancelled {
            finish_scan_cancelled(&progress, &scan_result);
            return;
        }
        let (destination_count, cancelled) = count_music_files_with_cancel(
            destination,
            &["mp3", "wav", "aiff"],
            || scan_cancel.load(Ordering::SeqCst),
        );
        total += destination_count;
        if cancelled {
            finish_scan_cancelled(&progress, &scan_result);
            return;
        }
    }
    update_scan_progress(&progress, |state| {
        state.total = total;
        state.message = "正在准备扫描".to_string();
    });
    let mut previews = Vec::with_capacity(tasks.len());
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
        });
        !scan_cancel.load(Ordering::SeqCst)
    };

    for (slot_index, source, destination) in tasks {
        if scan_cancel.load(Ordering::SeqCst) {
            finish_scan_cancelled(&progress, &scan_result);
            return;
        }

        let preview = match build_sync_preview_with_settings_and_netease_observed(
            &source,
            &destination,
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
            netease_filename_format,
            Some(&mut observer),
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
    for preview in previews {
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
        if preview.preview.candidates.is_empty() {
            return Err(format!("任务 {} 没有可转换的歌曲", preview.slot_index + 1));
        }
    }
    validate_unique_planned_outputs(previews)
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
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    if previews.is_empty() {
        return Err(String::from("没有可处理的转换任务"));
    }

    let batch_id = format!("batch-{}", unique_timestamp());
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

    {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        let state_mode = controller.state().mode;
        let state_lossless_format = controller.state().lossless_format;
        let enhanced_mode = controller.state().enhanced_mode;
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
            if !is_history_retry
                && (slot_preview.mode != state_mode
                    || slot_preview.lossless_format != state_lossless_format
                    || slot_preview.conflict_strategy != state_conflict_strategy
                    || slot_preview.filename_rule != state_filename_rule
                    || slot_preview.netease_filename_format != state_netease_filename_format
                    || slot_preview.preview.source_directory != slot.source_directory
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

        validate_unique_planned_outputs(&validated_previews)?;

        let allow_error_only_retry = retry_of.is_some()
            || validated_previews
                .iter()
                .any(|preview| preview.retry_of.is_some());
        let processable_previews =
            collect_processable_previews(validated_previews.clone(), allow_error_only_retry)?;

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

    for job in jobs {
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
        netease_filename_format: NeteaseFilenameFormat::default(),
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
            history_write_lock: Arc::new(Mutex::new(())),
            destination_coordinator: DestinationCoordinator::default(),
            scan_progress: Arc::new(Mutex::new(ScanProgress::default())),
            scan_cancel: Arc::new(AtomicBool::new(false)),
            scan_result: Arc::new(Mutex::new(None)),
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
            start_confirmed_sync,
            load_history,
            retry_history_failures,
            export_history_error_report,
            delete_history_entry_command,
            clear_history_command,
            app_info,
            check_for_updates,
            open_external_url,
            open_destination,
            list_audio_files,
            read_audio_file,
            read_audio_metadata,
            get_audio_file_fingerprint,
            load_track_analyses,
            save_track_analyses,
            clear_track_analyses,
            export_rekordbox_xml
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

fn apply_analysis_for_candidate(
    candidate: &PreviewCandidate,
    analyses: &HashMap<String, EmbeddedAnalysis>,
) -> io::Result<()> {
    let Some(analysis) = analyses.get(&candidate.source_path) else {
        return Ok(());
    };
    apply_track_analysis_metadata(Path::new(&candidate.destination_path), analysis)
}

fn embedded_analysis_from_track(analysis: &TrackAnalysis) -> EmbeddedAnalysis {
    EmbeddedAnalysis {
        path: analysis.path.clone(),
        title: analysis.title.clone(),
        artist: analysis.artist.clone(),
        album: analysis.album.clone(),
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
        report_path: None,
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

                let result = update_existing_metadata(
                    Path::new(&candidate.source_path),
                    Path::new(&candidate.destination_path),
                )
                .and_then(|()| apply_analysis_for_candidate(candidate, &analysis_lookup));
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
                sync_music_library_with_observer(
                    &queued_files,
                    &job.destination,
                    &job.mode,
                    job.lossless_format,
                    &task_controller,
                    |name, task, error| {
                        let candidate = candidate_lookup.get(name);
                        let analysis_error = if error.is_none() {
                            candidate.and_then(|candidate| {
                                apply_analysis_for_candidate(candidate, &analysis_lookup).err()
                            })
                        } else {
                            None
                        };
                        let failed_file = if let Some(error) = error.or(analysis_error.as_ref()) {
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
        report_path: None,
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
    use super::DestinationCoordinator;
    use super::apply_preflight_summary;
    use super::collect_processable_previews;
    use super::deduplicate_cross_slot_candidates;
    use super::history_status_for;
    use super::validate_destination_directory;
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
