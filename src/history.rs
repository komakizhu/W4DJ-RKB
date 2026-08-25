use crate::config::{
    CandidateOperation, ConflictStrategy, FilenameRule, LosslessFormat, Mode, NeteaseFilenameFormat,
};
use crate::sync::MetadataDiagnostic;
use serde::{Deserialize, Serialize};
use std::fmt::Write as FmtWrite;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_destination_path: Option<String>,
    #[serde(default)]
    pub operation: CandidateOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    pub metadata_diagnostics: Vec<MetadataDiagnostic>,
    #[serde(default)]
    pub logs: Vec<String>,
    pub status: HistoryStatus,
    pub retry_of: Option<String>,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
    #[serde(default)]
    pub filename_rule: FilenameRule,
    #[serde(default)]
    pub netease_filename_format: NeteaseFilenameFormat,
    #[serde(default)]
    pub report_path: Option<String>,
    #[serde(default)]
    pub analysis_reports: Vec<AnalysisReport>,
    /// Absolute path of the runtime-session directory created for this
    /// conversion. Older history entries omit it and are resolved by batch
    /// id when exported or displayed.
    #[serde(default)]
    pub runtime_session_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisReport {
    pub source_path: String,
    pub destination_path: String,
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub drop_status: Option<String>,
    #[serde(default)]
    pub drop_loudness_lufs: Option<String>,
    #[serde(default)]
    pub model_status: Option<String>,
    #[serde(default)]
    pub model_details: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    /// Distinguishes basic Essentia output from the strict whole-song
    /// completion state. All fields are optional for old history.json files.
    #[serde(default)]
    pub basic_status: Option<String>,
    #[serde(default)]
    pub basic_danceability: Option<f64>,
    #[serde(default)]
    pub discogs_danceability_status: Option<String>,
    #[serde(default)]
    pub discogs_danceability: Option<f64>,
    #[serde(default)]
    pub discogs_completed_heads: Option<usize>,
    #[serde(default)]
    pub discogs_total_heads: Option<usize>,
    #[serde(default)]
    pub cached: Option<bool>,
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
    let mut entries = load_history(path)?;
    entries.insert(0, entry);
    entries.truncate(MAX_HISTORY_ENTRIES);

    write_history(path, &entries)
}

pub fn upsert_history(path: impl AsRef<Path>, entry: HistoryEntry) -> io::Result<()> {
    let path = path.as_ref();
    let mut entries = load_history(path)?;
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

pub fn append_analysis_reports(
    path: impl AsRef<Path>,
    batch_id: &str,
    reports: Vec<AnalysisReport>,
) -> io::Result<bool> {
    if reports.is_empty() {
        return Ok(false);
    }

    let path = path.as_ref();
    let mut entries = load_history(path)?;
    let mut updated = false;
    for entry in &mut entries {
        if entry.batch_id != batch_id {
            continue;
        }
        for report in &reports {
            entry
                .analysis_reports
                .retain(|existing| existing.source_path != report.source_path);
            entry.analysis_reports.push(report.clone());
        }
        updated = true;
    }

    if updated {
        write_history(path, &entries)?;
    }
    Ok(updated)
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
    format_error_report_with_runtime(entry, None)
}

pub fn format_error_report_with_runtime(
    entry: &HistoryEntry,
    runtime_session: Option<&serde_json::Value>,
) -> String {
    let mut report = String::new();
    report.push_str("W4DJ RKB 转换报告\n");
    report.push_str("报告格式版本：2\n\n");

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

    report.push_str("[报告文件]\n");
    report.push_str(&format!(
        "导出保存位置：{}\n\n",
        entry.report_path.as_deref().unwrap_or("尚未手动导出")
    ));

    report.push_str("[统计]\n");
    report.push_str(&format!("新增文件：{}\n", entry.new_count));
    report.push_str(&format!("已存在文件：{}\n", entry.existing_count));
    report.push_str(&format!("跳过文件：{}\n", entry.skipped_count));
    report.push_str(&format!("错误文件：{}\n", entry.error_count));
    report.push_str(&format!("完成文件：{}\n", entry.completed_count));
    report.push_str(&format!("失败文件：{}\n", entry.failed_count));
    report.push_str(&format!(
        "待处理文件：{}\n转换待处理文件：{}\n\n",
        entry.pending_files.len(),
        entry.pending_files.len()
    ));

    report.push_str("[转换状态]\n");
    report.push_str(&format!(
        "转换：{}/{}\n转换失败：{}\n转换待处理：{}\n\n",
        entry.completed_count,
        entry.new_count,
        entry.failed_count,
        entry.pending_files.len()
    ));

    let analysis_completed = entry
        .analysis_reports
        .iter()
        .filter(|analysis| analysis.status == "completed")
        .count();
    let analysis_timed_out = entry
        .analysis_reports
        .iter()
        .filter(|analysis| analysis.status == "timeout")
        .count();
    let analysis_failed = entry
        .analysis_reports
        .iter()
        .filter(|analysis| analysis.status == "failed")
        .count();
    let analysis_cancelled = entry
        .analysis_reports
        .iter()
        .filter(|analysis| analysis.status == "cancelled")
        .count();
    let analysis_pending = entry
        .analysis_reports
        .iter()
        .filter(|analysis| analysis.status == "pending" || analysis.status == "running")
        .count();
    let basic_danceability_count = entry
        .analysis_reports
        .iter()
        .filter(|analysis| analysis.basic_danceability.is_some())
        .count();
    let discogs_danceability_completed = entry
        .analysis_reports
        .iter()
        .filter(|analysis| analysis.discogs_danceability_status.as_deref() == Some("completed"))
        .count();
    let discogs_danceability_failed = entry
        .analysis_reports
        .iter()
        .filter(|analysis| {
            matches!(
                analysis.discogs_danceability_status.as_deref(),
                Some("failed" | "model_missing" | "timeout" | "cancelled")
            )
        })
        .count();
    report.push_str("[增强分析总览]\n");
    report.push_str(&format!(
        "完整分析完成：{}\n失败：{}\n超时：{}\n取消：{}\n待处理/运行中：{}\n已记录逐曲结果：{}\n基础 Danceability 有值：{}\nDiscogs Danceability completed：{}\nDiscogs Danceability failed/missing/timeout/cancelled：{}\n\n",
        analysis_completed,
        analysis_failed,
        analysis_timed_out,
        analysis_cancelled,
        analysis_pending,
        entry.analysis_reports.len(),
        basic_danceability_count,
        discogs_danceability_completed,
        discogs_danceability_failed,
    ));
    if let Some(runtime_tracks) = runtime_analysis_tracks(runtime_session) {
        let runtime_completed = runtime_tracks
            .iter()
            .filter(|track| track.status == "completed")
            .count();
        let runtime_failed = runtime_tracks
            .iter()
            .filter(|track| track.status == "failed")
            .count();
        let runtime_timeout = runtime_tracks
            .iter()
            .filter(|track| track.status == "timeout")
            .count();
        let runtime_pending = runtime_tracks
            .iter()
            .filter(|track| track.status == "pending" || track.status == "running")
            .count();
        report.push_str(&format!(
            "运行会话逐曲总览：完成 {runtime_completed}/{}，失败 {runtime_failed}，超时 {runtime_timeout}，待处理 {runtime_pending}\n\n",
            runtime_tracks.len()
        ));
    }
    if let Some(runtime_state) = runtime_session
        .and_then(|value| value.get("runtimeSession"))
        .and_then(|value| value.get("files"))
        .and_then(|value| value.get("analysis-state.json"))
    {
        report.push_str("[增强分析状态快照]\n");
        for (label, key) in [
            ("状态", "status"),
            ("请求时间", "requestedAt"),
            ("开始时间", "startedAt"),
            ("结束时间", "finishedAt"),
            ("最后心跳", "lastHeartbeatAt"),
            ("当前歌曲", "currentItem"),
            ("当前阶段", "currentStage"),
            ("Worker", "workerJobId"),
            ("终止原因", "terminationReason"),
        ] {
            let value = runtime_state
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("无");
            report.push_str(&format!("{label}：{value}\n"));
        }
        if let Some(stage_processed) = runtime_state.get("stageProcessed") {
            report.push_str(&format!("当前阶段进度：{}", stage_processed));
            if let Some(stage_total) = runtime_state.get("stageTotal") {
                report.push_str(&format!("/{}", stage_total));
            }
            report.push('\n');
        }
        report.push('\n');
    }

    report.push_str("[增强分析逐曲状态]\n");
    if entry.analysis_reports.is_empty() {
        report.push_str("未记录\n\n");
    }
    for (index, analysis) in entry.analysis_reports.iter().enumerate() {
        report.push_str(&format!(
            "{}. 状态：{}\n源文件：{}\n目标文件：{}\n阶段：{}\n耗时：{}\n原因：{}\n\n",
            index + 1,
            analysis.status,
            analysis.source_path,
            analysis.destination_path,
            analysis.stage.as_deref().unwrap_or("无"),
            analysis
                .elapsed_ms
                .map(|value| format!("{value} ms"))
                .unwrap_or_else(|| "无".to_string()),
            analysis.message.as_deref().unwrap_or("无"),
        ));
    }
    if let Some(runtime_tracks) = runtime_analysis_tracks(runtime_session) {
        report.push_str("\n运行会话逐曲状态（包含未开始歌曲）：\n");
        for (index, track) in runtime_tracks.iter().enumerate() {
            report.push_str(&format!(
                "{}. 歌曲：{}\n状态：{}\n源文件：{}\n目标文件：{}\n开始时间：{}\n结束时间：{}\n阶段：{}\nWorker：{}\n耗时：{}\n缓存复用：{}\n终止原因：{}\n\n",
                index + 1,
                track.name,
                track.status,
                track.source_path,
                track.destination_path,
                if track.started_at.is_empty() { "无" } else { &track.started_at },
                if track.finished_at.is_empty() { "无" } else { &track.finished_at },
                track.stage,
                track.worker_job_id,
                track.elapsed_ms,
                track
                    .cached
                    .map(|value| if value { "是" } else { "否" })
                    .unwrap_or("未记录"),
                track.termination_reason,
            ));
        }
    }

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

    report.push_str("[逐曲元数据诊断]\n");
    if entry.metadata_diagnostics.is_empty() {
        report.push_str("未记录（旧版任务或尚未处理歌曲）\n\n");
    } else {
        let database_path = entry
            .metadata_diagnostics
            .iter()
            .filter_map(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .and_then(|recovery| recovery.database_path.as_deref())
            })
            .next()
            .unwrap_or("未加载");
        let database_loaded = entry.metadata_diagnostics.iter().any(|diagnostic| {
            diagnostic
                .netease_recovery
                .as_ref()
                .is_some_and(|recovery| recovery.database_loaded)
        });
        let record_count = entry
            .metadata_diagnostics
            .iter()
            .filter_map(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .map(|recovery| recovery.database_record_count)
            })
            .max()
            .unwrap_or_default();
        let matched = entry
            .metadata_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .is_some_and(|recovery| recovery.matched)
            })
            .count();
        let ambiguous = entry
            .metadata_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .and_then(|recovery| recovery.match_method)
                    == Some(crate::netease::NeteaseRecordMatchMethod::Ambiguous)
            })
            .count();
        let no_match = entry
            .metadata_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .and_then(|recovery| recovery.match_method)
                    == Some(crate::netease::NeteaseRecordMatchMethod::NoMatch)
            })
            .count();
        let local_cover = entry
            .metadata_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .is_some_and(|recovery| {
                        matches!(
                            recovery.cover_source,
                            Some(
                                crate::netease::NeteaseCoverSource::Embedded
                                    | crate::netease::NeteaseCoverSource::DatabaseBlob
                                    | crate::netease::NeteaseCoverSource::ExplicitLocalPath
                                    | crate::netease::NeteaseCoverSource::LocalCache
                            )
                        )
                    })
            })
            .count();
        let remote_only = entry
            .metadata_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .is_some_and(|recovery| {
                        recovery.cover_source
                            == Some(crate::netease::NeteaseCoverSource::RemoteOnly)
                    })
            })
            .count();
        let missing_or_invalid = entry
            .metadata_diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .netease_recovery
                    .as_ref()
                    .is_some_and(|recovery| {
                        matches!(
                            recovery.cover_source,
                            Some(
                                crate::netease::NeteaseCoverSource::Missing
                                    | crate::netease::NeteaseCoverSource::Invalid
                            )
                        )
                    })
            })
            .count();
        report.push_str("[网易云数据库与封面恢复]\n");
        report.push_str(&format!(
            "有效数据库路径：{}\n数据库已加载：{}\n加载记录数：{}\n已匹配：{}\n歧义：{}\n无匹配：{}\n本地封面成功：{}\n远程仅有 URL：{}\n缺失/无效：{}\n选择来源：{}\n\n",
            database_path,
            if database_loaded { "是" } else { "否" },
            record_count,
            matched,
            ambiguous,
            no_match,
            local_cover,
            remote_only,
            missing_or_invalid,
            if database_loaded { "自动定位或手动偏好" } else { "无有效数据库" },
        ));
    }

    report.push_str("[增强分析报告]\n");
    if entry.analysis_reports.is_empty() {
        report.push_str("未记录\n\n");
    }
    for (index, analysis) in entry.analysis_reports.iter().enumerate() {
        report.push_str(&format!(
            "{}. 源文件：{}\n目标文件：{}\n整首最终状态：{}\n基础分析状态：{}\n基础 Danceability：{}\nDiscogs Danceability 状态：{}\nDiscogs Danceability：{}\nDiscogs head 完成：{}/{}\n阶段：{}\n耗时：{}\n原因：{}\nDrop 状态：{}\nDrop LUFS：{}\n模型状态：{}\n模型详情：{}\n缓存复用：{}\n\n",
            index + 1,
            analysis.source_path,
            analysis.destination_path,
            analysis.status,
            analysis.basic_status.as_deref().unwrap_or("旧版未记录"),
            analysis
                .basic_danceability
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "无".to_string()),
            analysis
                .discogs_danceability_status
                .as_deref()
                .unwrap_or("旧版未记录"),
            analysis
                .discogs_danceability
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "无".to_string()),
            analysis
                .discogs_completed_heads
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string()),
            analysis
                .discogs_total_heads
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string()),
            analysis.stage.as_deref().unwrap_or("无"),
            analysis
                .elapsed_ms
                .map(|value| format!("{value} ms"))
                .unwrap_or_else(|| "无".to_string()),
            analysis.message.as_deref().unwrap_or("无"),
            analysis.drop_status.as_deref().unwrap_or("无"),
            analysis.drop_loudness_lufs.as_deref().unwrap_or("无"),
            analysis.model_status.as_deref().unwrap_or("无"),
            analysis.model_details.as_deref().unwrap_or("无"),
            analysis
                .cached
                .map(|value| if value { "是" } else { "否" })
                .unwrap_or("旧版未记录"),
        ));
    }

    report.push_str("[Discogs-EffNet 逐 head 状态]\n");
    let mut discogs_head_count = 0usize;
    for analysis in &entry.analysis_reports {
        let Some(details) = analysis
            .model_details
            .as_deref()
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        else {
            continue;
        };
        let Some(heads) = details
            .get("discogsEffnet")
            .and_then(|value| value.get("heads"))
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        for (head_id, head) in heads {
            discogs_head_count += 1;
            let labels = head
                .get("labels")
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| {
                            let label = value.get("label")?.as_str()?;
                            let confidence = value
                                .get("confidence")
                                .and_then(serde_json::Value::as_f64)
                                .map(|value| format!(" {:.1}%", value * 100.0))
                                .unwrap_or_default();
                            Some(format!("{label}{confidence}"))
                        })
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    head.get("selectedClass")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("无")
                        .to_string()
                });
            let selected_confidence = head
                .get("selectedConfidence")
                .and_then(serde_json::Value::as_f64)
                .map(|value| format!("{:.1}%", value * 100.0))
                .unwrap_or_else(|| "无".to_string());
            let _ = writeln!(
                report,
                "{}. 歌曲：{}\nHead：{}\n状态：{}\n版本：{}\n标签/类别：{}\n选中置信度：{}\n帧数：{}\n原因：{}\n耗时：{}\n",
                discogs_head_count,
                analysis.destination_path,
                head_id,
                head.get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown"),
                head.get("version")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("无"),
                labels,
                selected_confidence,
                head.get("frameCount")
                    .and_then(serde_json::Value::as_u64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "无".to_string()),
                head.get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("无"),
                analysis
                    .elapsed_ms
                    .map(|value| format!("{value} ms"))
                    .unwrap_or_else(|| "无".to_string()),
            );
        }
    }
    if discogs_head_count == 0 {
        report.push_str("未记录（该任务没有 Discogs-EffNet head 结果）\n");
    }
    report.push('\n');

    for (index, diagnostic) in entry.metadata_diagnostics.iter().enumerate() {
        report.push_str(&format!(
            "{}. 源文件：{}\n目标文件：{}\n源文件名：{}\n源格式：{}\n源大小：{}\n输出大小：{}\n源标题：{}\n源歌手：{}\n源专辑：{}\n输出标题：{}\n输出歌手：{}\n输出专辑：{}\n文件名判断：{}\n识别结论：{}\n校验依据：{}\n输出标签实际匹配：{}\n源封面：{}\n输出封面：{}\n网易云匹配方式：{}\n网易云封面来源：{}\n网易云曲目 ID：{}\n网易云专辑 ID：{}\n网易云终止原因：{}\n最终校验：{}\n\n",
            index + 1,
            diagnostic.source_path,
            diagnostic.destination_path,
            diagnostic.source_filename,
            diagnostic.source_extension,
            diagnostic.source_size_bytes.map(|value| format!("{value} bytes")).unwrap_or_else(|| "无法读取".to_string()),
            diagnostic.output_size_bytes.map(|value| format!("{value} bytes")).unwrap_or_else(|| "不存在或无法读取".to_string()),
            diagnostic.source_title.as_deref().unwrap_or("无"),
            diagnostic.source_artist.as_deref().unwrap_or("无"),
            diagnostic.source_album.as_deref().unwrap_or("无"),
            diagnostic.output_title.as_deref().unwrap_or("无"),
            diagnostic.output_artist.as_deref().unwrap_or("无"),
            diagnostic.output_album.as_deref().unwrap_or("无"),
            diagnostic.detected_filename_layout,
            diagnostic.decision,
            diagnostic.validation_basis.as_deref().unwrap_or("旧版未记录"),
            diagnostic
                .output_tags_match
                .map(|value| if value { "是" } else { "否" })
                .unwrap_or("无法读取"),
            if diagnostic.source_artwork { "有（有效图片）" } else { "无或无效" },
            match diagnostic.output_artwork { Some(true) => "有（有效图片）", Some(false) => "无或无效", None => "无法读取" },
            diagnostic
                .netease_recovery
                .as_ref()
                .and_then(|recovery| recovery.match_method.map(|method| serde_enum_label(&method)))
                .as_deref()
                .unwrap_or("未记录"),
            diagnostic
                .netease_recovery
                .as_ref()
                .and_then(|recovery| recovery.cover_source.map(|source| serde_enum_label(&source)))
                .as_deref()
                .unwrap_or("未记录"),
            diagnostic
                .netease_recovery
                .as_ref()
                .and_then(|recovery| recovery.track_id.as_deref())
                .unwrap_or("无"),
            diagnostic
                .netease_recovery
                .as_ref()
                .and_then(|recovery| recovery.album_id.as_deref())
                .unwrap_or("无"),
            diagnostic
                .netease_recovery
                .as_ref()
                .and_then(|recovery| recovery.message.as_deref())
                .unwrap_or("无"),
            diagnostic.metadata_validation,
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

    if let Some(runtime_session) = runtime_session {
        report.push_str("\n[运行会话]\n");
        if let Some(summary) = runtime_session.get("readableSummary") {
            report.push_str(&format!("摘要：{}\n", summary));
        }
        if let Some(state) = runtime_session
            .get("runtimeSession")
            .and_then(|session| session.get("files"))
            .and_then(|files| files.get("analysis-state.json"))
        {
            report.push_str("[增强分析状态快照]\n");
            for key in [
                "status",
                "total",
                "completed",
                "failed",
                "timedOut",
                "pending",
                "currentItem",
                "currentStage",
                "workerJobId",
                "lastHeartbeatAt",
                "terminationReason",
            ] {
                if let Some(value) = state.get(key) {
                    report.push_str(&format!("{}：{}\n", key, value));
                }
            }
            report.push('\n');
        }
        if let Some(events) = runtime_session
            .get("runtimeSession")
            .and_then(|session| session.get("files"))
            .and_then(|files| files.get("events.jsonl"))
            .and_then(serde_json::Value::as_array)
        {
            for event in events {
                report.push_str("- ");
                report.push_str(&event.to_string());
                report.push('\n');
            }
        } else {
            report.push_str("未找到运行会话事件\n");
        }
    }

    report
}

#[derive(Debug, Default)]
struct RuntimeAnalysisTrack {
    name: String,
    source_path: String,
    destination_path: String,
    status: String,
    stage: String,
    worker_job_id: String,
    started_at: String,
    finished_at: String,
    elapsed_ms: String,
    termination_reason: String,
    cached: Option<bool>,
}

fn runtime_analysis_tracks(
    runtime_session: Option<&serde_json::Value>,
) -> Option<Vec<RuntimeAnalysisTrack>> {
    let files = runtime_session?
        .get("runtimeSession")
        .and_then(|session| session.get("files"))?;
    let mut tracks = std::collections::BTreeMap::<String, RuntimeAnalysisTrack>::new();
    if let Some(slots) = files
        .get("candidates.json")
        .and_then(serde_json::Value::as_array)
    {
        for slot in slots {
            let Some(candidates) = slot
                .get("preview")
                .and_then(|preview| preview.get("candidates"))
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            for candidate in candidates {
                let Some(source_path) = candidate
                    .get("source_path")
                    .and_then(serde_json::Value::as_str)
                else {
                    continue;
                };
                tracks.insert(
                    source_path.to_string(),
                    RuntimeAnalysisTrack {
                        name: candidate
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(source_path)
                            .to_string(),
                        source_path: source_path.to_string(),
                        destination_path: candidate
                            .get("destination_path")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        status: String::from("pending"),
                        stage: String::from("pending"),
                        ..RuntimeAnalysisTrack::default()
                    },
                );
            }
        }
    }
    if let Some(reports) = files
        .get("analysis-reports.json")
        .and_then(serde_json::Value::as_array)
    {
        for report in reports {
            let Some(source_path) = report
                .get("source_path")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let track =
                tracks
                    .entry(source_path.to_string())
                    .or_insert_with(|| RuntimeAnalysisTrack {
                        source_path: source_path.to_string(),
                        ..RuntimeAnalysisTrack::default()
                    });
            track.status = report
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("failed")
                .to_string();
            if let Some(stage) = report.get("stage").and_then(serde_json::Value::as_str) {
                track.stage = stage.to_string();
            }
            if let Some(elapsed_ms) = report.get("elapsed_ms") {
                track.elapsed_ms = elapsed_ms.to_string();
            }
            if let Some(message) = report.get("message").and_then(serde_json::Value::as_str) {
                track.termination_reason = message.to_string();
            }
        }
    }
    let analysis_state = files.get("analysis-state.json");
    let state_interrupted = analysis_state
        .and_then(|state| state.get("status"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| status == "interrupted")
        || analysis_state
            .and_then(|state| state.get("lastHeartbeatEpochMs"))
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|heartbeat| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or_default();
                now.saturating_sub(heartbeat) > 15_000
                    && analysis_state
                        .and_then(|state| state.get("status"))
                        .and_then(serde_json::Value::as_str)
                        == Some("running")
            });
    if let Some(state_tracks) = analysis_state
        .and_then(|state| state.get("tracks"))
        .and_then(serde_json::Value::as_object)
    {
        for (source_path, state_track) in state_tracks {
            let track = tracks
                .entry(source_path.clone())
                .or_insert_with(|| RuntimeAnalysisTrack {
                    source_path: source_path.clone(),
                    ..RuntimeAnalysisTrack::default()
                });
            if let Some(name) = state_track.get("name").and_then(serde_json::Value::as_str) {
                track.name = name.to_string();
            }
            if let Some(destination_path) = state_track
                .get("destinationPath")
                .and_then(serde_json::Value::as_str)
            {
                track.destination_path = destination_path.to_string();
            }
            if let Some(status) = state_track
                .get("status")
                .and_then(serde_json::Value::as_str)
            {
                track.status = status.to_string();
            }
            if let Some(stage) = state_track.get("stage").and_then(serde_json::Value::as_str) {
                track.stage = stage.to_string();
            }
            if let Some(worker_job_id) = state_track
                .get("workerJobId")
                .and_then(serde_json::Value::as_str)
            {
                track.worker_job_id = worker_job_id.to_string();
            }
            if let Some(started_at) = state_track
                .get("startedAt")
                .and_then(serde_json::Value::as_str)
            {
                track.started_at = started_at.to_string();
            }
            if let Some(finished_at) = state_track
                .get("finishedAt")
                .and_then(serde_json::Value::as_str)
            {
                track.finished_at = finished_at.to_string();
            }
            if let Some(elapsed_ms) = state_track.get("elapsedMs") {
                track.elapsed_ms = elapsed_ms.to_string();
            }
            if let Some(reason) = state_track
                .get("terminationReason")
                .and_then(serde_json::Value::as_str)
            {
                track.termination_reason = reason.to_string();
            }
        }
    }
    if let Some(events) = files
        .get("events.jsonl")
        .and_then(serde_json::Value::as_array)
    {
        let mut cancelled = false;
        let mut failed = false;
        let mut terminal_reason = String::new();
        for event in events {
            let Some(event_name) = event.get("event").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let details = event.get("details").unwrap_or(&serde_json::Value::Null);
            if event_name == "analysis_cancelled" {
                cancelled = true;
                let cancelled_at = event
                    .get("at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                for track in tracks.values_mut() {
                    if track.status == "pending" || track.status == "running" {
                        track.finished_at = cancelled_at.clone();
                    }
                }
                continue;
            }
            if event_name == "analysis_error" {
                failed = true;
                let failed_at = event
                    .get("at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                terminal_reason = details
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("分析任务错误")
                    .to_string();
                for track in tracks.values_mut() {
                    if track.status == "pending" || track.status == "running" {
                        track.finished_at = failed_at.clone();
                    }
                }
                continue;
            }
            let Some(source_path) = details
                .get("source_path")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let track =
                tracks
                    .entry(source_path.to_string())
                    .or_insert_with(|| RuntimeAnalysisTrack {
                        source_path: source_path.to_string(),
                        ..RuntimeAnalysisTrack::default()
                    });
            if let Some(name) = details.get("name").and_then(serde_json::Value::as_str) {
                track.name = name.to_string();
            }
            if let Some(destination_path) = details
                .get("destination_path")
                .and_then(serde_json::Value::as_str)
            {
                track.destination_path = destination_path.to_string();
            }
            if let Some(worker_job_id) = details
                .get("worker_job_id")
                .and_then(serde_json::Value::as_str)
            {
                track.worker_job_id = worker_job_id.to_string();
            }
            match event_name {
                "analysis_candidate_started" => {
                    track.status = String::from("running");
                    track.stage = String::from("preparing");
                    track.started_at = event
                        .get("at")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                "analysis_candidate_progress" => {
                    track.status = String::from("running");
                    if track.started_at.is_empty() {
                        track.started_at = event
                            .get("at")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    if let Some(stage) = details.get("stage").and_then(serde_json::Value::as_str) {
                        track.stage = stage.to_string();
                    }
                }
                "analysis_candidate_finished" => {
                    track.finished_at = event
                        .get("at")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(status) = details.get("status").and_then(serde_json::Value::as_str)
                    {
                        track.status = status.to_string();
                    }
                    if let Some(stage) = details.get("stage").and_then(serde_json::Value::as_str) {
                        track.stage = stage.to_string();
                    }
                    if let Some(elapsed_ms) = details.get("elapsed_ms") {
                        track.elapsed_ms = elapsed_ms.to_string();
                    }
                    if let Some(error) = details.get("error").and_then(serde_json::Value::as_str) {
                        track.termination_reason = error.to_string();
                    }
                    if let Some(cached) = details.get("cached").and_then(serde_json::Value::as_bool)
                    {
                        track.cached = Some(cached);
                    }
                }
                "analysis_candidate_persisted" => {
                    track.status = details
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("completed")
                        .to_string();
                    if track.finished_at.is_empty() {
                        track.finished_at = event
                            .get("at")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                }
                _ => {}
            }
        }
        if cancelled {
            for track in tracks.values_mut() {
                if track.status == "pending" || track.status == "running" {
                    track.status = String::from("cancelled");
                    track.stage = String::from("cancelled");
                }
            }
        }
        if failed {
            for track in tracks.values_mut() {
                if track.status == "pending" || track.status == "running" {
                    track.status = String::from("failed");
                    track.stage = String::from("error");
                    track.termination_reason = terminal_reason.clone();
                }
            }
        }
    }
    if state_interrupted {
        for track in tracks.values_mut() {
            if track.status == "pending" || track.status == "running" {
                track.status = String::from("interrupted");
                if track.stage == "pending" || track.stage.is_empty() {
                    track.stage = String::from("interrupted");
                }
                if track.termination_reason.is_empty() {
                    track.termination_reason = String::from("运行会话心跳已过期");
                }
            }
        }
    }
    if tracks.is_empty() {
        None
    } else {
        Some(tracks.into_values().collect())
    }
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

fn serde_enum_label<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| String::from("未记录"))
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Mode, NeteaseFilenameFormat};

    fn test_entry() -> HistoryEntry {
        HistoryEntry {
            id: String::from("history-1"),
            batch_id: String::from("batch-1"),
            slot_index: 0,
            started_at: String::from("2026-08-06 12:00"),
            finished_at: String::from("2026-08-06 12:01"),
            duration_seconds: 60,
            source_directory: String::from("/music/in"),
            destination_directory: String::from("/music/out"),
            mode: Mode::Compat,
            lossless_format: None,
            new_count: 1,
            existing_count: 0,
            skipped_count: 0,
            error_count: 0,
            completed_count: 1,
            failed_count: 0,
            failed_files: Vec::new(),
            pending_files: Vec::new(),
            metadata_diagnostics: Vec::new(),
            logs: Vec::new(),
            status: HistoryStatus::Completed,
            retry_of: None,
            conflict_strategy: ConflictStrategy::default(),
            filename_rule: FilenameRule::default(),
            netease_filename_format: NeteaseFilenameFormat::default(),
            report_path: None,
            analysis_reports: Vec::new(),
            runtime_session_dir: None,
        }
    }

    #[test]
    fn corrupted_history_is_not_replaced_by_append_or_upsert() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("history.json");
        let original = b"{ this is not valid history }";
        fs::write(&path, original).expect("corrupt history should be written");

        let append_error = append_history(&path, test_entry()).expect_err("append should fail");
        assert_eq!(append_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            fs::read(&path).expect("history should remain readable"),
            original
        );

        let upsert_error = upsert_history(&path, test_entry()).expect_err("upsert should fail");
        assert_eq!(upsert_error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            fs::read(&path).expect("history should remain readable"),
            original
        );
    }

    #[test]
    fn history_defaults_legacy_netease_format_and_roundtrips_selected_format() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("history.json");

        let mut legacy = serde_json::to_value(test_entry()).expect("entry should serialize");
        legacy
            .as_object_mut()
            .expect("history entry should be an object")
            .remove("netease_filename_format");
        fs::write(&path, serde_json::to_vec(&vec![legacy]).unwrap()).unwrap();

        let loaded_legacy = load_history(&path).expect("legacy history should load");
        assert_eq!(
            loaded_legacy[0].netease_filename_format,
            NeteaseFilenameFormat::default()
        );

        let mut selected = test_entry();
        selected.netease_filename_format = NeteaseFilenameFormat::ArtistTitle;
        append_history(&path, selected).expect("selected format should be saved");

        let loaded = load_history(&path).expect("history should reload");
        assert_eq!(
            loaded[0].netease_filename_format,
            NeteaseFilenameFormat::ArtistTitle
        );
    }
}
