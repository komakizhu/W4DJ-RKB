use crate::config::{FilenameRule, LosslessFormat, Mode};
use crate::metadata::{
    FlacMetadata, Metadata, Mp3Metadata, build_id3_tag, build_id3_tag_from_flac,
};
use crate::task::{TaskController, TaskSnapshot};
use id3::{TagLike, Version};
use ncmdump::Ncmdump;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Error, ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SUPPORTED_SOURCE_EXTENSIONS: &[&str] = &["mp3", "flac", "ncm", "wav", "aiff"];

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
    let source_path = Path::new(folder);
    if source_path.is_file() && !is_supported_source_file(source_path) {
        return (HashMap::new(), Vec::new());
    }

    collect_music_dict_with_scan_issues(folder, SUPPORTED_SOURCE_EXTENSIONS, filename_rule)
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
    collect_music_dict_with_scan_issues(folder, &["mp3", "wav", "aiff"], filename_rule).0
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
) -> (HashMap<String, (String, PathBuf)>, Vec<MusicScanIssue>) {
    let mut music_dict = HashMap::new();
    let mut scan_issues = Vec::new();

    for entry_result in walkdir::WalkDir::new(folder) {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) => {
                if let Some(path) = error.path().filter(|path| {
                    !is_ignored_music_file(path) && has_allowed_extension(path, allowed_extensions)
                }) {
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

        let path = entry.path().to_path_buf();
        let song_name = derive_song_name_with_rule(entry.path(), filename_rule);
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

    (music_dict, scan_issues)
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

        let task_result = process_music_file(name, info, dest_folder, mode, lossless_format, &bar);
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
    #[allow(deprecated)]
    let result = match destination_extension.as_str() {
        "wav" => source_tag.write_to_wav_path(destination_path, Version::Id3v24),
        "aiff" | "aif" => source_tag.write_to_aiff_path(destination_path, Version::Id3v24),
        "mp3" => source_tag.write_to_path(destination_path, Version::Id3v24),
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

fn process_music_file(
    name: &str,
    info: &(String, PathBuf),
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    bar: &indicatif::ProgressBar,
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

    match extension.as_str() {
        "mp3" => {
            bar.set_message(format!("Copying MP3: {}", name));
            let output_policy = resolve_output_policy(*mode, lossless_format, &extension);
            let output_path = target_output_path(dest_folder, name, output_policy.output_extension);
            let result = match output_policy.target_profile {
                TargetProfile::CompatMp3 => copy_file(src_path, &output_path),
                _ => convert_audio_to_target_format(
                    src_path,
                    &output_path,
                    output_policy.target_profile,
                    name,
                ),
            };

            if result.is_ok() {
                if matches!(output_policy.target_profile, TargetProfile::CompatMp3) {
                    strip_163_key_from_mp3(&output_path)?;
                }
                remove_conflicting_outputs(
                    dest_folder,
                    name,
                    output_policy.output_extension,
                    src_path,
                )?;
            }

            result
        }
        "wav" | "aiff" => {
            bar.set_message(format!("Processing {}: {}", extension.to_uppercase(), name));
            let output_policy = resolve_output_policy(*mode, lossless_format, &extension);
            let output_path = target_output_path(dest_folder, name, output_policy.output_extension);
            let result = match output_policy.target_profile {
                TargetProfile::CompatMp3
                | TargetProfile::LosslessWav
                | TargetProfile::LosslessAiff => convert_audio_to_target_format(
                    src_path,
                    &output_path,
                    output_policy.target_profile,
                    name,
                ),
            };

            if result.is_ok() {
                remove_conflicting_outputs(
                    dest_folder,
                    name,
                    output_policy.output_extension,
                    src_path,
                )?;
            }

            result
        }
        "flac" => {
            bar.set_message(format!("Processing FLAC: {}", name));
            let output_policy = resolve_output_policy(*mode, lossless_format, &extension);
            let output_path = target_output_path(dest_folder, name, output_policy.output_extension);
            let result = match mode {
                Mode::Lossless => convert_audio_to_target_format(
                    src_path,
                    &output_path,
                    output_policy.target_profile,
                    name,
                ),
                Mode::Compat => convert_flac_to_mp3(src_path, dest_folder, name),
            };

            if result.is_ok() {
                if matches!(
                    output_policy.target_profile,
                    TargetProfile::LosslessWav | TargetProfile::LosslessAiff
                ) {
                    write_container_tags_from_flac_source(
                        src_path,
                        &output_path,
                        output_policy.target_profile,
                    )?;
                }
                remove_conflicting_outputs(
                    dest_folder,
                    name,
                    output_policy.output_extension,
                    src_path,
                )?;
            }

            result
        }
        "ncm" => {
            bar.set_message(format!("Dumping NCM: {}", name));
            let result = process_ncm_file(src_path, dest_folder, name, *mode, lossless_format);
            if result.is_ok() {
                let file_format =
                    detect_ncm_output_extension(src_path).unwrap_or_else(|_| "flac".to_string());
                let output_policy = resolve_output_policy(*mode, lossless_format, &file_format);
                if matches!(output_policy.target_profile, TargetProfile::CompatMp3) {
                    let output_path =
                        target_output_path(dest_folder, name, output_policy.output_extension);
                    strip_163_key_from_mp3(&output_path)?;
                }
                remove_conflicting_outputs(
                    dest_folder,
                    name,
                    output_policy.output_extension,
                    src_path,
                )?;
            }
            result
        }
        _ => unreachable!(
            "Invalid file extension '{}' for song '{}'. Filter failed.",
            extension, name
        ),
    }
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

fn convert_flac_to_mp3(src_path: &Path, dest_folder: &str, name_stem: &str) -> io::Result<()> {
    let output_path = target_output_path(dest_folder, name_stem, "mp3");
    convert_audio_to_target_format(src_path, &output_path, TargetProfile::CompatMp3, name_stem)
}

fn convert_audio_to_target_format(
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

    let status = command.arg(output_path).status().map_err(|error| {
        Error::new(
            error.kind(),
            format!("Failed to start FFmpeg at {}: {}", ffmpeg_path, error),
        )
    })?;

    if !status.success() {
        return Err(Error::other(format!(
            "FFmpeg conversion failed for {}",
            name_stem
        )));
    }

    ensure_generated_output(output_path, name_stem)
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

#[cfg(target_os = "windows")]
fn configure_background_process(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn configure_background_process(_command: &mut Command) {}

fn process_ncm_file(
    src_path: &Path,
    dest_folder: &str,
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
    let output_path = target_output_path(dest_folder, name_stem, output_policy.output_extension);
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
                fs::copy(&temp_source_path, &output_path)?;
            } else {
                convert_audio_to_target_format(
                    &temp_source_path,
                    &output_path,
                    TargetProfile::CompatMp3,
                    name_stem,
                )?;
            }
        }
        TargetProfile::LosslessWav | TargetProfile::LosslessAiff => {
            convert_audio_to_target_format(
                &temp_source_path,
                &output_path,
                output_policy.target_profile,
                name_stem,
            )?;

            write_container_tags(
                &output_path,
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
        "mp3" | "wav" | "aiff" => song_name_from_audio_tag(path, filename_rule, &fallback_name),
        "flac" => song_name_from_flac(path, filename_rule, &fallback_name),
        "ncm" => song_name_from_ncm(path, filename_rule, &fallback_name),
        _ => None,
    };

    candidate.unwrap_or_else(|| normalize_fallback_song_name(&fallback_name, filename_rule))
}

fn song_name_from_flac(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
) -> Option<String> {
    let tag = metaflac::Tag::read_from_path(path).ok()?;
    let comments = tag.vorbis_comments();
    let title = comments.and_then(|comments| first_non_empty(comments.title()));
    let artist = comments.and_then(|comments| {
        join_non_empty(comments.artist()).or_else(|| join_non_empty(comments.album_artist()))
    });
    let identity = infer_song_identity(fallback_name, title, artist.as_deref());
    build_song_name_with_rule(&identity.title, &identity.artist, filename_rule)
}

fn song_name_from_audio_tag(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
) -> Option<String> {
    let tag = id3::Tag::read_from_path(path).ok()?;
    let artist = tag.artist().or_else(|| tag.album_artist());
    let identity = infer_song_identity(fallback_name, tag.title(), artist);
    build_song_name_with_rule(&identity.title, &identity.artist, filename_rule)
}

fn song_name_from_ncm(
    path: &Path,
    filename_rule: FilenameRule,
    fallback_name: &str,
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
    let identity = infer_song_identity(fallback_name, Some(&info.name), Some(&artist));
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
    let (fallback_title, fallback_artist) = parse_filename_identity(fallback_name);
    let title = normalize_filename_part(metadata_title).unwrap_or(fallback_title);
    let artist = normalize_filename_part(metadata_artist).unwrap_or(fallback_artist);

    SongIdentity { title, artist }
}

fn parse_filename_identity(fallback_name: &str) -> (String, String) {
    let display = normalize_display_text(fallback_name);
    display
        .split_once(" - ")
        .map(|(artist, title)| (title.to_string(), artist.to_string()))
        .unwrap_or((display, String::new()))
}

fn normalize_filename_part(value: Option<&str>) -> Option<String> {
    let value = value?;
    let normalized = sanitize_filename_component(&normalize_display_text(value));
    (!normalized.is_empty()).then_some(normalized)
}

fn first_non_empty(values: Option<&Vec<String>>) -> Option<&str> {
    values.and_then(|values| {
        values
            .iter()
            .map(String::as_str)
            .find(|value| !value.trim().is_empty())
    })
}

fn join_non_empty(values: Option<&Vec<String>>) -> Option<String> {
    let values = values?;
    let joined = values
        .iter()
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<&str>>()
        .join(", ");
    (!joined.is_empty()).then_some(joined)
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
        TargetProfile::LosslessWav => tag
            .write_to_wav_path(output_path, Version::Id3v24)
            .map_err(io::Error::other),
        TargetProfile::LosslessAiff => tag
            .write_to_aiff_path(output_path, Version::Id3v24)
            .map_err(io::Error::other),
        _ => Ok(()),
    }
}

fn write_container_tags_from_flac_source(
    source_path: &Path,
    output_path: &Path,
    target_profile: TargetProfile,
) -> io::Result<()> {
    let tag = metaflac::Tag::read_from_path(source_path).map_err(io::Error::other)?;
    let id3_tag = build_id3_tag_from_flac(&tag);

    #[allow(deprecated)]
    match target_profile {
        TargetProfile::LosslessWav => id3_tag
            .write_to_wav_path(output_path, Version::Id3v24)
            .map_err(io::Error::other),
        TargetProfile::LosslessAiff => id3_tag
            .write_to_aiff_path(output_path, Version::Id3v24)
            .map_err(io::Error::other),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SongIdentity, build_song_name, build_song_name_with_rule, compare_music_dicts,
        derive_song_name, derive_song_name_with_rule, ensure_generated_output,
        find_ffmpeg_next_to_exe, infer_song_identity, remove_conflicting_outputs,
        sanitize_filename_component, strip_163_key_from_mp3,
    };
    use crate::config::{FilenameRule, LosslessFormat, Mode};
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
