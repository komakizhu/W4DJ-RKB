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
    #[serde(default)]
    pub logs: Vec<String>,
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
    report.push_str("W4DJ RKB 完整错误报告\n");
    report.push_str("报告格式版本：1\n\n");

    report.push_str("[软件与系统]\n");
    report.push_str(&format!("软件版本：{}\n", env!("CARGO_PKG_VERSION")));
    report.push_str(&format!(
        "构建类型：{}\n",
        if cfg!(debug_assertions) {
            "Debug"
        } else {
            "Release"
        }
    ));
    report.push_str(&format!("操作系统：{}\n", std::env::consts::OS));
    report.push_str(&format!("系统家族：{}\n", std::env::consts::FAMILY));
    report.push_str(&format!("CPU 架构：{}\n", std::env::consts::ARCH));
    report.push_str(&format!(
        "程序路径：{}\n",
        std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("无法读取（{error}）"))
    ));
    report.push_str(&format!(
        "FFmpeg 路径：{}\n\n",
        crate::sync::find_ffmpeg().unwrap_or_else(|| "未找到".to_string())
    ));
    report.push_str("隐私提醒：本报告包含完整本地路径和运行日志，仅在你主动发送时分享。\n\n");

    report.push_str("[任务信息]\n");
    report.push_str(&format!("任务 ID：{}\n", entry.id));
    report.push_str(&format!("批次 ID：{}\n", entry.batch_id));
    report.push_str(&format!("任务编号：{}\n", entry.slot_index + 1));
    report.push_str(&format!(
        "任务状态：{}\n",
        history_status_label(&entry.status)
    ));
    report.push_str(&format!("开始时间：{}\n", entry.started_at));
    report.push_str(&format!("结束时间：{}\n", entry.finished_at));
    report.push_str(&format!("运行时长：{} 秒\n", entry.duration_seconds));
    report.push_str(&format!(
        "重试来源：{}\n\n",
        entry.retry_of.as_deref().unwrap_or("无")
    ));

    report.push_str("[任务配置]\n");
    report.push_str(&format!("输出模式：{}\n", mode_label(entry.mode)));
    report.push_str(&format!(
        "无损格式：{}\n",
        lossless_format_label(entry.lossless_format)
    ));
    report.push_str(&format!(
        "冲突策略：{}\n",
        conflict_strategy_label(entry.conflict_strategy)
    ));
    report.push_str(&format!(
        "文件名规则：{}\n\n",
        filename_rule_label(entry.filename_rule)
    ));

    report.push_str("[路径]\n");
    report.push_str(&format!("输入来源：{}\n", entry.source_directory));
    report.push_str(&format!("输出目录：{}\n\n", entry.destination_directory));

    report.push_str("[统计]\n");
    report.push_str(&format!("新增文件：{}\n", entry.new_count));
    report.push_str(&format!("已存在文件：{}\n", entry.existing_count));
    report.push_str(&format!("跳过文件：{}\n", entry.skipped_count));
    report.push_str(&format!("错误文件：{}\n", entry.error_count));
    report.push_str(&format!("完成文件：{}\n", entry.completed_count));
    report.push_str(&format!("失败文件：{}\n", entry.failed_count));
    report.push_str(&format!("待处理文件：{}\n\n", entry.pending_files.len()));

    report.push_str("[失败文件详情]\n");
    if entry.failed_files.is_empty() {
        report.push_str("无\n\n");
    }

    for (index, failed_file) in entry.failed_files.iter().enumerate() {
        report.push_str(&format!(
            "{}. 歌曲：{}\n源文件：{}\n目标文件：{}\n错误类型：{}\n原因：{}\n\n",
            index + 1,
            failed_file.name,
            failed_file.source_path,
            failed_file.destination_path,
            failed_file.category.label(),
            failed_file.message
        ));
    }

    report.push_str("[待处理文件详情]\n");
    if entry.pending_files.is_empty() {
        report.push_str("无\n\n");
    }
    for (index, pending_file) in entry.pending_files.iter().enumerate() {
        report.push_str(&format!(
            "{}. 歌曲：{}\n源文件：{}\n目标文件：{}\n源文件大小：{} bytes\n预计输出大小：{}\n操作：{}\n\n",
            index + 1,
            pending_file.name,
            pending_file.source_path,
            pending_file.destination_path,
            pending_file.source_size_bytes,
            pending_file
                .estimated_output_bytes
                .map(|value| format!("{value} bytes"))
                .unwrap_or_else(|| "未知".to_string()),
            candidate_operation_label(pending_file.operation),
        ));
    }

    report.push_str("[运行日志]\n");
    if entry.logs.is_empty() {
        report.push_str("未记录\n");
    } else {
        for line in &entry.logs {
            report.push_str("- ");
            report.push_str(line);
            report.push('\n');
        }
    }

    report
}

fn history_status_label(status: &HistoryStatus) -> &'static str {
    match status {
        HistoryStatus::Completed => "已完成",
        HistoryStatus::Partial => "部分完成",
        HistoryStatus::Cancelled => "已取消",
        HistoryStatus::Error => "错误",
    }
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Compat => "兼容模式",
        Mode::Lossless => "无损模式",
    }
}

fn lossless_format_label(format: Option<LosslessFormat>) -> &'static str {
    match format {
        Some(LosslessFormat::Wav) => "WAV",
        Some(LosslessFormat::Aiff) => "AIFF",
        None => "不适用",
    }
}

fn conflict_strategy_label(strategy: ConflictStrategy) -> &'static str {
    match strategy {
        ConflictStrategy::Skip => "跳过",
        ConflictStrategy::Overwrite => "覆盖",
        ConflictStrategy::Rename => "自动重命名",
        ConflictStrategy::UpdateMetadata => "仅更新元数据",
    }
}

fn filename_rule_label(rule: FilenameRule) -> &'static str {
    match rule {
        FilenameRule::TitleArtist => "标题 - 艺术家",
        FilenameRule::ArtistTitle => "艺术家 - 标题",
        FilenameRule::Original => "保留原文件名",
    }
}

fn candidate_operation_label(operation: CandidateOperation) -> &'static str {
    match operation {
        CandidateOperation::Convert => "转换",
        CandidateOperation::UpdateMetadata => "更新元数据",
    }
}
