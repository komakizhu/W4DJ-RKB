use crate::config::{CandidateOperation, ConflictStrategy, FilenameRule, LosslessFormat, Mode};
use crate::history::HistoryEntry;
use crate::sync::{
    effective_source_extension, find_ffmpeg, get_destination_music_dict_with_rule,
    get_music_dict_with_scan_issues_with_rule, is_supported_source_file, resolve_output_policy,
    target_output_path,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::io;
use std::path::Path;

pub use crate::config::CandidateOperation as PreviewOperation;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewCandidate {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub source_size_bytes: u64,
    pub estimated_output_bytes: Option<u64>,
    #[serde(default)]
    pub operation: CandidateOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncPreview {
    pub source_directory: String,
    pub destination_directory: String,
    pub new_count: usize,
    pub existing_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub estimated_output_bytes: Option<u64>,
    pub candidates: Vec<PreviewCandidate>,
    pub skipped: Vec<PreviewIssue>,
    pub errors: Vec<PreviewIssue>,
    pub warnings: Vec<PreviewIssue>,
    #[serde(default)]
    pub available_space_bytes: Option<u64>,
    #[serde(default)]
    pub disk_space_sufficient: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlotPreview {
    pub slot_index: usize,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
    #[serde(default)]
    pub filename_rule: FilenameRule,
    pub preview: SyncPreview,
    pub retry_of: Option<String>,
}

pub fn build_sync_preview(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
) -> io::Result<SyncPreview> {
    build_sync_preview_with_settings(
        source_directory,
        destination_directory,
        mode,
        lossless_format,
        ConflictStrategy::default(),
        FilenameRule::default(),
    )
}

pub fn build_sync_preview_with_settings(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
    conflict_strategy: ConflictStrategy,
    filename_rule: FilenameRule,
) -> io::Result<SyncPreview> {
    let mut preview = SyncPreview {
        source_directory: source_directory.to_string(),
        destination_directory: destination_directory.to_string(),
        new_count: 0,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        estimated_output_bytes: Some(0),
        candidates: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        available_space_bytes: None,
        disk_space_sufficient: None,
    };

    let source_path = Path::new(source_directory);
    if !source_path.exists() {
        preview.warnings.push(PreviewIssue {
            path: source_directory.to_string(),
            message: "输入来源不存在或不可读取".to_string(),
        });
        preview.estimated_output_bytes = None;
        return Ok(preview);
    }

    if source_path.is_file() && !is_supported_source_file(source_path) {
        preview.errors.push(PreviewIssue {
            path: source_directory.to_string(),
            message: "不支持的单曲格式；请选择 MP3、FLAC、NCM、WAV 或 AIFF 文件".to_string(),
        });
        preview.error_count = 1;
        preview.estimated_output_bytes = None;
        return Ok(preview);
    }

    if !source_path.is_dir() && !source_path.is_file() {
        preview.warnings.push(PreviewIssue {
            path: source_directory.to_string(),
            message: "输入来源不是文件夹或音频文件".to_string(),
        });
        preview.estimated_output_bytes = None;
        return Ok(preview);
    }

    if !destination_directory.trim().is_empty() {
        let destination_path = Path::new(destination_directory);
        if destination_path.exists() && !destination_path.is_dir() {
            preview.warnings.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: "输出路径不是文件夹".to_string(),
            });
        } else if !destination_path.exists()
            && destination_path
                .parent()
                .is_some_and(|parent| !parent.exists())
        {
            preview.warnings.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: "输出目录及其父目录不存在".to_string(),
            });
        }
    }

    let (source_files, scan_issues) =
        get_music_dict_with_scan_issues_with_rule(source_directory, filename_rule);
    for issue in scan_issues {
        preview.errors.push(PreviewIssue {
            path: issue.path.display().to_string(),
            message: issue.message,
        });
        preview.error_count += 1;
    }
    let destination_files = if Path::new(destination_directory).is_dir() {
        get_destination_music_dict_with_rule(destination_directory, filename_rule)
    } else {
        Default::default()
    };
    let mut occupied_paths = destination_files
        .values()
        .map(|(_, path)| path.clone())
        .collect::<HashSet<_>>();
    let mut source_entries = source_files.iter().collect::<Vec<_>>();
    source_entries.sort_by(|(left, _), (right, _)| left.cmp(right));

    for (name, (_, path)) in source_entries {
        let source_size = match fs::metadata(path) {
            Ok(metadata) if metadata.len() > 0 => metadata.len(),
            Ok(_) => {
                preview.errors.push(PreviewIssue {
                    path: path.display().to_string(),
                    message: "源文件为空，无法转换".to_string(),
                });
                preview.error_count += 1;
                continue;
            }
            Err(error) => {
                preview.errors.push(PreviewIssue {
                    path: path.display().to_string(),
                    message: format!("无法读取源文件：{error}"),
                });
                preview.error_count += 1;
                continue;
            }
        };

        let source_extension = effective_source_extension(path);
        let output_extension =
            resolve_output_policy(mode, lossless_format, &source_extension).output_extension;
        let expected_path = target_output_path(destination_directory, name, output_extension);
        let existing_path = destination_files
            .get(name)
            .filter(|(_, path)| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(output_extension))
            })
            .map(|(_, path)| path.clone())
            .or_else(|| expected_path.exists().then_some(expected_path.clone()));
        let has_existing = existing_path.is_some();
        let mut candidate_name = name.clone();
        let mut operation = CandidateOperation::Convert;

        if has_existing {
            preview.existing_count += 1;
            match conflict_strategy {
                ConflictStrategy::Skip => {
                    preview.skipped_count += 1;
                    continue;
                }
                ConflictStrategy::Overwrite => {}
                ConflictStrategy::Rename => {}
                ConflictStrategy::UpdateMetadata => {
                    if supports_metadata_update(
                        path,
                        existing_path.as_ref().expect("checked above"),
                    ) {
                        operation = CandidateOperation::UpdateMetadata;
                    } else {
                        preview.warnings.push(PreviewIssue {
                            path: path.display().to_string(),
                            message: "此格式暂不支持仅更新元数据；将保留现有输出文件".to_string(),
                        });
                        preview.skipped_count += 1;
                        continue;
                    }
                }
            }
        }

        if matches!(conflict_strategy, ConflictStrategy::Rename)
            && matches!(operation, CandidateOperation::Convert)
        {
            let desired_path =
                target_output_path(destination_directory, &candidate_name, output_extension);
            if has_existing || desired_path.exists() || occupied_paths.contains(&desired_path) {
                candidate_name = next_available_name(
                    destination_directory,
                    name,
                    output_extension,
                    &mut occupied_paths,
                );
            } else {
                occupied_paths.insert(desired_path);
            }
        }

        let estimated_bytes = if matches!(operation, CandidateOperation::UpdateMetadata) {
            0
        } else {
            source_size
        };
        let estimated_output_bytes = Some(estimated_bytes);
        let destination_path = if matches!(operation, CandidateOperation::UpdateMetadata) {
            existing_path.unwrap_or_else(|| {
                target_output_path(destination_directory, &candidate_name, output_extension)
            })
        } else {
            target_output_path(destination_directory, &candidate_name, output_extension)
        };
        if paths_refer_to_same_file(path, &destination_path) {
            preview.errors.push(PreviewIssue {
                path: path.display().to_string(),
                message: "输出文件与源文件相同；请选择其他输出目录，避免覆盖原曲".to_string(),
            });
            preview.error_count += 1;
            continue;
        }
        preview.candidates.push(PreviewCandidate {
            name: candidate_name.clone(),
            source_path: path.display().to_string(),
            destination_path: destination_path.display().to_string(),
            source_size_bytes: source_size,
            estimated_output_bytes,
            operation,
        });
        preview.new_count += 1;
        preview.estimated_output_bytes = preview
            .estimated_output_bytes
            .and_then(|total| total.checked_add(estimated_bytes));
    }

    let requires_ffmpeg = preview.candidates.iter().any(|candidate| {
        if matches!(candidate.operation, CandidateOperation::UpdateMetadata) {
            return false;
        }
        let extension = Path::new(&candidate.source_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_lowercase();
        !matches!(mode, Mode::Compat if extension == "mp3")
            && !matches!(mode, Mode::Lossless if extension == "mp3")
    });
    if requires_ffmpeg && find_ffmpeg().is_none() {
        preview.warnings.push(PreviewIssue {
            path: destination_directory.to_string(),
            message: "当前转换需要 FFmpeg，但未找到 FFmpeg".to_string(),
        });
    }

    preview.available_space_bytes = available_disk_space(Path::new(destination_directory));
    if let (Some(required), Some(available)) = (
        preview.estimated_output_bytes,
        preview.available_space_bytes,
    ) {
        let sufficient = available >= required;
        preview.disk_space_sufficient = Some(sufficient);
        if !sufficient {
            preview.warnings.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: format!(
                    "磁盘空间不足：预计需要 {} 字节，当前可用 {} 字节",
                    required, available
                ),
            });
        }
    }

    Ok(preview)
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

pub fn build_retry_preview(entry: &HistoryEntry) -> SyncPreview {
    let mut preview = SyncPreview {
        source_directory: entry.source_directory.clone(),
        destination_directory: entry.destination_directory.clone(),
        new_count: 0,
        existing_count: 0,
        skipped_count: 0,
        error_count: 0,
        estimated_output_bytes: Some(0),
        candidates: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        available_space_bytes: None,
        disk_space_sufficient: None,
    };

    for pending_file in &entry.pending_files {
        let source_path = Path::new(&pending_file.source_path);
        match fs::metadata(source_path) {
            Ok(metadata) if metadata.len() > 0 => {
                let estimated = pending_file.estimated_output_bytes.or(Some(metadata.len()));
                preview.estimated_output_bytes = match (preview.estimated_output_bytes, estimated) {
                    (Some(total), Some(value)) => total.checked_add(value),
                    _ => None,
                };
                preview.candidates.push(PreviewCandidate {
                    name: pending_file.name.clone(),
                    source_path: pending_file.source_path.clone(),
                    destination_path: pending_file.destination_path.clone(),
                    source_size_bytes: metadata.len(),
                    estimated_output_bytes: estimated,
                    operation: pending_file.operation,
                });
                preview.new_count += 1;
            }
            Ok(_) => {
                preview.errors.push(PreviewIssue {
                    path: pending_file.source_path.clone(),
                    message: "源文件为空，无法继续".to_string(),
                });
                preview.error_count += 1;
            }
            Err(error) => {
                preview.errors.push(PreviewIssue {
                    path: pending_file.source_path.clone(),
                    message: format!("继续任务时找不到源文件：{error}"),
                });
                preview.error_count += 1;
            }
        }
    }

    for failed_file in &entry.failed_files {
        let source_path = Path::new(&failed_file.source_path);
        match fs::metadata(source_path) {
            Ok(metadata) if metadata.len() > 0 => {
                let candidate = PreviewCandidate {
                    name: failed_file.name.clone(),
                    source_path: failed_file.source_path.clone(),
                    destination_path: failed_file.destination_path.clone(),
                    source_size_bytes: metadata.len(),
                    estimated_output_bytes: Some(metadata.len()),
                    operation: CandidateOperation::Convert,
                };
                preview.estimated_output_bytes = preview
                    .estimated_output_bytes
                    .and_then(|total| total.checked_add(metadata.len()));
                preview.candidates.push(candidate);
                preview.new_count += 1;
            }
            Ok(_) => {
                preview.errors.push(PreviewIssue {
                    path: failed_file.source_path.clone(),
                    message: "源文件为空，无法重试".to_string(),
                });
                preview.error_count += 1;
            }
            Err(error) => {
                preview.errors.push(PreviewIssue {
                    path: failed_file.source_path.clone(),
                    message: format!("重试时找不到源文件：{error}"),
                });
                preview.error_count += 1;
            }
        }
    }

    if preview.candidates.is_empty() && preview.errors.is_empty() {
        preview.estimated_output_bytes = None;
    }
    preview.available_space_bytes = available_disk_space(Path::new(&preview.destination_directory));
    if let (Some(required), Some(available)) = (
        preview.estimated_output_bytes,
        preview.available_space_bytes,
    ) {
        preview.disk_space_sufficient = Some(available >= required);
    }
    preview
}

fn next_available_name(
    destination_directory: &str,
    base_name: &str,
    extension: &str,
    occupied_paths: &mut HashSet<std::path::PathBuf>,
) -> String {
    let mut index = 2usize;
    loop {
        let candidate = format!("{} ({})", base_name, index);
        let path = target_output_path(destination_directory, &candidate, extension);
        if !path.exists() && !occupied_paths.contains(&path) {
            occupied_paths.insert(path);
            return candidate;
        }
        index += 1;
    }
}

fn supports_metadata_update(source: &Path, destination: &Path) -> bool {
    let extension = |path: &Path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_lowercase())
    };
    matches!(
        extension(source).as_deref(),
        Some("mp3") | Some("wav") | Some("aiff") | Some("flac") | Some("ncm")
    ) && matches!(
        extension(destination).as_deref(),
        Some("mp3") | Some("wav") | Some("aiff")
    )
}

#[cfg(unix)]
fn available_disk_space(path: &Path) -> Option<u64> {
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let probe = path.ancestors().find(|candidate| candidate.exists())?;
    let c_path = CString::new(probe.as_os_str().as_bytes()).ok()?;
    let mut stat = MaybeUninit::<libc::statvfs>::zeroed();
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    (stat.f_bavail as u64).checked_mul(stat.f_frsize)
}

#[cfg(target_os = "windows")]
fn available_disk_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory_name: *const u16,
            free_bytes_available: *mut u64,
            total_bytes: *mut u64,
            total_free_bytes: *mut u64,
        ) -> i32;
    }

    let probe = path.ancestors().find(|candidate| candidate.exists())?;
    let wide = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (result != 0).then_some(available)
}

#[cfg(not(any(unix, target_os = "windows")))]
fn available_disk_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::build_sync_preview;
    use crate::config::Mode;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn previews_a_single_supported_audio_file() {
        let source_dir = tempdir().unwrap();
        let destination_dir = tempdir().unwrap();
        let source_file = source_dir.path().join("single-track.mp3");
        fs::write(&source_file, b"not-empty-audio-placeholder").unwrap();

        let preview = build_sync_preview(
            source_file.to_str().unwrap(),
            destination_dir.path().to_str().unwrap(),
            Mode::Compat,
            None,
        )
        .unwrap();

        assert_eq!(preview.new_count, 1);
        assert_eq!(preview.error_count, 0);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(
            preview.candidates[0].source_path,
            source_file.display().to_string()
        );
    }
}
