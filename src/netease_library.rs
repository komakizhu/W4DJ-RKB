//! Read-only NetEase import and normalization for the W4DJ library index.

use crate::library_catalog::{
    CatalogLocalFile, CatalogSnapshot, CatalogSource, CatalogSourceRecord, CatalogTrack,
    LibraryCatalog, LocalStatus,
};
use crate::media_probe::probe_local_audio;
use crate::netease::{
    NeteaseDatabaseSummary, NeteaseRecord, choose_record, database_candidates,
    load_records_from_db, load_records_from_db_observed, probe_netease_database,
};
use rusqlite::{Connection, OpenFlags};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogBuildError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for CatalogBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("歌曲库刷新已取消"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CatalogBuildError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogBuildProgress {
    pub stage: &'static str,
    pub processed: usize,
    pub total: Option<usize>,
    pub current_item: String,
}

/// Progress emitted while locating a local NetEase library.  Discovery is
/// deliberately a small, typed value so the desktop layer can expose it
/// without leaking absolute paths to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeteaseDiscoveryProgress {
    pub stage: &'static str,
    pub processed: usize,
    pub total: Option<usize>,
    pub current_item: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseDiscovery {
    pub database_path: Option<PathBuf>,
    pub music_folder: Option<PathBuf>,
    pub record_count: usize,
    pub local_file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeteasePathLookupError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for NeteasePathLookupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("网易云目录发现已取消"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for NeteasePathLookupError {}

/// Resolve the conventional NetEase music directory without opening a
/// database.  This is intentionally the first discovery step so a known local
/// folder can be returned to the UI immediately.
pub fn known_netease_music_folder() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("W4DJ_NETEASE_MUSIC_DIR").map(PathBuf::from)
        && path.is_dir()
    {
        return Some(path);
    }

    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    [
        home.join("Music/网易云音乐"),
        home.join("Music/NetEase CloudMusic"),
        home.join("Music/Netease Cloud Music"),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

/// Read only the path-bearing columns from supported NetEase tables and stop
/// as soon as an existing music root is found. No record JSON, lyrics, cover,
/// or complete metadata row is materialized.
pub fn derive_netease_music_folder_from_database_observed<Cancel, Observe>(
    database_path: &Path,
    mut is_cancelled: Cancel,
    mut observe: Observe,
) -> Result<Option<PathBuf>, NeteasePathLookupError>
where
    Cancel: FnMut() -> bool,
    Observe: FnMut(usize, usize),
{
    let connection = Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| {
        NeteasePathLookupError::Failed(format!("无法只读打开网易云数据库：{error}"))
    })?;
    let mut tables = Vec::new();
    for table in ["track", "web_track", "web_offline_track", "web_cloud_track"] {
        if is_cancelled() {
            return Err(NeteasePathLookupError::Cancelled);
        }
        let exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                NeteasePathLookupError::Failed(format!("无法读取网易云数据库表结构：{error}"))
            })?
            != 0;
        if exists {
            tables.push(table);
        }
    }

    let mut processed = 0usize;
    for table in tables {
        let columns = connection
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(1))
                    .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            })
            .map_err(|error| {
                NeteasePathLookupError::Failed(format!("无法读取网易云路径字段：{error}"))
            })?;
        let has = |name: &str| {
            columns
                .iter()
                .any(|column| column.eq_ignore_ascii_case(name))
        };
        let path_column = ["file", "librarypath", "relative_path"]
            .into_iter()
            .find(|name| has(name));
        let directory_column = ["dir", "parentdir"].into_iter().find(|name| has(name));
        let Some(path_column) = path_column else {
            continue;
        };
        let path_expr = format!("COALESCE(\"{path_column}\", '')");
        let directory_expr = directory_column
            .map(|column| format!("COALESCE(\"{column}\", '')"))
            .unwrap_or_else(|| "''".to_string());
        let total = connection
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or_default()
            .max(0) as usize;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {path_expr}, {directory_expr} FROM \"{table}\""
            ))
            .map_err(|error| {
                NeteasePathLookupError::Failed(format!("无法读取网易云目录字段：{error}"))
            })?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| {
                NeteasePathLookupError::Failed(format!("无法遍历网易云目录字段：{error}"))
            })?;
        for row in rows {
            if is_cancelled() {
                return Err(NeteasePathLookupError::Cancelled);
            }
            let (path, directory) = row.map_err(|error| {
                NeteasePathLookupError::Failed(format!("无法读取网易云目录字段：{error}"))
            })?;
            processed += 1;
            observe(processed, total);
            if let Some(root) = existing_netease_root(&path, &directory) {
                return Ok(Some(root));
            }
        }
    }
    Ok(None)
}

fn existing_netease_root(path_value: &str, directory: &str) -> Option<PathBuf> {
    let combined = if path_value.trim().is_empty() {
        directory.to_string()
    } else if directory.trim().is_empty() {
        path_value.to_string()
    } else {
        let path = Path::new(path_value);
        if path.is_absolute() {
            path_value.to_string()
        } else {
            Path::new(directory)
                .join(path)
                .to_string_lossy()
                .into_owned()
        }
    };
    let mut candidate = PathBuf::from(combined.replace('\\', "/"));
    if candidate.is_file() {
        candidate.pop();
    }
    let components = candidate.components().collect::<Vec<_>>();
    components
        .iter()
        .rposition(|component| {
            let name = component.as_os_str().to_string_lossy();
            name == "网易云音乐"
                || name.eq_ignore_ascii_case("NetEase CloudMusic")
                || name.eq_ignore_ascii_case("Netease Cloud Music")
        })
        .map(|index| components[..=index].iter().collect::<PathBuf>())
        .filter(|root| root.is_dir())
        .or_else(|| candidate.is_dir().then_some(candidate))
}

pub fn discover_netease_library() -> NeteaseDiscovery {
    discover_netease_library_with_local_count(true)
}

pub fn discover_netease_library_for_refresh() -> NeteaseDiscovery {
    discover_netease_library_with_local_count(false)
}

fn discover_netease_library_with_local_count(include_local_file_count: bool) -> NeteaseDiscovery {
    discover_netease_library_observed(include_local_file_count, |_| {})
}

fn load_records_for_summary_observed<Observe>(
    summary: &NeteaseDatabaseSummary,
    mut observe: Observe,
) -> Result<Vec<NeteaseRecord>, String>
where
    Observe: FnMut(NeteaseDiscoveryProgress),
{
    load_records_from_db_observed(&summary.path, 2, |table, processed, total| {
        observe(NeteaseDiscoveryProgress {
            stage: "readingRecords",
            processed,
            total: Some(total),
            current_item: table.to_string(),
            message: "正在读取网易云数据库".to_string(),
        });
    })
    .map_err(|error| format!("无法只读打开网易云数据库：{error}"))
}

fn probe_database_candidates(candidates: &[PathBuf]) -> Vec<NeteaseDatabaseSummary> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let worker_count = 2usize.min(candidates.len()).max(1);
    let queue = Arc::new(Mutex::new(
        candidates.iter().cloned().collect::<VecDeque<_>>(),
    ));
    let (sender, receiver) = mpsc::channel();
    let mut workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let sender = sender.clone();
        workers.push(thread::spawn(move || {
            loop {
                let path = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                let Some(path) = path else {
                    break;
                };
                if let Ok(summary) = probe_netease_database(&path) {
                    let _ = sender.send(summary);
                }
            }
        }));
    }
    drop(sender);
    let summaries = receiver.into_iter().collect::<Vec<_>>();
    for worker in workers {
        let _ = worker.join();
    }
    summaries
}

pub fn discover_netease_library_observed<Observe>(
    include_local_file_count: bool,
    mut observe: Observe,
) -> NeteaseDiscovery
where
    Observe: FnMut(NeteaseDiscoveryProgress),
{
    let candidates = database_candidates();
    let mut best: Option<NeteaseDatabaseSummary> = None;
    observe(NeteaseDiscoveryProgress {
        stage: "locatingDatabase",
        processed: 0,
        total: Some(candidates.len()),
        current_item: String::new(),
        message: "正在查找网易云数据库".to_string(),
    });
    for (index, _path) in candidates.iter().enumerate() {
        observe(NeteaseDiscoveryProgress {
            stage: "locatingDatabase",
            processed: index + 1,
            total: Some(candidates.len()),
            current_item: "网易云数据库候选".to_string(),
            message: "正在查找网易云数据库".to_string(),
        });
    }
    for summary in probe_database_candidates(&candidates) {
        if summary.supported
            && summary.record_count > best.as_ref().map_or(0, |item| item.record_count)
        {
            best = Some(summary);
        }
    }

    let Some(summary) = best else {
        return NeteaseDiscovery {
            database_path: None,
            music_folder: None,
            record_count: 0,
            local_file_count: 0,
        };
    };

    let records = load_records_for_summary_observed(&summary, &mut observe).unwrap_or_default();
    let music_folder = candidate_music_folder(&summary.path, &records);
    let local_file_count = if include_local_file_count {
        observe(NeteaseDiscoveryProgress {
            stage: "checkingMusicFolder",
            processed: 0,
            total: None,
            current_item: String::new(),
            message: "正在查找音乐目录".to_string(),
        });
        let count = music_folder
            .as_deref()
            .map(|folder| {
                count_audio_files_observed(folder, |processed, path| {
                    observe(NeteaseDiscoveryProgress {
                        stage: "checkingMusicFolder",
                        processed,
                        total: None,
                        current_item: path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        message: "正在查找音乐目录".to_string(),
                    });
                })
            })
            .unwrap_or_default();
        observe(NeteaseDiscoveryProgress {
            stage: "checkingMusicFolder",
            processed: count,
            total: Some(count),
            current_item: String::new(),
            message: "音乐目录查找完成".to_string(),
        });
        count
    } else {
        0
    };
    NeteaseDiscovery {
        database_path: Some(summary.path),
        music_folder,
        record_count: summary.record_count,
        local_file_count,
    }
}

pub fn discover_netease_library_from_database(
    database_path: &Path,
) -> Result<NeteaseDiscovery, String> {
    discover_netease_library_from_database_with_local_count(database_path, true)
}

pub fn discover_netease_library_from_database_for_refresh(
    database_path: &Path,
) -> Result<NeteaseDiscovery, String> {
    discover_netease_library_from_database_with_local_count(database_path, false)
}

fn discover_netease_library_from_database_with_local_count(
    database_path: &Path,
    include_local_file_count: bool,
) -> Result<NeteaseDiscovery, String> {
    discover_netease_library_from_database_observed(database_path, include_local_file_count, |_| {})
}

pub fn discover_netease_library_from_database_observed<Observe>(
    database_path: &Path,
    include_local_file_count: bool,
    mut observe: Observe,
) -> Result<NeteaseDiscovery, String>
where
    Observe: FnMut(NeteaseDiscoveryProgress),
{
    let summary = probe_netease_database(database_path)
        .map_err(|error| format!("无法读取所选 SQLite 文件结构：{error}"))?;
    if !summary.supported {
        return Err("所选 SQLite 文件不包含可识别的网易云数据表".to_string());
    }
    let records = load_records_for_summary_observed(&summary, &mut observe)?;
    let music_folder = candidate_music_folder(database_path, &records);
    let local_file_count = if include_local_file_count {
        observe(NeteaseDiscoveryProgress {
            stage: "checkingMusicFolder",
            processed: 0,
            total: None,
            current_item: String::new(),
            message: "正在查找音乐目录".to_string(),
        });
        music_folder
            .as_deref()
            .map(|folder| {
                count_audio_files_observed(folder, |processed, path| {
                    observe(NeteaseDiscoveryProgress {
                        stage: "checkingMusicFolder",
                        processed,
                        total: None,
                        current_item: path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        message: "正在查找音乐目录".to_string(),
                    });
                })
            })
            .unwrap_or_default()
    } else {
        0
    };
    if include_local_file_count {
        observe(NeteaseDiscoveryProgress {
            stage: "checkingMusicFolder",
            processed: local_file_count,
            total: Some(local_file_count),
            current_item: String::new(),
            message: "音乐目录查找完成".to_string(),
        });
    }
    Ok(NeteaseDiscovery {
        database_path: Some(database_path.to_path_buf()),
        music_folder,
        record_count: summary.record_count,
        local_file_count,
    })
}

pub fn build_catalog_snapshot(
    database_path: &Path,
    music_folder: Option<&Path>,
) -> Result<CatalogSnapshot, String> {
    build_catalog_snapshot_incremental(database_path, music_folder, None)
}

pub fn build_catalog_snapshot_incremental(
    database_path: &Path,
    music_folder: Option<&Path>,
    catalog: Option<&LibraryCatalog>,
) -> Result<CatalogSnapshot, String> {
    build_catalog_snapshot_incremental_observed(
        database_path,
        music_folder,
        catalog,
        || false,
        |_| {},
    )
    .map_err(|error| error.to_string())
}

pub fn build_catalog_snapshot_incremental_observed<Cancel, Observe>(
    database_path: &Path,
    music_folder: Option<&Path>,
    catalog: Option<&LibraryCatalog>,
    mut is_cancelled: Cancel,
    mut observe: Observe,
) -> Result<CatalogSnapshot, CatalogBuildError>
where
    Cancel: FnMut() -> bool,
    Observe: FnMut(CatalogBuildProgress),
{
    observe(CatalogBuildProgress {
        stage: "readingRecords",
        processed: 0,
        total: None,
        current_item: String::new(),
    });
    if is_cancelled() {
        return Err(CatalogBuildError::Cancelled);
    }
    let records = load_records_from_db(database_path)
        .map_err(|error| CatalogBuildError::Failed(format!("无法只读打开网易云数据库：{error}")))?;
    let total_records = records.len();
    for (index, record) in records.iter().enumerate() {
        if is_cancelled() {
            return Err(CatalogBuildError::Cancelled);
        }
        observe(CatalogBuildProgress {
            stage: "readingRecords",
            processed: index + 1,
            total: Some(total_records),
            current_item: record.title.clone(),
        });
    }
    let local_paths = if let Some(path) = music_folder.filter(|path| path.is_dir()) {
        collect_audio_files_observed(path, &mut is_cancelled, &mut observe)?
    } else {
        Vec::new()
    };

    let mut matched_paths: HashMap<String, PathBuf> = HashMap::new();
    let total_local_paths = local_paths.len();
    for (index, path) in local_paths.iter().enumerate() {
        if is_cancelled() {
            return Err(CatalogBuildError::Cancelled);
        }
        observe(CatalogBuildProgress {
            stage: "checkingLocalFiles",
            processed: index + 1,
            total: Some(total_local_paths),
            current_item: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
        if let Some(record) = choose_record(path, &records) {
            matched_paths
                .entry(record_key(record))
                .or_insert_with(|| path.clone());
        }
    }

    let mut local_files = Vec::with_capacity(local_paths.len());
    for (index, path) in local_paths.iter().enumerate() {
        if is_cancelled() {
            return Err(CatalogBuildError::Cancelled);
        }
        observe(CatalogBuildProgress {
            stage: "probingLocalFiles",
            processed: index + 1,
            total: Some(total_local_paths),
            current_item: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
        if let Some(file) = local_file_for_path_cached(path, &records, catalog) {
            local_files.push(file);
        }
    }

    let mut tracks = Vec::new();
    let mut source_records = Vec::new();
    let mut seen = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        if is_cancelled() {
            return Err(CatalogBuildError::Cancelled);
        }
        observe(CatalogBuildProgress {
            stage: "readingRecords",
            processed: index + 1,
            total: Some(total_records),
            current_item: record.title.clone(),
        });
        let key = record_key(record);
        if !seen.insert(key.clone()) {
            continue;
        }
        let local_path = matched_paths.get(&key);
        tracks.push(track_from_record(record, local_path));
        source_records.push(source_record_from_record(record, &key));
    }

    for (index, path) in local_paths.iter().enumerate() {
        if is_cancelled() {
            return Err(CatalogBuildError::Cancelled);
        }
        observe(CatalogBuildProgress {
            stage: "checkingLocalFiles",
            processed: index + 1,
            total: Some(total_local_paths),
            current_item: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
        if records
            .iter()
            .any(|record| choose_record(path, std::slice::from_ref(record)).is_some())
        {
            continue;
        }
        let key = format!("source:local:{}", path.to_string_lossy());
        if seen.insert(key.clone()) {
            tracks.push(local_only_track(&key, path));
        }
    }

    attach_local_measurements(&mut tracks, &local_files);

    if is_cancelled() {
        return Err(CatalogBuildError::Cancelled);
    }
    let metadata = fs::metadata(database_path).map_err(|error| {
        CatalogBuildError::Failed(format!("无法读取网易云数据库文件信息：{error}"))
    })?;
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64);
    Ok(CatalogSnapshot {
        tracks,
        local_files,
        source_records,
        sources: vec![CatalogSource {
            database_path: database_path.to_path_buf(),
            database_size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            database_modified_at_ms: modified_at_ms,
            last_imported_at_ms: now_ms(),
            import_status: "success".to_string(),
            last_error: None,
        }],
    })
}

fn candidate_music_folder(database_path: &Path, records: &[NeteaseRecord]) -> Option<PathBuf> {
    if let Some(path) = known_netease_music_folder() {
        return Some(path);
    }

    // A database copied from another macOS account often retains absolute
    // paths such as /Users/mac/Music/网易云音乐. Reuse that directory when it
    // is present instead of assuming the current OS account owns the library.
    records
        .iter()
        .filter_map(|record| {
            let path = Path::new(&record.path);
            let components = path.components().collect::<Vec<_>>();
            components
                .iter()
                .position(|component| component.as_os_str() == "网易云音乐")
                .map(|index| components[..=index].iter().collect::<PathBuf>())
        })
        .find(|path| path.is_dir())
        .or_else(|| {
            database_path
                .parent()
                .map(Path::to_path_buf)
                .filter(|path| path.is_dir())
        })
}

pub fn count_audio_files(root: &Path) -> usize {
    collect_audio_files(root).len()
}

pub fn count_audio_files_observed<Observe>(root: &Path, mut observe: Observe) -> usize
where
    Observe: FnMut(usize, &Path),
{
    let mut count = 0;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || !is_audio_file(entry.path()) {
            continue;
        }
        count += 1;
        observe(count, entry.path());
    }
    count
}

/// Count audio files while allowing a background discovery worker to stop
/// cooperatively.  The callback receives the number of files seen and the
/// current filename; no full metadata is loaded during this pass.
pub fn count_audio_files_observed_cancellable<Cancel, Observe>(
    root: &Path,
    mut is_cancelled: Cancel,
    mut observe: Observe,
) -> Result<usize, NeteasePathLookupError>
where
    Cancel: FnMut() -> bool,
    Observe: FnMut(usize, &Path),
{
    let mut count = 0usize;
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        if is_cancelled() {
            return Err(NeteasePathLookupError::Cancelled);
        }
        let entry = entry.map_err(|error| {
            NeteasePathLookupError::Failed(format!("无法检查网易云音乐目录：{error}"))
        })?;
        if !entry.file_type().is_file() || !is_audio_file(entry.path()) {
            continue;
        }
        count += 1;
        observe(count, entry.path());
    }
    Ok(count)
}

fn collect_audio_files(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| is_audio_file(path))
        .collect()
}

fn collect_audio_files_observed<Cancel, Observe>(
    root: &Path,
    is_cancelled: &mut Cancel,
    observe: &mut Observe,
) -> Result<Vec<PathBuf>, CatalogBuildError>
where
    Cancel: FnMut() -> bool,
    Observe: FnMut(CatalogBuildProgress),
{
    let mut paths = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        if is_cancelled() {
            return Err(CatalogBuildError::Cancelled);
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() || !is_audio_file(entry.path()) {
            continue;
        }
        paths.push(entry.path().to_path_buf());
        observe(CatalogBuildProgress {
            stage: "checkingLocalFiles",
            processed: paths.len(),
            total: None,
            current_item: entry.file_name().to_string_lossy().into_owned(),
        });
    }
    Ok(paths)
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "wav" | "aif" | "aiff" | "ncm"
            )
        })
        .unwrap_or(false)
}

fn record_key(record: &NeteaseRecord) -> String {
    if !record.track_id.trim().is_empty() {
        format!("netease:{}", record.track_id.trim())
    } else if !record.path.trim().is_empty() {
        format!("source:netease:path:{}", record.path.trim())
    } else {
        format!("source:netease:file:{}", record.file_name.trim())
    }
}

fn track_from_record(record: &NeteaseRecord, local_path: Option<&PathBuf>) -> CatalogTrack {
    let mut track = CatalogTrack {
        track_key: record_key(record),
        netease_track_id: nonempty(record.track_id.clone()),
        title: record.title.clone(),
        artists: record.artist.clone(),
        artist_list_json: artist_list_json(&record.artist),
        album: record.album.clone(),
        netease_genre: record.genre.clone(),
        aliases_json: record.aliases_json.clone(),
        copyright_text: record.copyright_text.clone(),
        publish_date: record.publish_date.clone(),
        lyric_plain_text: record.lyric_plain_text.clone(),
        lyric_translated_text: record.lyric_translated_text.clone(),
        lyric_romanized_text: record.lyric_romanized_text.clone(),
        lyric_lrc_text: record.lyric_lrc_text.clone(),
        lyric_language: record.lyric_language.clone(),
        lyric_sync_type: record.lyric_sync_type.clone(),
        lyric_source: record.lyric_source.clone(),
        cover_path: nonempty(record.cover_path.clone()),
        cover_available: record.cover_data.is_some()
            || record.cover_path.trim() != ""
            || !record.cover_references.is_empty(),
        local_status: local_path
            .map(|_| LocalStatus::Available)
            .unwrap_or(LocalStatus::DatabaseOnly),
        db_duration_seconds: record.duration_ms.map(|value| value as f64 / 1000.0),
        db_size_bytes: record
            .size_bytes
            .and_then(|value| i64::try_from(value).ok()),
        db_format: Path::new(&record.file_name)
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        ..CatalogTrack::default()
    };
    track.effective_duration_seconds = track.db_duration_seconds;
    track.duration_source = track
        .db_duration_seconds
        .map(|_| crate::library_catalog::DurationSource::Netease);
    track.effective_format = track.db_format.clone();
    track.effective_size_bytes = track.db_size_bytes;
    track
}

fn local_only_track(key: &str, path: &Path) -> CatalogTrack {
    let mut track = CatalogTrack {
        track_key: key.to_string(),
        title: path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        local_status: LocalStatus::Available,
        measured_format: path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        effective_format: path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        ..CatalogTrack::default()
    };
    track.updated_at_ms = now_ms();
    track
}

fn local_file_for_path_cached(
    path: &Path,
    records: &[NeteaseRecord],
    catalog: Option<&LibraryCatalog>,
) -> Option<CatalogLocalFile> {
    let metadata = fs::metadata(path).ok()?;
    let record = records
        .iter()
        .find(|record| choose_record(path, std::slice::from_ref(record)).is_some());
    let track_key = record
        .map(record_key)
        .unwrap_or_else(|| format!("source:local:{}", path.to_string_lossy()));
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64);
    let base = CatalogLocalFile {
        id: None,
        track_key,
        path: path.to_path_buf(),
        size_bytes: metadata.len().min(i64::MAX as u64) as i64,
        modified_at_ms,
        measured_format: path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        measured_bitrate_bps: None,
        measured_duration_seconds: None,
        sample_rate_hz: None,
        channels: None,
        readable: true,
        probe_error: None,
    };
    if let Some(previous) = catalog
        .and_then(|catalog| catalog.local_file_by_path(path).ok().flatten())
        .filter(|previous| {
            previous.size_bytes == base.size_bytes && previous.modified_at_ms == base.modified_at_ms
        })
    {
        return Some(previous);
    }

    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("ncm"))
    {
        return Some(base);
    }

    match probe_local_audio(path) {
        Ok(facts) => Some(CatalogLocalFile {
            measured_format: Some(facts.format),
            measured_bitrate_bps: facts.average_bitrate_bps,
            measured_duration_seconds: facts.duration_seconds,
            sample_rate_hz: facts.sample_rate_hz,
            channels: facts.channels,
            ..base
        }),
        Err(error) => Some(CatalogLocalFile {
            measured_format: None,
            readable: false,
            probe_error: Some(error.to_string()),
            ..base
        }),
    }
}

fn attach_local_measurements(tracks: &mut [CatalogTrack], local_files: &[CatalogLocalFile]) {
    let mut by_track: HashMap<String, Vec<&CatalogLocalFile>> = HashMap::new();
    for local_file in local_files {
        by_track
            .entry(local_file.track_key.clone())
            .or_default()
            .push(local_file);
    }
    for track in tracks {
        let Some(files) = by_track.get(&track.track_key) else {
            continue;
        };
        let preferred = files
            .iter()
            .copied()
            .find(|file| file.readable)
            .or_else(|| files.first().copied());
        let Some(file) = preferred else {
            continue;
        };
        track.local_status = if file.readable {
            LocalStatus::Available
        } else {
            LocalStatus::Unreadable
        };
        track.measured_format = file.measured_format.clone();
        track.measured_bitrate_bps = file.measured_bitrate_bps;
        track.measured_duration_seconds = file.measured_duration_seconds;
        track.measured_size_bytes = Some(file.size_bytes);
        track.effective_format = file.measured_format.clone().or(track.db_format.clone());
        track.effective_bitrate_bps = file.measured_bitrate_bps.or(track.db_bitrate_bps);
        track.effective_size_bytes = Some(file.size_bytes);
        let (duration, source) = crate::library_catalog::effective_duration(
            track.essentia_duration_seconds,
            file.measured_duration_seconds,
            track.db_duration_seconds,
        );
        track.effective_duration_seconds = duration;
        track.duration_source = source;
    }
}

fn source_record_from_record(record: &NeteaseRecord, key: &str) -> CatalogSourceRecord {
    CatalogSourceRecord {
        track_key: key.to_string(),
        source_table: "netease".to_string(),
        source_primary_key: nonempty(record.track_id.clone())
            .or_else(|| nonempty(record.path.clone()))
            .unwrap_or_else(|| record.file_name.clone()),
        source_version: Some("local-sqlite-v1".to_string()),
        raw_json: if record.raw_json.trim().is_empty() {
            serde_json::json!({
                "title": record.title,
                "artist": record.artist,
                "album": record.album,
                "trackId": record.track_id,
                "albumId": record.album_id,
            })
            .to_string()
        } else {
            record.raw_json.clone()
        },
        imported_at_ms: now_ms(),
    }
}

fn artist_list_json(artists: &str) -> String {
    serde_json::to_string(
        &artists
            .split(",")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string())
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}
