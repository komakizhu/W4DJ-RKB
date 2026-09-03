use crate::analysis::{DropAnalysisDetails, HighLevelAnalysis};
use crate::concurrency::{ConcurrencyPermit, GlobalConcurrencyBudget};
use crate::config::{
    FilenameNormalizationPolicy, FilenameRule, LosslessFormat, Mode, NeteaseFilenameFormat,
};
use crate::metadata::build_id3_tag_from_flac;
#[cfg(feature = "ncm-decryption")]
use crate::metadata::{FlacMetadata, Metadata, Mp3Metadata, build_id3_tag};
use crate::netease::{NeteaseMetadataResolver, NeteaseRecoveryDiagnostic, RecoveredMetadata};
use crate::scan_cache::{ScanCache, ScanCacheEntry, can_reuse_derived_name_entry_normalized};
use crate::task::{TaskController, TaskSnapshot};
use id3::frame::{Comment, ExtendedText, Lyrics, Picture};
use id3::{TagLike, Version};
#[cfg(feature = "ncm-decryption")]
use ncmdump::Ncmdump;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs::{self, File};
use std::io::{self, Error, ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(target_os = "macos")]
use std::os::macos::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "windows")]
use std::os::windows::fs::MetadataExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;
use std::time::UNIX_EPOCH;

#[cfg(feature = "ncm-decryption")]
pub const SUPPORTED_SOURCE_EXTENSIONS: &[&str] = &["mp3", "flac", "ncm", "wav", "aiff"];
#[cfg(not(feature = "ncm-decryption"))]
pub const SUPPORTED_SOURCE_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "aiff"];

pub const NCM_DECRYPTION_UNAVAILABLE_MESSAGE: &str =
    "此 Legacy 版本不包含 NCM 解密功能，请先使用标准版处理 .ncm 文件";

pub fn ncm_decryption_available() -> bool {
    cfg!(feature = "ncm-decryption")
}

#[cfg(not(feature = "ncm-decryption"))]
fn ncm_decryption_unavailable_error() -> io::Error {
    Error::new(ErrorKind::Unsupported, NCM_DECRYPTION_UNAVAILABLE_MESSAGE)
}
const SOURCE_ENTRY_KEY_SEPARATOR: char = '\u{1f}';

/// Return the presentation name from a source-scan key. The suffix is an
/// internal collision discriminator and must never become an output filename
/// or metadata field.
pub fn source_entry_name(value: &str) -> &str {
    value
        .split_once(SOURCE_ENTRY_KEY_SEPARATOR)
        .map(|(name, _)| name)
        .unwrap_or(value)
}

fn unique_source_entry_key(
    name: &str,
    path: &Path,
    entries: &HashMap<String, (String, PathBuf)>,
) -> String {
    if !entries.contains_key(name) {
        return name.to_string();
    }
    let normalized = path.to_string_lossy().into_owned();
    format!("{name}{SOURCE_ENTRY_KEY_SEPARATOR}{normalized}")
}

/// Registry of FFmpeg children owned by the current application.  Cancelling
/// a task can kill only children started by W4DJ; unrelated user processes are
/// never touched.
#[derive(Debug, Default)]
pub struct ActiveFfmpegRegistry {
    next_id: AtomicU64,
    children: Mutex<HashMap<u64, Child>>,
    output_locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl ActiveFfmpegRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_count(&self) -> usize {
        self.children
            .lock()
            .expect("FFmpeg registry lock poisoned")
            .len()
    }

    pub fn terminate_all(&self) {
        if let Ok(mut children) = self.children.lock() {
            for child in children.values_mut() {
                let _ = child.kill();
            }
        }
    }

    fn insert(&self, child: Child) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.children
            .lock()
            .expect("FFmpeg registry lock poisoned")
            .insert(id, child);
        id
    }

    /// Return a lock for one destination path.  The conversion coordinator
    /// uses this instead of locking an entire destination directory, so two
    /// different output files can be encoded concurrently while an identical
    /// target path is still protected from a cross-slot race.
    fn lock_for_output(&self, output_path: &Path) -> Arc<Mutex<()>> {
        let key = fs::canonicalize(output_path).unwrap_or_else(|_| {
            let parent = output_path
                .parent()
                .and_then(|parent| fs::canonicalize(parent).ok())
                .unwrap_or_else(|| {
                    output_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .to_path_buf()
                });
            output_path
                .file_name()
                .map(|name| parent.join(name))
                .unwrap_or_else(|| output_path.to_path_buf())
        });
        let mut locks = self
            .output_locks
            .lock()
            .expect("FFmpeg output lock map poisoned");
        Arc::clone(locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
    }

    fn wait_for(&self, id: u64, cancelled: &AtomicBool) -> io::Result<ExitStatus> {
        loop {
            let mut children = self
                .children
                .lock()
                .map_err(|_| io::Error::other("FFmpeg registry lock poisoned"))?;
            let Some(child) = children.get_mut(&id) else {
                return Err(io::Error::other("FFmpeg child was not registered"));
            };
            if cancelled.load(Ordering::SeqCst) {
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    children.remove(&id);
                    return Ok(status);
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    children.remove(&id);
                    return Err(error);
                }
            }
            drop(children);
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Per-track metadata evidence saved in the conversion report.  It is kept
/// deliberately textual so that users can attach it to a bug report without
/// needing the original music files.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct MetadataDiagnostic {
    pub source_path: String,
    pub destination_path: String,
    pub source_filename: String,
    pub source_extension: String,
    pub source_size_bytes: Option<u64>,
    pub output_size_bytes: Option<u64>,
    pub source_title: Option<String>,
    pub source_artist: Option<String>,
    pub source_album: Option<String>,
    pub output_title: Option<String>,
    pub output_artist: Option<String>,
    pub output_album: Option<String>,
    pub source_artwork: bool,
    pub output_artwork: Option<bool>,
    pub detected_filename_layout: String,
    pub decision: String,
    pub metadata_validation: String,
    #[serde(default)]
    pub validation_basis: Option<String>,
    #[serde(default)]
    pub output_tags_match: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_difference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_difference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_difference: Option<String>,
    #[serde(default)]
    pub netease_recovery: Option<NeteaseRecoveryDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct ConversionMetadataContext {
    pub netease: Arc<NeteaseMetadataResolver>,
}

impl Default for ConversionMetadataContext {
    fn default() -> Self {
        Self {
            netease: Arc::new(NeteaseMetadataResolver::load(None).unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ScanPhase {
    Source,
    Destination,
    Metadata,
}

pub type ScanObserver<'a> = dyn FnMut(ScanPhase, &Path) -> bool + 'a;

/// The result of the directory walk used by the preview scanner.  Keeping
/// enumeration separate from file-level work lets the coordinator reuse the
/// exact same path list for totals and scanning instead of walking an input
/// directory a second time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumeratedMusicFiles {
    pub paths: Vec<PathBuf>,
    pub snapshots: Vec<ScannedFileSnapshot>,
    pub issues: Vec<MusicScanIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFileSnapshot {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_at_ms: Option<u64>,
    pub source_extension: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEnumerationError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for ScanEnumerationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("扫描已取消"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ScanEnumerationError {}

/// Enumerate supported music files once.  The total is intentionally unknown
/// while WalkDir is active (a directory can be very large); the final
/// callback reports the exact count after the walk completes.
pub fn enumerate_music_files_observed<F>(
    folder: &str,
    allowed_extensions: &[&str],
    cancel: &AtomicBool,
    mut observe: F,
) -> Result<EnumeratedMusicFiles, ScanEnumerationError>
where
    F: FnMut(usize, Option<usize>, &Path),
{
    if folder.trim().is_empty() {
        return Ok(EnumeratedMusicFiles {
            paths: Vec::new(),
            snapshots: Vec::new(),
            issues: Vec::new(),
        });
    }
    let source_path = Path::new(folder);
    let mut paths = Vec::new();
    let mut snapshots = Vec::new();
    let mut issues = Vec::new();
    if source_path.is_file() {
        if cancel.load(Ordering::SeqCst) {
            return Err(ScanEnumerationError::Cancelled);
        }
        if is_supported_source_file_with_extensions(source_path, allowed_extensions) {
            let path = source_path.to_path_buf();
            let metadata = fs::metadata(&path).ok();
            paths.push(path.clone());
            snapshots.push(ScannedFileSnapshot {
                source_extension: path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_lowercase(),
                path: path.clone(),
                size_bytes: metadata.as_ref().map_or(0, fs::Metadata::len),
                modified_at_ms: metadata.as_ref().and_then(metadata_modified_at_ms),
            });
            observe(1, Some(1), source_path);
        }
        return Ok(EnumeratedMusicFiles {
            paths,
            snapshots,
            issues,
        });
    }
    if !source_path.exists() {
        return Err(ScanEnumerationError::Failed(format!(
            "输入目录不存在：{}",
            source_path.display()
        )));
    }

    for entry_result in walkdir::WalkDir::new(folder)
        .into_iter()
        .filter_entry(|entry| !is_hidden_path(entry.path()))
    {
        if cancel.load(Ordering::SeqCst) {
            return Err(ScanEnumerationError::Cancelled);
        }
        match entry_result {
            Ok(entry) => {
                if entry.file_type().is_file()
                    && !is_ignored_music_file(entry.path())
                    && has_allowed_extension(entry.path(), allowed_extensions)
                {
                    let path = entry.path().to_path_buf();
                    let metadata = entry.metadata().ok();
                    paths.push(path.clone());
                    snapshots.push(ScannedFileSnapshot {
                        source_extension: path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .unwrap_or_default()
                            .to_lowercase(),
                        path,
                        size_bytes: metadata.as_ref().map_or(0, fs::Metadata::len),
                        modified_at_ms: metadata.as_ref().and_then(metadata_modified_at_ms),
                    });
                    observe(paths.len(), None, entry.path());
                }
            }
            Err(error) => {
                if let Some(path) = error.path().filter(|path| !is_ignored_music_file(path)) {
                    issues.push(MusicScanIssue {
                        path: path.to_path_buf(),
                        message: format!("无法扫描歌曲文件：{error}"),
                    });
                }
            }
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(ScanEnumerationError::Cancelled);
    }
    if let Some(last) = paths.last() {
        observe(paths.len(), Some(paths.len()), last);
    } else {
        observe(0, Some(0), source_path);
    }
    Ok(EnumeratedMusicFiles {
        paths,
        snapshots,
        issues,
    })
}

fn is_supported_source_file_with_extensions(path: &Path, allowed_extensions: &[&str]) -> bool {
    path.is_file()
        && !is_ignored_music_file(path)
        && has_allowed_extension(path, allowed_extensions)
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct EmbeddedAnalysis {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub scale: Option<String>,
    pub key_strength: Option<f64>,
    pub integrated_loudness_lufs: Option<f64>,
    pub loudness_range_lu: Option<f64>,
    pub energy: Option<f64>,
    pub danceability: Option<f64>,
    pub beat_positions: Vec<f64>,
    pub analyzer: String,
    pub analysis_version: String,
    pub drop_loudness_lufs: Option<f64>,
    pub drop_analysis: Option<DropAnalysisDetails>,
    pub high_level: Option<HighLevelAnalysis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputPolicy {
    pub output_extension: &'static str,
    pub target_profile: TargetProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetProfile {
    CompatMp3,
    LosslessWav,
    LosslessAiff,
}

impl TargetProfile {
    fn output_extension(self) -> &'static str {
        match self {
            TargetProfile::CompatMp3 => "mp3",
            TargetProfile::LosslessWav => "wav",
            TargetProfile::LosslessAiff => "aiff",
        }
    }
}

pub fn resolve_output_policy(
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    source_extension: &str,
) -> OutputPolicy {
    let source_extension = source_extension.trim().to_lowercase();

    match mode {
        Mode::Compat => OutputPolicy {
            output_extension: "mp3",
            target_profile: TargetProfile::CompatMp3,
        },
        Mode::Lossless if source_extension == "mp3" => OutputPolicy {
            output_extension: "mp3",
            target_profile: TargetProfile::CompatMp3,
        },
        Mode::Lossless => {
            let target_profile = match lossless_format.unwrap_or(LosslessFormat::Wav) {
                LosslessFormat::Wav => TargetProfile::LosslessWav,
                LosslessFormat::Aiff => TargetProfile::LosslessAiff,
            };

            OutputPolicy {
                output_extension: target_profile.output_extension(),
                target_profile,
            }
        }
    }
}

pub fn find_ffmpeg() -> Option<String> {
    if let Ok(explicit_path) = env::var("W4DJ_FFMPEG_PATH") {
        let candidate = PathBuf::from(explicit_path);
        if is_usable_ffmpeg_candidate(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }

    if let Ok(exe_dir) = env::current_exe()
        && let Some(found) = find_ffmpeg_next_to_exe(&exe_dir)
    {
        return Some(found.to_string_lossy().into_owned());
    }

    if let Ok(path) = which::which("ffmpeg") {
        return Some(path.to_string_lossy().into_owned());
    }

    #[cfg(windows)]
    {
        if let Ok(path) = which::which("ffmpeg.exe") {
            return Some(path.to_string_lossy().into_owned());
        }
    }

    None
}

fn find_ffmpeg_next_to_exe(exe_path: &Path) -> Option<PathBuf> {
    let exe_dir = exe_path.parent()?;
    let search_dirs = [exe_dir.to_path_buf(), exe_dir.join("binaries")];

    for candidate_name in preferred_ffmpeg_candidate_names() {
        for dir in &search_dirs {
            let candidate = dir.join(candidate_name);
            if is_usable_ffmpeg_candidate(&candidate) {
                return Some(candidate);
            }
        }
    }

    for dir in search_dirs {
        if let Some(found) = find_ffmpeg_sidecar_in_dir(&dir) {
            return Some(found);
        }
    }

    None
}

fn find_ffmpeg_sidecar_in_dir(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        let is_ffmpeg = entry
            .file_name()
            .to_string_lossy()
            .to_lowercase()
            .starts_with("ffmpeg");

        if !is_ffmpeg {
            continue;
        }

        if !is_usable_ffmpeg_candidate(&path) {
            continue;
        }

        return Some(path);
    }

    None
}

fn is_usable_ffmpeg_candidate(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    if !metadata.is_file() || metadata.len() == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(target_os = "windows")]
    {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        true
    }
}

fn preferred_ffmpeg_candidate_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        return match std::env::consts::ARCH {
            "x86_64" => &["ffmpeg-x86_64-pc-windows-msvc.exe", "ffmpeg.exe", "ffmpeg"],
            "aarch64" => &["ffmpeg-aarch64-pc-windows-msvc.exe", "ffmpeg.exe", "ffmpeg"],
            _ => &["ffmpeg.exe", "ffmpeg"],
        };
    }

    #[cfg(target_os = "macos")]
    {
        match std::env::consts::ARCH {
            "aarch64" => &["ffmpeg-aarch64-apple-darwin", "ffmpeg"],
            "x86_64" => &["ffmpeg-x86_64-apple-darwin", "ffmpeg"],
            _ => &["ffmpeg"],
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        &["ffmpeg"]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicScanIssue {
    pub path: PathBuf,
    pub message: String,
}

pub fn get_music_dict_with_scan_issues(
    folder: &str,
) -> (HashMap<String, (String, PathBuf)>, Vec<MusicScanIssue>) {
    get_music_dict_with_scan_issues_with_rule(folder, FilenameRule::default())
}

pub fn get_music_dict_with_scan_issues_with_rule(
    folder: &str,
    filename_rule: FilenameRule,
) -> (HashMap<String, (String, PathBuf)>, Vec<MusicScanIssue>) {
    get_music_dict_with_scan_issues_with_settings(
        folder,
        filename_rule,
        NeteaseFilenameFormat::default(),
    )
}

pub fn get_music_dict_with_scan_issues_with_settings(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
) -> (HashMap<String, (String, PathBuf)>, Vec<MusicScanIssue>) {
    get_music_dict_with_scan_issues_with_settings_and_policy(
        folder,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
    )
}

pub fn get_music_dict_with_scan_issues_with_settings_and_policy(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> (HashMap<String, (String, PathBuf)>, Vec<MusicScanIssue>) {
    let source_path = Path::new(folder);
    if source_path.is_file() && !is_supported_source_file(source_path) {
        return (HashMap::new(), Vec::new());
    }

    collect_music_dict_with_scan_issues(
        folder,
        SUPPORTED_SOURCE_EXTENSIONS,
        filename_rule,
        netease_filename_format,
        filename_policy,
    )
}

#[allow(dead_code)]
pub fn get_music_dict_with_scan_issues_with_rule_and_observer(
    folder: &str,
    filename_rule: FilenameRule,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    get_music_dict_with_scan_issues_with_settings_and_observer(
        folder,
        filename_rule,
        NeteaseFilenameFormat::default(),
        observer,
    )
}

pub fn get_music_dict_with_scan_issues_with_settings_and_observer(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    get_music_dict_with_scan_issues_with_settings_and_observer_with_policy(
        folder,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        observer,
    )
}

pub fn get_music_dict_with_scan_issues_with_settings_and_observer_with_policy(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let source_path = Path::new(folder);
    if source_path.is_file() && !is_supported_source_file(source_path) {
        return (HashMap::new(), Vec::new(), false);
    }

    collect_music_dict_with_scan_issues_observed(
        folder,
        SUPPORTED_SOURCE_EXTENSIONS,
        filename_rule,
        netease_filename_format,
        filename_policy,
        Some(observer),
        ScanPhase::Source,
    )
}

/// Scan source files while reusing the derived identity from the independent
/// scan cache when the file fingerprint and naming context are unchanged.
/// Destination files are intentionally not cached: their current state is
/// needed for every preview.
pub fn get_music_dict_with_scan_issues_with_settings_and_cache_observer(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    output_directory: &Path,
    cache: &mut ScanCache,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_policy(
        folder,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        output_directory,
        cache,
        observer,
    )
}

pub fn get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_policy(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    output_directory: &Path,
    cache: &mut ScanCache,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let source_path = Path::new(folder);
    if source_path.is_file() && !is_supported_source_file(source_path) {
        return (HashMap::new(), Vec::new(), false);
    }

    let enumeration_cancel = AtomicBool::new(false);
    let enumerated = enumerate_music_files_observed(
        folder,
        SUPPORTED_SOURCE_EXTENSIONS,
        &enumeration_cancel,
        |_, _, path| {
            if !observer(ScanPhase::Source, path) {
                enumeration_cancel.store(true, Ordering::SeqCst);
            }
        },
    );
    let (snapshots, mut scan_issues) = match enumerated {
        Ok(result) => (result.snapshots, result.issues),
        Err(ScanEnumerationError::Cancelled) => return (HashMap::new(), Vec::new(), true),
        Err(ScanEnumerationError::Failed(message)) => (
            Vec::new(),
            vec![MusicScanIssue {
                path: source_path.to_path_buf(),
                message,
            }],
        ),
    };
    if enumeration_cancel.load(Ordering::SeqCst) {
        return (HashMap::new(), scan_issues, true);
    }
    let mut snapshot_observer = |_: ScanPhase, _: &Path| true;
    let (music_dict, helper_issues, helper_cancelled) = music_dict_from_snapshots_with_cache(
        &snapshots,
        source_path,
        output_directory,
        filename_rule,
        netease_filename_format,
        filename_policy,
        cache,
        &AtomicBool::new(false),
        &mut snapshot_observer,
    );
    scan_issues.extend(helper_issues);
    (music_dict, scan_issues, helper_cancelled)
}

/// File-level scanner with a bounded scan-only worker set.  The configured
/// budget supplies the worker limit, but no conversion permit is acquired:
/// scanning must not consume FFmpeg capacity. Enumeration stays on the
/// coordinator thread, while metadata/name derivation runs in a fixed worker
/// set. Results are sorted back by enumeration index before they touch the
/// cache or duplicate-selection map.
#[allow(clippy::too_many_arguments)]
pub fn get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_budget(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    output_directory: &Path,
    cache: &mut ScanCache,
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_budget_and_policy(
        folder,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        output_directory,
        cache,
        budget,
        cancel,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn get_music_dict_with_scan_issues_with_settings_and_cache_observer_with_budget_and_policy(
    folder: &str,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    output_directory: &Path,
    cache: &mut ScanCache,
    _budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let normalized_source_root = crate::scan_cache::normalize_path(Path::new(folder));
    let normalized_output_directory = crate::scan_cache::normalize_path(output_directory);
    let (snapshots, mut scan_issues, enumeration_cancelled) =
        enumerate_music_snapshots(folder, SUPPORTED_SOURCE_EXTENSIONS, &cancel);
    if enumeration_cancelled {
        return (HashMap::new(), scan_issues, true);
    }
    let observed_paths = snapshots
        .iter()
        .map(|snapshot| snapshot.path.to_string_lossy().into_owned())
        .collect::<std::collections::HashSet<_>>();
    let mut music_dict = HashMap::new();
    let mut cancelled = cancel.load(Ordering::SeqCst);
    for snapshot in snapshots {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        let path = snapshot.path.clone();
        let path_key = path.to_string_lossy().into_owned();
        let cached = cache
            .entries
            .get(&path_key)
            .filter(|cached| {
                can_reuse_derived_name_entry_normalized(
                    cached,
                    Path::new(&path_key),
                    &normalized_source_root,
                    filename_rule_cache_key(filename_rule),
                    netease_filename_format_cache_key(netease_filename_format),
                    filename_policy.cache_key(),
                    snapshot.size_bytes,
                    snapshot.modified_at_ms,
                )
            })
            .cloned();
        let (song_name, cached_issue) = if let Some(cached) = cached {
            if !observer(ScanPhase::Source, &path) {
                cancelled = true;
                break;
            }
            (
                cached.safe_output_stem.unwrap_or(cached.derived_name),
                cached.scan_issue,
            )
        } else {
            if !observer(ScanPhase::Source, &path) {
                cancelled = true;
                break;
            }
            let song_name = derive_song_name_from_filename(
                &path,
                filename_rule,
                netease_filename_format,
                filename_policy,
            );
            cache.insert(ScanCacheEntry {
                source_path: path_key,
                source_root: normalized_source_root.to_string_lossy().into_owned(),
                output_directory: normalized_output_directory.to_string_lossy().into_owned(),
                filename_rule: filename_rule_cache_key(filename_rule).to_string(),
                netease_filename_format: netease_filename_format_cache_key(netease_filename_format)
                    .to_string(),
                filename_policy: filename_policy.cache_key().to_string(),
                size_bytes: snapshot.size_bytes,
                modified_at_ms: snapshot.modified_at_ms,
                derived_name: song_name.clone(),
                source_extension: snapshot.source_extension.clone(),
                scan_issue: None,
                safe_output_stem: Some(song_name.clone()),
                ..Default::default()
            });
            (song_name, None)
        };
        if let Some(issue) = cached_issue {
            scan_issues.push(MusicScanIssue {
                path: path.clone(),
                message: issue,
            });
        }
        let key = unique_source_entry_key(&song_name, &path, &music_dict);
        music_dict.insert(key, (snapshot.size_bytes.to_string(), path));
    }
    if !cancelled {
        cache.remove_missing_sources_from_snapshot(&normalized_source_root, &observed_paths);
    }
    (music_dict, scan_issues, cancelled)
}

#[allow(clippy::too_many_arguments)]
pub fn get_music_dict_with_scan_issues_with_settings_and_observer_with_budget(
    folder: &str,
    allowed_extensions: &[&str],
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
    phase: ScanPhase,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    get_music_dict_with_scan_issues_with_settings_and_observer_with_budget_and_policy(
        folder,
        allowed_extensions,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        budget,
        cancel,
        phase,
        observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn get_music_dict_with_scan_issues_with_settings_and_observer_with_budget_and_policy(
    folder: &str,
    allowed_extensions: &[&str],
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
    phase: ScanPhase,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let (snapshots, scan_issues, enumeration_cancelled) =
        enumerate_music_snapshots(folder, allowed_extensions, &cancel);
    if enumeration_cancelled {
        return (HashMap::new(), scan_issues, true);
    }
    let (results, worker_error) = scan_paths_with_budget(
        snapshots,
        budget,
        Arc::clone(&cancel),
        filename_rule,
        netease_filename_format,
        filename_policy,
        phase,
        observer,
    );
    if let Some(error) = worker_error {
        let mut scan_issues = scan_issues;
        scan_issues.push(MusicScanIssue {
            path: Path::new(folder).to_path_buf(),
            message: format!("扫描 worker 发生异常：{error}"),
        });
        return (HashMap::new(), scan_issues, false);
    }
    let mut music_dict = HashMap::new();
    let mut cancelled = cancel.load(Ordering::SeqCst);
    for result in results {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        let size = result.size_bytes.to_string();
        if matches!(phase, ScanPhase::Source) {
            let key = unique_source_entry_key(&result.derived_name, &result.path, &music_dict);
            music_dict.insert(key, (size, result.path));
        } else {
            let should_replace = music_dict
                .get(&result.derived_name)
                .map(|existing| should_prefer_file(&result.path, &size, existing))
                .unwrap_or(true);
            if should_replace {
                music_dict.insert(result.derived_name, (size, result.path));
            }
        }
    }
    (music_dict, scan_issues, cancelled)
}

#[derive(Debug)]
struct ScanWorkerResult {
    index: usize,
    path: PathBuf,
    size_bytes: u64,
    derived_name: String,
}

fn metadata_modified_at_ms(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

#[allow(dead_code)]
fn enumerate_music_paths(
    folder: &str,
    allowed_extensions: &[&str],
    cancel: &AtomicBool,
) -> (Vec<PathBuf>, Vec<MusicScanIssue>, bool) {
    match enumerate_music_files_observed(folder, allowed_extensions, cancel, |_, _, _| {}) {
        Ok(result) => (result.paths, result.issues, false),
        Err(ScanEnumerationError::Cancelled) => (Vec::new(), Vec::new(), true),
        Err(ScanEnumerationError::Failed(message)) => (
            Vec::new(),
            vec![MusicScanIssue {
                path: Path::new(folder).to_path_buf(),
                message,
            }],
            false,
        ),
    }
}

fn enumerate_music_snapshots(
    folder: &str,
    allowed_extensions: &[&str],
    cancel: &AtomicBool,
) -> (Vec<ScannedFileSnapshot>, Vec<MusicScanIssue>, bool) {
    match enumerate_music_files_observed(folder, allowed_extensions, cancel, |_, _, _| {}) {
        Ok(result) => (result.snapshots, result.issues, false),
        Err(ScanEnumerationError::Cancelled) => (Vec::new(), Vec::new(), true),
        Err(ScanEnumerationError::Failed(message)) => (
            Vec::new(),
            vec![MusicScanIssue {
                path: Path::new(folder).to_path_buf(),
                message,
            }],
            false,
        ),
    }
}

/// Build a source map from an already collected filesystem snapshot. This is
/// the shared-root fast path used by the two task slots; no directory walk or
/// per-file stat is performed here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn music_dict_from_snapshots_with_cache(
    snapshots: &[ScannedFileSnapshot],
    source_root: &Path,
    output_directory: &Path,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    cache: &mut ScanCache,
    cancel: &AtomicBool,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let normalized_source_root = crate::scan_cache::normalize_path(source_root);
    let normalized_output_directory = crate::scan_cache::normalize_path(output_directory);
    let mut result = HashMap::new();
    let issues = Vec::new();
    let observed_paths = snapshots
        .iter()
        .map(|snapshot| snapshot.path.to_string_lossy().into_owned())
        .collect::<std::collections::HashSet<_>>();
    let mut ordered_snapshots = snapshots.iter().collect::<Vec<_>>();
    ordered_snapshots.sort_by(|left, right| left.path.cmp(&right.path));
    for snapshot in ordered_snapshots {
        if cancel.load(Ordering::SeqCst) || !observer(ScanPhase::Source, &snapshot.path) {
            return (result, issues, true);
        }
        let path_key = snapshot.path.to_string_lossy().into_owned();
        let cached = cache.entries.get(&path_key).filter(|entry| {
            can_reuse_derived_name_entry_normalized(
                entry,
                Path::new(&path_key),
                &normalized_source_root,
                filename_rule_cache_key(filename_rule),
                netease_filename_format_cache_key(netease_filename_format),
                filename_policy.cache_key(),
                snapshot.size_bytes,
                snapshot.modified_at_ms,
            )
        });
        let name = if let Some(entry) = cached {
            entry
                .safe_output_stem
                .clone()
                .unwrap_or_else(|| entry.derived_name.clone())
        } else {
            let name = derive_song_name_from_filename(
                &snapshot.path,
                filename_rule,
                netease_filename_format,
                filename_policy,
            );
            cache.insert(ScanCacheEntry {
                source_path: path_key,
                source_root: normalized_source_root.to_string_lossy().into_owned(),
                output_directory: normalized_output_directory.to_string_lossy().into_owned(),
                filename_rule: filename_rule_cache_key(filename_rule).to_string(),
                netease_filename_format: netease_filename_format_cache_key(netease_filename_format)
                    .to_string(),
                filename_policy: filename_policy.cache_key().to_string(),
                size_bytes: snapshot.size_bytes,
                modified_at_ms: snapshot.modified_at_ms,
                derived_name: name.clone(),
                source_extension: snapshot.source_extension.clone(),
                safe_output_stem: Some(name.clone()),
                ..Default::default()
            });
            name
        };
        let key = unique_source_entry_key(&name, &snapshot.path, &result);
        result.insert(
            key,
            (snapshot.size_bytes.to_string(), snapshot.path.clone()),
        );
    }
    cache.remove_missing_sources_from_snapshot(&normalized_source_root, &observed_paths);
    (result, issues, false)
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(crate) fn music_dict_from_snapshots_with_budget(
    snapshots: &[ScannedFileSnapshot],
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    phase: ScanPhase,
    observer: &mut ScanObserver<'_>,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let owned = snapshots.to_vec();
    let (results, worker_error) = scan_paths_with_budget(
        owned,
        budget,
        Arc::clone(&cancel),
        filename_rule,
        netease_filename_format,
        filename_policy,
        phase,
        observer,
    );
    let mut issues = Vec::new();
    if let Some(error) = worker_error {
        issues.push(MusicScanIssue {
            path: snapshots
                .first()
                .map(|s| s.path.clone())
                .unwrap_or_default(),
            message: format!("扫描 worker 发生异常：{error}"),
        });
    }
    let mut result = HashMap::new();
    for item in results {
        let size = item.size_bytes.to_string();
        if matches!(phase, ScanPhase::Source) {
            let key = unique_source_entry_key(&item.derived_name, &item.path, &result);
            result.insert(key, (size, item.path));
        } else if result
            .get(&item.derived_name)
            .map(|existing| should_prefer_file(&item.path, &size, existing))
            .unwrap_or(true)
        {
            result.insert(item.derived_name, (size, item.path));
        }
    }
    (result, issues, cancel.load(Ordering::SeqCst))
}

#[allow(clippy::too_many_arguments)]
fn scan_paths_with_budget(
    snapshots: Vec<ScannedFileSnapshot>,
    budget: Arc<GlobalConcurrencyBudget>,
    cancel: Arc<AtomicBool>,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    phase: ScanPhase,
    observer: &mut ScanObserver<'_>,
) -> (Vec<ScanWorkerResult>, Option<String>) {
    if snapshots.is_empty() {
        return (Vec::new(), None);
    }
    let queue = Arc::new(Mutex::new(
        snapshots.into_iter().enumerate().collect::<VecDeque<_>>(),
    ));
    let (sender, receiver) = mpsc::channel();
    let worker_error = Arc::new(Mutex::new(None::<String>));
    let worker_count = budget
        .limit()
        .min(queue.lock().map(|q| q.len()).unwrap_or(0))
        .max(1);
    let mut workers = Vec::with_capacity(worker_count);
    for worker_index in 0..worker_count {
        let queue = Arc::clone(&queue);
        let sender = sender.clone();
        let cancel = Arc::clone(&cancel);
        let worker_error = Arc::clone(&worker_error);
        workers.push(thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let item = queue.lock().expect("scan queue lock poisoned").pop_front();
                    let Some((index, snapshot)) = item else {
                        break;
                    };
                    let result = ScanWorkerResult {
                        index,
                        size_bytes: snapshot.size_bytes,
                        derived_name: derive_song_name_from_filename(
                            &snapshot.path,
                            filename_rule,
                            netease_filename_format,
                            filename_policy,
                        ),
                        path: snapshot.path,
                    };
                    if sender.send(result).is_err() {
                        break;
                    }
                }
            }));
            if let Err(payload) = result {
                let message = panic_message(payload);
                if let Ok(mut error) = worker_error.lock()
                    && error.is_none()
                {
                    *error = Some(format!("worker {worker_index}: {message}"));
                }
                eprintln!("scan worker {worker_index} panicked: {message}");
            }
        }));
    }
    drop(sender);
    let mut results = Vec::new();
    while let Ok(result) = receiver.recv() {
        if !cancel.load(Ordering::SeqCst) && !observer(phase, &result.path) {
            cancel.store(true, Ordering::SeqCst);
        }
        results.push(result);
    }
    for worker in workers {
        let _ = worker.join();
    }
    results.sort_by_key(|result| result.index);
    let worker_error = worker_error.lock().ok().and_then(|error| error.clone());
    (results, worker_error)
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "未知 panic".to_string())
}

pub fn filename_rule_cache_key(rule: FilenameRule) -> &'static str {
    match rule {
        FilenameRule::TitleArtist => "title_artist",
        FilenameRule::ArtistTitle => "artist_title",
        FilenameRule::Original => "original",
    }
}

pub fn netease_filename_format_cache_key(format: NeteaseFilenameFormat) -> &'static str {
    match format {
        NeteaseFilenameFormat::TitleOnly => "title_only",
        NeteaseFilenameFormat::ArtistTitle => "artist_title",
        NeteaseFilenameFormat::TitleArtist => "title_artist",
    }
}

pub fn is_supported_source_file(path: &Path) -> bool {
    path.is_file()
        && !is_ignored_music_file(path)
        && has_allowed_extension(path, SUPPORTED_SOURCE_EXTENSIONS)
}

pub fn get_music_dict(folder: &str) -> HashMap<String, (String, PathBuf)> {
    get_music_dict_with_scan_issues(folder).0
}

pub fn get_destination_music_dict(folder: &str) -> HashMap<String, (String, PathBuf)> {
    get_destination_music_dict_with_rule(folder, FilenameRule::default())
}

pub fn get_destination_music_dict_with_rule(
    folder: &str,
    filename_rule: FilenameRule,
) -> HashMap<String, (String, PathBuf)> {
    collect_music_dict_with_scan_issues(
        folder,
        &["mp3", "wav", "aiff"],
        filename_rule,
        NeteaseFilenameFormat::default(),
        FilenameNormalizationPolicy::SoundCloud,
    )
    .0
}

/// Return every current, visible audio output without collapsing same-name
/// files into a single map entry. Conflict planning needs the complete set so
/// a confirmed identity can replace older container or filename variants.
pub fn get_destination_music_files(folder: &str) -> Vec<PathBuf> {
    let mut paths = walkdir::WalkDir::new(folder)
        .into_iter()
        .filter_entry(|entry| !is_hidden_path(entry.path()))
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_file()
                && !is_ignored_music_file(entry.path())
                && has_allowed_extension(entry.path(), &["mp3", "wav", "aiff"])
        })
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[allow(dead_code)]
pub fn get_destination_music_dict_with_rule_and_observer(
    folder: &str,
    filename_rule: FilenameRule,
    observer: &mut ScanObserver<'_>,
) -> (HashMap<String, (String, PathBuf)>, bool) {
    let (music_dict, _issues, cancelled) = collect_music_dict_with_scan_issues_observed(
        folder,
        &["mp3", "wav", "aiff"],
        filename_rule,
        NeteaseFilenameFormat::default(),
        FilenameNormalizationPolicy::SoundCloud,
        Some(observer),
        ScanPhase::Destination,
    );
    (music_dict, cancelled)
}

#[allow(dead_code)]
pub fn count_music_files(folder: &str, allowed_extensions: &[&str]) -> usize {
    count_music_files_with_cancel(folder, allowed_extensions, || false).0
}

#[allow(dead_code)]
pub fn count_music_files_with_cancel<F: FnMut() -> bool>(
    folder: &str,
    allowed_extensions: &[&str],
    mut should_cancel: F,
) -> (usize, bool) {
    if folder.trim().is_empty() {
        return (0, false);
    }
    let mut count = 0;
    for entry in walkdir::WalkDir::new(folder)
        .into_iter()
        .filter_entry(|entry| !is_hidden_path(entry.path()))
        .filter_map(Result::ok)
    {
        if should_cancel() {
            return (count, true);
        }
        if entry.file_type().is_file()
            && !is_ignored_music_file(entry.path())
            && has_allowed_extension(entry.path(), allowed_extensions)
        {
            count += 1;
        }
    }
    (count, false)
}

pub fn cleanup_temporary_outputs(folder: &str) -> io::Result<()> {
    // Kept as a compatibility no-op. Prefix-only cleanup could delete a user's
    // legitimate hidden audio file. New temporary files are self-cleaning.
    let _ = folder;
    Ok(())
}

fn collect_music_dict_with_scan_issues(
    folder: &str,
    allowed_extensions: &[&str],
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> (HashMap<String, (String, PathBuf)>, Vec<MusicScanIssue>) {
    let (music_dict, scan_issues, _cancelled) = collect_music_dict_with_scan_issues_observed(
        folder,
        allowed_extensions,
        filename_rule,
        netease_filename_format,
        filename_policy,
        None,
        ScanPhase::Source,
    );
    (music_dict, scan_issues)
}

fn collect_music_dict_with_scan_issues_observed(
    folder: &str,
    allowed_extensions: &[&str],
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    mut observer: Option<&mut ScanObserver<'_>>,
    phase: ScanPhase,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let mut music_dict = HashMap::new();
    let mut scan_issues = Vec::new();

    for entry_result in walkdir::WalkDir::new(folder)
        .into_iter()
        .filter_entry(|entry| !is_hidden_path(entry.path()))
    {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) => {
                if let Some(path) = error.path().filter(|path| !is_ignored_music_file(path)) {
                    scan_issues.push(MusicScanIssue {
                        path: path.to_path_buf(),
                        message: format!("无法扫描歌曲文件：{error}"),
                    });
                }
                continue;
            }
        };

        if !entry.file_type().is_file()
            || is_ignored_music_file(entry.path())
            || !has_allowed_extension(entry.path(), allowed_extensions)
        {
            continue;
        }

        if let Some(observer) = observer.as_deref_mut()
            && !observer(phase, entry.path())
        {
            return (music_dict, scan_issues, true);
        }

        let path = entry.path().to_path_buf();
        let song_name = derive_song_name_with_policy(
            entry.path(),
            filename_rule,
            netease_filename_format,
            filename_policy,
        );
        let size = entry
            .metadata()
            .map(|m| m.len().to_string())
            .unwrap_or_else(|_| "0".to_string());

        if matches!(phase, ScanPhase::Source) {
            let key = unique_source_entry_key(&song_name, &path, &music_dict);
            music_dict.insert(key, (size, path));
        } else {
            let should_replace = music_dict
                .get(&song_name)
                .map(|existing| should_prefer_file(&path, &size, existing))
                .unwrap_or(true);
            if should_replace {
                music_dict.insert(song_name, (size, path));
            }
        }
    }

    (music_dict, scan_issues, false)
}

fn has_allowed_extension(path: &Path, allowed_extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext_str| {
            let lower = ext_str.to_lowercase();
            allowed_extensions.iter().any(|allowed| *allowed == lower)
        })
}

fn is_temporary_artifact(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".w4dj-"))
}

pub fn is_ignored_music_file(path: &Path) -> bool {
    is_hidden_path(path) || is_temporary_artifact(path) || is_macos_appledouble_file(path)
}

/// Shared hidden-path policy for every source and destination scan.
pub fn is_hidden_path(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with('.') && !is_os_temp_fixture_directory(ancestor, name)
            })
            || ancestor_metadata_is_hidden(ancestor)
    })
}

// Rust's `tempfile::tempdir()` deliberately creates `.tmp*` directories.
// They are test fixtures rather than user-selected hidden music folders; keep
// fixture scans usable without weakening the hidden-path rule elsewhere.
fn is_os_temp_fixture_directory(path: &Path, name: &str) -> bool {
    if !name.starts_with(".tmp") {
        return false;
    }
    if !fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
    {
        return false;
    }
    let Ok(temp_root) = fs::canonicalize(std::env::temp_dir()) else {
        return false;
    };
    fs::canonicalize(path)
        .ok()
        .is_some_and(|candidate| candidate.starts_with(&temp_root))
}

fn ancestor_metadata_is_hidden(path: &Path) -> bool {
    // macOS marks the system `/var` mount with a filesystem flag that is not
    // the user-facing Hidden attribute.  Never let that ancestor hide every
    // temporary or application path beneath it.
    if path == Path::new("/var") || path == Path::new("/private/var") {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(target_os = "macos")]
    {
        const UF_HIDDEN: u32 = 0x0000_8000;
        if metadata.st_flags() & UF_HIDDEN != 0 {
            return true;
        }
    }
    #[cfg(target_os = "windows")]
    {
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        if metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0 {
            return true;
        }
    }
    false
}

fn is_macos_appledouble_file(path: &Path) -> bool {
    let has_appledouble_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("._"));
    if !has_appledouble_name {
        return false;
    }

    let mut magic = [0_u8; 4];
    match File::open(path) {
        Ok(mut file) => file.read_exact(&mut magic).is_ok() && magic == [0x00, 0x05, 0x16, 0x07],
        Err(_) => true,
    }
}

fn should_prefer_file(
    candidate_path: &Path,
    candidate_size: &str,
    current: &(String, PathBuf),
) -> bool {
    let candidate_rank = file_rank(candidate_path);
    let current_rank = file_rank(&current.1);

    candidate_rank > current_rank
        || (candidate_rank == current_rank
            && candidate_size.parse::<u64>().unwrap_or(0) >= current.0.parse::<u64>().unwrap_or(0))
}

fn file_rank(path: &Path) -> u8 {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .as_deref()
    {
        Some("wav") | Some("aiff") => 4,
        Some("flac") => 3,
        Some("ncm") => 2,
        Some("mp3") => 1,
        _ => 0,
    }
}

pub fn compare_music_dicts<'a>(
    wf_dict: &'a HashMap<String, (String, PathBuf)>,
    sf_dict: &'a HashMap<String, (String, PathBuf)>,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
) -> HashMap<&'a String, &'a (String, PathBuf)> {
    wf_dict
        .iter()
        .filter(|(name, wf_info)| match mode {
            Mode::Compat => {
                let expected_extension =
                    resolve_output_policy(*mode, lossless_format, "mp3").output_extension;
                needs_regeneration(
                    sf_dict.get(source_entry_name(name)),
                    mode,
                    "mp3",
                    expected_extension,
                )
            }
            Mode::Lossless => {
                let source_extension = effective_source_extension(&wf_info.1);
                let expected_extension =
                    resolve_output_policy(*mode, lossless_format, &source_extension)
                        .output_extension;

                needs_regeneration(
                    sf_dict.get(source_entry_name(name)),
                    mode,
                    &source_extension,
                    expected_extension,
                )
            }
        })
        .collect()
}

fn needs_regeneration(
    existing: Option<&(String, PathBuf)>,
    mode: &Mode,
    source_extension: &str,
    expected_extension: &str,
) -> bool {
    let Some(existing) = existing else {
        return true;
    };

    if existing.0.parse::<u64>().unwrap_or(0) == 0 {
        return true;
    }

    let existing_extension = existing
        .1
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    match mode {
        Mode::Compat => existing_extension != expected_extension,
        Mode::Lossless if source_extension == "mp3" => false,
        Mode::Lossless => existing_extension != expected_extension,
    }
}

pub fn sync_music_library_with_policy(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
) -> io::Result<TaskSnapshot> {
    let task_controller = TaskController::running(new_songs.len());
    sync_music_library_with_task(
        new_songs,
        dest_folder,
        mode,
        lossless_format,
        &task_controller,
    )
}

pub fn sync_music_library_with_task(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    task_controller: &TaskController,
) -> io::Result<TaskSnapshot> {
    sync_music_library_with_observer(
        new_songs,
        dest_folder,
        mode,
        lossless_format,
        task_controller,
        |_, _, _| {},
    )
}

pub fn sync_music_library_with_observer(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    task_controller: &TaskController,
    after_file: impl FnMut(&str, &TaskController, Option<&io::Error>),
) -> io::Result<TaskSnapshot> {
    sync_music_library_transactional_with_observer(
        new_songs,
        dest_folder,
        mode,
        lossless_format,
        NeteaseFilenameFormat::default(),
        task_controller,
        |_, _| Ok(()),
        after_file,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn sync_music_library_transactional_with_observer(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    netease_filename_format: NeteaseFilenameFormat,
    task_controller: &TaskController,
    finalize_output: impl FnMut(&str, &Path) -> io::Result<()>,
    after_file: impl FnMut(&str, &TaskController, Option<&io::Error>),
) -> io::Result<TaskSnapshot> {
    sync_music_library_transactional_with_observer_and_context(
        new_songs,
        dest_folder,
        mode,
        lossless_format,
        netease_filename_format,
        task_controller,
        finalize_output,
        after_file,
        &ConversionMetadataContext::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn sync_music_library_transactional_with_observer_and_context(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    netease_filename_format: NeteaseFilenameFormat,
    task_controller: &TaskController,
    mut finalize_output: impl FnMut(&str, &Path) -> io::Result<()>,
    mut after_file: impl FnMut(&str, &TaskController, Option<&io::Error>),
    metadata_context: &ConversionMetadataContext,
) -> io::Result<TaskSnapshot> {
    if new_songs.is_empty() {
        return Ok(task_controller.snapshot());
    }

    let bar = indicatif::ProgressBar::new(new_songs.len() as u64);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})\n{msg}",
        )
        .unwrap(),
    );

    let mut queued_files = new_songs.iter().collect::<Vec<_>>();
    queued_files.sort_by_key(|(left_name, _)| *left_name);
    let mut failed_files = 0usize;
    let mut last_error: Option<io::Error> = None;

    for (&name, info) in queued_files {
        if task_controller.is_cancelled() {
            bar.abandon_with_message("Sync cancelled.");
            return Ok(task_controller.snapshot());
        }

        if !task_controller.should_start_next_file() {
            bar.abandon_with_message("Sync paused after current file.");
            return Ok(task_controller.snapshot());
        }

        let task_result = process_music_file(
            name,
            info,
            dest_folder,
            mode,
            lossless_format,
            netease_filename_format,
            FilenameNormalizationPolicy::SoundCloud,
            &bar,
            &mut finalize_output,
            None,
            metadata_context,
        );
        match task_result {
            Ok(()) => {
                task_controller.complete_current_file();
                bar.inc(1);
                after_file(name, task_controller, None);
            }
            Err(err) => {
                if err.kind() == ErrorKind::Interrupted && task_controller.is_cancelled() {
                    continue;
                }
                let error_message = err.to_string();
                failed_files += 1;
                last_error = Some(io::Error::new(err.kind(), error_message.clone()));
                bar.inc(1);
                after_file(name, task_controller, Some(&err));
                bar.println(format!("Failed {}: {}", name, error_message));
            }
        }
    }

    let snapshot = task_controller.snapshot();
    if snapshot.completed == 0 && failed_files > 0 {
        bar.abandon_with_message(format!("Sync failed after failing {} files.", failed_files));
        return Err(last_error.unwrap_or_else(|| {
            io::Error::other(format!("Sync failed after failing {} files.", failed_files))
        }));
    }

    bar.finish_with_message(format!(
        "Sync processing complete. {}/{} files processed, {} failed.",
        snapshot.completed, snapshot.total, failed_files
    ));
    Ok(snapshot)
}

/// Concurrent conversion entry point used by the desktop coordinator.  The
/// caller owns all shared state; workers only return per-song results and the
/// `after_file` callback is invoked serially on this coordinator thread.
#[allow(clippy::too_many_arguments)]
pub fn sync_music_library_transactional_with_observer_and_budget(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    netease_filename_format: NeteaseFilenameFormat,
    task_controller: &TaskController,
    finalize_output: impl Fn(&str, &Path) -> io::Result<()> + Send + Sync + 'static,
    mut after_file: impl FnMut(&str, &TaskController, Option<&io::Error>),
    budget: Arc<GlobalConcurrencyBudget>,
    ffmpeg_registry: Arc<ActiveFfmpegRegistry>,
) -> io::Result<TaskSnapshot> {
    sync_music_library_transactional_with_observer_and_budget_and_context(
        new_songs,
        dest_folder,
        mode,
        lossless_format,
        netease_filename_format,
        task_controller,
        finalize_output,
        move |name, task, error| after_file(name, task, error),
        budget,
        ffmpeg_registry,
        &ConversionMetadataContext::default(),
    )
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn sync_music_library_transactional_with_observer_and_budget_and_context(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    netease_filename_format: NeteaseFilenameFormat,
    task_controller: &TaskController,
    finalize_output: impl Fn(&str, &Path) -> io::Result<()> + Send + Sync + 'static,
    mut after_file: impl FnMut(&str, &TaskController, Option<&io::Error>),
    budget: Arc<GlobalConcurrencyBudget>,
    ffmpeg_registry: Arc<ActiveFfmpegRegistry>,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<TaskSnapshot> {
    sync_music_library_transactional_with_observer_and_budget_and_context_with_policy(
        new_songs,
        dest_folder,
        mode,
        lossless_format,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        task_controller,
        finalize_output,
        move |name, task, error| {
            after_file(name, task, error);
            Ok(())
        },
        budget,
        ffmpeg_registry,
        metadata_context,
    )
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn sync_music_library_transactional_with_observer_and_budget_and_context_with_policy(
    new_songs: &HashMap<&String, &(String, PathBuf)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    task_controller: &TaskController,
    finalize_output: impl Fn(&str, &Path) -> io::Result<()> + Send + Sync + 'static,
    mut after_file: impl FnMut(&str, &TaskController, Option<&io::Error>) -> io::Result<()>,
    budget: Arc<GlobalConcurrencyBudget>,
    ffmpeg_registry: Arc<ActiveFfmpegRegistry>,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<TaskSnapshot> {
    if new_songs.is_empty() {
        return Ok(task_controller.snapshot());
    }

    let bar = indicatif::ProgressBar::new(new_songs.len() as u64);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})\\n{msg}",
        )
        .unwrap(),
    );

    let mut queued = new_songs
        .iter()
        .map(|(&name, (size, path))| OwnedConversionItem {
            name: name.clone(),
            info: (size.clone(), path.clone()),
        })
        .collect::<Vec<_>>();
    queued.sort_by(|left, right| left.name.cmp(&right.name));
    let queue = Arc::new(Mutex::new(VecDeque::from(queued)));
    let (sender, receiver) = mpsc::channel::<(String, io::Result<()>)>();
    let worker_count = budget.limit().min(new_songs.len()).max(1);
    let destination = dest_folder.to_string();
    let mode = *mode;
    let callback: Arc<dyn Fn(&str, &Path) -> io::Result<()> + Send + Sync> =
        Arc::new(finalize_output);
    let mut workers = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let sender = sender.clone();
        let task_controller = task_controller.clone();
        let budget = Arc::clone(&budget);
        let registry = Arc::clone(&ffmpeg_registry);
        let destination = destination.clone();
        let callback = Arc::clone(&callback);
        let metadata_context = metadata_context.clone();
        let cancel_signal = task_controller.cancellation_flag();
        workers.push(thread::spawn(move || {
            let worker_result = catch_unwind(AssertUnwindSafe(|| {
                loop {
                    if task_controller.is_cancelled() || task_controller.pause_after_current_file()
                    {
                        break;
                    }
                    let item = queue
                        .lock()
                        .expect("conversion queue lock poisoned")
                        .pop_front();
                    let Some(item) = item else {
                        break;
                    };
                    let Some(_permit): Option<ConcurrencyPermit> = budget.acquire(|| {
                        task_controller.is_cancelled() || task_controller.pause_after_current_file()
                    }) else {
                        break;
                    };
                    if task_controller.is_cancelled() || task_controller.pause_after_current_file()
                    {
                        break;
                    }
                    let bar = indicatif::ProgressBar::hidden();
                    let mut finalize = |name: &str, path: &Path| callback(name, path);
                    let result = process_music_file(
                        &item.name,
                        &item.info,
                        &destination,
                        &mode,
                        lossless_format,
                        netease_filename_format,
                        filename_policy,
                        &bar,
                        &mut finalize,
                        Some((&registry, &cancel_signal)),
                        &metadata_context,
                    );
                    if sender.send((item.name, result)).is_err() {
                        break;
                    }
                }
            }));
            if worker_result.is_err() && !task_controller.is_cancelled() {
                let _ = sender.send((
                    "<worker panic>".to_string(),
                    Err(io::Error::other("转换 worker 发生异常")),
                ));
            }
        }));
    }
    drop(sender);

    let mut failed_files = 0usize;
    let mut last_error: Option<io::Error> = None;
    while let Ok((name, result)) = receiver.recv() {
        match result {
            Ok(()) => {
                task_controller.complete_current_file();
                match after_file(&name, task_controller, None) {
                    Ok(()) => {}
                    Err(error) => {
                        task_controller.revert_completed_file();
                        failed_files += 1;
                        last_error = Some(io::Error::new(error.kind(), error.to_string()));
                        after_file(&name, task_controller, Some(&error))?;
                    }
                }
                bar.inc(1);
            }
            Err(error) => {
                if error.kind() == ErrorKind::Interrupted && task_controller.is_cancelled() {
                    continue;
                }
                failed_files += 1;
                last_error = Some(io::Error::new(error.kind(), error.to_string()));
                bar.inc(1);
                after_file(&name, task_controller, Some(&error))?;
            }
        }
    }
    for worker in workers {
        let _ = worker.join();
    }

    let snapshot = task_controller.snapshot();
    if snapshot.completed == 0 && failed_files > 0 && !snapshot.cancelled {
        bar.abandon_with_message(format!("Sync failed after failing {failed_files} files."));
        return Err(last_error.unwrap_or_else(|| {
            io::Error::other(format!("Sync failed after failing {failed_files} files."))
        }));
    }
    bar.finish_with_message(format!(
        "Sync processing complete. {}/{} files processed, {} failed.",
        snapshot.completed, snapshot.total, failed_files
    ));
    Ok(snapshot)
}

#[derive(Debug)]
struct OwnedConversionItem {
    name: String,
    info: (String, PathBuf),
}

#[allow(dead_code)]
#[allow(deprecated)]
pub fn update_existing_metadata(source_path: &Path, destination_path: &Path) -> io::Result<()> {
    // The legacy test/helper entry point is intentionally source-only. The
    // desktop conversion coordinator supplies its real lazy resolver through
    // the context-aware APIs below.
    let context = ConversionMetadataContext {
        netease: Arc::new(NeteaseMetadataResolver::default()),
    };
    update_existing_metadata_with_resolver(source_path, destination_path, &context)
}

pub fn update_existing_metadata_with_resolver(
    source_path: &Path,
    destination_path: &Path,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<()> {
    update_existing_metadata_with_resolver_and_policy(
        source_path,
        destination_path,
        metadata_context,
        FilenameNormalizationPolicy::SoundCloud,
    )
}

pub fn update_existing_metadata_with_resolver_and_policy(
    source_path: &Path,
    destination_path: &Path,
    metadata_context: &ConversionMetadataContext,
    filename_policy: FilenameNormalizationPolicy,
) -> io::Result<()> {
    let source_extension = source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let source_tag = match source_extension.as_str() {
        "flac" => {
            let tag = metaflac::Tag::read_from_path(source_path).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("无法读取 FLAC 元数据：{error}"),
                )
            })?;
            build_id3_tag_from_flac(&tag)
        }
        #[cfg(feature = "ncm-decryption")]
        "ncm" => {
            let file = File::open(source_path)?;
            let mut ncm = Ncmdump::from_reader(file).map_err(|error| {
                Error::new(ErrorKind::InvalidData, format!("NCM 解析错误：{error}"))
            })?;
            let info = ncm.get_info().map_err(|error| {
                Error::new(ErrorKind::InvalidData, format!("NCM 元数据错误：{error}"))
            })?;
            let image = ncm.get_image().map_err(|error| {
                Error::new(ErrorKind::InvalidData, format!("NCM 封面读取错误：{error}"))
            })?;
            build_id3_tag(&info, &image)
        }
        #[cfg(not(feature = "ncm-decryption"))]
        "ncm" => return Err(ncm_decryption_unavailable_error()),
        _ => id3::Tag::read_from_path(source_path).map_err(|error| {
            Error::new(
                ErrorKind::InvalidData,
                format!("无法读取源文件元数据：{error}"),
            )
        })?,
    };
    let mut source_tag = source_tag;
    if matches!(filename_policy, FilenameNormalizationPolicy::PreserveSource)
        && !matches!(source_extension.as_str(), "ncm")
        && let Some(recovered) = crate::netease::recover_local_metadata_with_resolver(
            source_path,
            &metadata_context.netease,
        )
        .metadata
    {
        merge_recovered_metadata(&mut source_tag, &recovered);
    }

    let destination_extension = destination_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let result: io::Result<()> = match destination_extension.as_str() {
        "wav" => write_id3_tag_for_output(&source_tag, destination_path),
        "aiff" | "aif" => source_tag
            .write_to_path(destination_path, Version::Id3v24)
            .map_err(io::Error::other),
        "mp3" => source_tag
            .write_to_path(destination_path, Version::Id3v24)
            .map_err(io::Error::other),
        _ => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("不支持更新此输出格式的元数据：{destination_extension}"),
            ));
        }
    };
    result.map_err(|error| {
        Error::other(format!(
            "无法更新输出文件元数据 {}：{}",
            destination_path.display(),
            error
        ))
    })
}

#[allow(dead_code)]
pub fn update_existing_metadata_transactionally(
    source_path: &Path,
    destination_path: &Path,
    netease_filename_format: NeteaseFilenameFormat,
    finalize_output: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let context = ConversionMetadataContext {
        netease: Arc::new(NeteaseMetadataResolver::default()),
    };
    update_existing_metadata_transactionally_with_context(
        source_path,
        destination_path,
        netease_filename_format,
        finalize_output,
        &context,
    )
}

pub fn update_existing_metadata_transactionally_with_context(
    source_path: &Path,
    destination_path: &Path,
    netease_filename_format: NeteaseFilenameFormat,
    finalize_output: impl FnOnce(&Path) -> io::Result<()>,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<()> {
    update_existing_metadata_transactionally_with_context_and_policy(
        source_path,
        destination_path,
        netease_filename_format,
        finalize_output,
        metadata_context,
        FilenameNormalizationPolicy::SoundCloud,
    )
}

pub fn update_existing_metadata_transactionally_with_context_and_policy(
    source_path: &Path,
    destination_path: &Path,
    netease_filename_format: NeteaseFilenameFormat,
    finalize_output: impl FnOnce(&Path) -> io::Result<()>,
    metadata_context: &ConversionMetadataContext,
    filename_policy: FilenameNormalizationPolicy,
) -> io::Result<()> {
    let name_stem = destination_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("metadata-update");
    run_output_transaction(destination_path, name_stem, |temporary_output| {
        copy_file(destination_path, temporary_output)?;
        update_existing_metadata_with_resolver_and_policy(
            source_path,
            temporary_output,
            metadata_context,
            filename_policy,
        )?;
        ensure_output_metadata_with_settings_with_context_and_policy(
            source_path,
            temporary_output,
            netease_filename_format,
            filename_policy,
            metadata_context,
        )?;
        finalize_output(temporary_output)
    })
}

/// Applies analysis tags to an already converted output without requiring the
/// original source file to remain available.
///
/// Enhanced analysis is intentionally decoupled from conversion. The source
/// may be an NCM file that has since been moved or removed, while the output
/// still exists and can safely receive the cached analysis values.
#[allow(dead_code)]
pub fn update_analysis_metadata_transactionally(
    destination_path: &Path,
    finalize_output: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let name_stem = destination_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("analysis-update");
    run_output_transaction(destination_path, name_stem, |temporary_output| {
        copy_file(destination_path, temporary_output)?;
        finalize_output(temporary_output)
    })
}

#[allow(clippy::too_many_arguments)]
fn process_music_file(
    name: &str,
    info: &(String, PathBuf),
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    bar: &indicatif::ProgressBar,
    finalize_output: &mut impl FnMut(&str, &Path) -> io::Result<()>,
    control: Option<(&ActiveFfmpegRegistry, &AtomicBool)>,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<()> {
    let name = source_entry_name(name);
    ensure_not_cancelled(control)?;
    let src_path = info.1.as_path();
    if !src_path.exists() {
        return Err(Error::new(
            ErrorKind::NotFound,
            format!("Source file missing: {}", src_path.display()),
        ));
    }
    let extension = src_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    let source_format = if extension == "ncm" {
        detect_ncm_output_extension(src_path).unwrap_or_else(|_| "flac".to_string())
    } else {
        extension.clone()
    };
    let output_policy = resolve_output_policy(*mode, lossless_format, &source_format);
    let output_path = target_output_path_with_policy(
        dest_folder,
        name,
        output_policy.output_extension,
        filename_policy,
    );
    let output_lock = control.map(|(registry, _)| registry.lock_for_output(&output_path));
    let _output_guard = output_lock
        .as_ref()
        .map(|lock| lock.lock().expect("FFmpeg output lock poisoned"));

    let result = match extension.as_str() {
        "mp3" | "wav" | "aiff" | "flac" | "ncm" => {
            bar.set_message(format!("Processing {}: {}", extension.to_uppercase(), name));
            run_output_transaction(&output_path, name, |temporary_output| {
                match extension.as_str() {
                    "mp3" if matches!(output_policy.target_profile, TargetProfile::CompatMp3) => {
                        copy_file(src_path, temporary_output)?;
                    }
                    #[cfg(feature = "ncm-decryption")]
                    "ncm" => {
                        if let Some((registry, cancelled)) = control {
                            process_ncm_file_to_output_managed(
                                src_path,
                                temporary_output,
                                name,
                                *mode,
                                lossless_format,
                                registry,
                                cancelled,
                            )?
                        } else {
                            process_ncm_file_to_output(
                                src_path,
                                temporary_output,
                                name,
                                *mode,
                                lossless_format,
                            )?
                        }
                    }
                    #[cfg(not(feature = "ncm-decryption"))]
                    "ncm" => return Err(ncm_decryption_unavailable_error()),
                    _ => {
                        if let Some((registry, cancelled)) = control {
                            convert_audio_to_output_path_managed(
                                src_path,
                                temporary_output,
                                output_policy.target_profile,
                                name,
                                registry,
                                cancelled,
                            )?
                        } else {
                            convert_audio_to_output_path(
                                src_path,
                                temporary_output,
                                output_policy.target_profile,
                                name,
                            )?
                        }
                    }
                }

                ensure_not_cancelled(control)?;
                ensure_output_metadata_with_settings_with_context_and_policy(
                    src_path,
                    temporary_output,
                    netease_filename_format,
                    filename_policy,
                    metadata_context,
                )?;
                ensure_not_cancelled(control)?;
                finalize_output(name, temporary_output)
            })
        }
        _ => unreachable!(
            "Invalid file extension '{}' for song '{}'. Filter failed.",
            extension, name
        ),
    };

    if result.is_ok() {
        remove_conflicting_outputs(dest_folder, name, output_policy.output_extension, src_path)?;
        if let Some(recovered) = crate::netease::recover_local_metadata_with_resolver(
            src_path,
            metadata_context.netease.as_ref(),
        )
        .metadata
            && !recovered.lyric_lrc_text.trim().is_empty()
            && let Err(error) = write_lrc_sidecar(&output_path, &recovered.lyric_lrc_text)
        {
            bar.println(format!(
                "歌词 sidecar 写入失败（不影响音频）：{}: {}",
                name, error
            ));
        }
    }

    result
}

fn copy_file(src_path: &Path, dest_path: &Path) -> io::Result<()> {
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src_path, dest_path).map(|_| ()).map_err(|error| {
        Error::new(
            error.kind(),
            format!(
                "Failed to copy {} to {}: {}",
                src_path.display(),
                dest_path.display(),
                error
            ),
        )
    })
}

fn ensure_not_cancelled(control: Option<(&ActiveFfmpegRegistry, &AtomicBool)>) -> io::Result<()> {
    if control.is_some_and(|(_, cancelled)| cancelled.load(Ordering::SeqCst)) {
        return Err(Error::new(ErrorKind::Interrupted, "转换已取消"));
    }
    Ok(())
}

fn convert_audio_to_output_path(
    src_path: &Path,
    output_path: &Path,
    target_profile: TargetProfile,
    name_stem: &str,
) -> io::Result<()> {
    let registry = ActiveFfmpegRegistry::new();
    let cancelled = AtomicBool::new(false);
    convert_audio_to_output_path_managed(
        src_path,
        output_path,
        target_profile,
        name_stem,
        &registry,
        &cancelled,
    )
}

fn convert_audio_to_output_path_managed(
    src_path: &Path,
    output_path: &Path,
    target_profile: TargetProfile,
    name_stem: &str,
    registry: &ActiveFfmpegRegistry,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let ffmpeg_path = find_ffmpeg().ok_or_else(|| {
        Error::new(
            ErrorKind::NotFound,
            "FFmpeg not found. Put the sidecar next to the app, in a binaries/ folder, set W4DJ_FFMPEG_PATH, or install FFmpeg in PATH.",
        )
    })?;

    let mut command = Command::new(&ffmpeg_path);
    configure_background_process(&mut command);
    command
        .arg("-y")
        .arg("-i")
        .arg(src_path)
        .arg("-loglevel")
        .arg("quiet")
        .arg("-map_metadata")
        .arg("0");

    match target_profile {
        TargetProfile::CompatMp3 => {
            command.arg("-q:a").arg("0").arg("-id3v2_version").arg("3");
        }
        TargetProfile::LosslessWav => {
            command.arg("-c:a").arg("pcm_s24le");
        }
        TargetProfile::LosslessAiff => {
            command.arg("-c:a").arg("pcm_s24be");
        }
    }

    if cancelled.load(Ordering::SeqCst) {
        let _ = fs::remove_file(output_path);
        return Err(Error::new(ErrorKind::Interrupted, "转换已取消"));
    }
    let child = match command.arg(output_path).spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = fs::remove_file(output_path);
            return Err(Error::new(
                error.kind(),
                format!("Failed to start FFmpeg at {}: {}", ffmpeg_path, error),
            ));
        }
    };
    let child_id = registry.insert(child);
    let status = registry.wait_for(child_id, cancelled)?;

    if !status.success() {
        let _ = fs::remove_file(output_path);
        if cancelled.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Interrupted, "转换已取消"));
        }
        return Err(Error::other(format!(
            "FFmpeg conversion failed for {}",
            name_stem
        )));
    }

    ensure_generated_output(output_path, name_stem)
}

fn create_persistent_temp_path(
    output_path: &Path,
    prefix: &str,
    remove_placeholder: bool,
) -> io::Result<PathBuf> {
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let suffix = output_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let temp_file = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(&suffix)
        .tempfile_in(parent)
        .map_err(|error| {
            Error::new(
                error.kind(),
                format!("Failed to create a temporary output file: {error}"),
            )
        })?;
    let (file, path) = temp_file.keep().map_err(|error| {
        Error::new(
            error.error.kind(),
            format!("Failed to keep a temporary output file: {}", error.error),
        )
    })?;
    drop(file);
    if remove_placeholder {
        fs::remove_file(&path)?;
    }
    Ok(path)
}

fn commit_temporary_output(temporary_path: &Path, output_path: &Path) -> io::Result<()> {
    if !output_path.exists() {
        return fs::rename(temporary_path, output_path);
    }

    let backup_path = create_persistent_temp_path(output_path, ".w4dj-backup-", true)?;
    fs::rename(output_path, &backup_path)?;

    match fs::rename(temporary_path, output_path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup_path);
            Ok(())
        }
        Err(error) => {
            if let Err(restore_error) = fs::rename(&backup_path, output_path) {
                return Err(Error::other(format!(
                    "Failed to commit converted output {}: {}; restoring the previous file also failed: {}",
                    output_path.display(),
                    error,
                    restore_error
                )));
            }
            Err(error)
        }
    }
}

fn run_output_transaction(
    output_path: &Path,
    name_stem: &str,
    operation: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let temporary_output = create_persistent_temp_path(output_path, ".w4dj-", true)?;
    let result = operation(&temporary_output)
        .and_then(|()| ensure_generated_output(&temporary_output, name_stem))
        .and_then(|()| strip_163_key_from_output(&temporary_output))
        .and_then(|()| commit_temporary_output(&temporary_output, output_path));

    if result.is_err() {
        let _ = fs::remove_file(&temporary_output);
    }

    result
}

fn ensure_generated_output(output_path: &Path, name_stem: &str) -> io::Result<()> {
    let metadata = fs::metadata(output_path).map_err(|error| {
        Error::new(
            error.kind(),
            format!(
                "FFmpeg reported success for {}, but output {} is unavailable: {}",
                name_stem,
                output_path.display(),
                error
            ),
        )
    })?;

    if metadata.len() == 0 {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "FFmpeg produced an empty output for {}: {}",
                name_stem,
                output_path.display()
            ),
        ));
    }

    Ok(())
}

fn strip_163_key_from_output(path: &Path) -> io::Result<()> {
    let is_mp3 = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"));
    if is_mp3 {
        strip_163_key_from_mp3(path)?;
    }
    Ok(())
}

#[allow(dead_code)]
fn ensure_output_metadata(source_path: &Path, output_path: &Path) -> io::Result<()> {
    let netease_filename_format = if source_prefers_title_artist_filename(source_path) {
        NeteaseFilenameFormat::TitleArtist
    } else {
        NeteaseFilenameFormat::ArtistTitle
    };
    let context = ConversionMetadataContext {
        netease: Arc::new(NeteaseMetadataResolver::default()),
    };
    ensure_output_metadata_with_settings_with_context_and_policy(
        source_path,
        output_path,
        netease_filename_format,
        FilenameNormalizationPolicy::PreserveSource,
        &context,
    )
}

#[cfg(test)]
fn ensure_output_metadata_with_settings(
    source_path: &Path,
    output_path: &Path,
    netease_filename_format: NeteaseFilenameFormat,
) -> io::Result<()> {
    let context = ConversionMetadataContext {
        netease: Arc::new(NeteaseMetadataResolver::default()),
    };
    ensure_output_metadata_with_settings_with_context(
        source_path,
        output_path,
        netease_filename_format,
        &context,
    )
}

#[cfg(test)]
fn ensure_output_metadata_with_settings_with_context(
    source_path: &Path,
    output_path: &Path,
    netease_filename_format: NeteaseFilenameFormat,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<()> {
    ensure_output_metadata_with_settings_with_context_and_policy(
        source_path,
        output_path,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
        metadata_context,
    )
}

fn ensure_output_metadata_with_settings_with_context_and_policy(
    source_path: &Path,
    output_path: &Path,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<()> {
    let source_tag = match filename_policy {
        FilenameNormalizationPolicy::PreserveSource => {
            // Task 1 preserves the source filename character policy, while
            // still allowing metadata to fall back to the read-only NetEase
            // database for untagged FLAC/MP3 inputs.
            source_metadata_as_id3_with_resolver(source_path, &metadata_context.netease)
        }
        FilenameNormalizationPolicy::SoundCloud => {
            source_metadata_as_id3_without_resolver(source_path)
        }
    };
    let mut output_tag = read_id3_tag_or_empty(output_path);
    let fallback_name = source_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();

    let source_artist = source_tag.artist().or_else(|| source_tag.album_artist());
    let identity = infer_song_identity_with_netease_filename_format(
        fallback_name,
        source_tag.title(),
        source_artist,
        netease_filename_format,
        filename_policy,
    );
    let source_has_valid_cover = source_tag
        .pictures()
        .any(|picture| crate::metadata::get_image_mime_type(&picture.data) != "image/*");

    let output_album_was_blank = is_blank(output_tag.album());
    let changed_identity = fill_missing_metadata(&mut output_tag, &source_tag, &identity);
    let changed_lyrics = merge_lyrics_metadata(&mut output_tag, &source_tag);
    if !changed_identity && !changed_lyrics {
        return Ok(());
    }

    write_id3_tag_for_output(&output_tag, output_path)?;
    let expected_album = output_album_was_blank
        .then(|| non_empty(source_tag.album()))
        .flatten();
    validate_written_metadata(
        output_path,
        &identity,
        source_has_valid_cover,
        expected_album,
    )
}

/// Writes Essentia's analysis into the native metadata of a converted file.
///
/// BPM and key use the standard ID3 text frames (`TBPM` and `TKEY`) so DJ
/// applications can discover them directly. The richer values are kept in
/// named `TXXX` frames and a readable comment for applications that do not
/// expose custom tags in their library columns.
#[allow(dead_code)]
pub fn apply_track_analysis_metadata(
    output_path: &Path,
    analysis: &EmbeddedAnalysis,
) -> io::Result<()> {
    let context = ConversionMetadataContext::default();
    apply_track_analysis_metadata_with_context(output_path, analysis, &context)
}

pub fn apply_track_analysis_metadata_with_context(
    output_path: &Path,
    analysis: &EmbeddedAnalysis,
    metadata_context: &ConversionMetadataContext,
) -> io::Result<()> {
    let mut tag = read_id3_tag_or_empty(output_path);
    if let Some(recovered) = crate::netease::recover_local_metadata_with_resolver(
        Path::new(&analysis.path),
        &metadata_context.netease,
    )
    .metadata
    {
        merge_recovered_metadata(&mut tag, &recovered);
    }

    if is_blank(tag.title()) && !analysis.title.trim().is_empty() {
        tag.set_title(analysis.title.trim());
    }
    if is_blank(tag.artist()) && !analysis.artist.trim().is_empty() {
        tag.set_artist(analysis.artist.trim());
    }
    if is_blank(tag.album()) && !analysis.album.trim().is_empty() {
        tag.set_album(analysis.album.trim());
    }
    if is_blank(tag.genre())
        && let Some(genre) = (!analysis.genre.trim().is_empty())
            .then_some(analysis.genre.trim())
            .or_else(|| {
                analysis.high_level.as_ref().and_then(|high_level| {
                    high_level.genre.iter().find_map(|label| {
                        (label.confidence.is_finite() && label.confidence >= 0.75)
                            .then(|| label.label.trim())
                            .filter(|label| !label.is_empty())
                    })
                })
            })
    {
        tag.set_genre(genre);
    }

    if let Some(bpm) = analysis
        .bpm
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        tag.set_text("TBPM", format!("{bpm:.2}"));
    }
    if let Some(key) = analysis_key(analysis) {
        tag.set_text("TKEY", key);
    }

    let custom_values = [
        (
            "W4DJ-Loudness-LUFS",
            analysis
                .integrated_loudness_lufs
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.2}")),
        ),
        (
            "W4DJ-Loudness-Range-LU",
            analysis
                .loudness_range_lu
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.2}")),
        ),
        (
            "W4DJ-Energy",
            analysis
                .energy
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.4}")),
        ),
        (
            "W4DJ-Danceability",
            analysis
                .danceability
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.4}")),
        ),
        (
            "W4DJ-Key-Confidence",
            analysis
                .key_strength
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.4}")),
        ),
        (
            "W4DJ-Drop-Loudness-LUFS",
            analysis
                .drop_loudness_lufs
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.2}")),
        ),
        (
            "W4DJ-Beat-Positions",
            (!analysis.beat_positions.is_empty()).then(|| {
                serde_json::to_string(
                    &analysis
                        .beat_positions
                        .iter()
                        .copied()
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .take(2000)
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()
            }),
        ),
        ("W4DJ-Analyzer", Some(analysis.analyzer.clone())),
        (
            "W4DJ-Analysis-Version",
            Some(analysis.analysis_version.clone()),
        ),
    ];

    for (description, value) in custom_values {
        tag.remove_extended_text(Some(description), None);
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            tag.add_frame(ExtendedText {
                description: description.to_string(),
                value,
            });
        }
    }

    // Discogs-EffNet is an optional, namespaced projection.  Keep each head
    // independent: a missing or failed head must not erase a previously
    // successful value written by an earlier analysis run.  JSON is used here
    // rather than a lossy label-only string so the Dashboard can reproduce
    // thresholds, scores and the model version from the output file alone.
    if let Some(discogs) = analysis
        .high_level
        .as_ref()
        .and_then(|high| high.discogs_effnet.as_ref())
    {
        const NAMESPACED_HEADS: [(&str, &str); 5] = [
            ("moodTheme", "W4DJ-Discogs-MoodTheme"),
            ("approachability", "W4DJ-Discogs-Approachability"),
            ("instrumentation", "W4DJ-Discogs-Instrumentation"),
            ("timbre", "W4DJ-Discogs-Timbre"),
            ("danceability", "W4DJ-Discogs-Danceability"),
        ];
        for (head_id, description) in NAMESPACED_HEADS {
            let Some(head) = discogs.heads.get(head_id) else {
                continue;
            };
            if head.status != "completed" {
                continue;
            }
            let Ok(value) = serde_json::to_string(head) else {
                continue;
            };
            if value.trim().is_empty() {
                continue;
            }
            tag.remove_extended_text(Some(description), None);
            tag.add_frame(ExtendedText {
                description: description.to_string(),
                value,
            });
        }
    }

    tag.remove_comment(Some("W4DJ Essentia"), None);
    tag.add_frame(Comment {
        lang: String::from("eng"),
        description: String::from("W4DJ Essentia"),
        text: analysis_summary(analysis),
    });

    write_id3_tag_for_output(&tag, output_path)
}

/// Re-reads the native output after an analysis write and verifies the
/// fields that make the result discoverable to DJ applications. A successful
/// `write_id3_tag_for_output` alone is not enough: a later container/permission
/// failure must not be reported as a completed analysis.
pub fn validate_track_analysis_metadata(
    output_path: &Path,
    analysis: &EmbeddedAnalysis,
) -> io::Result<()> {
    let tag = id3::Tag::read_from_path(output_path).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("无法重新读取分析元数据：{error}"),
        )
    })?;

    if analysis.bpm.is_some() && tag.text_for_frame_id("TBPM").is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "分析元数据回读缺少 TBPM",
        ));
    }
    if analysis.key.is_some() && tag.text_for_frame_id("TKEY").is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "分析元数据回读缺少 TKEY",
        ));
    }

    let expected = [
        (
            "W4DJ-Danceability",
            analysis
                .danceability
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.4}")),
        ),
        (
            "W4DJ-Energy",
            analysis
                .energy
                .filter(|value| value.is_finite())
                .map(|value| format!("{value:.4}")),
        ),
        (
            "W4DJ-Analysis-Version",
            Some(analysis.analysis_version.clone()),
        ),
    ];
    for (description, expected_value) in expected {
        let Some(expected_value) = expected_value else {
            continue;
        };
        let found = tag
            .extended_texts()
            .find(|frame| frame.description == description)
            .map(|frame| frame.value.as_str());
        if found != Some(expected_value.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("分析元数据回读缺少或不匹配 {description}"),
            ));
        }
    }

    if let Some(discogs) = analysis
        .high_level
        .as_ref()
        .and_then(|high| high.discogs_effnet.as_ref())
    {
        const NAMESPACED_HEADS: [(&str, &str); 5] = [
            ("moodTheme", "W4DJ-Discogs-MoodTheme"),
            ("approachability", "W4DJ-Discogs-Approachability"),
            ("instrumentation", "W4DJ-Discogs-Instrumentation"),
            ("timbre", "W4DJ-Discogs-Timbre"),
            ("danceability", "W4DJ-Discogs-Danceability"),
        ];
        for (head_id, description) in NAMESPACED_HEADS {
            let Some(head) = discogs.heads.get(head_id) else {
                continue;
            };
            if head.status != "completed" {
                continue;
            }
            let expected = serde_json::to_string(head).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("生成 {description} 校验值失败：{error}"),
                )
            })?;
            let found = tag
                .extended_texts()
                .find(|frame| frame.description == description)
                .map(|frame| frame.value.as_str());
            if found != Some(expected.as_str()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("分析元数据回读缺少或不匹配 {description}"),
                ));
            }
        }
    }
    if !tag
        .comments()
        .any(|comment| comment.description == "W4DJ Essentia")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "分析元数据回读缺少 W4DJ Essentia Comment",
        ));
    }
    Ok(())
}

#[allow(dead_code)]
fn analysis_key(analysis: &EmbeddedAnalysis) -> Option<String> {
    let key = analysis
        .key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .replace('♯', "#")
        .replace('♭', "b");
    let key = if analysis
        .scale
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("minor"))
    {
        format!("{key}m")
    } else {
        key
    };
    Some(key)
}

#[allow(dead_code)]
fn analysis_summary(analysis: &EmbeddedAnalysis) -> String {
    let mut values = Vec::new();
    if let Some(value) = analysis
        .bpm
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        values.push(format!("BPM {value:.2}"));
    }
    if let Some(value) = analysis_key(analysis) {
        values.push(format!("Key {value}"));
    }
    if let Some(value) = analysis
        .integrated_loudness_lufs
        .filter(|value| value.is_finite())
    {
        values.push(format!("Loudness {value:.2} LUFS"));
    }
    if let Some(value) = analysis.energy.filter(|value| value.is_finite()) {
        values.push(format!("Energy {value:.4}"));
    }
    if let Some(value) = analysis.danceability.filter(|value| value.is_finite()) {
        values.push(format!("Danceability {value:.4}"));
    }
    if let Some(value) = analysis
        .drop_loudness_lufs
        .filter(|value| value.is_finite())
    {
        values.push(format!("Drop {value:.2} LUFS"));
    }
    if let Some(high_level) = &analysis.high_level {
        let moods = high_level
            .mood
            .iter()
            .map(|label| label.label.trim())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        if !moods.is_empty() {
            values.push(format!("Mood {}", moods.join(", ")));
        }
        let instruments = high_level
            .instrument
            .iter()
            .map(|label| label.label.trim())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        if !instruments.is_empty() {
            values.push(format!("Instrument {}", instruments.join(", ")));
        }
        let genres = high_level
            .genre
            .iter()
            .map(|label| label.label.trim())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        if !genres.is_empty() {
            values.push(format!("Genre {}", genres.join(", ")));
        }
        if let Some(discogs) = &high_level.discogs_effnet {
            for (head_id, title) in [
                ("moodTheme", "Discogs Mood/Theme"),
                ("approachability", "Discogs Approachability"),
                ("instrumentation", "Discogs Instrumentation"),
                ("timbre", "Discogs Timbre"),
                ("danceability", "Discogs Danceability"),
            ] {
                let Some(head) = discogs.heads.get(head_id) else {
                    continue;
                };
                if head.status != "completed" {
                    continue;
                }
                let display = head
                    .selected_class
                    .as_deref()
                    .or_else(|| head.labels.first().map(|label| label.label.as_str()))
                    .unwrap_or_default();
                if !display.trim().is_empty() {
                    values.push(format!("{title} {display}"));
                }
            }
        }
    }
    if values.is_empty() {
        String::from("W4DJ Essentia analysis")
    } else {
        format!("W4DJ Essentia | {}", values.join(" | "))
    }
}

fn source_metadata_as_id3_without_resolver(source_path: &Path) -> id3::Tag {
    read_source_container_metadata(source_path)
}

fn source_metadata_as_id3_with_resolver(
    source_path: &Path,
    resolver: &NeteaseMetadataResolver,
) -> id3::Tag {
    let mut tag = read_source_container_metadata(source_path);

    if !matches!(
        source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "ncm"
    ) && let Some(recovered) =
        crate::netease::recover_local_metadata_with_resolver(source_path, resolver).metadata
    {
        merge_recovered_metadata(&mut tag, &recovered);
    }

    tag
}

fn read_source_container_metadata(source_path: &Path) -> id3::Tag {
    let extension = source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "flac" => metaflac::Tag::read_from_path(source_path)
            .map(|tag| build_id3_tag_from_flac(&tag))
            .unwrap_or_else(|_| id3::Tag::new()),
        #[cfg(feature = "ncm-decryption")]
        "ncm" => {
            let Ok(file) = File::open(source_path) else {
                return id3::Tag::new();
            };
            let Ok(mut ncm) = Ncmdump::from_reader(file) else {
                return id3::Tag::new();
            };
            let Ok(info) = ncm.get_info() else {
                return id3::Tag::new();
            };
            let image = ncm.get_image().unwrap_or_default();
            build_id3_tag(&info, &image)
        }
        #[cfg(not(feature = "ncm-decryption"))]
        "ncm" => id3::Tag::new(),
        _ => read_id3_tag_or_empty(source_path),
    }
}

fn merge_recovered_metadata(tag: &mut id3::Tag, recovered: &RecoveredMetadata) -> bool {
    let mut changed = false;

    if is_blank(tag.title()) && !recovered.title.trim().is_empty() {
        tag.set_title(recovered.title.trim());
        changed = true;
    }
    if is_blank(tag.artist()) && !recovered.artist.trim().is_empty() {
        tag.set_artist(recovered.artist.trim());
        changed = true;
    }
    if is_blank(tag.album()) && !recovered.album.trim().is_empty() {
        tag.set_album(recovered.album.trim());
        changed = true;
    }
    if is_blank(tag.genre()) && !recovered.genre.trim().is_empty() {
        tag.set_genre(recovered.genre.trim());
        changed = true;
    }
    if tag.get("TDRC").is_none() && !recovered.publish_date.trim().is_empty() {
        tag.set_text("TDRC", recovered.publish_date.trim());
        changed = true;
    }
    changed |= add_extended_text_if_missing(tag, "W4DJ-Netease-Aliases", &recovered.aliases_json);
    changed |=
        add_extended_text_if_missing(tag, "W4DJ-Netease-Copyright", &recovered.copyright_text);

    let has_cover = tag
        .pictures()
        .any(|picture| crate::metadata::get_image_mime_type(&picture.data) != "image/*");
    if !has_cover
        && let Some(cover) = recovered.cover.as_deref()
        && crate::metadata::get_image_mime_type(cover) != "image/*"
    {
        tag.add_frame(Picture {
            mime_type: crate::metadata::get_image_mime_type(cover).to_string(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: cover.to_vec(),
        });
        changed = true;
    }

    changed |= add_lyrics_if_missing(
        tag,
        &recovered.lyric_plain_text,
        &recovered.lyric_language,
        "W4DJ NetEase",
    );
    changed |= add_lyrics_if_missing(
        tag,
        &recovered.lyric_translated_text,
        &recovered.lyric_language,
        "W4DJ NetEase (translated)",
    );
    changed |= add_lyrics_if_missing(
        tag,
        &recovered.lyric_romanized_text,
        &recovered.lyric_language,
        "W4DJ NetEase (romanized)",
    );

    changed
}

fn add_extended_text_if_missing(tag: &mut id3::Tag, description: &str, value: &str) -> bool {
    let value = value.trim();
    if value.is_empty()
        || tag
            .extended_texts()
            .any(|frame| frame.description == description)
    {
        return false;
    }
    tag.add_frame(ExtendedText {
        description: description.to_string(),
        value: value.to_string(),
    });
    true
}

fn add_lyrics_if_missing(
    tag: &mut id3::Tag,
    text: &str,
    language: &str,
    description: &str,
) -> bool {
    let text = text.trim();
    if text.is_empty()
        || tag
            .lyrics()
            .any(|frame| frame.description == description && frame.text.trim() == text)
    {
        return false;
    }
    tag.add_frame(Lyrics {
        lang: id3_language(language),
        description: description.to_string(),
        text: text.to_string(),
    });
    true
}

fn merge_lyrics_metadata(target: &mut id3::Tag, source: &id3::Tag) -> bool {
    let source_lyrics = source
        .lyrics()
        .map(|frame| {
            (
                frame.lang.clone(),
                frame.description.clone(),
                frame.text.clone(),
            )
        })
        .collect::<Vec<_>>();
    let mut changed = false;
    for (lang, description, text) in source_lyrics {
        if text.trim().is_empty()
            || target
                .lyrics()
                .any(|frame| frame.description == description && frame.text.trim() == text.trim())
        {
            continue;
        }
        target.add_frame(Lyrics {
            lang,
            description,
            text,
        });
        changed = true;
    }
    changed
}

fn write_lrc_sidecar(audio_path: &Path, value: &str) -> io::Result<Option<PathBuf>> {
    #[cfg(test)]
    {
        let contents = value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .trim()
            .to_string();
        if contents.is_empty() {
            return Ok(None);
        }
        let parent = audio_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let stem = audio_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("track");
        let path = parent.join(format!("{stem}.lrc"));
        fs::write(&path, contents)?;
        Ok(Some(path))
    }
    #[cfg(not(test))]
    {
        crate::lyrics::write_lrc_sidecar(audio_path, value)
    }
}

fn id3_language(value: &str) -> String {
    let value = value.trim();
    if value.len() == 3 && value.is_ascii() {
        value.to_ascii_lowercase()
    } else {
        String::from("und")
    }
}

fn read_id3_tag_or_empty(path: &Path) -> id3::Tag {
    id3::Tag::read_from_path(path).unwrap_or_else(|_| id3::Tag::new())
}

fn fill_missing_metadata(
    output_tag: &mut id3::Tag,
    source_tag: &id3::Tag,
    identity: &SongIdentity,
) -> bool {
    let mut changed = false;

    if !identity.title.is_empty() && output_tag.title() != Some(identity.title.as_str()) {
        output_tag.set_title(&identity.title);
        changed = true;
    }

    if !identity.artist.is_empty() && output_tag.artist() != Some(identity.artist.as_str()) {
        output_tag.set_artist(&identity.artist);
        changed = true;
    }

    if is_blank(output_tag.album())
        && let Some(album) = non_empty(source_tag.album())
    {
        output_tag.set_album(album);
        changed = true;
    }

    let source_pictures = source_tag
        .pictures()
        .filter(|picture| crate::metadata::get_image_mime_type(&picture.data) != "image/*")
        .collect::<Vec<_>>();
    if !source_pictures.is_empty() {
        output_tag.remove("APIC");
        for picture in source_pictures {
            output_tag.add_frame(Picture {
                mime_type: picture.mime_type.clone(),
                picture_type: picture.picture_type,
                description: picture.description.clone(),
                data: picture.data.clone(),
            });
            changed = true;
        }
    }

    changed
}

fn validate_written_metadata(
    output_path: &Path,
    identity: &SongIdentity,
    expect_cover: bool,
    expected_album: Option<&str>,
) -> io::Result<()> {
    let written = read_id3_tag_or_empty(output_path);
    if !identity.title.is_empty() && written.title() != Some(identity.title.as_str()) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("输出标题写入校验失败：{}", output_path.display()),
        ));
    }
    if !identity.artist.is_empty() && written.artist() != Some(identity.artist.as_str()) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("输出歌手写入校验失败：{}", output_path.display()),
        ));
    }
    if let Some(expected_album) = expected_album.filter(|value| !value.trim().is_empty())
        && written.album() != Some(expected_album)
    {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("输出专辑写入校验失败：{}", output_path.display()),
        ));
    }
    if expect_cover
        && !written
            .pictures()
            .any(|picture| crate::metadata::get_image_mime_type(&picture.data) != "image/*")
    {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("输出封面写入校验失败：{}", output_path.display()),
        ));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn inspect_metadata_diagnostic(source_path: &Path, output_path: &Path) -> MetadataDiagnostic {
    let resolver = NeteaseMetadataResolver::load(None).unwrap_or_default();
    inspect_metadata_diagnostic_with_resolver(source_path, output_path, &resolver)
}

pub fn inspect_metadata_diagnostic_with_resolver(
    source_path: &Path,
    output_path: &Path,
    resolver: &NeteaseMetadataResolver,
) -> MetadataDiagnostic {
    let source_tag = read_source_container_metadata(source_path);
    let recovery = crate::netease::recover_local_metadata_with_resolver(source_path, resolver);
    let recovered = recovery.metadata.as_ref();
    let mut effective_tag = source_tag.clone();
    if let Some(recovered) = recovered {
        merge_recovered_metadata(&mut effective_tag, recovered);
    }
    let output_exists = output_path.is_file();
    let output_tag = output_exists.then(|| read_id3_tag_or_empty(output_path));
    let fallback_name = source_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let prefer_title_artist = source_prefers_title_artist_filename(source_path);
    let source_title = non_empty(source_tag.title()).map(str::to_string);
    let source_artist =
        non_empty(source_tag.artist().or_else(|| source_tag.album_artist())).map(str::to_string);
    let source_album = non_empty(source_tag.album()).map(str::to_string);
    let filename_identity = infer_song_identity_with_filename_preference(
        fallback_name,
        source_title.as_deref(),
        source_artist.as_deref(),
        prefer_title_artist,
    );
    let (selected_title, title_source) = select_metadata_field(
        source_title.as_deref(),
        recovered.map(|value| value.title.as_str()),
        Some(filename_identity.title.as_str()),
    );
    let (selected_artist, artist_source) = select_metadata_field(
        source_artist.as_deref(),
        recovered.map(|value| value.artist.as_str()),
        Some(filename_identity.artist.as_str()),
    );
    let (selected_album, album_source) = select_metadata_field(
        source_album.as_deref(),
        recovered.map(|value| value.album.as_str()),
        None,
    );
    let source_tags_are_reliable = source_title.is_some() && source_artist.is_some();
    let valid_cover = |tag: &id3::Tag| {
        tag.pictures()
            .any(|picture| crate::metadata::get_image_mime_type(&picture.data) != "image/*")
    };
    let output_matches = output_tag.as_ref().is_some_and(|tag| {
        selected_title
            .as_deref()
            .is_none_or(|expected| tag.title() == Some(expected))
            && selected_artist
                .as_deref()
                .is_none_or(|expected| tag.artist() == Some(expected))
            && selected_album
                .as_deref()
                .is_none_or(|expected| tag.album() == Some(expected))
            && (!valid_cover(&effective_tag) || valid_cover(tag))
    });
    let output_title = output_tag
        .as_ref()
        .and_then(|tag| non_empty(tag.title()).map(str::to_string));
    let output_artist = output_tag
        .as_ref()
        .and_then(|tag| non_empty(tag.artist().or_else(|| tag.album_artist())).map(str::to_string));
    let output_album = output_tag
        .as_ref()
        .and_then(|tag| non_empty(tag.album()).map(str::to_string));
    let validation_basis = if source_tags_are_reliable {
        "source_tags"
    } else if title_source.as_deref() == Some("neteaseDatabase")
        || artist_source.as_deref() == Some("neteaseDatabase")
        || album_source.as_deref() == Some("neteaseDatabase")
    {
        "netease_database"
    } else {
        "filename_inference"
    };

    MetadataDiagnostic {
        source_path: source_path.display().to_string(),
        destination_path: output_path.display().to_string(),
        source_filename: fallback_name.to_string(),
        source_extension: source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase(),
        source_size_bytes: fs::metadata(source_path)
            .ok()
            .map(|metadata| metadata.len()),
        output_size_bytes: fs::metadata(output_path)
            .ok()
            .map(|metadata| metadata.len()),
        source_title,
        source_artist,
        source_album,
        output_title: output_title.clone(),
        output_artist: output_artist.clone(),
        output_album: output_album.clone(),
        source_artwork: valid_cover(&source_tag),
        output_artwork: output_tag.as_ref().map(valid_cover),
        detected_filename_layout: if split_filename_identity(fallback_name).is_some() {
            if prefer_title_artist {
                "标题 - 歌手"
            } else {
                "歌手 - 标题"
            }
            .to_string()
        } else {
            "未检测到分隔符".to_string()
        },
        decision: format!(
            "最终标题：{}；最终歌手：{}；最终专辑：{}",
            selected_title.as_deref().unwrap_or("无"),
            selected_artist.as_deref().unwrap_or("无"),
            selected_album.as_deref().unwrap_or("无")
        ),
        metadata_validation: if !output_exists {
            "输出文件不存在或转换失败".to_string()
        } else if output_matches {
            if source_tags_are_reliable {
                "通过：按源文件可靠字段比较，输出标题、歌手、专辑和可用封面已精确校验".to_string()
            } else if validation_basis == "netease_database" {
                "通过：按网易云正式数据库补全字段比较，输出标题、歌手、专辑和可用封面已精确校验"
                    .to_string()
            } else {
                "通过：按可靠文件名推断比较，输出标题、歌手、专辑和可用封面已精确校验".to_string()
            }
        } else if source_tags_are_reliable {
            "未通过：输出标题、歌手、专辑或封面与源文件可靠字段不一致".to_string()
        } else if validation_basis == "netease_database" {
            "未通过：输出标题、歌手、专辑或封面与网易云正式数据库字段不一致".to_string()
        } else {
            "未通过：输出标题、歌手、专辑或封面与文件名推断不一致".to_string()
        },
        validation_basis: Some(validation_basis.to_string()),
        output_tags_match: output_exists.then_some(output_matches),
        title_source,
        artist_source,
        album_source,
        title_difference: Some(metadata_difference(
            selected_title.as_deref(),
            output_title.as_deref(),
        )),
        artist_difference: Some(metadata_difference(
            selected_artist.as_deref(),
            output_artist.as_deref(),
        )),
        album_difference: Some(metadata_difference(
            selected_album.as_deref(),
            output_album.as_deref(),
        )),
        netease_recovery: Some(recovery.diagnostic),
    }
}

fn select_metadata_field(
    source: Option<&str>,
    database: Option<&str>,
    filename: Option<&str>,
) -> (Option<String>, Option<String>) {
    for (value, source_name) in [
        (source, "sourceTag"),
        (database, "neteaseDatabase"),
        (filename, "filenameInference"),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            return (
                Some(value.trim().to_string()),
                Some(source_name.to_string()),
            );
        }
    }
    (None, None)
}

fn metadata_difference(expected: Option<&str>, actual: Option<&str>) -> String {
    let Some(expected) = expected else {
        return "exact".to_string();
    };
    let Some(actual) = actual else {
        return "different".to_string();
    };
    if expected == actual {
        return "exact".to_string();
    }
    if expected.eq_ignore_ascii_case(actual) {
        return "caseOnly".to_string();
    }
    let collapse_whitespace = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapse_whitespace(expected) == collapse_whitespace(actual) {
        return "whitespaceOnly".to_string();
    }
    if crate::netease::tolerant_comparison_key(expected)
        == crate::netease::tolerant_comparison_key(actual)
    {
        return "punctuationOnly".to_string();
    }
    "different".to_string()
}

fn write_id3_tag_for_output(tag: &id3::Tag, output_path: &Path) -> io::Result<()> {
    let extension = output_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    #[allow(deprecated)]
    let result = match extension.as_str() {
        "mp3" => tag.write_to_path(output_path, Version::Id3v24),
        "wav" => {
            tag.write_to_wav_path(output_path, Version::Id3v24)
                .map_err(io::Error::other)?;
            return write_riff_info_metadata(output_path, tag);
        }
        "aiff" | "aif" => tag.write_to_aiff_path(output_path, Version::Id3v24),
        _ => return Ok(()),
    };

    result.map_err(io::Error::other)
}

fn write_riff_info_metadata(output_path: &Path, tag: &id3::Tag) -> io::Result<()> {
    let mut input = File::open(output_path)?;
    let mut header = [0u8; 12];
    input.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("不是有效的 RIFF/WAVE 文件：{}", output_path.display()),
        ));
    }

    let temporary_path = create_persistent_temp_path(output_path, ".w4dj-riff-", true)?;
    let result = (|| {
        let mut output = File::create(&temporary_path)?;
        output.write_all(&header)?;
        let mut chunk_header = [0u8; 8];

        loop {
            let bytes_read = input.read(&mut chunk_header)?;
            if bytes_read == 0 {
                break;
            }
            if bytes_read != chunk_header.len() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "WAV 文件包含不完整的 chunk 头",
                ));
            }

            let chunk_id: [u8; 4] = chunk_header[0..4]
                .try_into()
                .expect("chunk id is four bytes");
            let chunk_size = u32::from_le_bytes(
                chunk_header[4..8]
                    .try_into()
                    .expect("chunk size is four bytes"),
            ) as u64;
            let padded_size = chunk_size + (chunk_size & 1);
            let is_info_list = chunk_id == *b"LIST" && chunk_size >= 4;

            if is_info_list {
                let mut list_type = [0u8; 4];
                input.read_exact(&mut list_type)?;
                if &list_type == b"INFO" {
                    skip_bytes(&mut input, padded_size - 4)?;
                    continue;
                }

                output.write_all(&chunk_header)?;
                output.write_all(&list_type)?;
                copy_bytes(&mut input, &mut output, padded_size - 4)?;
            } else {
                output.write_all(&chunk_header)?;
                copy_bytes(&mut input, &mut output, padded_size)?;
            }
        }

        let mut info_payload = Vec::from(*b"INFO");
        append_riff_info_field(&mut info_payload, *b"INAM", tag.title());
        append_riff_info_field(&mut info_payload, *b"IART", tag.artist());
        append_riff_info_field(&mut info_payload, *b"IPRD", tag.album());
        if info_payload.len() > 4 {
            output.write_all(b"LIST")?;
            let info_size = u32::try_from(info_payload.len())
                .map_err(|_| Error::new(ErrorKind::InvalidData, "WAV INFO 元数据过大"))?;
            output.write_all(&info_size.to_le_bytes())?;
            output.write_all(&info_payload)?;
            if info_payload.len() & 1 == 1 {
                output.write_all(&[0])?;
            }
        }

        output.flush()?;
        output.sync_all()?;
        let file_size = output.metadata()?.len();
        let riff_size = u32::try_from(file_size.saturating_sub(8))
            .map_err(|_| Error::new(ErrorKind::InvalidData, "WAV 文件超过 RIFF 支持的大小"))?;
        output.seek(SeekFrom::Start(4))?;
        output.write_all(&riff_size.to_le_bytes())?;
        output.flush()?;
        output.sync_all()?;
        commit_temporary_output(&temporary_path, output_path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn append_riff_info_field(payload: &mut Vec<u8>, id: [u8; 4], value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    let Ok(size) = u32::try_from(bytes.len()) else {
        return;
    };
    payload.extend_from_slice(&id);
    payload.extend_from_slice(&size.to_le_bytes());
    payload.extend_from_slice(&bytes);
    if bytes.len() & 1 == 1 {
        payload.push(0);
    }
}

fn copy_bytes(input: &mut File, output: &mut File, mut bytes: u64) -> io::Result<()> {
    let mut buffer = [0u8; 8192];
    while bytes > 0 {
        let requested = usize::try_from(bytes.min(buffer.len() as u64)).unwrap_or(buffer.len());
        input.read_exact(&mut buffer[..requested])?;
        output.write_all(&buffer[..requested])?;
        bytes -= requested as u64;
    }
    Ok(())
}

fn skip_bytes(input: &mut File, mut bytes: u64) -> io::Result<()> {
    let mut buffer = [0u8; 8192];
    while bytes > 0 {
        let requested = usize::try_from(bytes.min(buffer.len() as u64)).unwrap_or(buffer.len());
        input.read_exact(&mut buffer[..requested])?;
        bytes -= requested as u64;
    }
    Ok(())
}

fn is_blank(value: Option<&str>) -> bool {
    value.is_none_or(|value| value.trim().is_empty())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(target_os = "windows")]
fn configure_background_process(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn configure_background_process(_command: &mut Command) {}

#[cfg(feature = "ncm-decryption")]
fn process_ncm_file_to_output(
    src_path: &Path,
    output_path: &Path,
    name_stem: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
) -> io::Result<()> {
    process_ncm_file_to_output_control(
        src_path,
        output_path,
        name_stem,
        mode,
        lossless_format,
        None,
    )
}

#[cfg(feature = "ncm-decryption")]
fn process_ncm_file_to_output_managed(
    src_path: &Path,
    output_path: &Path,
    name_stem: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    registry: &ActiveFfmpegRegistry,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    process_ncm_file_to_output_control(
        src_path,
        output_path,
        name_stem,
        mode,
        lossless_format,
        Some((registry, cancelled)),
    )
}

#[cfg(feature = "ncm-decryption")]
fn process_ncm_file_to_output_control(
    src_path: &Path,
    output_path: &Path,
    name_stem: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    control: Option<(&ActiveFfmpegRegistry, &AtomicBool)>,
) -> io::Result<()> {
    if control.is_some_and(|(_, cancelled)| cancelled.load(Ordering::SeqCst)) {
        return Err(Error::new(ErrorKind::Interrupted, "转换已取消"));
    }
    let file = File::open(src_path).map_err(|error| {
        Error::new(
            error.kind(),
            format!(
                "Failed to open source file {}: {}",
                src_path.display(),
                error
            ),
        )
    })?;
    let mut ncm = Ncmdump::from_reader(file).map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("NCM 解析错误 {}: {}", name_stem, e),
        )
    })?;
    // 提取原始音频数据
    let music_data = ncm.get_data().map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("NCM 数据提取错误 {}: {}", name_stem, e),
        )
    })?;
    // 提取专辑封面（关键修改点）
    let image_data = ncm.get_image().map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("NCM 封面提取错误 {}: {}", name_stem, e),
        )
    })?;
    // 提取歌曲元数据
    let ncm_metadata = ncm.get_info().map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("NCM 元数据错误 {}: {}", name_stem, e),
        )
    })?;
    // 确定输出格式（保持你的逻辑）
    let file_format = if ncm_metadata.format.is_empty() {
        "flac".to_string()
    } else {
        ncm_metadata.format.to_lowercase()
    };
    let output_policy = resolve_output_policy(mode, lossless_format, &file_format);
    let temp_source_extension = if file_format.as_str() == "mp3" {
        "mp3"
    } else {
        "flac"
    };
    let temp_data = match file_format.as_str() {
        "mp3" => Mp3Metadata::new(&ncm_metadata, &image_data, &music_data)
            .inject_metadata(music_data.clone())
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("MP3元数据注入失败 {}: {}", name_stem, e),
                )
            })?,
        "flac" => FlacMetadata::new(&ncm_metadata, &image_data, &music_data)
            .inject_metadata(music_data.clone())
            .map_err(|e| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("FLAC元数据注入失败 {}: {}", name_stem, e),
                )
            })?,
        _ => music_data,
    };

    let temp_suffix = format!(".{temp_source_extension}");
    let mut temp_file = tempfile::Builder::new()
        .prefix("w4dj-rkb-")
        .suffix(&temp_suffix)
        .tempfile()
        .map_err(|error| {
            Error::new(
                error.kind(),
                format!("Failed to create a temporary audio file: {error}"),
            )
        })?;
    temp_file.write_all(&temp_data).map_err(|error| {
        Error::new(
            error.kind(),
            format!("Failed to write temporary audio data: {error}"),
        )
    })?;
    temp_file.flush().map_err(|error| {
        Error::new(
            error.kind(),
            format!("Failed to flush temporary audio data: {error}"),
        )
    })?;
    let temp_source_path = temp_file.into_temp_path();

    match output_policy.target_profile {
        TargetProfile::CompatMp3 => {
            if file_format.as_str() == "mp3" {
                fs::copy(&temp_source_path, output_path)?;
            } else if let Some((registry, cancelled)) = control {
                convert_audio_to_output_path_managed(
                    &temp_source_path,
                    output_path,
                    TargetProfile::CompatMp3,
                    name_stem,
                    registry,
                    cancelled,
                )?;
            } else {
                convert_audio_to_output_path(
                    &temp_source_path,
                    output_path,
                    TargetProfile::CompatMp3,
                    name_stem,
                )?;
            }
        }
        TargetProfile::LosslessWav | TargetProfile::LosslessAiff => {
            if let Some((registry, cancelled)) = control {
                convert_audio_to_output_path_managed(
                    &temp_source_path,
                    output_path,
                    output_policy.target_profile,
                    name_stem,
                    registry,
                    cancelled,
                )?;
            } else {
                convert_audio_to_output_path(
                    &temp_source_path,
                    output_path,
                    output_policy.target_profile,
                    name_stem,
                )?;
            }

            write_container_tags(
                output_path,
                output_policy.target_profile,
                &ncm_metadata,
                &image_data,
            )?;
        }
    }

    Ok(())
}

pub(crate) fn target_output_path_with_policy(
    dest_folder: &str,
    name_stem: &str,
    output_extension: &str,
    filename_policy: FilenameNormalizationPolicy,
) -> PathBuf {
    let stem = match filename_policy {
        FilenameNormalizationPolicy::PreserveSource => {
            sanitize_preserve_source_filename_component(name_stem)
        }
        FilenameNormalizationPolicy::SoundCloud => sanitize_filename_component(name_stem),
    };
    Path::new(dest_folder).join(format!("{}.{}", stem, output_extension))
}

/// Compute the final output path using the same policy as the converter.  It
/// is intentionally a pure planning helper apart from the required NCM header
/// read; callers use it after a successful commit to register that one known
/// output instead of scanning the destination directory.
pub fn planned_output_path_with_policy(
    dest_folder: &str,
    name_stem: &str,
    source_path: &Path,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    filename_policy: FilenameNormalizationPolicy,
) -> io::Result<PathBuf> {
    let source_format = effective_source_extension(source_path);
    let output_policy = resolve_output_policy(mode, lossless_format, &source_format);
    Ok(target_output_path_with_policy(
        dest_folder,
        name_stem,
        output_policy.output_extension,
        filename_policy,
    ))
}

pub(crate) fn effective_source_extension(source_path: &Path) -> String {
    let path = source_path;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    if extension != "ncm" {
        return extension;
    }

    detect_ncm_output_extension(path).unwrap_or(extension)
}

#[cfg(feature = "ncm-decryption")]
fn detect_ncm_output_extension(src_path: &Path) -> io::Result<String> {
    let file = File::open(src_path)?;
    let mut ncm = Ncmdump::from_reader(file).map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("NCM 解析错误 {}: {}", src_path.display(), e),
        )
    })?;
    let info = ncm.get_info().map_err(|e| {
        Error::new(
            ErrorKind::InvalidData,
            format!("NCM 元数据错误 {}: {}", src_path.display(), e),
        )
    })?;

    Ok(info.format.trim().to_lowercase())
}

#[cfg(not(feature = "ncm-decryption"))]
fn detect_ncm_output_extension(_src_path: &Path) -> io::Result<String> {
    Err(ncm_decryption_unavailable_error())
}

fn remove_conflicting_outputs(
    dest_folder: &str,
    name_stem: &str,
    keep_extension: &str,
    protected_source_path: &Path,
) -> io::Result<()> {
    // Do not delete a same-stem file from another container without an
    // explicit ownership record proving it belongs to this source.
    let _ = (
        dest_folder,
        name_stem,
        keep_extension,
        protected_source_path,
    );
    Ok(())
}

/// Remove an old output only after its replacement has been committed.
/// Missing old paths are treated as already cleaned up. The source and the
/// current output are protected so a stale preview cannot delete user input.
pub fn remove_replaced_output(
    previous_path: &Path,
    current_path: &Path,
    source_path: &Path,
) -> io::Result<bool> {
    if previous_path == current_path || previous_path == source_path {
        return Ok(false);
    }
    if !previous_path.exists() {
        return Ok(false);
    }
    if current_path.exists() && paths_refer_to_same_file(previous_path, current_path) {
        return Ok(false);
    }
    if paths_refer_to_same_file(previous_path, source_path) {
        return Ok(false);
    }
    fs::remove_file(previous_path)?;
    Ok(true)
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn is_163_key_marker(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    normalized == "163 key"
        || normalized.starts_with("163 key(")
        || normalized.starts_with("163 key (")
}

fn strip_163_key_from_mp3(path: &Path) -> io::Result<()> {
    let mut tag = match id3::Tag::read_from_path(path) {
        Ok(tag) => tag,
        Err(error) if error.to_string().contains("NoTag") => return Ok(()),
        Err(error) => return Err(io::Error::other(error)),
    };
    let comments_to_remove = tag
        .comments()
        .filter(|comment| {
            is_163_key_marker(&comment.text) || is_163_key_marker(&comment.description)
        })
        .map(|comment| {
            (
                comment.lang.clone(),
                comment.description.clone(),
                comment.text.clone(),
            )
        })
        .collect::<Vec<(String, String, String)>>();
    let extended_texts_to_remove = tag
        .extended_texts()
        .filter(|text| is_163_key_marker(&text.description))
        .map(|text| text.description.clone())
        .collect::<Vec<String>>();

    if comments_to_remove.is_empty() && extended_texts_to_remove.is_empty() {
        return Ok(());
    }

    for (_, description, text) in comments_to_remove {
        tag.remove_comment(Some(&description), Some(&text));
    }

    for description in extended_texts_to_remove {
        tag.remove_extended_text(Some(&description), None);
    }
    tag.write_to_path(path, Version::Id3v24)
        .map_err(io::Error::other)
}

#[allow(dead_code)]
fn derive_song_name(path: &Path) -> String {
    derive_song_name_with_rule(path, FilenameRule::default())
}

fn derive_song_name_with_rule(path: &Path, filename_rule: FilenameRule) -> String {
    let netease_filename_format = if source_prefers_title_artist_filename(path) {
        NeteaseFilenameFormat::TitleArtist
    } else {
        // Keep the legacy smart fallback for ordinary audio files. The explicit
        // NetEase setting is applied by derive_song_name_with_settings.
        NeteaseFilenameFormat::ArtistTitle
    };
    derive_song_name_with_settings(path, filename_rule, netease_filename_format)
}

fn derive_song_name_with_settings(
    path: &Path,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
) -> String {
    derive_song_name_with_policy(
        path,
        filename_rule,
        netease_filename_format,
        FilenameNormalizationPolicy::SoundCloud,
    )
}

pub(crate) fn derive_song_name_with_policy(
    path: &Path,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> String {
    let fallback_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();

    if matches!(filename_rule, FilenameRule::Original) {
        return match filename_policy {
            FilenameNormalizationPolicy::PreserveSource => {
                sanitize_preserve_source_filename_component(&fallback_name)
            }
            FilenameNormalizationPolicy::SoundCloud => sanitize_filename_component(&fallback_name),
        };
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    let candidate = match extension.as_str() {
        "mp3" | "wav" | "aiff" => song_name_from_audio_tag(
            path,
            filename_rule,
            &fallback_name,
            netease_filename_format,
            filename_policy,
        ),
        "flac" => song_name_from_flac(
            path,
            filename_rule,
            &fallback_name,
            netease_filename_format,
            filename_policy,
        ),
        #[cfg(feature = "ncm-decryption")]
        "ncm" => song_name_from_ncm(
            path,
            filename_rule,
            &fallback_name,
            netease_filename_format,
            filename_policy,
        ),
        #[cfg(not(feature = "ncm-decryption"))]
        "ncm" => None,
        _ => None,
    };

    candidate.unwrap_or_else(|| {
        normalize_fallback_song_name(&fallback_name, filename_rule, filename_policy)
    })
}

/// Fast scan-only filename derivation.  It deliberately never opens the
/// source file; tags, NCM payloads and database identities belong to the
/// conversion stage.  Existing filename rules are retained as context, but
/// when a filename does not expose a reliable title/artist split its stem is
/// the only safe identity available during a filesystem snapshot.
pub(crate) fn derive_song_name_from_filename(
    path: &Path,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> String {
    let fallback_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    if matches!(filename_rule, FilenameRule::Original) {
        return match filename_policy {
            FilenameNormalizationPolicy::PreserveSource => {
                sanitize_preserve_source_filename_component(fallback_name)
            }
            FilenameNormalizationPolicy::SoundCloud => sanitize_filename_component(fallback_name),
        };
    }
    // The existing fallback parser is entirely string based. Reusing its
    // normalization keeps SoundCloud-style separators and collaboration
    // markers stable without opening an audio container. PreserveSource also
    // honors the configured NetEase filename orientation when a split is
    // visible in the source filename.
    let identity = if matches!(filename_policy, FilenameNormalizationPolicy::PreserveSource) {
        infer_song_identity_with_netease_filename_format(
            fallback_name,
            None,
            None,
            netease_filename_format,
            filename_policy,
        )
    } else {
        infer_song_identity(fallback_name, None, None)
    };
    build_song_name_with_policy(
        &identity.title,
        &identity.artist,
        filename_rule,
        filename_policy,
    )
    .unwrap_or_else(|| normalize_text_for_policy(fallback_name, filename_policy))
}

/// Derive a destination stem using a conservative NetEase identity when the
/// caller explicitly supplies the Task 1 metadata resolver. The resolver is
/// consulted only while the destination path is first planned; later tag,
/// cover and analysis updates never recalculate or rename this path.
#[allow(dead_code)]
pub(crate) fn derive_song_name_with_policy_and_resolver(
    path: &Path,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    resolver: Option<&NeteaseMetadataResolver>,
) -> String {
    derive_song_name_with_policy_and_resolver_cancellable(
        path,
        filename_rule,
        netease_filename_format,
        filename_policy,
        resolver,
        None,
    )
}

pub(crate) fn derive_song_name_with_policy_and_resolver_cancellable(
    path: &Path,
    filename_rule: FilenameRule,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
    resolver: Option<&NeteaseMetadataResolver>,
    cancel: Option<&AtomicBool>,
) -> String {
    if matches!(filename_rule, FilenameRule::Original)
        || !matches!(filename_policy, FilenameNormalizationPolicy::PreserveSource)
    {
        return derive_song_name_with_policy(
            path,
            filename_rule,
            netease_filename_format,
            filename_policy,
        );
    }

    let identity = resolver.and_then(|resolver| {
        cancel
            .map(|cancel| resolver.track_identity_cancellable(path, cancel))
            .unwrap_or_else(|| resolver.track_identity(path))
    });
    if let Some(identity) = identity
        && let Some(name) = build_song_name_with_policy(
            &identity.title,
            &identity.artists,
            filename_rule,
            filename_policy,
        )
    {
        return name;
    }

    derive_song_name_with_policy(
        path,
        filename_rule,
        netease_filename_format,
        filename_policy,
    )
}

fn song_name_from_flac(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> Option<String> {
    // Filename derivation is based on the file's own tags and filename. A
    // separately discovered NetEase record may enrich output metadata later,
    // but it must not change the identity used for the destination path.
    let tag = source_metadata_as_id3_without_resolver(path);
    let identity = infer_song_identity_with_netease_filename_format(
        fallback_name,
        tag.title(),
        tag.artist().or_else(|| tag.album_artist()),
        netease_filename_format,
        filename_policy,
    );
    build_song_name_with_policy(
        &identity.title,
        &identity.artist,
        filename_rule,
        filename_policy,
    )
}

fn song_name_from_audio_tag(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> Option<String> {
    let tag = source_metadata_as_id3_without_resolver(path);
    let artist = tag.artist().or_else(|| tag.album_artist());
    let identity = infer_song_identity_with_netease_filename_format(
        fallback_name,
        tag.title(),
        artist,
        netease_filename_format,
        filename_policy,
    );
    build_song_name_with_policy(
        &identity.title,
        &identity.artist,
        filename_rule,
        filename_policy,
    )
}

#[cfg(feature = "ncm-decryption")]
fn song_name_from_ncm(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut ncm = Ncmdump::from_reader(file).ok()?;
    let info = ncm.get_info().ok()?;
    let artist = info
        .artist
        .iter()
        .map(|item| item.0.as_str())
        .collect::<Vec<&str>>()
        .join(", ");
    let identity = infer_song_identity_with_netease_filename_format(
        fallback_name,
        Some(&info.name),
        Some(&artist),
        netease_filename_format,
        filename_policy,
    );
    build_song_name_with_policy(
        &identity.title,
        &identity.artist,
        filename_rule,
        filename_policy,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SongIdentity {
    title: String,
    artist: String,
}

fn infer_song_identity(
    fallback_name: &str,
    metadata_title: Option<&str>,
    metadata_artist: Option<&str>,
) -> SongIdentity {
    infer_song_identity_with_filename_preference(
        fallback_name,
        metadata_title,
        metadata_artist,
        false,
    )
}

fn infer_song_identity_with_netease_filename_format(
    fallback_name: &str,
    metadata_title: Option<&str>,
    metadata_artist: Option<&str>,
    netease_filename_format: NeteaseFilenameFormat,
    filename_policy: FilenameNormalizationPolicy,
) -> SongIdentity {
    let title = normalize_filename_part(metadata_title);
    let artist = normalize_filename_part(metadata_artist);
    if title.is_some() && artist.is_some() {
        return SongIdentity {
            title: title.unwrap_or_default(),
            artist: artist.unwrap_or_default(),
        };
    }

    let fallback = normalize_text_for_policy(fallback_name, filename_policy);
    let (fallback_title, fallback_artist) = match netease_filename_format {
        NeteaseFilenameFormat::TitleOnly => (fallback, String::new()),
        NeteaseFilenameFormat::ArtistTitle => {
            split_filename_identity_with_policy(&fallback, filename_policy)
                .map(|(artist, title)| (title, artist))
                .unwrap_or_else(|| (fallback, String::new()))
        }
        NeteaseFilenameFormat::TitleArtist => {
            split_filename_identity_with_policy(&fallback, filename_policy)
                .unwrap_or_else(|| (fallback, String::new()))
        }
    };

    SongIdentity {
        title: title.unwrap_or_else(|| normalize_identity_part(&fallback_title, filename_policy)),
        artist: artist
            .unwrap_or_else(|| normalize_identity_part(&fallback_artist, filename_policy)),
    }
}

fn infer_song_identity_with_filename_preference(
    fallback_name: &str,
    metadata_title: Option<&str>,
    metadata_artist: Option<&str>,
    prefer_title_artist_filename: bool,
) -> SongIdentity {
    let (fallback_title, fallback_artist) =
        parse_filename_identity(fallback_name, prefer_title_artist_filename);
    let title = normalize_filename_part(metadata_title);
    let artist = normalize_filename_part(metadata_artist);

    if let (Some(title), Some(artist)) = (&title, &artist) {
        if let Some((left, right)) = split_filename_identity(fallback_name) {
            let filename_identity = if prefer_title_artist_filename {
                SongIdentity {
                    title: left,
                    artist: right,
                }
            } else {
                SongIdentity {
                    title: right,
                    artist: left,
                }
            };
            if *title == filename_identity.artist && *artist == filename_identity.title {
                return filename_identity;
            }
        }
        return SongIdentity {
            title: title.clone(),
            artist: artist.clone(),
        };
    }

    let title = title.unwrap_or(fallback_title);
    let artist = artist.unwrap_or(fallback_artist);

    SongIdentity { title, artist }
}

fn source_prefers_title_artist_filename(path: &Path) -> bool {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ncm"))
    {
        return true;
    }

    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let lower = name.to_ascii_lowercase();
                lower.contains("netease") || name.contains("网易云")
            })
    })
}

fn parse_filename_identity(
    fallback_name: &str,
    prefer_title_artist_filename: bool,
) -> (String, String) {
    let display = normalize_display_text(fallback_name);
    if let Some((left, right)) =
        split_filename_identity_with_policy(&display, FilenameNormalizationPolicy::SoundCloud)
    {
        return if prefer_title_artist_filename {
            (left, right)
        } else {
            (right, left)
        };
    }
    (display, String::new())
}

fn split_filename_identity(fallback_name: &str) -> Option<(String, String)> {
    split_filename_identity_with_policy(fallback_name, FilenameNormalizationPolicy::SoundCloud)
}

fn split_filename_identity_with_policy(
    fallback_name: &str,
    filename_policy: FilenameNormalizationPolicy,
) -> Option<(String, String)> {
    let display = normalize_text_for_policy(fallback_name, filename_policy);
    display
        .split_once(" - ")
        .map(|(left, right)| (left.to_string(), right.to_string()))
}

fn normalize_filename_part(value: Option<&str>) -> Option<String> {
    let value = value?;
    let normalized = value.trim();
    (!normalized.is_empty()).then_some(normalized.to_string())
}

#[cfg(test)]
fn build_song_name(title: &str, artist: &str) -> Option<String> {
    build_song_name_with_rule(title, artist, FilenameRule::default())
}

#[cfg(test)]
fn build_song_name_with_rule(
    title: &str,
    artist: &str,
    filename_rule: FilenameRule,
) -> Option<String> {
    build_song_name_with_policy(
        title,
        artist,
        filename_rule,
        FilenameNormalizationPolicy::SoundCloud,
    )
}

fn build_song_name_with_policy(
    title: &str,
    artist: &str,
    filename_rule: FilenameRule,
    filename_policy: FilenameNormalizationPolicy,
) -> Option<String> {
    let title = normalize_identity_part(title, filename_policy);
    let artist = normalize_identity_part(artist, filename_policy);

    match (title.is_empty(), artist.is_empty()) {
        (true, true) => None,
        (false, true) => Some(title),
        (true, false) => Some(artist),
        (false, false) => match filename_rule {
            FilenameRule::TitleArtist | FilenameRule::Original => {
                Some(format!("{} - {}", title, artist))
            }
            FilenameRule::ArtistTitle => Some(format!("{} - {}", artist, title)),
        },
    }
}

fn normalize_fallback_song_name(
    fallback_name: &str,
    filename_rule: FilenameRule,
    filename_policy: FilenameNormalizationPolicy,
) -> String {
    let identity = if matches!(filename_policy, FilenameNormalizationPolicy::PreserveSource) {
        infer_song_identity_with_netease_filename_format(
            fallback_name,
            None,
            None,
            NeteaseFilenameFormat::TitleArtist,
            filename_policy,
        )
    } else {
        infer_song_identity(fallback_name, None, None)
    };
    build_song_name_with_policy(
        &identity.title,
        &identity.artist,
        filename_rule,
        filename_policy,
    )
    .unwrap_or_else(|| normalize_text_for_policy(fallback_name, filename_policy))
}

fn normalize_text_for_policy(value: &str, filename_policy: FilenameNormalizationPolicy) -> String {
    match filename_policy {
        FilenameNormalizationPolicy::PreserveSource => value.to_string(),
        FilenameNormalizationPolicy::SoundCloud => normalize_display_text(value),
    }
}

fn normalize_identity_part(value: &str, filename_policy: FilenameNormalizationPolicy) -> String {
    match filename_policy {
        FilenameNormalizationPolicy::PreserveSource => {
            sanitize_preserve_source_filename_component(value)
        }
        FilenameNormalizationPolicy::SoundCloud => {
            sanitize_filename_component(&normalize_display_text(value))
        }
    }
}

/// Keeps the configured filename rule and all valid source characters while
/// making only the two observed macOS path hazards representable in one
/// filename component. This function is filename-only; source metadata keeps
/// its original title and artist values.
fn sanitize_preserve_source_filename_component(value: &str) -> String {
    // Preserve the source spelling verbatim.  A leading dot would make the
    // resulting output a macOS/Unix hidden file, so only that one filesystem
    // hazard is made visible.  Path separators and NUL are still mapped to
    // their existing safe representations because they cannot be part of a
    // filename component on the target filesystem.
    let mut output = String::with_capacity(value.len() + 1);
    if value.starts_with('.') {
        output.push('_');
    }
    for character in value.chars() {
        match character {
            '\0' => output.push_str(", "),
            '/' => output.push('／'),
            other => output.push(other),
        }
    }
    output
}

fn normalize_display_text(value: &str) -> String {
    let mut text = value.trim().to_string();
    if text.is_empty() {
        return text;
    }

    let aggressive_soundcloud_cleanup = looks_like_soundcloud_text(&text);
    text = normalize_unicode_punctuation(&text);
    text = text.replace('_', " ");
    text = text.replace('/', ", ");
    text = strip_promotional_suffixes(&text);
    if aggressive_soundcloud_cleanup {
        text = strip_common_trailing_tokens(&text);
    }
    text = normalize_collaboration_markers(&text);
    text = normalize_spacing_around_punctuation(&text);

    text.split_whitespace().collect::<Vec<&str>>().join(" ")
}

fn looks_like_soundcloud_text(value: &str) -> bool {
    let lowered = value.to_lowercase();
    lowered.contains('_')
        || lowered.contains("free_dl")
        || lowered.contains("freedl")
        || lowered.contains("soundcloud")
        || lowered.contains("unreleased")
        || lowered.contains("id_id")
        || lowered.ends_with(" id")
        || lowered.ends_with(" free")
        || lowered.ends_with(" dl")
        || lowered.ends_with(" remix")
}

fn normalize_unicode_punctuation(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '，' => ',',
            '。' => '.',
            '：' => ':',
            '；' => ';',
            '！' => '!',
            '？' => '?',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            '《' => '<',
            '》' => '>',
            '“' | '”' => '"',
            '‘' | '’' => '\'',
            '／' | '∕' => '/',
            '—' | '–' | '－' => '-',
            '·' => '·',
            other => other,
        })
        .collect()
}

fn strip_promotional_suffixes(value: &str) -> String {
    let mut text = value.trim().to_string();

    while let Some((open, close)) = trailing_bracket_pair(&text) {
        let Some((start, inner)) = extract_trailing_bracket_content(&text, open, close) else {
            break;
        };

        if is_promotional_suffix(inner) {
            text.truncate(start);
            text = text
                .trim_end_matches(&[' ', '-', '_', '|', '~', '/', '·'][..])
                .to_string();
            continue;
        }

        break;
    }

    text
}

fn strip_common_trailing_tokens(value: &str) -> String {
    let mut text = value.trim().to_string();

    while let Some(last_token) = text.split_whitespace().last() {
        let normalized = last_token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '.' | ','
                        | ';'
                        | ':'
                        | '!'
                        | '?'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '\''
                        | '"'
                )
            })
            .to_lowercase();

        let is_year = normalized.len() == 4
            && (normalized.starts_with("19") || normalized.starts_with("20"))
            && normalized.chars().all(|ch| ch.is_ascii_digit());

        let should_strip = matches!(
            normalized.as_str(),
            "id" | "unreleased"
                | "free"
                | "dl"
                | "freedl"
                | "free_dl"
                | "soundcloud"
                | "preview"
                | "snippet"
                | "teaser"
                | "promo"
                | "promotion"
                | "official"
                | "audio"
                | "video"
                | "live"
        ) || is_year;

        if !should_strip {
            break;
        }

        let new_len = text
            .rsplit_once(last_token)
            .map(|(prefix, _)| prefix.trim_end().len())
            .unwrap_or(0);
        text.truncate(new_len);
        text = text
            .trim_end_matches(&[' ', '-', '_', '|', '~', '/', '·', '.', ',', ';', ':'][..])
            .to_string();
    }

    text
}

fn trailing_bracket_pair(text: &str) -> Option<(char, char)> {
    let trimmed = text.trim_end();
    let close = trimmed.chars().last()?;
    let open = match close {
        ')' => '(',
        ']' => '[',
        '}' => '{',
        '>' => '<',
        _ => return None,
    };

    Some((open, close))
}

fn extract_trailing_bracket_content(text: &str, open: char, close: char) -> Option<(usize, &str)> {
    let trimmed = text.trim_end();
    let close_index = trimmed.char_indices().rev().find(|(_, ch)| *ch == close)?.0;
    let prefix = &trimmed[..close_index];
    let open_index = prefix.char_indices().rev().find(|(_, ch)| *ch == open)?.0;
    let inner = &trimmed[open_index + open.len_utf8()..close_index];
    Some((open_index, inner.trim()))
}

fn is_promotional_suffix(value: &str) -> bool {
    let lowered = value.to_lowercase();
    let compact = lowered.split_whitespace().collect::<String>();

    let keywords = [
        "officialaudio",
        "officialvideo",
        "officialmusicvideo",
        "musicvideo",
        "lyricvideo",
        "lyricsvideo",
        "lyrics",
        "lyric",
        "audio",
        "video",
        "visualizer",
        "visualiser",
        "mv",
        "m/v",
        "performancevideo",
        "live",
        "liveaudio",
        "clean",
        "explicit",
        "promo",
        "promotion",
        "trailer",
        "snippet",
        "teaser",
        "preview",
        "remaster",
        "remastered",
        "edit",
        "radioedit",
        "clubedit",
        "extendedmix",
        "instrumental",
        "karaoke",
        "specialedition",
        "singleversion",
        "soundcloud",
        "网易云音乐",
        "网易云",
        "free_dl",
        "freedl",
    ];

    keywords.iter().any(|keyword| compact.contains(keyword))
}

fn normalize_collaboration_markers(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            let trimmed = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '.' | ',' | ';' | ':' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            });
            let lowered = trimmed.to_lowercase();

            match trimmed {
                "×" => String::from("feat."),
                _ if matches!(lowered.as_str(), "feat" | "ft" | "featuring" | "with" | "x") => {
                    String::from("feat.")
                }
                _ => token.to_string(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn normalize_spacing_around_punctuation(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut prev_was_space = false;
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        let normalized = match ch {
            ',' | ':' | ';' | '!' | '?' => ch,
            '.' => ch,
            '(' | '[' | '{' => ch,
            ')' | ']' | '}' => ch,
            '/' => '/',
            _ => ch,
        };

        if normalized.is_whitespace() {
            if !prev_was_space {
                output.push(' ');
                prev_was_space = true;
            }
            continue;
        }

        if matches!(normalized, ',' | ':' | ';' | '!' | '?' | '.') {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push(normalized);
            if chars.peek().is_some_and(|next| {
                !next.is_whitespace()
                    && !matches!(next, ',' | ':' | ';' | '!' | '?' | '.' | ')' | ']' | '}')
            }) {
                output.push(' ');
                prev_was_space = true;
            } else {
                prev_was_space = false;
            }
            continue;
        }

        if matches!(normalized, ')' | ']' | '}') {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push(normalized);
            prev_was_space = false;
            continue;
        }

        if matches!(normalized, '(' | '[' | '{') {
            if !output.is_empty() && !output.ends_with(' ') {
                output.push(' ');
            }
            output.push(normalized);
            prev_was_space = false;
            continue;
        }

        output.push(normalized);
        prev_was_space = false;
    }

    output
}

fn sanitize_filename_component(value: &str) -> String {
    crate::filename_policy::sanitize_filename_component(value)
}

#[cfg(feature = "ncm-decryption")]
fn write_container_tags(
    output_path: &Path,
    target_profile: TargetProfile,
    ncm_metadata: &ncmdump::NcmInfo,
    image_data: &[u8],
) -> io::Result<()> {
    let tag = build_id3_tag(ncm_metadata, image_data);

    #[allow(deprecated)]
    match target_profile {
        TargetProfile::LosslessWav => write_id3_tag_for_output(&tag, output_path),
        TargetProfile::LosslessAiff => tag
            .write_to_aiff_path(output_path, Version::Id3v24)
            .map_err(io::Error::other),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveFfmpegRegistry, ConversionMetadataContext, EmbeddedAnalysis,
        SUPPORTED_SOURCE_EXTENSIONS, SongIdentity, apply_track_analysis_metadata, build_song_name,
        build_song_name_with_policy, build_song_name_with_rule, commit_temporary_output,
        compare_music_dicts, derive_song_name, derive_song_name_with_policy,
        derive_song_name_with_policy_and_resolver, derive_song_name_with_rule,
        derive_song_name_with_settings, ensure_generated_output, ensure_output_metadata,
        ensure_output_metadata_with_settings,
        ensure_output_metadata_with_settings_with_context_and_policy,
        enumerate_music_files_observed, fill_missing_metadata, find_ffmpeg_next_to_exe,
        infer_song_identity, inspect_metadata_diagnostic_with_resolver, is_hidden_path,
        is_ignored_music_file, merge_recovered_metadata, ncm_decryption_available,
        remove_conflicting_outputs, run_output_transaction, sanitize_filename_component,
        sanitize_preserve_source_filename_component, strip_163_key_from_mp3,
        sync_music_library_transactional_with_observer_and_budget_and_context,
        target_output_path_with_policy, update_analysis_metadata_transactionally,
        update_existing_metadata_transactionally,
        update_existing_metadata_transactionally_with_context_and_policy,
        validate_track_analysis_metadata, write_riff_info_metadata,
    };
    use crate::concurrency::GlobalConcurrencyBudget;
    use crate::config::{
        FilenameNormalizationPolicy, FilenameRule, LosslessFormat, Mode, NeteaseFilenameFormat,
    };
    use crate::netease::NeteaseMetadataResolver;
    use id3::{Tag, TagLike, Version};
    use rusqlite::{Connection, params};
    use std::collections::{BTreeMap, HashMap};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn exposes_the_expected_ncm_capability_for_each_build_variant() {
        assert_eq!(ncm_decryption_available(), cfg!(feature = "ncm-decryption"));
        assert_eq!(
            SUPPORTED_SOURCE_EXTENSIONS.contains(&"ncm"),
            cfg!(feature = "ncm-decryption")
        );
    }

    fn write_executable_file(path: &Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }
    }

    fn write_test_wav(path: &Path) {
        let mut wav = Vec::with_capacity(48);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&40u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&4u32.to_le_bytes());
        wav.extend_from_slice(&[0, 0, 0, 0]);
        fs::write(path, wav).unwrap();
    }

    #[test]
    fn sanitizes_invalid_filename_characters() {
        assert_eq!(sanitize_filename_component("A/B:C*D?"), "A-B-C-D-");
        assert_eq!(sanitize_filename_component("CON"), "_CON");
        assert_eq!(sanitize_filename_component("Track..."), "Track");
    }

    #[test]
    fn preserve_source_only_replaces_nul_and_ascii_slash() {
        assert_eq!(
            sanitize_preserve_source_filename_component(
                "バギー・ブギー/Buggy Boogie - ミッキー吉野\0小林亜星"
            ),
            "バギー・ブギー／Buggy Boogie - ミッキー吉野, 小林亜星"
        );
        assert_eq!(
            sanitize_preserve_source_filename_component(
                r#"Mass Destruction ("P3" + "P3F" ver.) - Artist"#
            ),
            r#"Mass Destruction ("P3" + "P3F" ver.) - Artist"#
        );
        assert_eq!(
            sanitize_preserve_source_filename_component(". Song"),
            "_. Song"
        );
    }

    #[test]
    fn hidden_files_and_directories_are_excluded_from_enumeration() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("visible.mp3"), b"audio").unwrap();
        fs::create_dir(directory.path().join(".hidden")).unwrap();
        fs::write(directory.path().join(".hidden/song.mp3"), b"audio").unwrap();
        fs::write(directory.path().join(".hidden.mp3"), b"audio").unwrap();
        assert!(is_hidden_path(&directory.path().join(".hidden/song.mp3")));
        assert!(is_ignored_music_file(&directory.path().join(".hidden.mp3")));
        let cancelled = AtomicBool::new(false);
        let result = enumerate_music_files_observed(
            directory.path().to_string_lossy().as_ref(),
            &["mp3"],
            &cancelled,
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(result.paths.len(), 1);
        assert!(result.paths[0].ends_with("visible.mp3"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_hidden_flag_is_excluded_from_direct_file_checks() {
        let directory = tempdir().unwrap();
        let hidden = directory.path().join("flagged.mp3");
        fs::write(&hidden, b"audio").unwrap();
        let status = Command::new("chflags")
            .args(["hidden", hidden.to_string_lossy().as_ref()])
            .status()
            .unwrap();
        if !status.success() {
            return;
        }
        assert!(is_hidden_path(&hidden));
        assert!(is_ignored_music_file(&hidden));
    }

    #[test]
    fn title_artist_rule_sanitizes_filename_without_changing_rule_order() {
        assert_eq!(
            build_song_name_with_policy(
                "バギー・ブギー/Buggy Boogie",
                "ミッキー吉野\0小林亜星",
                FilenameRule::TitleArtist,
                FilenameNormalizationPolicy::PreserveSource,
            )
            .as_deref(),
            Some("バギー・ブギー／Buggy Boogie - ミッキー吉野, 小林亜星")
        );
        assert_eq!(
            build_song_name_with_policy(
                "バギー・ブギー/Buggy Boogie",
                "ミッキー吉野\0小林亜星",
                FilenameRule::ArtistTitle,
                FilenameNormalizationPolicy::PreserveSource,
            )
            .as_deref(),
            Some("ミッキー吉野, 小林亜星 - バギー・ブギー／Buggy Boogie")
        );
    }

    #[test]
    fn filename_rule_defaults_to_title_artist_and_can_be_reversed() {
        assert_eq!(
            build_song_name_with_rule("Title", "Artist", FilenameRule::TitleArtist).as_deref(),
            Some("Title - Artist")
        );
        assert_eq!(
            build_song_name_with_rule("Title", "Artist", FilenameRule::ArtistTitle).as_deref(),
            Some("Artist - Title")
        );
    }

    #[test]
    fn combines_title_and_artist_with_separator() {
        assert_eq!(
            build_song_name("paper hearts", "CLV Edit").as_deref(),
            Some("paper hearts - CLV Edit")
        );
    }

    #[test]
    fn strips_promotional_parenthetical_suffixes() {
        assert_eq!(
            build_song_name("Paper Hearts (Official Video)", "CLV Edit").as_deref(),
            Some("Paper Hearts - CLV Edit")
        );
    }

    #[test]
    fn normalizes_collaboration_markers_and_spacing() {
        assert_eq!(
            build_song_name("Paper Hearts ft. CLV", "A／B").as_deref(),
            Some("Paper Hearts feat. CLV - A, B")
        );
    }

    #[test]
    fn converts_with_and_unicode_punctuation_to_standard_form() {
        assert_eq!(
            build_song_name("Paper Hearts with CLV，Live", "Artist").as_deref(),
            Some("Paper Hearts feat. CLV, Live - Artist")
        );
    }

    #[test]
    fn normalizes_x_and_times_sign_collaboration_markers() {
        assert_eq!(
            build_song_name("Paper Hearts x CLV × Artist", "DJ").as_deref(),
            Some("Paper Hearts feat. CLV feat. Artist - DJ")
        );
    }

    #[test]
    fn preserves_regular_years_in_non_soundcloud_titles() {
        assert_eq!(
            build_song_name("Song 2023", "Artist").as_deref(),
            Some("Song 2023 - Artist")
        );
    }

    #[test]
    fn normalizes_soundcloud_style_filename_fallbacks() {
        assert_eq!(
            derive_song_name(std::path::Path::new(
                "/tmp/Knock2_ISOxo_Travis_Scott_Yeat_-_Smack_Talk_x_Fein_x_Breathe_Mantra_Edit_FREE_DL.mp3"
            )),
            "Smack Talk feat. Fein feat. Breathe Mantra Edit - Knock2 ISOxo Travis Scott Yeat"
        );
    }

    #[test]
    fn strips_soundcloud_trailing_noise_from_filename_fallbacks() {
        assert_eq!(
            derive_song_name(std::path::Path::new(
                "/tmp/Skrillex_ft_ISOxo_Zeina_Logan_olm_-_Take_It_All_Whisper_ID_ID_2023_unreleased.mp3"
            )),
            "Take It All Whisper - Skrillex feat. ISOxo Zeina Logan olm"
        );
    }

    #[test]
    fn applies_title_artist_rule_to_plain_artist_first_filename_fallback() {
        let path = std::path::Path::new("/tmp/Mr Wankerman - Mystic State, Third Degree.mp3");

        assert_eq!(
            derive_song_name_with_rule(path, FilenameRule::TitleArtist),
            "Mystic State, Third Degree - Mr Wankerman"
        );
        assert_eq!(
            derive_song_name_with_rule(path, FilenameRule::ArtistTitle),
            "Mr Wankerman - Mystic State, Third Degree"
        );
    }

    #[test]
    fn completes_partial_metadata_from_the_filename_identity() {
        let fallback = "Mr Wankerman - Mystic State, Third Degree";

        assert_eq!(
            infer_song_identity(fallback, Some("Mystic State, Third Degree"), None),
            SongIdentity {
                title: "Mystic State, Third Degree".to_string(),
                artist: "Mr Wankerman".to_string(),
            }
        );
        assert_eq!(
            infer_song_identity(fallback, None, Some("Mr Wankerman")),
            SongIdentity {
                title: "Mystic State, Third Degree".to_string(),
                artist: "Mr Wankerman".to_string(),
            }
        );
    }

    #[test]
    fn combines_audio_metadata_with_filename_identity_before_applying_rule() {
        let directory = tempdir().unwrap();
        let path = directory
            .path()
            .join("Mr Wankerman - Mystic State, Third Degree.mp3");
        fs::write(&path, b"audio-placeholder").unwrap();
        let mut tag = Tag::new();
        tag.set_title("Mystic State, Third Degree");
        tag.write_to_path(&path, Version::Id3v24).unwrap();

        assert_eq!(
            derive_song_name_with_rule(&path, FilenameRule::TitleArtist),
            "Mystic State, Third Degree - Mr Wankerman"
        );
    }

    #[test]
    fn netease_policy_preserves_quoted_title_and_soundcloud_policy_removes_quotes() {
        let directory = tempdir().unwrap();
        let path = directory
            .path()
            .join("Mass Destruction (P3 + P3F ver.) - 川村ゆみ, Lotus Juice.mp3");
        fs::write(&path, b"audio-placeholder").unwrap();
        let mut tag = Tag::new();
        tag.set_title(r#"Mass Destruction ("P3" + "P3F" ver.)"#);
        tag.set_artist("川村ゆみ, Lotus Juice");
        tag.write_to_path(&path, Version::Id3v24).unwrap();

        assert_eq!(
            derive_song_name_with_policy(
                &path,
                FilenameRule::TitleArtist,
                NeteaseFilenameFormat::TitleArtist,
                FilenameNormalizationPolicy::PreserveSource,
            ),
            r#"Mass Destruction ("P3" + "P3F" ver.) - 川村ゆみ, Lotus Juice"#
        );
        assert_eq!(
            derive_song_name_with_policy(
                &path,
                FilenameRule::TitleArtist,
                NeteaseFilenameFormat::TitleArtist,
                FilenameNormalizationPolicy::SoundCloud,
            ),
            "Mass Destruction (P3 + P3F ver.) - 川村ゆみ, Lotus Juice"
        );
        assert_eq!(
            target_output_path_with_policy(
                "/tmp/output",
                r#"Mass Destruction ("P3" + "P3F" ver.) - 川村ゆみ, Lotus Juice"#,
                "mp3",
                FilenameNormalizationPolicy::PreserveSource,
            )
            .file_name()
            .and_then(|name| name.to_str()),
            Some(r#"Mass Destruction ("P3" + "P3F" ver.) - 川村ゆみ, Lotus Juice.mp3"#)
        );
        assert_eq!(
            target_output_path_with_policy(
                "/tmp/output",
                "バギー・ブギー/Buggy Boogie - ミッキー吉野\0小林亜星",
                "mp3",
                FilenameNormalizationPolicy::PreserveSource,
            )
            .file_name()
            .and_then(|name| name.to_str()),
            Some("バギー・ブギー／Buggy Boogie - ミッキー吉野, 小林亜星.mp3")
        );
    }

    #[test]
    fn compare_music_dicts_skips_existing_lossless_output_without_using_source_size() {
        let mut source = HashMap::new();
        source.insert(
            "Song".to_string(),
            ("100".to_string(), PathBuf::from("/music/source/Song.flac")),
        );

        let mut destination = HashMap::new();
        destination.insert(
            "Song".to_string(),
            ("4096".to_string(), PathBuf::from("/music/dest/Song.wav")),
        );

        let diff = compare_music_dicts(
            &source,
            &destination,
            &Mode::Lossless,
            Some(LosslessFormat::Wav),
        );

        assert!(diff.is_empty());
    }

    #[test]
    fn compare_music_dicts_reprocesses_zero_byte_existing_output() {
        let mut source = HashMap::new();
        source.insert(
            "Song".to_string(),
            ("100".to_string(), PathBuf::from("/music/source/Song.mp3")),
        );

        let mut destination = HashMap::new();
        destination.insert(
            "Song".to_string(),
            ("0".to_string(), PathBuf::from("/music/dest/Song.mp3")),
        );

        let diff = compare_music_dicts(&source, &destination, &Mode::Compat, None);

        assert_eq!(diff.len(), 1);
    }

    #[test]
    fn finds_platform_specific_ffmpeg_sidecar_next_to_executable() {
        let dir = tempdir().unwrap();
        let exe_path = dir.path().join("w4dj.exe");
        let sidecar_path = dir.path().join("ffmpeg-x86_64-pc-windows-msvc.exe");

        fs::write(&exe_path, []).unwrap();
        write_executable_file(&sidecar_path, b"ffmpeg sidecar");

        let found = find_ffmpeg_next_to_exe(&exe_path).unwrap();
        assert_eq!(found, sidecar_path);
    }

    #[test]
    fn finds_ffmpeg_sidecar_inside_binaries_directory() {
        let dir = tempdir().unwrap();
        let exe_dir = dir.path();
        let exe_path = exe_dir.join("w4dj.exe");
        let binaries_dir = exe_dir.join("binaries");
        let sidecar_path = binaries_dir.join("ffmpeg-aarch64-apple-darwin");

        fs::create_dir_all(&binaries_dir).unwrap();
        fs::write(&exe_path, []).unwrap();
        write_executable_file(&sidecar_path, b"ffmpeg sidecar");

        let found = find_ffmpeg_next_to_exe(&exe_path).unwrap();
        assert_eq!(found, sidecar_path);
    }

    #[test]
    fn prefers_arch_specific_ffmpeg_sidecar_when_multiple_exist() {
        let dir = tempdir().unwrap();
        let exe_path = dir.path().join("w4dj.exe");
        let binaries_dir = dir.path().join("binaries");
        let preferred_windows = binaries_dir.join("ffmpeg-x86_64-pc-windows-msvc.exe");
        let preferred_macos = binaries_dir.join("ffmpeg-aarch64-apple-darwin");

        fs::create_dir_all(&binaries_dir).unwrap();
        fs::write(&exe_path, []).unwrap();
        write_executable_file(&preferred_windows, b"ffmpeg windows sidecar");
        write_executable_file(&preferred_macos, b"ffmpeg mac sidecar");

        let found = find_ffmpeg_next_to_exe(&exe_path).unwrap();

        #[cfg(target_os = "windows")]
        assert_eq!(found, preferred_windows);

        #[cfg(target_os = "macos")]
        assert_eq!(found, preferred_macos);
    }

    #[test]
    fn does_not_treat_desktop_executable_as_ffmpeg_sidecar() {
        let dir = tempdir().unwrap();
        let exe_path = dir.path().join("w4dj-desktop");

        write_executable_file(&exe_path, b"desktop executable");

        assert!(find_ffmpeg_next_to_exe(&exe_path).is_none());
    }

    #[test]
    fn rejects_successful_conversion_without_an_output_file() {
        let dir = tempdir().unwrap();
        let missing_output = dir.path().join("missing.aiff");

        let error = ensure_generated_output(&missing_output, "Missing Song").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(error.to_string().contains("missing.aiff"));
    }

    #[test]
    fn commits_temporary_output_without_leaving_the_previous_file() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("song.mp3");
        let temporary = dir.path().join(".w4dj-song.mp3");
        fs::write(&output, b"old output").unwrap();
        fs::write(&temporary, b"new output").unwrap();

        commit_temporary_output(&temporary, &output).unwrap();

        assert_eq!(fs::read(&output).unwrap(), b"new output");
        assert!(!temporary.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_metadata_finalization_does_not_publish_or_replace_output() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("song.mp3");
        fs::write(&output, b"previous good output").unwrap();

        let error = run_output_transaction(&output, "song", |temporary| {
            fs::write(temporary, b"new converted audio")?;
            Err(std::io::Error::other("metadata write failed"))
        })
        .expect_err("metadata failure must abort the output transaction");

        assert_eq!(error.to_string(), "metadata write failed");
        assert_eq!(fs::read(&output).unwrap(), b"previous good output");
        let temporary_files = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".w4dj-"))
            .count();
        assert_eq!(temporary_files, 0);
    }

    #[test]
    fn does_not_rewrite_an_mp3_without_163_metadata() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cover-song.mp3");
        fs::write(&path, b"audio").unwrap();
        let mut tag = Tag::new();
        tag.set_title("Cover Song");
        tag.add_frame(id3::frame::Picture {
            mime_type: "image/jpeg".into(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: vec![0xff, 0xd8, 0xff, 0xe1, 0x01, 0x02],
        });
        tag.write_to_path(&path, Version::Id3v23).unwrap();
        let original = fs::read(&path).unwrap();

        strip_163_key_from_mp3(&path).unwrap();

        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(Tag::read_from_path(&path).unwrap().pictures().count(), 1);
    }

    #[test]
    fn removes_163_metadata_while_preserving_the_cover_frame() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("netease-song.mp3");
        fs::write(&path, b"audio").unwrap();
        let mut tag = Tag::new();
        tag.add_frame(id3::frame::Comment {
            lang: "eng".into(),
            description: "163 key".into(),
            text: "163 key(secret)".into(),
        });
        tag.add_frame(id3::frame::ExtendedText {
            description: "163 key".into(),
            value: "secret".into(),
        });
        tag.add_frame(id3::frame::Picture {
            mime_type: "image/jpeg".into(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: vec![0xff, 0xd8, 0xff, 0xe1, 0x01, 0x02],
        });
        tag.write_to_path(&path, Version::Id3v24).unwrap();

        strip_163_key_from_mp3(&path).unwrap();

        let cleaned = Tag::read_from_path(&path).unwrap();
        assert_eq!(cleaned.comments().count(), 0);
        assert_eq!(cleaned.extended_texts().count(), 0);
        assert_eq!(cleaned.pictures().count(), 1);
    }

    #[test]
    fn removes_dont_modify_163_key_from_existing_output_transaction() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.mp3");
        let output_dir = dir.path().join("output");
        let output = output_dir.join("弹舌.mp3");
        fs::create_dir_all(&output_dir).unwrap();

        for path in [&source, &output] {
            fs::write(path, b"audio").unwrap();
            let mut tag = Tag::new();
            tag.set_title("弹舌");
            tag.set_artist("网易云歌手");
            tag.add_frame(id3::frame::Comment {
                lang: "und".into(),
                description: "163 key(Don't modify)".into(),
                text: "encrypted-value".into(),
            });
            tag.add_frame(id3::frame::ExtendedText {
                description: "163 key(Don't modify)".into(),
                value: "encrypted-value".into(),
            });
            tag.write_to_path(path, Version::Id3v24).unwrap();
        }

        update_existing_metadata_transactionally(
            &source,
            &output,
            NeteaseFilenameFormat::TitleArtist,
            |_| Ok(()),
        )
        .unwrap();

        let cleaned = Tag::read_from_path(&output).unwrap();
        assert_eq!(cleaned.comments().count(), 0);
        assert_eq!(cleaned.extended_texts().count(), 0);
        assert_eq!(cleaned.title(), Some("弹舌"));
        assert_eq!(cleaned.artist(), Some("网易云歌手"));
    }

    #[test]
    fn metadata_only_update_uses_batch_resolver_cover_without_reencoding() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("Song - Artist.mp3");
        let output = dir.path().join("output.mp3");
        for path in [&source, &output] {
            fs::write(path, b"audio").unwrap();
            let mut tag = Tag::new();
            tag.set_title("Song");
            tag.set_artist("Artist");
            tag.write_to_path(path, Version::Id3v24).unwrap();
        }
        let database = dir.path().join("sqlite_storage.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, cover BLOB);",
            )
            .unwrap();
        let cover = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x01, 0x02];
        connection
            .execute(
                "INSERT INTO track(file, title, artist, album, cover) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    source.to_string_lossy(),
                    "Song",
                    "Artist",
                    "Album",
                    cover.clone()
                ],
            )
            .unwrap();
        drop(connection);
        let resolver =
            Arc::new(crate::netease::NeteaseMetadataResolver::load(Some(&database)).unwrap());
        let context = ConversionMetadataContext { netease: resolver };
        let original_audio = fs::read(&output).unwrap();
        update_existing_metadata_transactionally_with_context_and_policy(
            &source,
            &output,
            NeteaseFilenameFormat::TitleArtist,
            |_| Ok(()),
            &context,
            FilenameNormalizationPolicy::PreserveSource,
        )
        .unwrap();
        let updated = Tag::read_from_path(&output).unwrap();
        assert_eq!(updated.album(), Some("Album"));
        assert_eq!(
            updated
                .pictures()
                .next()
                .map(|picture| picture.data.clone()),
            Some(cover)
        );
        assert!(fs::read(&output).unwrap().len() > original_audio.len());
    }

    #[test]
    fn fills_missing_metadata_from_the_original_filename() {
        let mut output = Tag::new();
        let source = Tag::new();
        let identity = infer_song_identity("Mr Wankerman - Mystic State, Third Degree", None, None);

        assert!(fill_missing_metadata(&mut output, &source, &identity));

        assert_eq!(output.title(), Some("Mystic State, Third Degree"));
        assert_eq!(output.artist(), Some("Mr Wankerman"));
    }

    #[test]
    fn writes_fallback_metadata_to_an_untagged_mp3_output() {
        let dir = tempdir().unwrap();
        let source_path = dir
            .path()
            .join("Mr Wankerman - Mystic State, Third Degree.mp3");
        let output_path = dir
            .path()
            .join("Mystic State, Third Degree - Mr Wankerman.mp3");
        fs::write(&source_path, b"audio").unwrap();
        fs::copy(&source_path, &output_path).unwrap();

        ensure_output_metadata(&source_path, &output_path).unwrap();

        let tag = Tag::read_from_path(&output_path).unwrap();
        assert_eq!(tag.title(), Some("Mystic State, Third Degree"));
        assert_eq!(tag.artist(), Some("Mr Wankerman"));
    }

    #[test]
    fn applies_selected_netease_filename_format_to_untagged_mp3_and_flac() {
        let dir = tempdir().unwrap();
        let mp3 = dir.path().join("歌手 - 歌曲.mp3");
        let flac = dir.path().join("歌曲 - 歌手.flac");
        fs::write(&mp3, b"audio").unwrap();
        fs::write(&flac, b"audio").unwrap();

        assert_eq!(
            derive_song_name_with_settings(
                &mp3,
                FilenameRule::TitleArtist,
                NeteaseFilenameFormat::ArtistTitle,
            ),
            "歌曲 - 歌手"
        );
        assert_eq!(
            derive_song_name_with_settings(
                &flac,
                FilenameRule::TitleArtist,
                NeteaseFilenameFormat::TitleArtist,
            ),
            "歌曲 - 歌手"
        );
    }

    #[test]
    fn writes_selected_netease_identity_to_untagged_output_metadata() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("歌手 - 歌曲.mp3");
        let output = dir.path().join("歌曲 - 歌手.mp3");
        fs::write(&source, b"audio").unwrap();
        fs::write(&output, b"audio").unwrap();

        ensure_output_metadata_with_settings(&source, &output, NeteaseFilenameFormat::ArtistTitle)
            .unwrap();

        let tag = Tag::read_from_path(&output).unwrap();
        assert_eq!(tag.title(), Some("歌曲"));
        assert_eq!(tag.artist(), Some("歌手"));
    }

    #[test]
    fn task1_resolver_fills_untagged_mp3_title_artist_and_album() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("Database Song - Database Artist.mp3");
        let output = dir.path().join("Database Song - Database Artist.mp3");
        let database = dir.path().join("sqlite_storage.sqlite3");
        fs::write(&source, b"audio").unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, tid TEXT, aid TEXT, filesize INTEGER);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track(file,title,artist,album,tid,aid,filesize) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    source.to_string_lossy(),
                    "Database Song",
                    "Database Artist",
                    "Database Album",
                    "track-1",
                    "album-1",
                    5_i64,
                ],
            )
            .unwrap();
        drop(connection);
        let resolver =
            Arc::new(crate::netease::NeteaseMetadataResolver::load_exact(&database).unwrap());
        let context = ConversionMetadataContext {
            netease: resolver.clone(),
        };

        ensure_output_metadata_with_settings_with_context_and_policy(
            &source,
            &output,
            NeteaseFilenameFormat::TitleArtist,
            FilenameNormalizationPolicy::PreserveSource,
            &context,
        )
        .unwrap();
        let tag = Tag::read_from_path(&output).unwrap();
        assert_eq!(tag.title(), Some("Database Song"));
        assert_eq!(tag.artist(), Some("Database Artist"));
        assert_eq!(tag.album(), Some("Database Album"));
        assert_eq!(
            derive_song_name_with_policy_and_resolver(
                &source,
                FilenameRule::TitleArtist,
                NeteaseFilenameFormat::TitleArtist,
                FilenameNormalizationPolicy::PreserveSource,
                Some(resolver.as_ref()),
            ),
            "Database Song - Database Artist"
        );
    }

    #[test]
    fn metadata_diagnostic_records_per_field_provenance_and_exact_output_values() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("Source - Artist.mp3");
        let output = dir.path().join("Source - Artist-out.mp3");
        let database = dir.path().join("sqlite_storage.sqlite3");

        fs::write(&source, b"audio").unwrap();
        let mut source_tag = Tag::new();
        source_tag.set_title("Source,  Title");
        source_tag.set_artist("Source Artist");
        source_tag.write_to_path(&source, Version::Id3v24).unwrap();

        fs::write(&output, b"audio").unwrap();
        let mut output_tag = Tag::new();
        output_tag.set_title("Source,  Title");
        output_tag.set_artist("Source Artist");
        output_tag.set_album("Database Album");
        output_tag.write_to_path(&output, Version::Id3v24).unwrap();

        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, tid TEXT, aid TEXT, filesize INTEGER);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track(file,title,artist,album,tid,aid,filesize) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    source.to_string_lossy(),
                    "Database Title",
                    "Database Artist",
                    "Database Album",
                    "track-1",
                    "album-1",
                    fs::metadata(&source).unwrap().len() as i64,
                ],
            )
            .unwrap();
        drop(connection);

        let resolver = NeteaseMetadataResolver::load_exact(&database).unwrap();
        let diagnostic = inspect_metadata_diagnostic_with_resolver(&source, &output, &resolver);

        assert_eq!(diagnostic.title_source.as_deref(), Some("sourceTag"));
        assert_eq!(diagnostic.artist_source.as_deref(), Some("sourceTag"));
        assert_eq!(diagnostic.album_source.as_deref(), Some("neteaseDatabase"));
        assert_eq!(diagnostic.title_difference.as_deref(), Some("exact"));
        assert_eq!(diagnostic.artist_difference.as_deref(), Some("exact"));
        assert_eq!(diagnostic.album_difference.as_deref(), Some("exact"));
        assert_eq!(diagnostic.output_tags_match, Some(true));
        assert!(diagnostic.decision.contains("Source,  Title"));
        assert!(diagnostic.decision.contains("Database Album"));
    }

    #[test]
    fn task2_source_policy_does_not_backfill_netease_metadata() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("Database Song - Database Artist.mp3");
        let output = dir.path().join("Database Song - Database Artist.mp3");
        let database = dir.path().join("sqlite_storage.sqlite3");
        fs::write(&source, b"audio").unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, tid TEXT, aid TEXT, filesize INTEGER);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track(file,title,artist,album,tid,aid,filesize) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    source.to_string_lossy(),
                    "Database Song",
                    "Database Artist",
                    "Database Album",
                    "track-1",
                    "album-1",
                    5_i64,
                ],
            )
            .unwrap();
        drop(connection);
        let resolver =
            Arc::new(crate::netease::NeteaseMetadataResolver::load_exact(&database).unwrap());
        let context = ConversionMetadataContext { netease: resolver };

        ensure_output_metadata_with_settings_with_context_and_policy(
            &source,
            &output,
            NeteaseFilenameFormat::TitleArtist,
            FilenameNormalizationPolicy::SoundCloud,
            &context,
        )
        .unwrap();
        let tag = Tag::read_from_path(&output).unwrap();
        assert_eq!(tag.title(), Some("Database Song"));
        assert_eq!(tag.artist(), Some("Database Artist"));
        assert_eq!(tag.album(), None);
    }

    #[test]
    fn writes_rekordbox_readable_riff_info_to_wav() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Song.wav");
        let data_len = 4u32;
        let riff_size = 36 + data_len;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&riff_size.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44_100u32.to_le_bytes());
        wav.extend_from_slice(&88_200u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&[0, 0, 0, 0]);
        fs::write(&path, wav).unwrap();

        let mut tag = Tag::new();
        tag.set_title("歌曲名");
        tag.set_artist("歌手");
        tag.set_album("专辑");
        write_riff_info_metadata(&path, &tag).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert!(bytes.windows(4).any(|window| window == b"LIST"));
        assert!(bytes.windows("INAM".len()).any(|window| window == b"INAM"));
        assert!(
            bytes
                .windows("歌曲名".len())
                .any(|window| window == "歌曲名".as_bytes())
        );
        assert!(
            bytes
                .windows("歌手".len())
                .any(|window| window == "歌手".as_bytes())
        );
        assert!(
            bytes
                .windows("专辑".len())
                .any(|window| window == "专辑".as_bytes())
        );
    }

    #[test]
    fn merges_recovered_netease_tags_and_cover_without_overwriting_existing_values() {
        let mut tag = Tag::new();
        let recovered = crate::netease::RecoveredMetadata {
            title: String::from("网易云歌曲"),
            artist: String::from("网易云歌手"),
            album: String::from("网易云专辑"),
            cover: Some(vec![0xFF, 0xD8, 0xFF, 0x00]),
            genre: String::new(),
            aliases_json: String::new(),
            copyright_text: String::new(),
            publish_date: String::new(),
            lyric_plain_text: String::new(),
            lyric_translated_text: String::new(),
            lyric_romanized_text: String::new(),
            lyric_lrc_text: String::new(),
            lyric_language: String::new(),
            lyric_sync_type: String::new(),
            lyric_source: String::new(),
            source: String::from("网易云本地数据库 + 本地封面"),
        };

        assert!(merge_recovered_metadata(&mut tag, &recovered));
        assert_eq!(tag.title(), Some("网易云歌曲"));
        assert_eq!(tag.artist(), Some("网易云歌手"));
        assert_eq!(tag.album(), Some("网易云专辑"));
        assert_eq!(tag.pictures().count(), 1);

        tag.set_title("用户标题");
        assert!(!merge_recovered_metadata(&mut tag, &recovered));
        assert_eq!(tag.title(), Some("用户标题"));
    }

    #[test]
    fn writes_essentia_analysis_to_native_output_metadata() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Song.mp3");
        fs::write(&path, b"audio").unwrap();
        let mut tag = Tag::new();
        tag.set_title("Song");
        tag.write_to_path(&path, Version::Id3v24).unwrap();

        let analysis = EmbeddedAnalysis {
            path: "/music/Song.mp3".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            genre: String::new(),
            bpm: Some(140.25),
            key: Some("F#".into()),
            scale: Some("minor".into()),
            key_strength: Some(0.92),
            integrated_loudness_lufs: Some(-7.3),
            loudness_range_lu: Some(4.2),
            energy: Some(0.81),
            danceability: Some(0.76),
            beat_positions: vec![0.0, 0.428],
            analyzer: "Essentia.js".into(),
            analysis_version: "0.1.3".into(),
            drop_loudness_lufs: None,
            drop_analysis: None,
            high_level: None,
        };

        apply_track_analysis_metadata(&path, &analysis).unwrap();
        validate_track_analysis_metadata(&path, &analysis).unwrap();

        let tag = Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.text_for_frame_id("TBPM"), Some("140.25"));
        assert_eq!(tag.text_for_frame_id("TKEY"), Some("F#m"));
        assert!(
            tag.extended_texts()
                .any(|frame| frame.description == "W4DJ-Energy" && frame.value == "0.8100")
        );
        assert!(
            tag.comments()
                .any(|comment| comment.description == "W4DJ Essentia")
        );
    }

    #[test]
    fn writes_discogs_heads_to_independent_namespaced_frames() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Discogs.mp3");
        fs::write(&path, b"audio").unwrap();
        let mut tag = Tag::new();
        tag.set_title("Discogs");
        tag.write_to_path(&path, Version::Id3v24).unwrap();

        let mut heads = BTreeMap::new();
        for (model, label) in [
            ("moodTheme", "dark"),
            ("approachability", "approachable"),
            ("instrumentation", "synth"),
            ("timbre", "bright"),
            ("danceability", "danceable"),
        ] {
            heads.insert(
                model.to_string(),
                crate::analysis::DiscogsEffnetHeadResult {
                    model: model.to_string(),
                    status: "completed".to_string(),
                    version: "discogs-test".to_string(),
                    labels: vec![crate::analysis::AnalysisLabel {
                        label: label.to_string(),
                        confidence: 0.91,
                    }],
                    scores: BTreeMap::from([(label.to_string(), 0.91)]),
                    frame_count: 2,
                    threshold: Some(0.35),
                    selected_class: Some(label.to_string()),
                    selected_confidence: Some(0.91),
                    reason: None,
                },
            );
        }
        let analysis = EmbeddedAnalysis {
            path: "/music/Discogs.mp3".into(),
            title: "Discogs".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            genre: String::new(),
            bpm: None,
            key: None,
            scale: None,
            key_strength: None,
            integrated_loudness_lufs: None,
            loudness_range_lu: None,
            energy: None,
            danceability: None,
            beat_positions: Vec::new(),
            analyzer: "Essentia.js".into(),
            analysis_version: "discogs-test".into(),
            drop_loudness_lufs: None,
            drop_analysis: None,
            high_level: Some(crate::analysis::HighLevelAnalysis {
                status: "completed".into(),
                model_version: Some("discogs-test".into()),
                reason: None,
                genre: Vec::new(),
                style: Vec::new(),
                mood: Vec::new(),
                instrument: Vec::new(),
                emotion_candidates: None,
                mood_cluster: Vec::new(),
                mood_cluster_status: None,
                mood_cluster_reason: None,
                filtered: Vec::new(),
                discogs_effnet: Some(crate::analysis::DiscogsEffnetAnalysis {
                    embedding_model: "discogs-effnet-bs64-1".into(),
                    embedding_dimensions: 1280,
                    input_shape: vec![64, 128, 96],
                    heads,
                }),
            }),
        };

        apply_track_analysis_metadata(&path, &analysis).unwrap();
        validate_track_analysis_metadata(&path, &analysis).unwrap();
        let tag = Tag::read_from_path(&path).unwrap();
        for description in [
            "W4DJ-Discogs-MoodTheme",
            "W4DJ-Discogs-Approachability",
            "W4DJ-Discogs-Instrumentation",
            "W4DJ-Discogs-Timbre",
            "W4DJ-Discogs-Danceability",
        ] {
            assert!(
                tag.extended_texts()
                    .any(|frame| frame.description == description)
            );
        }
        // The Discogs danceability head is namespaced and must not replace
        // the existing scalar W4DJ-Danceability field.
        assert!(
            !tag.extended_texts()
                .any(|frame| frame.description == "W4DJ-Danceability")
        );
    }

    #[test]
    fn writes_analysis_when_original_source_is_missing() {
        let dir = tempdir().unwrap();
        let output = dir.path().join("Song.mp3");
        fs::write(&output, b"audio").unwrap();
        let mut tag = Tag::new();
        tag.set_title("Song");
        tag.write_to_path(&output, Version::Id3v24).unwrap();

        let analysis = EmbeddedAnalysis {
            path: dir.path().join("removed-source.ncm").display().to_string(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: String::new(),
            genre: String::new(),
            bpm: None,
            key: None,
            scale: None,
            key_strength: None,
            integrated_loudness_lufs: None,
            loudness_range_lu: None,
            energy: Some(0.42),
            danceability: Some(0.88),
            beat_positions: Vec::new(),
            analyzer: "Essentia.js".into(),
            analysis_version: "0.2.0".into(),
            drop_loudness_lufs: None,
            drop_analysis: None,
            high_level: Some(crate::analysis::HighLevelAnalysis {
                status: "failed".into(),
                model_version: None,
                reason: Some("模型输入失败".into()),
                genre: Vec::new(),
                style: Vec::new(),
                mood: Vec::new(),
                instrument: Vec::new(),
                emotion_candidates: None,
                mood_cluster: Vec::new(),
                mood_cluster_status: None,
                mood_cluster_reason: None,
                filtered: Vec::new(),
                discogs_effnet: None,
            }),
        };

        update_analysis_metadata_transactionally(&output, |temporary_output| {
            apply_track_analysis_metadata(temporary_output, &analysis)
        })
        .unwrap();

        let tag = Tag::read_from_path(&output).unwrap();
        assert!(
            tag.extended_texts()
                .any(|frame| { frame.description == "W4DJ-Energy" && frame.value == "0.4200" })
        );
        assert!(
            tag.extended_texts().any(|frame| {
                frame.description == "W4DJ-Danceability" && frame.value == "0.8800"
            })
        );
    }

    #[test]
    fn corrects_conflicting_fields_and_preserves_source_cover() {
        let mut source = Tag::new();
        source.set_album("Existing Album");
        source.add_frame(id3::frame::Picture {
            mime_type: "image/jpeg".into(),
            picture_type: id3::frame::PictureType::CoverFront,
            description: String::new(),
            data: vec![0xff, 0xd8, 0xff, 0xe0, 0x01, 0x02],
        });

        let mut output = Tag::new();
        output.set_title("Existing Title");
        let identity = infer_song_identity("Artist - Fallback Title", None, None);

        assert!(fill_missing_metadata(&mut output, &source, &identity));

        assert_eq!(output.title(), Some("Fallback Title"));
        assert_eq!(output.artist(), Some("Artist"));
        assert_eq!(output.album(), Some("Existing Album"));
        assert_eq!(output.pictures().count(), 1);
    }

    #[test]
    fn conflicting_output_cleanup_never_deletes_the_source_file() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("Song.flac");
        let stale_output = dir.path().join("Song.wav");
        fs::write(&source, b"source-audio").unwrap();
        fs::write(&stale_output, b"stale-output").unwrap();

        remove_conflicting_outputs(dir.path().to_str().unwrap(), "Song", "mp3", &source).unwrap();

        assert!(source.exists());
        assert!(stale_output.exists());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn skips_placeholder_ffmpeg_sidecars() {
        let dir = tempdir().unwrap();
        let exe_path = dir.path().join("w4dj");
        let placeholder = dir.path().join("ffmpeg-aarch64-apple-darwin");
        let fallback = dir.path().join("ffmpeg");

        fs::write(&exe_path, []).unwrap();
        fs::write(&placeholder, b"local cargo-check placeholder\n").unwrap();
        write_executable_file(&fallback, b"real ffmpeg binary");

        let found = find_ffmpeg_next_to_exe(&exe_path).unwrap();

        assert_eq!(found, fallback);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn active_ffmpeg_registry_terminates_cancelled_children() {
        let registry = ActiveFfmpegRegistry::new();
        let child = Command::new("sh")
            .args(["-c", "sleep 10"])
            .spawn()
            .expect("shell should start");
        let child_id = registry.insert(child);
        let cancelled = AtomicBool::new(true);
        let started = std::time::Instant::now();
        let status = registry.wait_for(child_id, &cancelled).unwrap();

        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(registry.active_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn managed_conversion_runs_distinct_ffmpeg_outputs_in_parallel() {
        let dir = tempdir().unwrap();
        let fake_ffmpeg = dir.path().join("ffmpeg");
        let log_path = dir.path().join("ffmpeg.log");
        write_executable_file(
            &fake_ffmpeg,
            br##"#!/bin/sh
input=""
output=""
previous=""
for argument in "$@"; do
  if [ "$previous" = "-i" ]; then input="$argument"; fi
  output="$argument"
  previous="$argument"
done
printf '%s\n' START >> "$W4DJ_FAKE_FFMPEG_LOG"
sleep 0.25
cp "$input" "$output"
printf '%s\n' END >> "$W4DJ_FAKE_FFMPEG_LOG"
"##,
        );
        unsafe {
            std::env::set_var("W4DJ_FFMPEG_PATH", &fake_ffmpeg);
            std::env::set_var("W4DJ_FAKE_FFMPEG_LOG", &log_path);
        }

        let destination = dir.path().join("output");
        fs::create_dir_all(&destination).unwrap();
        let mut songs = HashMap::new();
        for index in 0..4 {
            let source = dir.path().join(format!("Song {index}.wav"));
            write_test_wav(&source);
            let name = format!("Song {index}");
            songs.insert(name, (String::from("1"), source));
        }
        let controller = crate::task::TaskController::running(songs.len());
        let queued_songs = songs.iter().collect();
        let result = sync_music_library_transactional_with_observer_and_budget_and_context(
            &queued_songs,
            destination.to_str().unwrap(),
            &Mode::Lossless,
            Some(LosslessFormat::Wav),
            NeteaseFilenameFormat::default(),
            &controller,
            |_, _| Ok(()),
            |_, _, _| {},
            Arc::new(GlobalConcurrencyBudget::new(2)),
            Arc::new(ActiveFfmpegRegistry::new()),
            &ConversionMetadataContext::default(),
        );
        unsafe {
            std::env::remove_var("W4DJ_FFMPEG_PATH");
            std::env::remove_var("W4DJ_FAKE_FFMPEG_LOG");
        }

        assert!(result.is_ok(), "conversion should succeed: {result:?}");
        assert_eq!(controller.snapshot().completed, 4);
        let log = fs::read_to_string(log_path).unwrap();
        let lines = log.lines().collect::<Vec<_>>();
        assert!(
            lines.len() >= 4,
            "fake FFmpeg did not run four times: {log}"
        );
        assert_eq!(&lines[..2], ["START", "START"]);
    }
}
