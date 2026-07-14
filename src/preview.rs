use crate::config::{LosslessFormat, Mode};
use crate::history::HistoryEntry;
use crate::sync::{
    compare_music_dicts, effective_source_extension, find_ffmpeg, get_destination_music_dict,
    get_music_dict_with_scan_issues, resolve_output_policy, target_output_path,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreviewCandidate {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub source_size_bytes: u64,
    pub estimated_output_bytes: Option<u64>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlotPreview {
    pub slot_index: usize,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub preview: SyncPreview,
    pub retry_of: Option<String>,
}

pub fn build_sync_preview(
    source_directory: &str,
    destination_directory: &str,
    mode: Mode,
    lossless_format: Option<LosslessFormat>,
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
    };

    let source_path = Path::new(source_directory);
    if !source_path.is_dir() {
        preview.errors.push(PreviewIssue {
            path: source_directory.to_string(),
            message: "歌曲下载目录不存在或不可读取".to_string(),
        });
        preview.estimated_output_bytes = None;
        return Ok(preview);
    }

    if !destination_directory.trim().is_empty() {
        let destination_path = Path::new(destination_directory);
        if destination_path.exists() && !destination_path.is_dir() {
            preview.errors.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: "输出路径不是文件夹".to_string(),
            });
        } else if !destination_path.exists()
            && destination_path
                .parent()
                .is_some_and(|parent| !parent.exists())
        {
            preview.errors.push(PreviewIssue {
                path: destination_directory.to_string(),
                message: "输出目录及其父目录不存在".to_string(),
            });
        }
    }

    let (source_files, scan_issues) = get_music_dict_with_scan_issues(source_directory);
    for issue in scan_issues {
        preview.errors.push(PreviewIssue {
            path: issue.path.display().to_string(),
            message: issue.message,
        });
        preview.error_count += 1;
    }
    let destination_files = if Path::new(destination_directory).is_dir() {
        get_destination_music_dict(destination_directory)
    } else {
        Default::default()
    };
    let new_songs = compare_music_dicts(&source_files, &destination_files, &mode, lossless_format);

    for (name, (_, path)) in &source_files {
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

        if !new_songs.contains_key(name) {
            preview.existing_count += 1;
            preview.skipped_count += 1;
            continue;
        }

        let source_extension = effective_source_extension(path);
        let output_extension =
            resolve_output_policy(mode, lossless_format, &source_extension).output_extension;
        let estimated_output_bytes = Some(source_size);
        preview.candidates.push(PreviewCandidate {
            name: (*name).clone(),
            source_path: path.display().to_string(),
            destination_path: target_output_path(destination_directory, name, output_extension)
                .display()
                .to_string(),
            source_size_bytes: source_size,
            estimated_output_bytes,
        });
        preview.new_count += 1;
        preview.estimated_output_bytes = preview
            .estimated_output_bytes
            .and_then(|total| total.checked_add(source_size));
    }

    let requires_ffmpeg = preview.candidates.iter().any(|candidate| {
        let extension = Path::new(&candidate.source_path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_lowercase();
        !matches!(mode, Mode::Compat if extension == "mp3")
            && !matches!(mode, Mode::Lossless if extension == "mp3")
    });
    if requires_ffmpeg && find_ffmpeg().is_none() {
        preview.errors.push(PreviewIssue {
            path: destination_directory.to_string(),
            message: "当前转换需要 FFmpeg，但未找到 FFmpeg".to_string(),
        });
    }

    Ok(preview)
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
    };

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
    preview
}
