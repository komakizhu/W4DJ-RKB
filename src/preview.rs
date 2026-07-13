use crate::config::{LosslessFormat, Mode};
use crate::sync::{
    compare_music_dicts, effective_source_extension, find_ffmpeg, get_destination_music_dict,
    get_music_dict, resolve_output_policy, target_output_path,
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
        preview.error_count = preview.errors.len();
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

    let source_files = get_music_dict(source_directory);
    let destination_files = if Path::new(destination_directory).is_dir() {
        get_destination_music_dict(destination_directory)
    } else {
        Default::default()
    };
    let new_songs = compare_music_dicts(
        &source_files,
        &destination_files,
        &mode,
        lossless_format,
    );

    for (name, (_, path)) in &source_files {
        let source_size = match fs::metadata(path) {
            Ok(metadata) if metadata.len() > 0 => metadata.len(),
            Ok(_) => {
                preview.errors.push(PreviewIssue {
                    path: path.display().to_string(),
                    message: "源文件为空，无法转换".to_string(),
                });
                continue;
            }
            Err(error) => {
                preview.errors.push(PreviewIssue {
                    path: path.display().to_string(),
                    message: format!("无法读取源文件：{error}"),
                });
                continue;
            }
        };

        if !new_songs.contains_key(name) {
            preview.existing_count += 1;
            continue;
        }

        let source_extension = effective_source_extension(path);
        let output_extension = resolve_output_policy(mode, lossless_format, &source_extension)
            .output_extension;
        let estimated_output_bytes = Some(source_size);
        preview.candidates.push(PreviewCandidate {
            name: (*name).clone(),
            source_path: path.display().to_string(),
            destination_path: target_output_path(
                destination_directory,
                name,
                output_extension,
            )
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

    if preview
        .candidates
        .iter()
        .any(|candidate| candidate.destination_path.ends_with(".mp3") && find_ffmpeg().is_none())
    {
        preview.errors.push(PreviewIssue {
            path: destination_directory.to_string(),
            message: "当前转换需要 FFmpeg，但未找到 FFmpeg".to_string(),
        });
    }

    preview.error_count = preview.errors.len();
    Ok(preview)
}
