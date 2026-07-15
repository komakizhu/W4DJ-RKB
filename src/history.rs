use crate::config::{CandidateOperation, ConflictStrategy, FilenameRule, LosslessFormat, Mode};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

pub const MAX_HISTORY_ENTRIES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistoryStatus {
    Completed,
    Partial,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    FileDamaged,
    UnsupportedFormat,
    Ffmpeg,
    OutputPermission,
    DiskSpace,
    InvalidFilename,
    #[default]
    Unknown,
}

impl ErrorCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::FileDamaged => "文件损坏或无法读取",
            Self::UnsupportedFormat => "格式不支持",
            Self::Ffmpeg => "FFmpeg 转换失败",
            Self::OutputPermission => "输出目录无权限",
            Self::DiskSpace => "磁盘空间不足",
            Self::InvalidFilename => "文件名非法",
            Self::Unknown => "其他错误",
        }
    }
}

pub fn classify_error(message: &str) -> ErrorCategory {
    let value = message.to_lowercase();
    if value.contains("ffmpeg") {
        ErrorCategory::Ffmpeg
    } else if value.contains("no space") || value.contains("磁盘空间") || value.contains("空间不足")
    {
        ErrorCategory::DiskSpace
    } else if value.contains("permission denied")
        || value.contains("access is denied")
        || value.contains("无权限")
        || value.contains("权限")
    {
        ErrorCategory::OutputPermission
    } else if value.contains("invalid filename")
        || value.contains("illegal filename")
        || value.contains("filename too long")
        || value.contains("file name too long")
        || value.contains("文件名非法")
        || value.contains("文件名过长")
    {
        ErrorCategory::InvalidFilename
    } else if value.contains("unsupported") || value.contains("不支持") {
        ErrorCategory::UnsupportedFormat
    } else if value.contains("ncm")
        || value.contains("invalid data")
        || value.contains("源文件为空")
        || value.contains("无法读取")
        || value.contains("无法扫描")
        || value.contains("not found")
        || value.contains("不存在")
        || value.contains("decode")
        || value.contains("corrupt")
    {
        ErrorCategory::FileDamaged
    } else {
        ErrorCategory::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedFile {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub message: String,
    #[serde(default)]
    pub category: ErrorCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingFile {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub source_size_bytes: u64,
    pub estimated_output_bytes: Option<u64>,
    #[serde(default)]
    pub operation: CandidateOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: String,
    pub batch_id: String,
    pub slot_index: usize,
    pub started_at: String,
    pub finished_at: String,
    pub duration_seconds: u64,
    pub source_directory: String,
    pub destination_directory: String,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub new_count: usize,
    pub existing_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub failed_files: Vec<FailedFile>,
    #[serde(default)]
    pub pending_files: Vec<PendingFile>,
    pub status: HistoryStatus,
    pub retry_of: Option<String>,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
    #[serde(default)]
    pub filename_rule: FilenameRule,
}

pub fn load_history(path: impl AsRef<Path>) -> io::Result<Vec<HistoryEntry>> {
    let path = path.as_ref();
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut entries: Vec<HistoryEntry> = serde_json::from_str(&contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    entries.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    entries.truncate(MAX_HISTORY_ENTRIES);
    Ok(entries)
}

pub fn append_history(path: impl AsRef<Path>, entry: HistoryEntry) -> io::Result<()> {
    let path = path.as_ref();
    let mut entries = load_history(path).unwrap_or_default();
    entries.insert(0, entry);
    entries.truncate(MAX_HISTORY_ENTRIES);

    write_history(path, &entries)
}

pub fn upsert_history(path: impl AsRef<Path>, entry: HistoryEntry) -> io::Result<()> {
    let path = path.as_ref();
    let mut entries = load_history(path).unwrap_or_default();
    entries.retain(|existing| existing.id != entry.id);
    entries.insert(0, entry);
    entries.truncate(MAX_HISTORY_ENTRIES);
    write_history(path, &entries)
}

pub fn delete_history_entry(path: impl AsRef<Path>, id: &str) -> io::Result<bool> {
    let path = path.as_ref();
    let mut entries = load_history(path)?;
    let original_length = entries.len();
    entries.retain(|entry| entry.id != id);
    if entries.len() == original_length {
        return Ok(false);
    }
    write_history(path, &entries)?;
    Ok(true)
}

pub fn clear_history(path: impl AsRef<Path>) -> io::Result<()> {
    write_history(path.as_ref(), &[])
}

fn write_history(path: &Path, entries: &[HistoryEntry]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(entries)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, contents)?;
    fs::rename(temporary_path, path)
}

pub fn format_error_report(entry: &HistoryEntry) -> String {
    let mut report = String::new();
    report.push_str("W4DJ RKB 错误报告\n");
    report.push_str(&format!("任务时间：{}\n", entry.started_at));
    report.push_str(&format!("输入来源：{}\n", entry.source_directory));
    report.push_str(&format!("输出目录：{}\n", entry.destination_directory));
    report.push_str(&format!("失败数量：{}\n\n", entry.failed_count));

    for failed_file in &entry.failed_files {
        report.push_str(&format!(
            "歌曲：{}\n源文件：{}\n目标文件：{}\n错误类型：{}\n原因：{}\n\n",
            failed_file.name,
            failed_file.source_path,
            failed_file.destination_path,
            failed_file.category.label(),
            failed_file.message
        ));
    }

    report
}
