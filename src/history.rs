use crate::config::{LosslessFormat, Mode};
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailedFile {
    pub name: String,
    pub source_path: String,
    pub destination_path: String,
    pub message: String,
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
    pub status: HistoryStatus,
    pub retry_of: Option<String>,
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

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(&entries)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, contents)?;
    fs::rename(temporary_path, path)
}

pub fn format_error_report(entry: &HistoryEntry) -> String {
    let mut report = String::new();
    report.push_str("W4DJ RKB 错误报告\n");
    report.push_str(&format!("任务时间：{}\n", entry.started_at));
    report.push_str(&format!("源目录：{}\n", entry.source_directory));
    report.push_str(&format!("输出目录：{}\n", entry.destination_directory));
    report.push_str(&format!("失败数量：{}\n\n", entry.failed_count));

    for failed_file in &entry.failed_files {
        report.push_str(&format!(
            "歌曲：{}\n源文件：{}\n目标文件：{}\n原因：{}\n\n",
            failed_file.name,
            failed_file.source_path,
            failed_file.destination_path,
            failed_file.message
        ));
    }

    report
}
