use crate::analysis::{DropAnalysisDetails, HighLevelAnalysis};
use crate::config::{FilenameRule, LosslessFormat, Mode, NeteaseFilenameFormat};
use crate::metadata::{
    FlacMetadata, Metadata, Mp3Metadata, build_id3_tag, build_id3_tag_from_flac,
};
use crate::netease::RecoveredMetadata;
use crate::scan_cache::{ScanCache, ScanCacheEntry, can_reuse_entry, modified_at_ms};
use crate::task::{TaskController, TaskSnapshot};
use id3::frame::{Comment, ExtendedText, Lyrics, Picture};
use id3::{TagLike, Version};
use ncmdump::Ncmdump;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Error, ErrorKind, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SUPPORTED_SOURCE_EXTENSIONS: &[&str] = &["mp3", "flac", "ncm", "wav", "aiff"];

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ScanPhase {
    Source,
    Destination,
}

pub type ScanObserver<'a> = dyn FnMut(ScanPhase, &Path) -> bool + 'a;

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
    let source_path = Path::new(folder);
    if source_path.is_file() && !is_supported_source_file(source_path) {
        return (HashMap::new(), Vec::new());
    }

    collect_music_dict_with_scan_issues(
        folder,
        SUPPORTED_SOURCE_EXTENSIONS,
        filename_rule,
        netease_filename_format,
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
    let source_path = Path::new(folder);
    if source_path.is_file() && !is_supported_source_file(source_path) {
        return (HashMap::new(), Vec::new(), false);
    }

    collect_music_dict_with_scan_issues_observed(
        folder,
        SUPPORTED_SOURCE_EXTENSIONS,
        filename_rule,
        netease_filename_format,
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
    let source_path = Path::new(folder);
    if source_path.is_file() && !is_supported_source_file(source_path) {
        return (HashMap::new(), Vec::new(), false);
    }

    let source_root = source_path.to_path_buf();
    let mut music_dict = HashMap::new();
    let mut scan_issues = Vec::new();
    let mut cancelled = false;

    for entry_result in walkdir::WalkDir::new(folder) {
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
            || !has_allowed_extension(entry.path(), SUPPORTED_SOURCE_EXTENSIONS)
        {
            continue;
        }

        if !observer(ScanPhase::Source, entry.path()) {
            cancelled = true;
            break;
        }

        let path = entry.path().to_path_buf();
        let path_key = crate::scan_cache::normalize_path(&path)
            .to_string_lossy()
            .into_owned();
        let metadata = entry.metadata().ok();
        let size_bytes = metadata.as_ref().map_or(0, std::fs::Metadata::len);
        let modified_at = modified_at_ms(&path);
        let cached = cache
            .entries
            .get(&path_key)
            .filter(|cached| {
                can_reuse_entry(
                    cached,
                    &path,
                    &source_root,
                    output_directory,
                    filename_rule_cache_key(filename_rule),
                    netease_filename_format_cache_key(netease_filename_format),
                    size_bytes,
                    modified_at,
                )
            })
            .cloned();

        let (song_name, cached_issue) = if let Some(cached) = cached {
            (cached.derived_name, cached.scan_issue)
        } else {
            let song_name =
                derive_song_name_with_settings(&path, filename_rule, netease_filename_format);
            let entry = ScanCacheEntry {
                source_path: path_key,
                source_root: crate::scan_cache::normalize_path(&source_root)
                    .to_string_lossy()
                    .into_owned(),
                output_directory: crate::scan_cache::normalize_path(output_directory)
                    .to_string_lossy()
                    .into_owned(),
                filename_rule: filename_rule_cache_key(filename_rule).to_string(),
                netease_filename_format: netease_filename_format_cache_key(netease_filename_format)
                    .to_string(),
                size_bytes,
                modified_at_ms: modified_at,
                derived_name: song_name.clone(),
                source_extension: path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_lowercase(),
                scan_issue: None,
            };
            cache.insert(entry);
            (song_name, None)
        };

        if let Some(issue) = cached_issue {
            scan_issues.push(MusicScanIssue {
                path: path.clone(),
                message: issue,
            });
        }

        let size = size_bytes.to_string();
        let should_replace = music_dict
            .get(&song_name)
            .map(|existing| should_prefer_file(&path, &size, existing))
            .unwrap_or(true);
        if should_replace {
            music_dict.insert(song_name, (size, path));
        }
    }

    if !cancelled {
        cache.remove_missing_sources(&source_root);
    }
    (music_dict, scan_issues, cancelled)
}

fn filename_rule_cache_key(rule: FilenameRule) -> &'static str {
    match rule {
        FilenameRule::TitleArtist => "title_artist",
        FilenameRule::ArtistTitle => "artist_title",
        FilenameRule::Original => "original",
    }
}

fn netease_filename_format_cache_key(format: NeteaseFilenameFormat) -> &'static str {
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
    )
    .0
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
) -> (HashMap<String, (String, PathBuf)>, Vec<MusicScanIssue>) {
    let (music_dict, scan_issues, _cancelled) = collect_music_dict_with_scan_issues_observed(
        folder,
        allowed_extensions,
        filename_rule,
        netease_filename_format,
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
    mut observer: Option<&mut ScanObserver<'_>>,
    phase: ScanPhase,
) -> (
    HashMap<String, (String, PathBuf)>,
    Vec<MusicScanIssue>,
    bool,
) {
    let mut music_dict = HashMap::new();
    let mut scan_issues = Vec::new();

    for entry_result in walkdir::WalkDir::new(folder) {
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
        let song_name =
            derive_song_name_with_settings(entry.path(), filename_rule, netease_filename_format);
        let size = entry
            .metadata()
            .map(|m| m.len().to_string())
            .unwrap_or_else(|_| "0".to_string());

        let should_replace = music_dict
            .get(&song_name)
            .map(|existing| should_prefer_file(&path, &size, existing))
            .unwrap_or(true);

        if should_replace {
            music_dict.insert(song_name, (size, path));
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

pub(crate) fn is_ignored_music_file(path: &Path) -> bool {
    is_temporary_artifact(path) || is_macos_appledouble_file(path)
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
                needs_regeneration(sf_dict.get(*name), mode, "mp3", expected_extension)
            }
            Mode::Lossless => {
                let source_extension = effective_source_extension(&wf_info.1);
                let expected_extension =
                    resolve_output_policy(*mode, lossless_format, &source_extension)
                        .output_extension;

                needs_regeneration(
                    sf_dict.get(*name),
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
    mut finalize_output: impl FnMut(&str, &Path) -> io::Result<()>,
    mut after_file: impl FnMut(&str, &TaskController, Option<&io::Error>),
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
    queued_files.sort_by(|(left_name, _), (right_name, _)| left_name.cmp(right_name));
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
            &bar,
            &mut finalize_output,
        );
        match task_result {
            Ok(()) => {
                task_controller.complete_current_file();
                bar.inc(1);
                after_file(name, task_controller, None);
            }
            Err(err) => {
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

#[allow(dead_code)]
#[allow(deprecated)]
pub fn update_existing_metadata(source_path: &Path, destination_path: &Path) -> io::Result<()> {
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
        _ => id3::Tag::read_from_path(source_path).map_err(|error| {
            Error::new(
                ErrorKind::InvalidData,
                format!("无法读取源文件元数据：{error}"),
            )
        })?,
    };

    let destination_extension = destination_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let result: io::Result<()> = match destination_extension.as_str() {
        "wav" => write_id3_tag_for_output(&source_tag, destination_path),
        "aiff" | "aif" => source_tag
            .write_to_aiff_path(destination_path, Version::Id3v24)
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
    let name_stem = destination_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("metadata-update");
    run_output_transaction(destination_path, name_stem, |temporary_output| {
        copy_file(destination_path, temporary_output)?;
        update_existing_metadata(source_path, temporary_output)?;
        ensure_output_metadata_with_settings(
            source_path,
            temporary_output,
            netease_filename_format,
        )?;
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
    bar: &indicatif::ProgressBar,
    finalize_output: &mut impl FnMut(&str, &Path) -> io::Result<()>,
) -> io::Result<()> {
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
    let output_path = target_output_path(dest_folder, name, output_policy.output_extension);

    let result = match extension.as_str() {
        "mp3" | "wav" | "aiff" | "flac" | "ncm" => {
            bar.set_message(format!("Processing {}: {}", extension.to_uppercase(), name));
            run_output_transaction(&output_path, name, |temporary_output| {
                match extension.as_str() {
                    "mp3" if matches!(output_policy.target_profile, TargetProfile::CompatMp3) => {
                        copy_file(src_path, temporary_output)?;
                    }
                    "ncm" => process_ncm_file_to_output(
                        src_path,
                        temporary_output,
                        name,
                        *mode,
                        lossless_format,
                    )?,
                    _ => convert_audio_to_output_path(
                        src_path,
                        temporary_output,
                        output_policy.target_profile,
                        name,
                    )?,
                }

                ensure_output_metadata_with_settings(
                    src_path,
                    temporary_output,
                    netease_filename_format,
                )?;
                if matches!(output_policy.target_profile, TargetProfile::CompatMp3) {
                    strip_163_key_from_mp3(temporary_output)?;
                }
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
        if let Some(recovered) = crate::netease::recover_local_metadata(src_path)
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

fn convert_audio_to_output_path(
    src_path: &Path,
    output_path: &Path,
    target_profile: TargetProfile,
    name_stem: &str,
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

    let status = match command.arg(output_path).status() {
        Ok(status) => status,
        Err(error) => {
            let _ = fs::remove_file(output_path);
            return Err(Error::new(
                error.kind(),
                format!("Failed to start FFmpeg at {}: {}", ffmpeg_path, error),
            ));
        }
    };

    if !status.success() {
        let _ = fs::remove_file(output_path);
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

#[allow(dead_code)]
fn ensure_output_metadata(source_path: &Path, output_path: &Path) -> io::Result<()> {
    let netease_filename_format = if source_prefers_title_artist_filename(source_path) {
        NeteaseFilenameFormat::TitleArtist
    } else {
        NeteaseFilenameFormat::ArtistTitle
    };
    ensure_output_metadata_with_settings(source_path, output_path, netease_filename_format)
}

fn ensure_output_metadata_with_settings(
    source_path: &Path,
    output_path: &Path,
    netease_filename_format: NeteaseFilenameFormat,
) -> io::Result<()> {
    let source_tag = source_metadata_as_id3(source_path);
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
    );
    let source_has_valid_cover = source_tag
        .pictures()
        .any(|picture| crate::metadata::get_image_mime_type(&picture.data) != "image/*");

    let changed_identity = fill_missing_metadata(&mut output_tag, &source_tag, &identity);
    let changed_lyrics = merge_lyrics_metadata(&mut output_tag, &source_tag);
    if !changed_identity && !changed_lyrics {
        return Ok(());
    }

    write_id3_tag_for_output(&output_tag, output_path)?;
    validate_written_metadata(output_path, &identity, source_has_valid_cover)
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
    let mut tag = read_id3_tag_or_empty(output_path);
    if let Some(recovered) = crate::netease::recover_local_metadata(Path::new(&analysis.path)) {
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

    tag.remove_comment(Some("W4DJ Essentia"), None);
    tag.add_frame(Comment {
        lang: String::from("eng"),
        description: String::from("W4DJ Essentia"),
        text: analysis_summary(analysis),
    });

    write_id3_tag_for_output(&tag, output_path)
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
    }
    if values.is_empty() {
        String::from("W4DJ Essentia analysis")
    } else {
        format!("W4DJ Essentia | {}", values.join(" | "))
    }
}

fn source_metadata_as_id3(source_path: &Path) -> id3::Tag {
    let extension = source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut tag = match extension.as_str() {
        "flac" => metaflac::Tag::read_from_path(source_path)
            .map(|tag| build_id3_tag_from_flac(&tag))
            .unwrap_or_else(|_| id3::Tag::new()),
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
        _ => read_id3_tag_or_empty(source_path),
    };

    if !matches!(extension.as_str(), "ncm")
        && let Some(recovered) = crate::netease::recover_local_metadata(source_path)
    {
        merge_recovered_metadata(&mut tag, &recovered);
    }

    tag
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
    let source_tag = source_metadata_as_id3(source_path);
    let output_exists = output_path.is_file();
    let output_tag = output_exists.then(|| read_id3_tag_or_empty(output_path));
    let fallback_name = source_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let prefer_title_artist = source_prefers_title_artist_filename(source_path);
    let identity = infer_song_identity_with_filename_preference(
        fallback_name,
        source_tag.title(),
        source_tag.artist().or_else(|| source_tag.album_artist()),
        prefer_title_artist,
    );
    let valid_cover = |tag: &id3::Tag| {
        tag.pictures()
            .any(|picture| crate::metadata::get_image_mime_type(&picture.data) != "image/*")
    };
    let output_matches = output_tag.as_ref().is_some_and(|tag| {
        (identity.title.is_empty() || tag.title() == Some(identity.title.as_str()))
            && (identity.artist.is_empty() || tag.artist() == Some(identity.artist.as_str()))
            && (!valid_cover(&source_tag) || valid_cover(tag))
    });

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
        source_title: non_empty(source_tag.title()).map(str::to_string),
        source_artist: non_empty(source_tag.artist().or_else(|| source_tag.album_artist()))
            .map(str::to_string),
        source_album: non_empty(source_tag.album()).map(str::to_string),
        output_title: output_tag
            .as_ref()
            .and_then(|tag| non_empty(tag.title()).map(str::to_string)),
        output_artist: output_tag.as_ref().and_then(|tag| {
            non_empty(tag.artist().or_else(|| tag.album_artist())).map(str::to_string)
        }),
        output_album: output_tag
            .as_ref()
            .and_then(|tag| non_empty(tag.album()).map(str::to_string)),
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
            "最终标题：{}；最终歌手：{}",
            identity.title, identity.artist
        ),
        metadata_validation: if !output_exists {
            "输出文件不存在或转换失败".to_string()
        } else if output_matches {
            "通过：标题、歌手和可用封面已校验".to_string()
        } else {
            "未通过：输出标签或封面与识别结果不一致".to_string()
        },
    }
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

fn process_ncm_file_to_output(
    src_path: &Path,
    output_path: &Path,
    name_stem: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
) -> io::Result<()> {
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
            convert_audio_to_output_path(
                &temp_source_path,
                output_path,
                output_policy.target_profile,
                name_stem,
            )?;

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

pub(crate) fn target_output_path(
    dest_folder: &str,
    name_stem: &str,
    output_extension: &str,
) -> PathBuf {
    Path::new(dest_folder).join(format!(
        "{}.{}",
        sanitize_filename_component(name_stem),
        output_extension
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

fn remove_conflicting_outputs(
    dest_folder: &str,
    name_stem: &str,
    keep_extension: &str,
    protected_source_path: &Path,
) -> io::Result<()> {
    for extension in ["mp3", "flac", "wav", "aiff"] {
        if extension == keep_extension {
            continue;
        }

        let candidate_path = target_output_path(dest_folder, name_stem, extension);
        if paths_refer_to_same_file(&candidate_path, protected_source_path) {
            continue;
        }
        if candidate_path.exists() {
            fs::remove_file(candidate_path)?;
        }
    }

    Ok(())
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

fn strip_163_key_from_mp3(path: &Path) -> io::Result<()> {
    let mut tag = match id3::Tag::read_from_path(path) {
        Ok(tag) => tag,
        Err(error) if error.to_string().contains("NoTag") => return Ok(()),
        Err(error) => return Err(io::Error::other(error)),
    };
    let comments_to_remove = tag
        .comments()
        .filter(|comment| comment.text.starts_with("163 key(") || comment.description == "163 key")
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
        .filter(|text| text.description == "163 key" || text.description.starts_with("163 key("))
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
    let fallback_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();

    if matches!(filename_rule, FilenameRule::Original) {
        return sanitize_filename_component(&fallback_name);
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    let candidate = match extension.as_str() {
        "mp3" | "wav" | "aiff" => {
            song_name_from_audio_tag(path, filename_rule, &fallback_name, netease_filename_format)
        }
        "flac" => song_name_from_flac(path, filename_rule, &fallback_name, netease_filename_format),
        "ncm" => song_name_from_ncm(path, filename_rule, &fallback_name, netease_filename_format),
        _ => None,
    };

    candidate.unwrap_or_else(|| normalize_fallback_song_name(&fallback_name, filename_rule))
}

fn song_name_from_flac(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
    netease_filename_format: NeteaseFilenameFormat,
) -> Option<String> {
    let tag = source_metadata_as_id3(path);
    let identity = infer_song_identity_with_netease_filename_format(
        fallback_name,
        tag.title(),
        tag.artist().or_else(|| tag.album_artist()),
        netease_filename_format,
    );
    build_song_name_with_rule(&identity.title, &identity.artist, filename_rule)
}

fn song_name_from_audio_tag(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
    netease_filename_format: NeteaseFilenameFormat,
) -> Option<String> {
    let tag = source_metadata_as_id3(path);
    let artist = tag.artist().or_else(|| tag.album_artist());
    let identity = infer_song_identity_with_netease_filename_format(
        fallback_name,
        tag.title(),
        artist,
        netease_filename_format,
    );
    build_song_name_with_rule(&identity.title, &identity.artist, filename_rule)
}

fn song_name_from_ncm(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
    netease_filename_format: NeteaseFilenameFormat,
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
    );
    build_song_name_with_rule(&identity.title, &identity.artist, filename_rule)
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
) -> SongIdentity {
    let title = normalize_filename_part(metadata_title);
    let artist = normalize_filename_part(metadata_artist);
    if title.is_some() && artist.is_some() {
        return SongIdentity {
            title: title.unwrap_or_default(),
            artist: artist.unwrap_or_default(),
        };
    }

    let fallback = normalize_display_text(fallback_name);
    let (fallback_title, fallback_artist) = match netease_filename_format {
        NeteaseFilenameFormat::TitleOnly => (fallback, String::new()),
        NeteaseFilenameFormat::ArtistTitle => split_filename_identity(&fallback)
            .map(|(artist, title)| (title, artist))
            .unwrap_or_else(|| (fallback, String::new())),
        NeteaseFilenameFormat::TitleArtist => {
            split_filename_identity(&fallback).unwrap_or_else(|| (fallback, String::new()))
        }
    };

    SongIdentity {
        title: title.unwrap_or_else(|| sanitize_filename_component(&fallback_title)),
        artist: artist.unwrap_or_else(|| sanitize_filename_component(&fallback_artist)),
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
    if let Some((left, right)) = split_filename_identity(&display) {
        return if prefer_title_artist_filename {
            (left, right)
        } else {
            (right, left)
        };
    }
    (display, String::new())
}

fn split_filename_identity(fallback_name: &str) -> Option<(String, String)> {
    let display = normalize_display_text(fallback_name);
    display
        .split_once(" - ")
        .map(|(left, right)| (left.to_string(), right.to_string()))
}

fn normalize_filename_part(value: Option<&str>) -> Option<String> {
    let value = value?;
    let normalized = sanitize_filename_component(&normalize_display_text(value));
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
fn build_song_name(title: &str, artist: &str) -> Option<String> {
    build_song_name_with_rule(title, artist, FilenameRule::default())
}

fn build_song_name_with_rule(
    title: &str,
    artist: &str,
    filename_rule: FilenameRule,
) -> Option<String> {
    let title = sanitize_filename_component(&normalize_display_text(title));
    let artist = sanitize_filename_component(&normalize_display_text(artist));

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

fn normalize_fallback_song_name(fallback_name: &str, filename_rule: FilenameRule) -> String {
    let identity = infer_song_identity(fallback_name, None, None);
    build_song_name_with_rule(&identity.title, &identity.artist, filename_rule)
        .unwrap_or_else(|| normalize_display_text(fallback_name))
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

    loop {
        let Some((open, close)) = trailing_bracket_pair(&text) else {
            break;
        };

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

    loop {
        let Some(last_token) = text.split_whitespace().last() else {
            break;
        };

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
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let cleaned = trimmed
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            control if control.is_control() => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    let cleaned = cleaned.trim_end_matches([' ', '.']).to_string();
    let cleaned = if cleaned.is_empty() {
        String::from("未命名")
    } else {
        cleaned
    };
    let stem = cleaned.split('.').next().unwrap_or_default();
    let reserved = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    let cleaned = if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    };

    cleaned.chars().take(180).collect()
}

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
        EmbeddedAnalysis, SongIdentity, apply_track_analysis_metadata, build_song_name,
        build_song_name_with_rule, commit_temporary_output, compare_music_dicts, derive_song_name,
        derive_song_name_with_rule, derive_song_name_with_settings, ensure_generated_output,
        ensure_output_metadata, ensure_output_metadata_with_settings, fill_missing_metadata,
        find_ffmpeg_next_to_exe, infer_song_identity, merge_recovered_metadata,
        remove_conflicting_outputs, run_output_transaction, sanitize_filename_component,
        strip_163_key_from_mp3, write_riff_info_metadata,
    };
    use crate::config::{FilenameRule, LosslessFormat, Mode, NeteaseFilenameFormat};
    use id3::{Tag, TagLike, Version};
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

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

    #[test]
    fn sanitizes_invalid_filename_characters() {
        assert_eq!(sanitize_filename_component("A/B:C*D?"), "A-B-C-D-");
        assert_eq!(sanitize_filename_component("CON"), "_CON");
        assert_eq!(sanitize_filename_component("Track..."), "Track");
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
        assert!(!stale_output.exists());
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
}
