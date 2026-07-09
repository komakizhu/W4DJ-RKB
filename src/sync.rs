use crate::config::{LosslessFormat, Mode};
use crate::metadata::{
    FlacMetadata, Metadata, Mp3Metadata, build_id3_tag, build_id3_tag_from_flac,
};
use crate::task::{TaskController, TaskSnapshot};
use id3::{TagLike, Version};
use ncmdump::Ncmdump;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Error, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use which;

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
    if let Ok(exe_dir) = env::current_exe() {
        if let Some(parent) = exe_dir.parent() {
            let local_ffmpeg = parent.join("ffmpeg");
            if local_ffmpeg.exists() {
                return Some(local_ffmpeg.to_string_lossy().into_owned());
            }
        }
    }

    if let Ok(path) = which::which("ffmpeg") {
        return Some(path.to_string_lossy().into_owned());
    }

    None
}

pub fn get_music_dict(folder: &str) -> HashMap<String, (String, String)> {
    let mut music_dict = HashMap::new();

    for entry in walkdir::WalkDir::new(folder)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map_or(false, |ext_str| {
                        matches!(ext_str.to_lowercase().as_str(), "mp3" | "flac" | "ncm")
                    })
        })
    {
        let path = entry.path().to_string_lossy().into_owned();
        let song_name = derive_song_name(entry.path());
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

    music_dict
}

fn should_prefer_file(
    candidate_path: &str,
    candidate_size: &str,
    current: &(String, String),
) -> bool {
    let candidate_rank = file_rank(candidate_path);
    let current_rank = file_rank(&current.1);

    candidate_rank > current_rank
        || (candidate_rank == current_rank
            && candidate_size.parse::<u64>().unwrap_or(0) >= current.0.parse::<u64>().unwrap_or(0))
}

fn file_rank(path: &str) -> u8 {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .as_deref()
    {
        Some("flac") => 3,
        Some("ncm") => 2,
        Some("mp3") => 1,
        _ => 0,
    }
}

pub fn compare_music_dicts<'a>(
    wf_dict: &'a HashMap<String, (String, String)>,
    sf_dict: &'a HashMap<String, (String, String)>,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
) -> HashMap<&'a String, &'a (String, String)> {
    wf_dict
        .iter()
        .filter(|(name, wf_info)| match mode {
            Mode::Compat => {
                let expected_extension =
                    resolve_output_policy(*mode, lossless_format, "mp3").output_extension;
                if let Some(existing) = sf_dict.get(*name) {
                    let existing_extension = Path::new(&existing.1)
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.to_lowercase())
                        .unwrap_or_default();

                    existing_extension != expected_extension
                } else {
                    true
                }
            }
            Mode::Lossless => {
                let source_extension = effective_source_extension(&wf_info.1);
                let expected_extension =
                    resolve_output_policy(*mode, lossless_format, &source_extension)
                        .output_extension;

                if let Some(sf_info) = sf_dict.get(*name) {
                    let existing_extension = Path::new(&sf_info.1)
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext.to_lowercase())
                        .unwrap_or_default();

                    if existing_extension != expected_extension {
                        return true;
                    }

                    if let (Ok(size1), Ok(size2)) =
                        (wf_info.0.parse::<u64>(), sf_info.0.parse::<u64>())
                    {
                        let max_size = size1.max(size2) as f64;
                        if max_size > 0.0 {
                            let diff = (size1 as f64 - size2 as f64).abs();
                            return (diff / max_size) >= 0.05;
                        }
                        return size1 != size2;
                    }
                    true
                } else {
                    true
                }
            }
        })
        .collect()
}

pub fn sync_music_library_with_policy(
    new_songs: &HashMap<&String, &(String, String)>,
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
    new_songs: &HashMap<&String, &(String, String)>,
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
        |_, _| {},
    )
}

pub fn sync_music_library_with_observer(
    new_songs: &HashMap<&String, &(String, String)>,
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    task_controller: &TaskController,
    mut after_file: impl FnMut(&str, &TaskController),
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

    for (&name, info) in new_songs.iter() {
        if task_controller.is_cancelled() {
            bar.abandon_with_message("Sync cancelled.");
            return Ok(task_controller.snapshot());
        }

        if !task_controller.should_start_next_file() {
            bar.abandon_with_message("Sync paused after current file.");
            return Ok(task_controller.snapshot());
        }

        let task_result = process_music_file(name, info, dest_folder, mode, lossless_format, &bar);
        if let Err(err) = task_result {
            bar.abandon_with_message(format!("Sync encountered errors. First error: {}", err));
            return Err(err);
        }

        task_controller.complete_current_file();
        bar.inc(1);
        after_file(name, task_controller);
    }

    let snapshot = task_controller.snapshot();
    bar.finish_with_message(format!(
        "Sync processing complete. {}/{} files processed.",
        snapshot.completed, snapshot.total
    ));
    Ok(snapshot)
}

fn process_music_file(
    name: &str,
    info: &(String, String),
    dest_folder: &str,
    mode: &Mode,
    lossless_format: Option<LosslessFormat>,
    bar: &indicatif::ProgressBar,
) -> io::Result<()> {
    let src_path = Path::new(&info.1);
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
                remove_conflicting_outputs(dest_folder, name, output_policy.output_extension)?;
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
                remove_conflicting_outputs(dest_folder, name, output_policy.output_extension)?;
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
                remove_conflicting_outputs(dest_folder, name, output_policy.output_extension)?;
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
    fs::copy(src_path, dest_path).map(|_| ())
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
            "FFmpeg not found. Please ensure it is installed and in your PATH.",
        )
    })?;

    let mut command = Command::new(&ffmpeg_path);
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

    let status = command.arg(output_path).status()?;

    if !status.success() {
        return Err(Error::new(
            ErrorKind::Other,
            format!("FFmpeg conversion failed for {}", name_stem),
        ));
    }

    Ok(())
}

fn process_ncm_file(
    src_path: &Path,
    dest_folder: &str,
    name_stem: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
) -> io::Result<()> {
    let file = File::open(src_path)?;
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
    let temp_source_name = format!(
        ".w4dj-{}.{}",
        sanitize_filename_component(name_stem),
        temp_source_extension
    );
    let temp_source_path = Path::new(dest_folder).join(&temp_source_name);
    if let Some(parent) = temp_source_path.parent() {
        fs::create_dir_all(parent)?;
    }

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

    let mut temp_file = File::create(&temp_source_path)?;
    temp_file.write_all(&temp_data)?;

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

    fs::remove_file(&temp_source_path)?;

    Ok(())
}

fn target_output_path(dest_folder: &str, name_stem: &str, output_extension: &str) -> PathBuf {
    Path::new(dest_folder).join(format!(
        "{}.{}",
        sanitize_filename_component(name_stem),
        output_extension
    ))
}

fn effective_source_extension(source_path: &str) -> String {
    let path = Path::new(source_path);
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
) -> io::Result<()> {
    for extension in ["mp3", "flac", "wav", "aiff"] {
        if extension == keep_extension {
            continue;
        }

        let candidate_path = target_output_path(dest_folder, name_stem, extension);
        if candidate_path.exists() {
            fs::remove_file(candidate_path)?;
        }
    }

    Ok(())
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

    for (_, description, text) in comments_to_remove {
        tag.remove_comment(Some(&description), Some(&text));
    }

    tag.remove_extended_text(Some("163 key"), None);
    tag.write_to_path(path, Version::Id3v24)
        .map_err(io::Error::other)
}

fn derive_song_name(path: &Path) -> String {
    let fallback_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    let candidate = match extension.as_str() {
        "mp3" => song_name_from_mp3(path),
        "flac" => song_name_from_flac(path),
        "ncm" => song_name_from_ncm(path),
        _ => None,
    };

    candidate.unwrap_or(fallback_name)
}

fn song_name_from_mp3(path: &Path) -> Option<String> {
    let tag = id3::Tag::read_from_path(path).ok()?;
    build_song_name(
        tag.title().unwrap_or_default(),
        tag.artist().unwrap_or_default(),
    )
}

fn song_name_from_flac(path: &Path) -> Option<String> {
    let tag = metaflac::Tag::read_from_path(path).ok()?;
    let id3_tag = build_id3_tag_from_flac(&tag);
    build_song_name(
        id3_tag.title().unwrap_or_default(),
        id3_tag.artist().unwrap_or_default(),
    )
}

fn song_name_from_ncm(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut ncm = Ncmdump::from_reader(file).ok()?;
    let info = ncm.get_info().ok()?;
    let artist = info
        .artist
        .iter()
        .map(|item| item.0.as_str())
        .collect::<Vec<&str>>()
        .join(" / ");
    build_song_name(&info.name, &artist)
}

fn build_song_name(title: &str, artist: &str) -> Option<String> {
    let title = sanitize_filename_component(title);
    let artist = sanitize_filename_component(artist);

    match (title.is_empty(), artist.is_empty()) {
        (true, true) => None,
        (false, true) => Some(title),
        (true, false) => Some(artist),
        (false, false) => Some(format!("{} - {}", title, artist)),
    }
}

fn sanitize_filename_component(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    trimmed
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            control if control.is_control() => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
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
    use super::{build_song_name, sanitize_filename_component};

    #[test]
    fn sanitizes_invalid_filename_characters() {
        assert_eq!(sanitize_filename_component("A/B:C*D?"), "A-B-C-D-");
    }

    #[test]
    fn combines_title_and_artist_with_separator() {
        assert_eq!(
            build_song_name("paper hearts", "CLV Edit").as_deref(),
            Some("paper hearts - CLV Edit")
        );
    }
}
