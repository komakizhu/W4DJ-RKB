//! Best-effort recovery of metadata left beside music downloaded by NetEase
//! Cloud Music.
//!
//! This module is intentionally local-only.  It never calls a NetEase API and
//! never tries to download artwork.  It reads the desktop client's local
//! SQLite library, matches a source file conservatively, and looks for an
//! explicitly named neighbouring cover image.  The converter can then merge
//! the recovered values into the output tags.

use rusqlite::{Connection, OpenFlags, Row, types::ValueRef};
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

const MAX_COVER_BYTES: u64 = 32 * 1024 * 1024;
const NETEASE_CONTAINER: &str = "com.netease.163music";
const NETEASE_COVER_DIR_ENV: &str = "W4DJ_NETEASE_COVER_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NeteaseRecordMatchMethod {
    ExactPath,
    PathSuffix,
    FileNameAndSize,
    FileNameAndIdentity,
    NoMatch,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NeteaseCoverSource {
    Embedded,
    DatabaseBlob,
    ExplicitLocalPath,
    LocalCache,
    RemoteOnly,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NeteaseRecoveryDiagnostic {
    pub database_path: Option<String>,
    pub database_loaded: bool,
    pub database_record_count: usize,
    pub matched: bool,
    pub match_method: Option<NeteaseRecordMatchMethod>,
    pub track_id: Option<String>,
    pub album_id: Option<String>,
    pub cover_source: Option<NeteaseCoverSource>,
    pub cover_bytes: Option<usize>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RecoveredMetadata {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
    pub(crate) cover: Option<Vec<u8>>,
    pub(crate) genre: String,
    pub(crate) aliases_json: String,
    pub(crate) copyright_text: String,
    pub(crate) publish_date: String,
    pub(crate) lyric_plain_text: String,
    pub(crate) lyric_translated_text: String,
    pub(crate) lyric_romanized_text: String,
    pub(crate) lyric_lrc_text: String,
    pub(crate) lyric_language: String,
    pub(crate) lyric_sync_type: String,
    pub(crate) lyric_source: String,
    pub(crate) source: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MetadataRecovery {
    pub(crate) metadata: Option<RecoveredMetadata>,
    pub(crate) diagnostic: NeteaseRecoveryDiagnostic,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NeteaseRecord {
    pub(crate) source_table: String,
    pub(crate) source_primary_key: String,
    pub(crate) source_version: Option<String>,
    pub(crate) path: String,
    pub(crate) file_name: String,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) track_id: String,
    pub(crate) album_id: String,
    pub(crate) cover_path: String,
    pub(crate) cover_data: Option<Vec<u8>>,
    pub(crate) cover_references: Vec<String>,
    pub(crate) genre: String,
    pub(crate) aliases_json: String,
    pub(crate) copyright_text: String,
    pub(crate) publish_date: String,
    pub(crate) lyric_plain_text: String,
    pub(crate) lyric_translated_text: String,
    pub(crate) lyric_romanized_text: String,
    pub(crate) lyric_lrc_text: String,
    pub(crate) lyric_language: String,
    pub(crate) lyric_sync_type: String,
    pub(crate) lyric_source: String,
    pub(crate) raw_json: String,
}

/// A persistent cache entry intentionally containing only matching keys and
/// the source row locator. Complete metadata is never stored here.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseTrackLocator {
    pub track_id: String,
    pub source_table: String,
    pub source_primary_key: String,
    pub source_version: Option<String>,
    pub normalized_path: String,
    pub normalized_file_name: String,
    pub size_bytes: Option<u64>,
    pub title_key: String,
    pub artist_key: String,
    pub album_key: String,
}

/// Stable metadata identity exposed to the preview/conversion coordinator.
/// Complete recovered metadata remains internal to the metadata writer.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseTrackIdentity {
    pub track_id: Option<String>,
    pub album_id: Option<String>,
    pub title: String,
    pub artists: String,
    pub album: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrackJsonMetadata {
    title: String,
    artist: String,
    album: String,
    track_id: String,
    album_id: String,
    genre: String,
    aliases_json: String,
    copyright_text: String,
    publish_date: String,
    lyric_plain_text: String,
    lyric_translated_text: String,
    lyric_romanized_text: String,
    lyric_lrc_text: String,
    lyric_language: String,
    lyric_sync_type: String,
    lyric_source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FileFingerprint {
    exists: bool,
    size: Option<u64>,
    modified_nanos: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DatabaseFingerprint {
    path: PathBuf,
    main: FileFingerprint,
    wal: FileFingerprint,
    shm: FileFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFingerprintView {
    pub exists: bool,
    pub size: Option<u64>,
    pub modified_nanos: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseFingerprintView {
    pub main: FileFingerprintView,
    pub wal: FileFingerprintView,
    pub shm: FileFingerprintView,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseDatabaseSummary {
    pub path: PathBuf,
    pub supported: bool,
    pub record_count: usize,
    pub fingerprint: DatabaseFingerprintView,
}

#[derive(Debug, Clone, Default)]
struct RecordCache {
    fingerprint: Vec<DatabaseFingerprint>,
    records: Arc<Vec<NeteaseRecord>>,
}

static RECORD_CACHE: OnceLock<Mutex<RecordCache>> = OnceLock::new();
type PathRecordCache = HashMap<PathBuf, (DatabaseFingerprint, Arc<Vec<NeteaseRecord>>)>;
static PATH_RECORD_CACHE: OnceLock<Mutex<PathRecordCache>> = OnceLock::new();

#[derive(Debug, Clone, Default)]
struct LocatorIndex {
    by_path: HashMap<String, Vec<usize>>,
    by_file_name: HashMap<String, Vec<usize>>,
    by_stem: HashMap<String, Vec<usize>>,
}

#[derive(Debug, Clone, Copy)]
struct LocatorMatch {
    index: usize,
}

impl LocatorIndex {
    fn build(locators: &[NeteaseTrackLocator]) -> Self {
        let mut index = Self::default();
        for (position, locator) in locators.iter().enumerate() {
            if !locator.normalized_path.is_empty() {
                index
                    .by_path
                    .entry(locator.normalized_path.clone())
                    .or_default()
                    .push(position);
            }
            if !locator.normalized_file_name.is_empty() {
                index
                    .by_file_name
                    .entry(locator.normalized_file_name.clone())
                    .or_default()
                    .push(position);
                let stem = file_stem_key(&locator.normalized_file_name);
                if !stem.is_empty() {
                    index.by_stem.entry(stem).or_default().push(position);
                }
            }
        }
        index
    }
}

/// Immutable metadata snapshot used by one conversion batch.  SQLite is read
/// only and is opened while constructing this value; individual files never
/// reopen the database or consult process-global selection state.
#[derive(Debug, Clone)]
pub struct NeteaseMetadataResolver {
    database_path: Option<PathBuf>,
    records: Arc<Vec<NeteaseRecord>>,
    locators: Arc<Vec<NeteaseTrackLocator>>,
    locator_index: Arc<LocatorIndex>,
    database_loaded: bool,
    warning: Option<String>,
}

impl Default for NeteaseMetadataResolver {
    fn default() -> Self {
        Self {
            database_path: None,
            records: Arc::new(Vec::new()),
            locators: Arc::new(Vec::new()),
            locator_index: Arc::new(LocatorIndex::default()),
            database_loaded: false,
            warning: None,
        }
    }
}

impl NeteaseMetadataResolver {
    /// Load exactly the database selected by the user.
    ///
    /// Unlike `load_with_warning`, this method never falls back to another
    /// candidate.  It is used by the manual-selection command so an invalid
    /// file cannot silently replace the user's choice with a different
    /// database.
    pub fn load_exact(database_path: &Path) -> io::Result<Self> {
        if !database_path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "网易云数据库文件不存在",
            ));
        }
        let supported = probe_netease_database(database_path)
            .map(|summary| summary.supported)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("无法检查网易云数据库 schema：{error}"),
                )
            })?;
        if !supported {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "网易云数据库 schema 不受支持",
            ));
        }
        let records = load_records_cached(database_path, 2).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("无法读取网易云数据库：{error}"),
            )
        })?;
        Ok(Self {
            database_path: Some(database_path.to_path_buf()),
            records,
            locators: Arc::new(Vec::new()),
            locator_index: Arc::new(LocatorIndex::default()),
            database_loaded: true,
            warning: None,
        })
    }

    pub fn load(preferred_database: Option<&Path>) -> io::Result<Self> {
        Ok(Self::load_with_warning(preferred_database)?.0)
    }

    pub fn load_with_warning(
        preferred_database: Option<&Path>,
    ) -> io::Result<(Self, Option<String>)> {
        let mut warning = None;
        let mut candidates = Vec::new();
        if let Some(preferred) = preferred_database {
            let supported = preferred.is_file()
                && probe_netease_database(preferred)
                    .map(|summary| summary.supported)
                    .unwrap_or(false);
            if supported {
                candidates.push(preferred.to_path_buf());
            } else {
                warning = Some(format!(
                    "保存的网易云数据库不可用或 schema 不受支持：{}，已尝试自动定位",
                    preferred.display()
                ));
            }
        }
        for candidate in database_candidates() {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }

        for path in candidates {
            match Self::load_exact(&path) {
                Ok(mut resolver) => {
                    resolver.warning = warning.clone();
                    return Ok((resolver, warning));
                }
                Err(error) if path.is_file() => {
                    warning = Some(format!("网易云数据库读取失败：{error}"));
                }
                Err(_) => {}
            }
        }

        Ok((
            Self {
                database_path: None,
                records: Arc::new(Vec::new()),
                locators: Arc::new(Vec::new()),
                locator_index: Arc::new(LocatorIndex::default()),
                database_loaded: false,
                warning: warning.clone(),
            },
            warning,
        ))
    }

    pub fn database_path(&self) -> Option<&Path> {
        self.database_path.as_deref()
    }

    pub fn record_count(&self) -> usize {
        if self.records.is_empty() {
            self.locators.len()
        } else {
            self.records.len()
        }
    }

    pub fn database_loaded(&self) -> bool {
        self.database_loaded
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// Construct a resolver from the persistent locator snapshot. This does
    /// not read the source song table; a complete row is fetched only by
    /// `recover` after a local file has matched a locator.
    pub fn from_locators(
        database_path: &Path,
        locators: Vec<NeteaseTrackLocator>,
        warning: Option<String>,
    ) -> Self {
        let locator_index = Arc::new(LocatorIndex::build(&locators));
        Self {
            database_path: Some(database_path.to_path_buf()),
            records: Arc::new(Vec::new()),
            locators: Arc::new(locators),
            locator_index,
            database_loaded: true,
            warning,
        }
    }

    /// Startup-safe resolver loading. Only the database schema/count probe
    /// and the already-built lightweight locator cache are touched here.
    pub fn load_lazy_with_warning(
        preferred_database: Option<&Path>,
        cache_path: &Path,
    ) -> io::Result<(Self, Option<String>)> {
        let mut warning = None;
        let mut candidates = Vec::new();
        if let Some(preferred) = preferred_database {
            if preferred.is_file()
                && probe_netease_database(preferred)
                    .map(|summary| summary.supported)
                    .unwrap_or(false)
            {
                candidates.push(preferred.to_path_buf());
            } else {
                warning = Some(format!(
                    "保存的网易云数据库不可用或 schema 不受支持：{}，已尝试自动定位",
                    preferred.display()
                ));
            }
        }
        for candidate in database_candidates() {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        for path in candidates {
            if !path.is_file() {
                continue;
            }
            let supported = probe_netease_database(&path)
                .map(|summary| summary.supported)
                .unwrap_or(false);
            if !supported {
                continue;
            }
            let fingerprint = database_fingerprint_view(&path);
            let summary =
                crate::netease_cache::read_summary(cache_path, Some(&path), Some(&fingerprint))
                    .map_err(|error| {
                        io::Error::new(io::ErrorKind::InvalidData, error.to_string())
                    })?;
            if summary.state == crate::netease_cache::CacheState::Ready {
                let locators =
                    crate::netease_cache::read_locators(cache_path).map_err(|error| {
                        io::Error::new(io::ErrorKind::InvalidData, error.to_string())
                    })?;
                return Ok((
                    Self::from_locators(&path, locators, warning.clone()),
                    warning,
                ));
            }
            warning = warning.or_else(|| {
                Some("网易云轻量索引尚未准备，转换将使用文件名/嵌入标签回退".to_string())
            });
            return Ok((Self::default(), warning));
        }
        Ok((Self::default(), warning))
    }

    /// Return the matched local database identity for a source file. This is
    /// read-only and returns `None` when matching is not conservative enough.
    pub fn track_identity(&self, source_path: &Path) -> Option<NeteaseTrackIdentity> {
        let recovery = self.recover(source_path);
        let metadata = recovery.metadata?;
        let diagnostic = recovery.diagnostic;
        Some(NeteaseTrackIdentity {
            track_id: diagnostic.track_id,
            album_id: diagnostic.album_id,
            title: metadata.title,
            artists: metadata.artist,
            album: metadata.album,
        })
    }

    /// Resolve the stable identity used by a cancellable scan. This path
    /// avoids opening SQLite for every candidate and checks cancellation while
    /// ranking records, so a large preview cannot hold the cancel button up.
    pub(crate) fn track_identity_cancellable(
        &self,
        source_path: &Path,
        cancel: &AtomicBool,
    ) -> Option<NeteaseTrackIdentity> {
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        let source_size = fs::metadata(source_path)
            .ok()
            .map(|metadata| metadata.len());
        if self.records.is_empty() && !self.locators.is_empty() {
            let matched = choose_locator_with_method_cancellable(
                source_path,
                &self.locators,
                self.locator_index.as_ref(),
                source_size,
                cancel,
            )?;
            let locator = &self.locators[matched.index];
            return Some(locator_match_identity(locator));
        }
        let matched_record =
            choose_record_with_method_cancellable(source_path, &self.records, source_size, cancel)
                .and_then(|matched| match matched {
                    RecordMatch::Matched { record, .. } => Some(record),
                    RecordMatch::NoMatch | RecordMatch::Ambiguous { .. } => None,
                })?;
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        Some(NeteaseTrackIdentity {
            track_id: non_empty_string(&matched_record.track_id),
            album_id: non_empty_string(&matched_record.album_id),
            title: matched_record.title.clone(),
            artists: matched_record.artist.clone(),
            album: matched_record.album.clone(),
        })
    }

    /// Resolve only the lightweight locator identity without opening the
    /// source SQLite database. Preview attachment uses this for a lazy cache;
    /// final conversion still calls `recover` when it needs complete tags.
    pub(crate) fn track_identity_for_preview(
        &self,
        source_path: &Path,
    ) -> Option<NeteaseTrackIdentity> {
        static NEVER_CANCELLED: AtomicBool = AtomicBool::new(false);
        if self.records.is_empty() && !self.locators.is_empty() {
            let matched = choose_locator_with_method_cancellable(
                source_path,
                &self.locators,
                self.locator_index.as_ref(),
                fs::metadata(source_path)
                    .ok()
                    .map(|metadata| metadata.len()),
                &NEVER_CANCELLED,
            )?;
            let locator = &self.locators[matched.index];
            return Some(locator_match_identity(locator));
        }
        self.track_identity(source_path)
    }

    pub(crate) fn recover(&self, source_path: &Path) -> MetadataRecovery {
        if self.records.is_empty() && !self.locators.is_empty() {
            let never_cancelled = AtomicBool::new(false);
            let Some(locator) = choose_locator_with_method_cancellable(
                source_path,
                &self.locators,
                self.locator_index.as_ref(),
                fs::metadata(source_path)
                    .ok()
                    .map(|metadata| metadata.len()),
                &never_cancelled,
            )
            .and_then(|matched| self.locators.get(matched.index)) else {
                return recover_with_records(
                    source_path,
                    &[],
                    self.database_path.as_deref(),
                    self.database_loaded,
                );
            };
            if let Some(database_path) = self.database_path.as_deref()
                && let Ok(Some(record)) = load_record_by_locator(database_path, locator)
            {
                return recover_with_records(
                    source_path,
                    &[record],
                    self.database_path.as_deref(),
                    self.database_loaded,
                );
            }
            return recover_with_records(
                source_path,
                &[],
                self.database_path.as_deref(),
                self.database_loaded,
            );
        }
        recover_with_records(
            source_path,
            &self.records,
            self.database_path.as_deref(),
            self.database_loaded,
        )
    }
}

fn locator_match_identity(locator: &NeteaseTrackLocator) -> NeteaseTrackIdentity {
    // Locator text fields are normalized matching keys, not canonical tags.
    // Expose only the stable track ID until `recover` reads the original row.
    NeteaseTrackIdentity {
        track_id: non_empty_string(&locator.track_id),
        album_id: None,
        title: String::new(),
        artists: String::new(),
        album: String::new(),
    }
}

/// Recover local NetEase metadata without contacting the network.
pub(crate) fn recover_local_metadata(source_path: &Path) -> Option<RecoveredMetadata> {
    let records = load_cached_records();
    recover_with_records(source_path, &records, None, !records.is_empty()).metadata
}

pub(crate) fn recover_local_metadata_with_resolver(
    source_path: &Path,
    resolver: &NeteaseMetadataResolver,
) -> MetadataRecovery {
    resolver.recover(source_path)
}

fn recover_with_records(
    source_path: &Path,
    records: &[NeteaseRecord],
    database_path: Option<&Path>,
    database_loaded: bool,
) -> MetadataRecovery {
    let match_result = choose_record_with_method(source_path, records);
    let (record, match_method, match_message) = match match_result {
        RecordMatch::Matched { record, method } => (Some(record), Some(method), None),
        RecordMatch::NoMatch => (None, Some(NeteaseRecordMatchMethod::NoMatch), None),
        RecordMatch::Ambiguous { candidates } => (
            None,
            Some(NeteaseRecordMatchMethod::Ambiguous),
            Some(format!("存在 {candidates} 个同分候选，已拒绝猜测")),
        ),
    };

    let embedded_cover = embedded_cover_for_path(source_path);
    let mut diagnostic = NeteaseRecoveryDiagnostic {
        database_path: database_path.map(|path| path.display().to_string()),
        database_loaded,
        database_record_count: records.len(),
        matched: record.is_some(),
        match_method,
        track_id: record.and_then(|record| non_empty_string(&record.track_id)),
        album_id: record.and_then(|record| non_empty_string(&record.album_id)),
        cover_source: embedded_cover
            .as_ref()
            .map(|_| NeteaseCoverSource::Embedded),
        cover_bytes: embedded_cover.as_ref().map(Vec::len),
        message: match_message,
    };

    let mut recovered = record.map(|record| RecoveredMetadata {
        title: record.title.clone(),
        artist: record.artist.clone(),
        album: record.album.clone(),
        cover: embedded_cover.clone(),
        genre: record.genre.clone(),
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
        source: String::from("网易云本地数据库"),
    });

    if recovered.is_none() && embedded_cover.is_some() {
        recovered = Some(RecoveredMetadata {
            cover: embedded_cover.clone(),
            source: String::from("本地嵌入封面"),
            ..RecoveredMetadata::default()
        });
    }

    if let Some(record) = record {
        if embedded_cover.is_none() {
            let (cover, source) = recover_cover_with_source(source_path, record);
            if let Some(cover) = cover {
                diagnostic.cover_bytes = Some(cover.len());
                diagnostic.cover_source = Some(source);
                if let Some(recovered) = recovered.as_mut() {
                    recovered.cover = Some(cover);
                }
            } else if diagnostic.cover_source.is_none() {
                diagnostic.cover_source = cover_failure_source(record);
            }
        }
    } else if embedded_cover.is_none()
        && let Some(cover) = find_source_directory_cover(source_path, None)
    {
        diagnostic.cover_source = Some(NeteaseCoverSource::LocalCache);
        diagnostic.cover_bytes = Some(cover.len());
        recovered = Some(RecoveredMetadata {
            cover: Some(cover),
            source: String::from("网易云本地封面"),
            ..RecoveredMetadata::default()
        });
    }

    if let Some(recovered) = recovered.as_mut() {
        if recovered.cover.is_some() && recovered.source.is_empty() {
            recovered.source = String::from("网易云本地封面");
        } else if recovered.cover.is_some() && !recovered.source.contains("封面") {
            recovered.source.push_str(" + 本地封面");
        }
    }

    let has_recoverable_metadata = recovered.as_ref().is_some_and(|value| {
        !value.title.trim().is_empty()
            || !value.artist.trim().is_empty()
            || !value.album.trim().is_empty()
            || value.cover.is_some()
            || !value.genre.trim().is_empty()
            || !value.lyric_plain_text.trim().is_empty()
            || !value.lyric_lrc_text.trim().is_empty()
    });
    if !has_recoverable_metadata && diagnostic.message.is_none() {
        diagnostic.message = Some(if database_loaded {
            "没有找到可安全匹配的网易云记录".to_string()
        } else {
            "网易云数据库未加载".to_string()
        });
    }

    MetadataRecovery {
        metadata: has_recoverable_metadata.then_some(recovered).flatten(),
        diagnostic,
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

pub(crate) fn load_cached_records() -> Arc<Vec<NeteaseRecord>> {
    let candidates = database_candidates();
    let fingerprint = candidates
        .iter()
        .map(|path| database_fingerprint(path))
        .collect::<Vec<_>>();
    let cache = RECORD_CACHE.get_or_init(|| Mutex::new(RecordCache::default()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if cache.fingerprint == fingerprint {
        return Arc::clone(&cache.records);
    }

    let mut records = Vec::new();
    for path in &candidates {
        if path.is_file()
            && let Ok(database_records) = load_records_cached(path, 2)
        {
            records.extend(database_records.iter().cloned());
        }
    }

    cache.fingerprint = fingerprint;
    cache.records = Arc::new(records);
    Arc::clone(&cache.records)
}

pub fn database_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = env::var_os("W4DJ_NETEASE_DB").filter(|value| !value.is_empty()) {
        paths.push(PathBuf::from(path));
    }

    let Some(home) = home_dir() else {
        return paths;
    };

    #[cfg(target_os = "macos")]
    {
        paths.push(
            home.join("Library/Containers")
                .join(NETEASE_CONTAINER)
                .join("Data/Documents/storage/sqlite_storage.sqlite3"),
        );
        paths.push(home.join(
            "Library/Application Support/Netease Cloud Music/storage/sqlite_storage.sqlite3",
        ));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(app_data) = env::var_os("APPDATA") {
            let app_data = PathBuf::from(app_data);
            paths.push(app_data.join("Netease/CloudMusic/storage/sqlite_storage.sqlite3"));
            paths.push(app_data.join("NetEase/CloudMusic/storage/sqlite_storage.sqlite3"));
            paths.push(app_data.join("Netease Cloud Music/storage/sqlite_storage.sqlite3"));
        }
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            paths.push(
                PathBuf::from(local_app_data)
                    .join("Netease/CloudMusic/storage/sqlite_storage.sqlite3"),
            );
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        paths.push(config_home.join("netease-cloud-music/storage/sqlite_storage.sqlite3"));
        paths.push(home.join(".config/netease-cloud-music/storage/sqlite_storage.sqlite3"));
    }

    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing: &PathBuf| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

pub fn locate_supported_database(preferred_database: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(preferred) = preferred_database {
        candidates.push(preferred.to_path_buf());
    }
    candidates.extend(database_candidates());
    candidates.into_iter().find(|path| {
        path.is_file()
            && probe_netease_database(path)
                .map(|summary| summary.supported)
                .unwrap_or(false)
    })
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn database_fingerprint(path: &Path) -> DatabaseFingerprint {
    DatabaseFingerprint {
        path: path.to_path_buf(),
        main: file_fingerprint(path),
        wal: file_fingerprint(&PathBuf::from(format!("{}-wal", path.display()))),
        shm: file_fingerprint(&PathBuf::from(format!("{}-shm", path.display()))),
    }
}

fn file_fingerprint_view(value: &FileFingerprint) -> FileFingerprintView {
    FileFingerprintView {
        exists: value.exists,
        size: value.size,
        modified_nanos: value.modified_nanos,
    }
}

pub fn database_fingerprint_view(path: &Path) -> DatabaseFingerprintView {
    let fingerprint = database_fingerprint(path);
    DatabaseFingerprintView {
        main: file_fingerprint_view(&fingerprint.main),
        wal: file_fingerprint_view(&fingerprint.wal),
        shm: file_fingerprint_view(&fingerprint.shm),
    }
}

fn file_fingerprint(path: &Path) -> FileFingerprint {
    let metadata = fs::metadata(path).ok();
    let modified_nanos = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos());

    FileFingerprint {
        exists: metadata.is_some(),
        size: metadata.map(|metadata| metadata.len()),
        modified_nanos,
    }
}

pub(crate) fn load_records_from_db(path: &Path) -> rusqlite::Result<Vec<NeteaseRecord>> {
    load_records_from_db_observed(path, 1, |_, _, _| {})
}

fn load_records_cached(
    path: &Path,
    parallelism: usize,
) -> rusqlite::Result<Arc<Vec<NeteaseRecord>>> {
    let fingerprint = database_fingerprint(path);
    let cache = PATH_RECORD_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some((cached_fingerprint, records)) = cache.get(path)
        && cached_fingerprint == &fingerprint
    {
        return Ok(Arc::clone(records));
    }
    let records = Arc::new(load_records_from_db_observed(
        path,
        parallelism,
        |_, _, _| {},
    )?);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(path.to_path_buf(), (fingerprint, Arc::clone(&records)));
    }
    Ok(records)
}

pub fn probe_netease_database(path: &Path) -> rusqlite::Result<NeteaseDatabaseSummary> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    let connection = Connection::open_with_flags(path, flags)?;
    let tables = supported_table_names(&connection)?;
    let mut record_count = 0usize;
    for table in &tables {
        record_count += count_table_rows(&connection, table)?;
    }
    Ok(NeteaseDatabaseSummary {
        path: path.to_path_buf(),
        supported: !tables.is_empty(),
        record_count,
        fingerprint: database_fingerprint_view(path),
    })
}

pub fn load_records_from_db_observed<Observe>(
    path: &Path,
    parallelism: usize,
    mut observe: Observe,
) -> rusqlite::Result<Vec<NeteaseRecord>>
where
    Observe: FnMut(&'static str, usize, usize),
{
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    let connection = Connection::open_with_flags(path, flags)?;
    let tables = supported_table_names(&connection)?;
    if tables.is_empty() {
        return Ok(Vec::new());
    }
    if parallelism.max(1) == 1 || tables.len() == 1 {
        let mut table_records = HashMap::new();
        for table in tables {
            let records = read_table_records_observed(&connection, table, |processed, total| {
                observe(table, processed, total);
            })?;
            table_records.insert(table, records);
        }
        return Ok(merge_table_records(table_records));
    }

    enum LoadMessage {
        Progress {
            table: &'static str,
            processed: usize,
            total: usize,
        },
        Result {
            table: &'static str,
            records: Vec<NeteaseRecord>,
        },
        Error(rusqlite::Error),
    }

    let worker_count = parallelism.max(1).min(tables.len());
    let queue = Arc::new(Mutex::new(
        tables.into_iter().collect::<VecDeque<&'static str>>(),
    ));
    let (sender, receiver) = mpsc::channel();
    let mut workers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let sender = sender.clone();
        let database_path = path.to_path_buf();
        workers.push(thread::spawn(move || {
            loop {
                let table = {
                    let mut queue = queue.lock().expect("netease queue lock poisoned");
                    queue.pop_front()
                };
                let Some(table) = table else {
                    break;
                };
                let connection = match Connection::open_with_flags(
                    &database_path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY,
                ) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = sender.send(LoadMessage::Error(error));
                        break;
                    }
                };
                let result = read_table_records_observed(&connection, table, |processed, total| {
                    let _ = sender.send(LoadMessage::Progress {
                        table,
                        processed,
                        total,
                    });
                });
                match result {
                    Ok(records) => {
                        let _ = sender.send(LoadMessage::Result { table, records });
                    }
                    Err(error) => {
                        let _ = sender.send(LoadMessage::Error(error));
                        break;
                    }
                }
            }
        }));
    }
    drop(sender);

    let mut first_error = None;
    let mut table_records = HashMap::new();
    while let Ok(message) = receiver.recv() {
        match message {
            LoadMessage::Progress {
                table,
                processed,
                total,
            } => observe(table, processed, total),
            LoadMessage::Result { table, records } => {
                table_records.insert(table, records);
            }
            LoadMessage::Error(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(merge_table_records(table_records))
}

/// Build the persistent cache payload without retaining complete JSON,
/// lyrics, cover bytes, or other metadata in memory after each row.
pub fn load_locators_from_db_observed<Observe>(
    path: &Path,
    mut observe: Observe,
) -> rusqlite::Result<Vec<NeteaseTrackLocator>>
where
    Observe: FnMut(&'static str, usize, usize) -> bool,
{
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut locators = Vec::new();
    for table in supported_table_names(&connection)? {
        let available = table_columns(&connection, table)?;
        let select = [
            "rowid AS source_primary_key".to_string(),
            select_expression(
                &available,
                &["file", "librarypath", "relative_path"],
                "path",
            ),
            select_expression(&available, &["dir", "parentdir"], "directory"),
            select_expression(&available, &["file", "track", "relative_path"], "file_name"),
            select_expression(&available, &["title", "name", "track_name"], "title"),
            select_expression(&available, &["artist", "artist_name"], "artist"),
            select_expression(&available, &["album", "album_name"], "album"),
            select_expression(&available, &["filesize", "size"], "size_bytes"),
            select_expression(&available, &["tid", "track_id", "id"], "track_id"),
            select_expression(
                &available,
                &["detail", "track", "source_text", "source_extra"],
                "metadata_json",
            ),
        ]
        .join(", ");
        let total = count_table_rows(&connection, table)?.min(200_000);
        let sql = format!("SELECT {select} FROM \"{table}\" LIMIT 200000");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            let source_primary_key = row_text(row, 0);
            let path_value = row_text(row, 1);
            let directory = row_text(row, 2);
            let raw_file_name = row_text(row, 3);
            let raw_json = row_text(row, 9);
            let json_metadata = track_json_metadata(&raw_json);
            let path = combine_path(&path_value, &directory);
            let file_name = Path::new(&raw_file_name.replace('\\', "/"))
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .or_else(|| {
                    Path::new(&path.replace('\\', "/"))
                        .file_name()
                        .and_then(|value| value.to_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            let title = prefer_nonempty(row_text(row, 4), &json_metadata.title);
            let artist = prefer_nonempty(row_text(row, 5), &json_metadata.artist);
            let album = prefer_nonempty(row_text(row, 6), &json_metadata.album);
            let track_id = prefer_nonempty(row_text(row, 8), &json_metadata.track_id);
            // `web_track.track` is a JSON payload, not a filename. When the
            // database has no local path, retain the same conservative
            // title/artist key used by the full-record loader so the lazy
            // locator cache can match ordinary downloaded files.
            let locator_file_name = if path.trim().is_empty()
                && !title.trim().is_empty()
                && !artist.trim().is_empty()
            {
                format!("{title} - {artist}")
            } else {
                file_name
            };
            Ok(NeteaseTrackLocator {
                track_id,
                source_table: table.to_string(),
                source_primary_key,
                source_version: None,
                normalized_path: normalized_path(&path),
                normalized_file_name: normalized_file_name(if locator_file_name.is_empty() {
                    &path
                } else {
                    &locator_file_name
                }),
                size_bytes: row_u64(row, 7),
                title_key: persistent_metadata_key(&title),
                artist_key: persistent_metadata_key(&artist),
                album_key: persistent_metadata_key(&album),
            })
        })?;
        let mut processed = 0usize;
        for row in rows {
            processed += 1;
            if (processed == total || processed.is_multiple_of(128))
                && !observe(table, processed, total)
            {
                return Ok(Vec::new());
            }
            if let Ok(locator) = row
                && (!locator.normalized_path.is_empty()
                    || !locator.normalized_file_name.is_empty()
                    || !locator.title_key.is_empty())
            {
                locators.push(locator);
            }
        }
        if !observe(table, processed, total) {
            return Ok(Vec::new());
        }
    }
    Ok(locators)
}

#[allow(dead_code)]
pub(crate) fn has_supported_netease_table(path: &Path) -> rusqlite::Result<bool> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    ["track", "web_offline_track", "web_cloud_track", "web_track"]
        .iter()
        .try_fold(false, |found, table| {
            Ok(found || table_exists(&connection, table)?)
        })
}

fn supported_table_names(connection: &Connection) -> rusqlite::Result<Vec<&'static str>> {
    let mut tables = Vec::new();
    for table in ["track", "web_offline_track", "web_cloud_track", "web_track"] {
        if table_exists(connection, table)? {
            tables.push(table);
        }
    }
    Ok(tables)
}

#[allow(dead_code)]
fn load_records_from_connection(connection: &Connection) -> rusqlite::Result<Vec<NeteaseRecord>> {
    let mut table_records = HashMap::new();
    for table in supported_table_names(connection)? {
        table_records.insert(table, read_table_records(connection, table)?);
    }
    Ok(merge_table_records(table_records))
}

fn merge_table_records(
    mut table_records: HashMap<&'static str, Vec<NeteaseRecord>>,
) -> Vec<NeteaseRecord> {
    let mut records = Vec::new();
    for table in ["track", "web_offline_track", "web_cloud_track"] {
        if let Some(mut loaded) = table_records.remove(table) {
            records.append(&mut loaded);
        }
    }
    if let Some(loaded) = table_records.remove("web_track") {
        merge_track_metadata(&mut records, loaded);
    }
    records
}

fn table_exists(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

#[allow(dead_code)]
fn read_table_records(
    connection: &Connection,
    table: &str,
) -> rusqlite::Result<Vec<NeteaseRecord>> {
    read_table_records_observed(connection, table, |_, _| {})
}

fn count_table_rows(connection: &Connection, table: &str) -> rusqlite::Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
    connection.query_row(&sql, [], |row| row.get::<_, usize>(0))
}

fn read_table_records_observed<Observe>(
    connection: &Connection,
    table: &str,
    mut observe: Observe,
) -> rusqlite::Result<Vec<NeteaseRecord>>
where
    Observe: FnMut(usize, usize),
{
    let available = table_columns(connection, table)?;
    let total = count_table_rows(connection, table)?.min(200_000);
    let select = record_select_sql(&available);
    let sql = format!("SELECT {select} FROM \"{table}\" LIMIT 200000");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], record_from_row)?;
    let mut processed = 0usize;
    let mut records = Vec::new();
    for row in rows {
        processed += 1;
        if processed == total || processed.is_multiple_of(128) {
            observe(processed, total);
        }
        let Ok(mut record) = row else {
            continue;
        };
        record.source_table = table.to_string();
        let has_any_value = !record.path.is_empty()
            || !record.file_name.is_empty()
            || !record.title.is_empty()
            || !record.artist.is_empty()
            || !record.album.is_empty();
        if has_any_value {
            records.push(record);
        }
    }
    if total == 0 {
        observe(0, 0);
    } else if processed < total {
        observe(processed, total);
    }
    Ok(records)
}

fn record_select_sql(available: &HashSet<String>) -> String {
    [
        "rowid AS source_primary_key".to_string(),
        select_expression(available, &["file", "librarypath", "relative_path"], "path"),
        select_expression(available, &["dir", "parentdir"], "directory"),
        select_expression(available, &["file", "track", "relative_path"], "file_name"),
        select_expression(available, &["title", "name", "track_name"], "title"),
        select_expression(available, &["artist", "artist_name"], "artist"),
        select_expression(available, &["album", "album_name"], "album"),
        select_expression(available, &["filesize", "size"], "size_bytes"),
        select_expression(available, &["duration", "duration_ms"], "duration_ms"),
        select_expression(available, &["tid", "track_id", "id"], "track_id"),
        select_expression(available, &["album_id", "aid"], "album_id"),
        select_expression(
            available,
            &["cover_path", "cover", "album_cover", "pic", "picture"],
            "cover_path",
        ),
        select_expression(
            available,
            &["detail", "track", "source_text", "source_extra"],
            "metadata_json",
        ),
    ]
    .join(", ")
}

fn load_record_by_locator(
    database_path: &Path,
    locator: &NeteaseTrackLocator,
) -> rusqlite::Result<Option<NeteaseRecord>> {
    if !matches!(
        locator.source_table.as_str(),
        "track" | "web_offline_track" | "web_cloud_track" | "web_track"
    ) {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let available = table_columns(&connection, &locator.source_table)?;
    let sql = format!(
        "SELECT {} FROM \"{}\" WHERE rowid = ?1 LIMIT 1",
        record_select_sql(&available),
        locator.source_table
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query([&locator.source_primary_key])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let mut record = record_from_row(row)?;
    record.source_table = locator.source_table.clone();
    record.source_version = locator.source_version.clone();
    if record.path.trim().is_empty()
        && !record.title.trim().is_empty()
        && !record.artist.trim().is_empty()
    {
        record.file_name = format!("{} - {}", record.title, record.artist);
    }
    Ok(Some(record))
}

fn table_columns(connection: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let sql = format!("PRAGMA table_info(\"{table}\")");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows
        .filter_map(|row| row.ok())
        .map(|column| column.to_ascii_lowercase())
        .collect())
}

fn select_expression(columns: &HashSet<String>, candidates: &[&str], alias: &str) -> String {
    candidates
        .iter()
        .find(|candidate| columns.contains(**candidate))
        .map(|column| format!("\"{column}\" AS \"{alias}\""))
        .unwrap_or_else(|| format!("NULL AS \"{alias}\""))
}

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<NeteaseRecord> {
    let source_primary_key = row_text(row, 0);
    let path_value = row_text(row, 1);
    let directory = row_text(row, 2);
    let file_name_value = row_text(row, 3);
    let metadata_json = row_text(row, 12);
    let json_metadata = track_json_metadata(&metadata_json);
    let path = combine_path(&path_value, &directory);
    let file_name_value =
        if file_name_value.trim().starts_with('{') || file_name_value.trim().starts_with('[') {
            String::new()
        } else {
            file_name_value
        };
    let file_name = Path::new(&file_name_value.replace('\\', "/"))
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            Path::new(&path.replace('\\', "/"))
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if !file_name_value.trim().is_empty() {
                file_name_value.clone()
            } else {
                json_metadata.title.clone()
            }
        });
    let cover_data = row_blob(row, 11).filter(|bytes| is_supported_image(bytes));
    let cover_references = cover_references_from_json(&metadata_json);

    Ok(NeteaseRecord {
        source_table: String::new(),
        source_primary_key,
        source_version: None,
        path,
        file_name,
        title: prefer_nonempty(row_text(row, 4), &json_metadata.title),
        artist: prefer_nonempty(row_text(row, 5), &json_metadata.artist),
        album: prefer_nonempty(row_text(row, 6), &json_metadata.album),
        size_bytes: row_u64(row, 7),
        duration_ms: row_u64(row, 8),
        track_id: prefer_nonempty(row_text(row, 9), &json_metadata.track_id),
        album_id: prefer_nonempty(row_text(row, 10), &json_metadata.album_id),
        cover_path: row_text(row, 11),
        cover_data,
        cover_references,
        genre: json_metadata.genre,
        aliases_json: json_metadata.aliases_json,
        copyright_text: json_metadata.copyright_text,
        publish_date: json_metadata.publish_date,
        lyric_plain_text: json_metadata.lyric_plain_text,
        lyric_translated_text: json_metadata.lyric_translated_text,
        lyric_romanized_text: json_metadata.lyric_romanized_text,
        lyric_lrc_text: json_metadata.lyric_lrc_text,
        lyric_language: json_metadata.lyric_language,
        lyric_sync_type: json_metadata.lyric_sync_type,
        lyric_source: json_metadata.lyric_source,
        raw_json: metadata_json,
    })
}

fn prefer_nonempty(primary: String, fallback: &str) -> String {
    if primary.trim().is_empty() {
        fallback.trim().to_string()
    } else {
        primary
    }
}

fn track_json_metadata(raw: &str) -> TrackJsonMetadata {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return TrackJsonMetadata::default();
    };
    let track = value
        .get("track")
        .filter(|value| value.is_object())
        .unwrap_or(&value);
    let album = track
        .get("album")
        .or_else(|| track.get("al"))
        .filter(|value| value.is_object());

    let artist = track
        .get("artists")
        .and_then(json_artist_text)
        .or_else(|| track.get("artist").and_then(json_artist_text))
        .unwrap_or_default();

    let album_name = album
        .and_then(|album| album.get("name").and_then(json_scalar_text))
        .or_else(|| track.get("album").and_then(json_scalar_text))
        .or_else(|| track.get("albumName").and_then(json_scalar_text))
        .unwrap_or_default();
    let album_id = album
        .and_then(|album| album.get("id").and_then(json_scalar_text))
        .or_else(|| track.get("albumId").and_then(json_scalar_text))
        .or_else(|| track.get("aid").and_then(json_scalar_text))
        .unwrap_or_default();

    let genre = json_find_text(&value, &["genre", "musicType", "music_type"]).unwrap_or_default();
    let aliases_json = json_find_array_or_text(&value, &["alias", "aliases", "transNames"])
        .map(|values| serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_else(|| "[]".to_string());
    let copyright_text = json_find_text(
        &value,
        &["copyright", "copyrightText", "copyrightDesc", "rightInfo"],
    )
    .unwrap_or_default();
    let publish_date = json_find_text(
        &value,
        &["publishDate", "publish_date", "publishTime", "releaseDate"],
    )
    .unwrap_or_default();
    let original_lrc =
        json_find_text(&value, &["lyric", "lyrics", "lrc", "originalLyric"]).unwrap_or_default();
    let translated_lrc =
        json_find_text(&value, &["tlyric", "translatedLyric", "translation"]).unwrap_or_default();
    let romanized_lrc =
        json_find_text(&value, &["romalrc", "romanizedLyric", "romanLyric"]).unwrap_or_default();
    let lyric_plain_text = strip_lrc_timestamps(&original_lrc);
    let lyric_language = json_find_text(&value, &["language", "lyricLanguage"]).unwrap_or_default();
    let lyric_sync_type = if original_lrc.contains('[') {
        "timed"
    } else {
        "plain"
    }
    .to_string();

    TrackJsonMetadata {
        title: track
            .get("name")
            .or_else(|| track.get("title"))
            .or_else(|| track.get("musicName"))
            .and_then(json_scalar_text)
            .unwrap_or_default(),
        artist,
        album: album_name,
        track_id: track
            .get("id")
            .or_else(|| track.get("musicId"))
            .or_else(|| track.get("tid"))
            .and_then(json_scalar_text)
            .unwrap_or_default(),
        album_id,
        genre,
        aliases_json,
        copyright_text,
        publish_date,
        lyric_plain_text,
        lyric_translated_text: strip_lrc_timestamps(&translated_lrc),
        lyric_romanized_text: strip_lrc_timestamps(&romanized_lrc),
        lyric_lrc_text: original_lrc,
        lyric_language,
        lyric_sync_type,
        lyric_source: "网易云本地数据库".to_string(),
    }
}

fn json_find_text(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => {
            for key in keys {
                if let Some(candidate) = object.get(*key).and_then(json_value_text)
                    && !candidate.trim().is_empty()
                {
                    return Some(candidate);
                }
            }
            object
                .values()
                .find_map(|child| json_find_text(child, keys))
        }
        serde_json::Value::Array(values) => {
            values.iter().find_map(|child| json_find_text(child, keys))
        }
        _ => None,
    }
}

fn json_find_array_or_text(value: &serde_json::Value, keys: &[&str]) -> Option<Vec<String>> {
    match value {
        serde_json::Value::Object(object) => {
            for key in keys {
                if let Some(candidate) = object.get(*key) {
                    let values = match candidate {
                        serde_json::Value::Array(items) => items
                            .iter()
                            .filter_map(json_value_text)
                            .map(|item| item.trim().to_string())
                            .filter(|item| !item.is_empty())
                            .collect::<Vec<_>>(),
                        _ => json_value_text(candidate)
                            .into_iter()
                            .flat_map(|item| {
                                item.split([';', ',', '，', '、'])
                                    .map(str::trim)
                                    .filter(|item| !item.is_empty())
                                    .map(str::to_string)
                                    .collect::<Vec<_>>()
                            })
                            .collect::<Vec<_>>(),
                    };
                    if !values.is_empty() {
                        return Some(values);
                    }
                }
            }
            object
                .values()
                .find_map(|child| json_find_array_or_text(child, keys))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|child| json_find_array_or_text(child, keys)),
        _ => None,
    }
}

fn json_value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.trim().to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Array(values) => {
            let text = values
                .iter()
                .filter_map(json_value_text)
                .filter(|item| !item.trim().is_empty())
                .collect::<Vec<_>>();
            (!text.is_empty()).then(|| text.join(", "))
        }
        serde_json::Value::Object(object) => object
            .get("lyric")
            .or_else(|| object.get("text"))
            .or_else(|| object.get("content"))
            .and_then(json_value_text),
        serde_json::Value::Null | serde_json::Value::Bool(_) => None,
    }
}

fn strip_lrc_timestamps(value: &str) -> String {
    let mut output = String::new();
    for line in value.lines() {
        let mut rest = line;
        loop {
            let Some(close) = rest.strip_prefix('[').and_then(|text| text.find(']')) else {
                break;
            };
            // `close` is measured in the string after the leading `[`, so
            // skip both the leading bracket and the closing bracket.
            rest = &rest[close + 2..];
        }
        let rest = rest.trim();
        if !rest.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(rest);
        }
    }
    output
}

fn json_scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn json_artist_text(value: &serde_json::Value) -> Option<String> {
    if let Some(name) = value.get("name").and_then(json_scalar_text) {
        return Some(name);
    }
    if let Some(artists) = value.as_array() {
        let names = artists
            .iter()
            .filter_map(|artist| {
                artist
                    .get("name")
                    .and_then(json_scalar_text)
                    .or_else(|| {
                        artist
                            .as_array()
                            .and_then(|artist| artist.first())
                            .and_then(json_scalar_text)
                    })
                    .or_else(|| json_scalar_text(artist))
            })
            .filter(|name| !name.trim().is_empty())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            return Some(names.join(", "));
        }
    }
    json_scalar_text(value)
}

fn merge_track_metadata(records: &mut Vec<NeteaseRecord>, metadata_records: Vec<NeteaseRecord>) {
    for mut metadata in metadata_records {
        let index = if metadata.track_id.trim().is_empty() {
            None
        } else {
            records
                .iter()
                .position(|record| record.track_id == metadata.track_id)
        }
        .or_else(|| {
            if metadata.title.trim().is_empty() || metadata.artist.trim().is_empty() {
                None
            } else {
                records
                    .iter()
                    .position(|record| same_track_identity(record, &metadata))
            }
        });

        let Some(index) = index else {
            // Some client versions only retain web_track JSON and do not
            // create a corresponding row in track/web_offline_track. Keep a
            // conservative, pathless candidate so a downloaded
            // "title - artist" file can still resolve its local meta cover.
            if !metadata.title.trim().is_empty() && !metadata.artist.trim().is_empty() {
                metadata.file_name = format!("{} - {}", metadata.title, metadata.artist);
                records.push(metadata);
            }
            continue;
        };
        let record = &mut records[index];
        // `web_track` is the canonical track payload.  Older download rows
        // can contain only the first artist (for example `Tyla`), while the
        // JSON payload contains the complete collaboration (`Tyla, Zara
        // Larsson`).  Prefer the richer value so ordinary MP3/FLAC files
        // receive the same identity as an NCM decode.
        prefer_track_text(&mut record.title, &metadata.title, false);
        prefer_track_text(&mut record.artist, &metadata.artist, true);
        prefer_track_text(&mut record.album, &metadata.album, false);
        if record.track_id.trim().is_empty() {
            record.track_id = metadata.track_id.clone();
        }
        if record.album_id.trim().is_empty() {
            record.album_id = metadata.album_id.clone();
        }
        if record.cover_path.trim().is_empty() {
            record.cover_path = metadata.cover_path.clone();
        }
        if record.cover_data.is_none() {
            record.cover_data = metadata.cover_data.clone();
        }
        prefer_track_text(&mut record.genre, &metadata.genre, false);
        prefer_track_text(&mut record.aliases_json, &metadata.aliases_json, false);
        prefer_track_text(&mut record.copyright_text, &metadata.copyright_text, false);
        prefer_track_text(&mut record.publish_date, &metadata.publish_date, false);
        prefer_track_text(
            &mut record.lyric_plain_text,
            &metadata.lyric_plain_text,
            false,
        );
        prefer_track_text(
            &mut record.lyric_translated_text,
            &metadata.lyric_translated_text,
            false,
        );
        prefer_track_text(
            &mut record.lyric_romanized_text,
            &metadata.lyric_romanized_text,
            false,
        );
        prefer_track_text(&mut record.lyric_lrc_text, &metadata.lyric_lrc_text, false);
        prefer_track_text(&mut record.lyric_language, &metadata.lyric_language, false);
        prefer_track_text(
            &mut record.lyric_sync_type,
            &metadata.lyric_sync_type,
            false,
        );
        prefer_track_text(&mut record.lyric_source, &metadata.lyric_source, false);
        record
            .cover_references
            .extend(metadata.cover_references.iter().cloned());
        record.cover_references.sort();
        record.cover_references.dedup();
    }
}

fn prefer_track_text(existing: &mut String, candidate: &str, artist: bool) {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return;
    }

    let replace = existing.trim().is_empty()
        || (artist && artist_name_count(candidate) > artist_name_count(existing));
    if replace {
        *existing = candidate.to_string();
    }
}

fn artist_name_count(value: &str) -> usize {
    value
        .split([',', '，', '、', ';', '；', '&', '/'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .count()
}

fn same_track_identity(left: &NeteaseRecord, right: &NeteaseRecord) -> bool {
    match (
        non_empty_string(&left.track_id),
        non_empty_string(&right.track_id),
    ) {
        (Some(left_id), Some(right_id)) => return left_id == right_id,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => {}
    }
    tolerant_comparison_key(&left.title) == tolerant_comparison_key(&right.title)
        && tolerant_comparison_key(&left.artist) == tolerant_comparison_key(&right.artist)
        && (!left.album.trim().is_empty()
            && !right.album.trim().is_empty()
            && tolerant_comparison_key(&left.album) == tolerant_comparison_key(&right.album))
}

fn combine_path(path: &str, directory: &str) -> String {
    if path.trim().is_empty() {
        return directory.trim().to_string();
    }
    if directory.trim().is_empty()
        || Path::new(&path.replace('\\', "/")).is_absolute()
        || path.contains('/')
        || path.contains('\\')
    {
        return path.trim().to_string();
    }
    Path::new(directory.trim())
        .join(path.trim())
        .display()
        .to_string()
}

fn row_text(row: &Row<'_>, index: usize) -> String {
    row.get_ref(index)
        .ok()
        .and_then(|value| match value {
            ValueRef::Text(bytes) => String::from_utf8(bytes.to_vec()).ok(),
            ValueRef::Integer(value) => Some(value.to_string()),
            ValueRef::Real(value) => Some(value.to_string()),
            ValueRef::Blob(_) | ValueRef::Null => None,
        })
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn row_u64(row: &Row<'_>, index: usize) -> Option<u64> {
    row.get_ref(index).ok().and_then(|value| match value {
        ValueRef::Integer(value) => u64::try_from(value).ok(),
        ValueRef::Real(value) if value.is_finite() && value >= 0.0 => Some(value as u64),
        ValueRef::Real(_) => None,
        ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).trim().parse().ok(),
        ValueRef::Blob(_) | ValueRef::Null => None,
    })
}

fn row_blob(row: &Row<'_>, index: usize) -> Option<Vec<u8>> {
    row.get_ref(index).ok().and_then(|value| match value {
        ValueRef::Blob(bytes) => Some(bytes.to_vec()),
        _ => None,
    })
}

pub(crate) fn choose_record<'a>(
    source_path: &Path,
    records: &'a [NeteaseRecord],
) -> Option<&'a NeteaseRecord> {
    match choose_record_with_method(source_path, records) {
        RecordMatch::Matched { record, .. } => Some(record),
        RecordMatch::NoMatch | RecordMatch::Ambiguous { .. } => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordMatch<'a> {
    Matched {
        record: &'a NeteaseRecord,
        method: NeteaseRecordMatchMethod,
    },
    NoMatch,
    Ambiguous {
        candidates: usize,
    },
}

fn choose_record_with_method<'a>(
    source_path: &Path,
    records: &'a [NeteaseRecord],
) -> RecordMatch<'a> {
    let source_size = fs::metadata(source_path)
        .ok()
        .map(|metadata| metadata.len());
    choose_record_with_method_and_size(source_path, records, source_size)
}

fn choose_record_with_method_and_size<'a>(
    source_path: &Path,
    records: &'a [NeteaseRecord],
    source_size: Option<u64>,
) -> RecordMatch<'a> {
    let mut ranked = records
        .iter()
        .map(|record| {
            let (score, method) = record_match_evidence_with_size(source_path, record, source_size);
            (score, method, record)
        })
        .filter(|(score, _, _)| *score >= 500)
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.cmp(&left.0));

    let Some((best_score, method, best)) = ranked.first().copied() else {
        return RecordMatch::NoMatch;
    };
    // A filename by itself is not enough: a user can have the same track in
    // multiple folders, and NetEase's database may retain stale entries.
    // Accept an exact/suffix path, a filename plus file size, or a filename
    // whose title/artist pair also agrees (scores 780+).
    if best_score < 780 {
        return RecordMatch::NoMatch;
    }
    let ambiguous = ranked
        .iter()
        .skip(1)
        .filter(|(score, _, other)| *score == best_score && !same_record_identity(best, other))
        .count();
    if ambiguous > 0 {
        return RecordMatch::Ambiguous {
            candidates: ambiguous + 1,
        };
    }
    RecordMatch::Matched {
        record: best,
        method,
    }
}

fn choose_record_with_method_cancellable<'a>(
    source_path: &Path,
    records: &'a [NeteaseRecord],
    source_size: Option<u64>,
    cancel: &AtomicBool,
) -> Option<RecordMatch<'a>> {
    // The normal recovery path canonicalizes both sides of a path because it
    // runs for a single file. A scan can compare hundreds of records for each
    // file, so avoid a filesystem canonicalize syscall per candidate. Exact
    // and suffix matching still work for the lexical normalized path, while
    // filename/size/identity evidence remains the conservative fallback.
    let source_key = normalized_path_for_scan(source_path.to_string_lossy().as_ref());
    let source_name = normalized_file_name(source_path.to_string_lossy().as_ref());
    let mut ranked = Vec::new();
    for record in records {
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        let (score, method) = record_match_evidence_for_scan(
            source_path,
            record,
            source_size,
            &source_key,
            &source_name,
        );
        if score >= 500 {
            ranked.push((score, method, record));
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return None;
    }
    ranked.sort_by(|left, right| right.0.cmp(&left.0));
    let Some((best_score, method, best)) = ranked.first().copied() else {
        return Some(RecordMatch::NoMatch);
    };
    if best_score < 780 {
        return Some(RecordMatch::NoMatch);
    }
    let ambiguous = ranked
        .iter()
        .skip(1)
        .filter(|(score, _, other)| *score == best_score && !same_record_identity(best, other))
        .count();
    if ambiguous > 0 {
        return Some(RecordMatch::Ambiguous {
            candidates: ambiguous + 1,
        });
    }
    Some(RecordMatch::Matched {
        record: best,
        method,
    })
}

fn choose_locator_with_method_cancellable(
    source_path: &Path,
    locators: &[NeteaseTrackLocator],
    index: &LocatorIndex,
    source_size: Option<u64>,
    cancel: &AtomicBool,
) -> Option<LocatorMatch> {
    if cancel.load(Ordering::SeqCst) {
        return None;
    }
    let source_key = normalized_path_for_scan(source_path.to_string_lossy().as_ref());
    let source_name = normalized_file_name(source_path.to_string_lossy().as_ref());
    let source_stem = file_stem_key(&source_name);
    let mut candidate_indexes = HashSet::new();
    for position in index
        .by_path
        .get(&source_key)
        .into_iter()
        .flatten()
        .chain(index.by_file_name.get(&source_name).into_iter().flatten())
        .chain(index.by_stem.get(&source_stem).into_iter().flatten())
    {
        candidate_indexes.insert(*position);
    }
    if candidate_indexes.is_empty() {
        return None;
    }

    let mut ranked = Vec::with_capacity(candidate_indexes.len());
    for index in candidate_indexes {
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        let locator = locators.get(index)?;
        let (score, method) = locator_match_evidence_for_scan(
            source_path,
            locator,
            source_size,
            &source_key,
            &source_name,
        );
        if score >= 500 {
            ranked.push((score, method, index));
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return None;
    }
    ranked.sort_by(|left, right| right.0.cmp(&left.0));
    let (best_score, _method, best_index) = ranked.first().copied()?;
    if best_score < 780 {
        return None;
    }
    let best = locators.get(best_index)?;
    let ambiguous = ranked.iter().skip(1).any(|(score, _, index)| {
        *score == best_score
            && locators
                .get(*index)
                .is_some_and(|other| !same_locator_identity(best, other))
    });
    if ambiguous {
        return None;
    }
    Some(LocatorMatch { index: best_index })
}

fn locator_match_evidence_for_scan(
    source_path: &Path,
    locator: &NeteaseTrackLocator,
    source_size: Option<u64>,
    source_key: &str,
    source_name: &str,
) -> (u32, NeteaseRecordMatchMethod) {
    let mut score = 0;
    let exact_path = !locator.normalized_path.is_empty() && locator.normalized_path == source_key;
    let suffix_path = !locator.normalized_path.is_empty()
        && !source_key.is_empty()
        && (source_key.ends_with(&format!("/{}", locator.normalized_path))
            || locator.normalized_path.ends_with(&format!("/{source_key}")));
    let same_name = !source_name.is_empty() && source_name == locator.normalized_file_name;
    let same_stem = same_file_stem(source_name, &locator.normalized_file_name);
    let same_size = source_size.is_some() && source_size == locator.size_bytes;
    let filename_identity = split_filename_parts(source_path).is_some_and(|(left, right)| {
        let left = tolerant_comparison_key(&left);
        let right = tolerant_comparison_key(&right);
        let title = tolerant_comparison_key(&locator.title_key);
        let artist = tolerant_comparison_key(&locator.artist_key);
        !locator.title_key.is_empty()
            && !locator.artist_key.is_empty()
            && ((title == left && artist == right) || (title == right && artist == left))
    });

    if exact_path {
        score += 1000;
    } else if suffix_path {
        score += 820;
    }
    if same_name || same_stem {
        score += 600;
    }
    if same_size {
        score += 220;
    }
    if filename_identity {
        score += 180;
    }

    let method = if exact_path {
        NeteaseRecordMatchMethod::ExactPath
    } else if suffix_path {
        NeteaseRecordMatchMethod::PathSuffix
    } else if (same_name || same_stem) && same_size {
        NeteaseRecordMatchMethod::FileNameAndSize
    } else if (same_name || same_stem) && filename_identity {
        NeteaseRecordMatchMethod::FileNameAndIdentity
    } else {
        NeteaseRecordMatchMethod::NoMatch
    };
    (score, method)
}

fn same_locator_identity(left: &NeteaseTrackLocator, right: &NeteaseTrackLocator) -> bool {
    match (
        non_empty_string(&left.track_id),
        non_empty_string(&right.track_id),
    ) {
        (Some(left_id), Some(right_id)) => return left_id == right_id,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => {}
    }
    tolerant_comparison_key(&left.title_key) == tolerant_comparison_key(&right.title_key)
        && tolerant_comparison_key(&left.artist_key) == tolerant_comparison_key(&right.artist_key)
        && tolerant_comparison_key(&left.album_key) == tolerant_comparison_key(&right.album_key)
}

fn record_match_evidence_for_scan(
    source_path: &Path,
    record: &NeteaseRecord,
    source_size: Option<u64>,
    source_key: &str,
    source_name: &str,
) -> (u32, NeteaseRecordMatchMethod) {
    let record_key = normalized_path_for_scan(&record.path);
    let record_name = normalized_file_name(if record.file_name.is_empty() {
        &record.path
    } else {
        &record.file_name
    });
    let mut score = 0;
    let exact_path = !record_key.is_empty() && record_key == source_key;
    let suffix_path = !record_key.is_empty()
        && !source_key.is_empty()
        && (source_key.ends_with(&format!("/{record_key}"))
            || record_key.ends_with(&format!("/{source_key}")));
    let same_name = !source_name.is_empty() && source_name == record_name;
    let same_stem = same_file_stem(source_name, &record_name);
    let same_size = source_size.is_some() && source_size == record.size_bytes;
    let filename_identity = split_filename_parts(source_path).is_some_and(|(left, right)| {
        let left = tolerant_comparison_key(&left);
        let right = tolerant_comparison_key(&right);
        let title = tolerant_comparison_key(&record.title);
        let artist = tolerant_comparison_key(&record.artist);
        !title.is_empty()
            && !artist.is_empty()
            && ((title == left && artist == right) || (title == right && artist == left))
    });

    if exact_path {
        score += 1000;
    } else if suffix_path {
        score += 820;
    }
    if same_name || same_stem {
        score += 600;
    }
    if same_size {
        score += 220;
    }
    if filename_identity {
        score += 180;
    }

    let method = if exact_path {
        NeteaseRecordMatchMethod::ExactPath
    } else if suffix_path {
        NeteaseRecordMatchMethod::PathSuffix
    } else if (same_name || same_stem) && same_size {
        NeteaseRecordMatchMethod::FileNameAndSize
    } else if (same_name || same_stem) && filename_identity {
        NeteaseRecordMatchMethod::FileNameAndIdentity
    } else {
        NeteaseRecordMatchMethod::NoMatch
    };
    (score, method)
}

#[cfg_attr(not(test), allow(dead_code))]
fn record_match_score(source_path: &Path, record: &NeteaseRecord) -> u32 {
    record_match_evidence(source_path, record).0
}

fn record_match_evidence(
    source_path: &Path,
    record: &NeteaseRecord,
) -> (u32, NeteaseRecordMatchMethod) {
    let source_size = fs::metadata(source_path)
        .ok()
        .map(|metadata| metadata.len());
    record_match_evidence_with_size(source_path, record, source_size)
}

fn record_match_evidence_with_size(
    source_path: &Path,
    record: &NeteaseRecord,
    source_size: Option<u64>,
) -> (u32, NeteaseRecordMatchMethod) {
    let source_key = normalized_path(source_path.to_string_lossy().as_ref());
    let record_key = normalized_path(&record.path);
    let source_name = normalized_file_name(source_path.to_string_lossy().as_ref());
    let record_name = normalized_file_name(if record.file_name.is_empty() {
        &record.path
    } else {
        &record.file_name
    });
    let mut score = 0;
    let exact_path = !record_key.is_empty() && record_key == source_key;
    let suffix_path = !record_key.is_empty()
        && !source_key.is_empty()
        && (source_key.ends_with(&format!("/{record_key}"))
            || record_key.ends_with(&format!("/{source_key}")));
    let same_name = !source_name.is_empty() && source_name == record_name;
    let same_stem = same_file_stem(&source_name, &record_name);
    let same_size = source_size.is_some() && source_size == record.size_bytes;
    let filename_identity = split_filename_parts(source_path).is_some_and(|(left, right)| {
        let left = tolerant_comparison_key(&left);
        let right = tolerant_comparison_key(&right);
        let title = tolerant_comparison_key(&record.title);
        let artist = tolerant_comparison_key(&record.artist);
        !title.is_empty()
            && !artist.is_empty()
            && ((title == left && artist == right) || (title == right && artist == left))
    });

    if exact_path {
        score += 1000;
    } else if suffix_path {
        score += 820;
    }
    if same_name {
        score += 600;
    } else if same_stem {
        // NetEase may retain the original .ncm name in its database after the
        // user has exported/decrypted the same track to .mp3 or .flac.
        score += 600;
    }
    if same_size {
        score += 220;
    }

    if filename_identity {
        score += 180;
    }

    let method = if exact_path {
        NeteaseRecordMatchMethod::ExactPath
    } else if suffix_path {
        NeteaseRecordMatchMethod::PathSuffix
    } else if (same_name || same_stem) && same_size {
        NeteaseRecordMatchMethod::FileNameAndSize
    } else if (same_name || same_stem) && filename_identity {
        NeteaseRecordMatchMethod::FileNameAndIdentity
    } else {
        NeteaseRecordMatchMethod::NoMatch
    };
    (score, method)
}

fn same_record_identity(left: &NeteaseRecord, right: &NeteaseRecord) -> bool {
    match (
        non_empty_string(&left.track_id),
        non_empty_string(&right.track_id),
    ) {
        (Some(left_id), Some(right_id)) => return left_id == right_id,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => {}
    }
    tolerant_comparison_key(&left.title) == tolerant_comparison_key(&right.title)
        && tolerant_comparison_key(&left.artist) == tolerant_comparison_key(&right.artist)
        && tolerant_comparison_key(&left.album) == tolerant_comparison_key(&right.album)
}

fn normalized_path(value: &str) -> String {
    let value = value.trim().replace('\\', "/");
    let value = value.trim_end_matches('/');
    if value.is_empty() {
        return String::new();
    }
    fs::canonicalize(value)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| value.to_string())
        .to_lowercase()
}

fn normalized_path_for_scan(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

fn normalized_file_name(value: &str) -> String {
    Path::new(&value.replace('\\', "/"))
        .file_name()
        .and_then(|name| name.to_str())
        .map(tolerant_comparison_key)
        .unwrap_or_default()
}

fn same_file_stem(left: &str, right: &str) -> bool {
    let left = file_stem_key(left);
    let right = file_stem_key(right);
    !left.is_empty() && left == right
}

fn file_stem_key(value: &str) -> String {
    let value = value.replace('\\', "/");
    let value = value.rsplit('/').next().unwrap_or(&value);
    let lower = value.to_ascii_lowercase();
    let stem = [".ncm", ".mp3", ".flac", ".wav", ".aiff", ".aif"]
        .iter()
        .find_map(|extension| {
            lower
                .ends_with(extension)
                .then(|| &value[..value.len().saturating_sub(extension.len())])
        })
        .unwrap_or(value);
    tolerant_comparison_key(stem)
}

/// Key persisted in the lightweight locator cache. It only trims the
/// outside and folds case; punctuation and internal spaces stay intact.
pub(crate) fn persistent_metadata_key(value: &str) -> String {
    value.trim().to_lowercase()
}

/// Runtime-only comparison key. This preserves the historical tolerant
/// matching behavior and must never be persisted or written to tags.
pub(crate) fn tolerant_comparison_key(value: &str) -> String {
    value
        // NetEase filenames and the web-track JSON frequently disagree only
        // on typographic punctuation. Treat those representations as the
        // same matching key, while keeping the original database values for
        // the tags and output identity.
        .replace('＂', "\"")
        .replace(['“', '”'], "\"")
        .replace(['＇', '‘', '’'], "'")
        .replace(['，', '、', '；'], ",")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(", ", ",")
        .replace(" ,", ",")
        .trim()
        .to_lowercase()
}

fn split_filename_parts(path: &Path) -> Option<(String, String)> {
    let stem = path.file_stem()?.to_str()?.trim();
    let (left, right) = stem.split_once(" - ")?;
    let left = left.trim();
    let right = right.trim();
    (!left.is_empty() && !right.is_empty()).then(|| (left.to_string(), right.to_string()))
}

fn embedded_cover_for_path(path: &Path) -> Option<Vec<u8>> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "flac" => metaflac::Tag::read_from_path(path).ok().and_then(|tag| {
            tag.pictures()
                .map(|picture| picture.data.clone())
                .find(|bytes| is_supported_image(bytes))
        }),
        "mp3" | "aiff" | "aif" => id3::Tag::read_from_path(path).ok().and_then(|tag| {
            tag.pictures()
                .map(|picture| picture.data.clone())
                .find(|bytes| is_supported_image(bytes))
        }),
        _ => None,
    }
}

fn recover_cover_with_source(
    source_path: &Path,
    record: &NeteaseRecord,
) -> (Option<Vec<u8>>, NeteaseCoverSource) {
    if let Some(cover) = record.cover_data.as_deref() {
        if is_supported_image(cover) {
            return (Some(cover.to_vec()), NeteaseCoverSource::DatabaseBlob);
        }
        return (None, NeteaseCoverSource::Invalid);
    }

    let mut references = Vec::new();
    if !record.cover_path.trim().is_empty() {
        references.push(record.cover_path.trim().to_string());
    }
    references.extend(record.cover_references.iter().cloned());
    let has_remote_reference = references.iter().any(|value| is_http_url(value));
    for raw in references.iter().filter(|value| !is_http_url(value)) {
        let candidate = PathBuf::from(raw);
        let candidates = [
            candidate.clone(),
            source_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&candidate),
            home_dir().unwrap_or_default().join(&candidate),
        ];
        if let Some(cover) = candidates.iter().find_map(|path| read_cover_file(path)) {
            return (Some(cover), NeteaseCoverSource::ExplicitLocalPath);
        }
    }

    if let Some(cover) = find_source_directory_cover(source_path, Some(record)) {
        return (Some(cover), NeteaseCoverSource::LocalCache);
    }
    if let Some(cover) = find_cached_cover(record) {
        return (Some(cover), NeteaseCoverSource::LocalCache);
    }
    if has_remote_reference {
        (None, NeteaseCoverSource::RemoteOnly)
    } else {
        (None, NeteaseCoverSource::Missing)
    }
}

fn cover_failure_source(record: &NeteaseRecord) -> Option<NeteaseCoverSource> {
    if record.cover_data.is_some()
        && record
            .cover_data
            .as_deref()
            .is_none_or(|data| !is_supported_image(data))
    {
        return Some(NeteaseCoverSource::Invalid);
    }
    if record
        .cover_path
        .split_whitespace()
        .chain(record.cover_references.iter().map(String::as_str))
        .any(is_http_url)
    {
        Some(NeteaseCoverSource::RemoteOnly)
    } else {
        Some(NeteaseCoverSource::Missing)
    }
}

fn is_http_url(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://") || value.starts_with("https://")
}

pub(crate) fn cover_from_record(source_path: &Path, record: &NeteaseRecord) -> Option<Vec<u8>> {
    if let Some(cover) = record
        .cover_data
        .as_deref()
        .filter(|bytes| is_supported_image(bytes))
    {
        return Some(cover.to_vec());
    }

    let mut references = Vec::new();
    if !record.cover_path.trim().is_empty() {
        references.push(record.cover_path.trim().to_string());
    }
    references.extend(record.cover_references.iter().cloned());

    for raw in references {
        let candidate = PathBuf::from(&raw);
        let candidates = [
            candidate.clone(),
            source_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&candidate),
            home_dir().unwrap_or_default().join(&candidate),
        ];
        if let Some(cover) = candidates.iter().find_map(|path| read_cover_file(path)) {
            return Some(cover);
        }
    }

    None
}

/// Resolve the same local-only cover candidates used by conversion metadata
/// recovery. The dashboard uses this to show a cover even when the SQLite row
/// only contains a track/album reference and the image lives in the client's
/// neighbouring `meta` cache.
pub fn recover_local_cover(source_path: &Path) -> Option<Vec<u8>> {
    let records = load_cached_records();
    let record = choose_record(source_path, &records);
    record
        .and_then(|record| cover_from_record(source_path, record))
        .or_else(|| find_source_directory_cover(source_path, record))
        .or_else(|| record.and_then(find_cached_cover))
}

fn cover_references_from_json(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };

    let mut references = Vec::new();
    collect_cover_references(&value, &mut references);
    references.sort();
    references.dedup();
    references
}

fn collect_cover_references(value: &serde_json::Value, references: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                let normalized_key = key.to_ascii_lowercase();
                if matches!(
                    normalized_key.as_str(),
                    "picurl"
                        | "pic"
                        | "coverurl"
                        | "cover_path"
                        | "albumcover"
                        | "album_cover"
                        | "albumpic"
                        | "albumpicdocid"
                        | "album_pic"
                        | "album_pic_doc_id"
                        | "picture"
                        | "image"
                ) && let Some(reference) = json_scalar_text(value)
                    && !reference.trim().is_empty()
                {
                    references.push(reference.trim().to_string());
                }
                collect_cover_references(value, references);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_cover_references(value, references);
            }
        }
        _ => {}
    }
}

fn cover_reference_names(raw: &str) -> Vec<String> {
    let mut names = Vec::new();
    let value = raw
        .trim()
        .trim_matches('"')
        .strip_prefix("file://")
        .unwrap_or(raw.trim().trim_matches('"'))
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    if let Some(name) = Path::new(&value.replace('\\', "/"))
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
    {
        names.push(name.to_string());
    }
    names
}

fn find_cached_cover(record: &NeteaseRecord) -> Option<Vec<u8>> {
    let mut names = cover_reference_names(&record.cover_path);
    for reference in &record.cover_references {
        names.extend(cover_reference_names(reference));
    }
    for id in [record.track_id.as_str(), record.album_id.as_str()] {
        append_track_cover_names(&mut names, id);
    }
    names.sort();
    names.dedup();
    find_cover_by_name_in_cache(&names)
}

fn find_cover_by_name_in_cache(names: &[String]) -> Option<Vec<u8>> {
    let roots = netease_cover_cache_roots();
    find_cover_by_name_in_roots_direct(names, &roots).or_else(|| {
        let recursive_roots = roots
            .iter()
            .filter(|root| is_likely_cover_cache_root(root))
            .cloned()
            .collect::<Vec<_>>();
        find_cover_by_name_in_roots_recursive(names, &recursive_roots)
    })
}

fn find_cover_by_name_in_roots(names: &[String], roots: &[PathBuf]) -> Option<Vec<u8>> {
    find_cover_by_name_in_roots_direct(names, roots)
        .or_else(|| find_cover_by_name_in_roots_recursive(names, roots))
}

fn find_cover_by_name_in_roots_direct(names: &[String], roots: &[PathBuf]) -> Option<Vec<u8>> {
    if names.is_empty() {
        return None;
    }

    for root in roots {
        for name in names {
            let candidate = root.join(name);
            if let Some(cover) = read_cover_file(&candidate) {
                return Some(cover);
            }
        }
    }
    None
}

fn find_cover_by_name_in_roots_recursive(names: &[String], roots: &[PathBuf]) -> Option<Vec<u8>> {
    if names.is_empty() {
        return None;
    }
    let names = names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(6)
            .into_iter()
            .filter_map(Result::ok)
            .take(20_000)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(file_name) = entry.file_name().to_str() else {
                continue;
            };
            if names.contains(&file_name.to_ascii_lowercase())
                && let Some(cover) = read_cover_file(entry.path())
            {
                return Some(cover);
            }
        }
    }
    None
}

fn is_likely_cover_cache_root(root: &Path) -> bool {
    if let Some(explicit_root) =
        env::var_os(NETEASE_COVER_DIR_ENV).filter(|value| !value.is_empty())
    {
        let explicit_root = PathBuf::from(explicit_root);
        if root.starts_with(&explicit_root) {
            return true;
        }
    }
    root.components().any(|component| {
        let component = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        [
            "cache", "cover", "image", "picture", "artwork", "album", "meta",
        ]
        .iter()
        .any(|word| component.contains(word))
    })
}

fn netease_cover_cache_roots() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(path) = env::var_os(NETEASE_COVER_DIR_ENV).filter(|value| !value.is_empty()) {
        bases.push(PathBuf::from(path));
    }

    for database in database_candidates() {
        let Some(storage) = database.parent() else {
            continue;
        };
        bases.push(storage.to_path_buf());
        if let Some(documents) = storage.parent() {
            bases.push(documents.to_path_buf());
            if let Some(data) = documents.parent() {
                bases.push(data.join("Caches"));
                bases.push(data.join("Library/Caches"));
            }
        }
    }

    if let Some(home) = home_dir() {
        #[cfg(target_os = "macos")]
        {
            let data = home
                .join("Library/Containers")
                .join(NETEASE_CONTAINER)
                .join("Data");
            bases.push(data.join("Caches"));
            bases.push(data.join("Documents"));
            bases.push(home.join("Library/Caches").join(NETEASE_CONTAINER));
            bases.push(home.join("Library/Application Support/Netease Cloud Music"));
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
                let local_app_data = PathBuf::from(local_app_data);
                bases.push(local_app_data.join("Netease/CloudMusic/Cache"));
                bases.push(local_app_data.join("Netease/CloudMusic/Cache/Cache"));
                bases.push(local_app_data.join("NetEase/CloudMusic/Cache"));
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            bases.push(home.join(".cache/netease-cloud-music"));
            bases.push(home.join(".local/share/netease-cloud-music"));
        }
    }

    let mut roots = Vec::new();
    for base in bases {
        for suffix in [
            "",
            "cover",
            "covers",
            "album",
            "album_cover",
            "image",
            "images",
            "picture",
            "pictures",
            "artwork",
            "artworks",
            "cache",
            "Cache",
            "Cache/Cache",
        ] {
            let root = if suffix.is_empty() {
                base.clone()
            } else {
                base.join(suffix)
            };
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

fn find_source_directory_cover(
    source_path: &Path,
    record: Option<&NeteaseRecord>,
) -> Option<Vec<u8>> {
    let parent = source_path.parent().unwrap_or_else(|| Path::new("."));
    let mut names = Vec::new();

    if let Some(record) = record {
        if !record.cover_path.trim().is_empty() {
            names.extend(cover_reference_names(&record.cover_path));
        }
        for reference in &record.cover_references {
            names.extend(cover_reference_names(reference));
        }
        for id in [record.track_id.as_str(), record.album_id.as_str()] {
            append_track_cover_names(&mut names, id);
        }
    }

    if let Some(stem) = source_path.file_stem().and_then(|value| value.to_str()) {
        append_image_names(&mut names, stem);
    }
    for base in ["cover", "folder", "album", "front"] {
        append_image_names(&mut names, base);
    }
    names.sort();
    names.dedup();

    let meta_roots = parent
        .ancestors()
        .take(5)
        .map(|directory| directory.join("meta"))
        .collect::<Vec<_>>();
    if let Some(cover) = find_cover_by_name_in_roots(&names, &meta_roots) {
        return Some(cover);
    }

    // A few client versions put one generated image in the meta directory
    // without a usable filename. It is safe to use it only when the directory
    // has a single image; never guess among several album covers.
    for meta in meta_roots {
        if let Some(cover) = find_single_image_in_directory(&meta) {
            return Some(cover);
        }
    }

    record.and_then(|record| find_adjacent_cover(source_path, record))
}

fn append_image_names(names: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let normalized = value.replace('\\', "/");
    let base = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
        .trim();
    if base.is_empty() {
        return;
    }
    names.push(base.to_string());
    if Path::new(base).extension().is_none() {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            names.push(format!("{base}.{extension}"));
        }
    }
}

fn append_track_cover_names(names: &mut Vec<String>, id: &str) {
    let id = id.trim();
    if id.is_empty() {
        return;
    }

    append_image_names(names, id);
    if !id.to_ascii_lowercase().starts_with("offline-") {
        append_image_names(names, &format!("offline-{id}"));
    }
}

fn find_single_image_in_directory(directory: &Path) -> Option<Vec<u8>> {
    if !directory.is_dir() {
        return None;
    }
    let images = WalkDir::new(directory)
        .follow_links(false)
        .max_depth(4)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| read_cover_file(entry.path()))
        .take(2)
        .collect::<Vec<_>>();
    (images.len() == 1)
        .then(|| images.into_iter().next())
        .flatten()
}

fn find_adjacent_cover(source_path: &Path, record: &NeteaseRecord) -> Option<Vec<u8>> {
    let parent = source_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let mut names = Vec::new();
    for base in ["cover", "folder", "album", "front"] {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            names.push(format!("{base}.{extension}"));
        }
    }
    if !stem.trim().is_empty() {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            names.push(format!("{stem}.{extension}"));
        }
    }
    for id in [record.track_id.as_str(), record.album_id.as_str()] {
        if !id.trim().is_empty() {
            for prefix in ["cover_", "album_", ""] {
                for extension in ["jpg", "jpeg", "png", "webp"] {
                    names.push(format!("{prefix}{id}.{extension}"));
                }
            }
        }
    }

    let mut candidates = Vec::new();
    for name in names {
        let candidate = parent.join(&name);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    if let Some(cover) = candidates.iter().find_map(|path| read_cover_file(path)) {
        return Some(cover);
    }

    // Some NetEase versions use a generated filename for the album art.  Do
    // not guess in a directory containing many images; a single image is a
    // safe folder-level cover fallback.
    let sibling_images = fs::read_dir(parent)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "jpg" | "jpeg" | "png" | "webp"
                    )
                })
        })
        .collect::<Vec<_>>();
    (sibling_images.len() == 1)
        .then(|| read_cover_file(&sibling_images[0]))
        .flatten()
}

fn read_cover_file(path: &Path) -> Option<Vec<u8>> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_COVER_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    is_supported_image(&bytes).then_some(bytes)
}

fn is_supported_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        || (bytes.len() >= 12
            && bytes.starts_with(b"RIFF")
            && bytes.get(8..12) == Some(b"WEBP".as_slice()))
        || bytes.starts_with(b"GIF8")
        || bytes.starts_with(b"BM")
}

#[cfg(test)]
mod tests {
    use super::{
        NeteaseCoverSource, NeteaseMetadataResolver, NeteaseRecord, NeteaseRecordMatchMethod,
        choose_record, choose_record_with_method, cover_from_record, cover_references_from_json,
        find_adjacent_cover, find_cover_by_name_in_roots, find_source_directory_cover,
        load_records_from_connection, persistent_metadata_key, record_match_score,
        tolerant_comparison_key,
    };
    use rusqlite::{Connection, params};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use tempfile::tempdir;

    #[test]
    fn persistent_and_tolerant_metadata_keys_are_separate() {
        let value = " Boogybytes,  Vol. 3 “Live” ";
        assert_eq!(persistent_metadata_key(value), "boogybytes,  vol. 3 “live”");
        assert_eq!(tolerant_comparison_key(value), "boogybytes,vol. 3 \"live\"");
    }

    #[test]
    fn recovery_diagnostic_uses_camel_case_and_reads_legacy_defaults() {
        let diagnostic = super::NeteaseRecoveryDiagnostic {
            database_path: Some("/tmp/sqlite_storage.sqlite3".into()),
            database_loaded: true,
            database_record_count: 3,
            matched: true,
            match_method: Some(NeteaseRecordMatchMethod::FileNameAndSize),
            track_id: Some("42".into()),
            album_id: Some("7".into()),
            cover_source: Some(NeteaseCoverSource::DatabaseBlob),
            cover_bytes: Some(12),
            message: None,
        };
        let json = serde_json::to_value(&diagnostic).unwrap();
        assert_eq!(json["databaseLoaded"], true);
        assert_eq!(json["databaseRecordCount"], 3);
        assert_eq!(json["matchMethod"], "fileNameAndSize");
        assert_eq!(json["coverSource"], "databaseBlob");
        assert!(json.get("database_loaded").is_none());

        let legacy: super::NeteaseRecoveryDiagnostic = serde_json::from_str("{}").unwrap();
        assert!(!legacy.database_loaded);
        assert_eq!(legacy.database_record_count, 0);
        assert_eq!(legacy.cover_source, None);
    }

    #[test]
    fn empty_resolver_reports_database_not_loaded_without_blocking_recovery() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        fs::write(&source, b"not a real flac").unwrap();
        let recovery = NeteaseMetadataResolver::default().recover(&source);
        assert!(!recovery.diagnostic.database_loaded);
        assert_eq!(
            recovery.diagnostic.match_method,
            Some(NeteaseRecordMatchMethod::NoMatch)
        );
        assert_eq!(
            recovery.diagnostic.message.as_deref(),
            Some("网易云数据库未加载")
        );
        assert!(recovery.metadata.is_none());
    }

    #[test]
    fn resolver_match_method_explains_size_and_rejects_ties() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        fs::write(&source, b"audio").unwrap();
        let records = vec![NeteaseRecord {
            file_name: String::from("Song - Artist.ncm"),
            title: String::from("Song"),
            artist: String::from("Artist"),
            ..NeteaseRecord::default()
        }];
        let result = choose_record_with_method(&source, &records);
        assert!(matches!(
            result,
            super::RecordMatch::Matched {
                method: NeteaseRecordMatchMethod::FileNameAndIdentity,
                ..
            }
        ));
    }

    #[test]
    fn cancellable_matching_keeps_conservative_filename_identity_rule() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        fs::write(&source, b"audio").unwrap();
        let records = vec![NeteaseRecord {
            file_name: String::from("Song - Artist.ncm"),
            title: String::from("Song"),
            artist: String::from("Artist"),
            ..NeteaseRecord::default()
        }];
        let cancel = AtomicBool::new(false);

        let result =
            super::choose_record_with_method_cancellable(&source, &records, Some(5), &cancel)
                .expect("matching should finish when cancellation is clear");

        assert!(matches!(
            result,
            super::RecordMatch::Matched {
                method: NeteaseRecordMatchMethod::FileNameAndIdentity,
                ..
            }
        ));
    }

    #[test]
    fn cancellable_matching_stops_before_scanning_when_cancelled() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        fs::write(&source, b"audio").unwrap();
        let records = vec![NeteaseRecord {
            file_name: String::from("Song - Artist.ncm"),
            title: String::from("Song"),
            artist: String::from("Artist"),
            ..NeteaseRecord::default()
        }];
        let cancel = AtomicBool::new(true);

        let result =
            super::choose_record_with_method_cancellable(&source, &records, Some(5), &cancel);

        assert!(result.is_none());
    }

    #[test]
    fn resolver_loads_supported_database_once_and_recovers_local_cover() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("sqlite_storage.sqlite3");
        let source = directory.path().join("Song - Artist.flac");
        let cover = directory.path().join("cover.jpg");
        fs::write(&source, b"audio").unwrap();
        fs::write(&cover, b"\xFF\xD8\xFF\xE0cover").unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (
                    file TEXT, title TEXT, artist TEXT, album TEXT,
                    tid TEXT, cover_path TEXT
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track(file, title, artist, album, tid, cover_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    source.to_string_lossy(),
                    "Song",
                    "Artist",
                    "Album",
                    "42",
                    cover.to_string_lossy(),
                ],
            )
            .unwrap();
        drop(connection);

        let (resolver, warning) =
            NeteaseMetadataResolver::load_with_warning(Some(&database)).unwrap();
        assert!(warning.is_none());
        assert!(resolver.database_loaded());
        assert_eq!(resolver.record_count(), 1);
        assert_eq!(resolver.database_path(), Some(database.as_path()));
        let recovery = resolver.recover(&source);
        assert_eq!(
            recovery.diagnostic.match_method,
            Some(NeteaseRecordMatchMethod::ExactPath)
        );
        assert_eq!(
            recovery.diagnostic.cover_source,
            Some(NeteaseCoverSource::ExplicitLocalPath)
        );
        assert_eq!(recovery.diagnostic.cover_bytes, Some(9));
        assert_eq!(
            recovery.metadata.as_ref().map(|value| value.title.as_str()),
            Some("Song")
        );
    }

    #[test]
    fn resolver_snapshot_is_read_only_and_survives_database_removal() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("sqlite_storage.sqlite3");
        let source = directory.path().join("Song - Artist.flac");
        fs::write(&source, b"audio").unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("CREATE TABLE track (file TEXT, title TEXT, artist TEXT);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO track(file, title, artist) VALUES (?1, ?2, ?3)",
                params![source.to_string_lossy(), "Song", "Artist"],
            )
            .unwrap();
        drop(connection);
        let before = fs::metadata(&database).unwrap();
        let modified_before = before.modified().unwrap();
        let size_before = before.len();
        let wal = PathBuf::from(format!("{}-wal", database.display()));
        let shm = PathBuf::from(format!("{}-shm", database.display()));
        assert!(!wal.exists());
        assert!(!shm.exists());
        let (resolver, warning) =
            NeteaseMetadataResolver::load_with_warning(Some(&database)).unwrap();
        assert!(warning.is_none());
        assert_eq!(fs::metadata(&database).unwrap().len(), size_before);
        assert_eq!(
            fs::metadata(&database).unwrap().modified().unwrap(),
            modified_before
        );
        assert!(!wal.exists());
        assert!(!shm.exists());
        fs::remove_file(&database).unwrap();
        let recovery = resolver.recover(&source);
        assert!(recovery.diagnostic.matched);
        assert_eq!(
            recovery.metadata.as_ref().map(|value| value.title.as_str()),
            Some("Song")
        );
    }

    #[test]
    fn invalid_preference_reports_fallback_warning_without_blocking_conversion() {
        let directory = tempdir().unwrap();
        let invalid = directory.path().join("missing.sqlite3");
        let (resolver, warning) =
            NeteaseMetadataResolver::load_with_warning(Some(&invalid)).unwrap();
        assert!(warning.is_some());
        assert_ne!(resolver.database_path(), Some(invalid.as_path()));
    }

    #[test]
    fn exact_resolver_rejects_unsupported_schema_without_fallback() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("wrong.sqlite3");
        Connection::open(&database)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (id INTEGER);")
            .unwrap();

        let error = NeteaseMetadataResolver::load_exact(&database).unwrap_err();
        assert!(error.to_string().contains("schema"));
    }

    #[test]
    fn remote_cover_reference_is_diagnosed_without_network_access() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song.flac");
        fs::write(&source, b"audio").unwrap();
        let record = NeteaseRecord {
            path: source.to_string_lossy().into_owned(),
            file_name: String::from("Song.flac"),
            title: String::from("Song"),
            artist: String::from("Artist"),
            cover_references: vec![String::from("https://p1.music.126.net/42.jpg")],
            ..NeteaseRecord::default()
        };
        let recovery = super::recover_with_records(
            &source,
            &[record],
            Some(Path::new("/readonly/sqlite_storage.sqlite3")),
            true,
        );
        assert_eq!(
            recovery.diagnostic.cover_source,
            Some(NeteaseCoverSource::RemoteOnly)
        );
        assert!(recovery.metadata.is_some());
    }

    #[test]
    fn reads_and_matches_an_exact_local_netease_track_path() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("download.flac");
        fs::write(&source, b"audio").unwrap();

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, dir TEXT, title TEXT, album TEXT, artist TEXT, duration INTEGER, filesize INTEGER, tid INTEGER, aid INTEGER);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track (file, dir, title, album, artist, duration, filesize, tid, aid) VALUES (?1, '', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![source.to_string_lossy(), "Song", "Album", "Artist", 180_000, 5, 42, 7],
            )
            .unwrap();

        let records = load_records_from_connection(&connection).unwrap();
        let record = choose_record(&source, &records).expect("the exact path should match");
        assert_eq!(record.title, "Song");
        assert_eq!(record.artist, "Artist");
        assert!(record_match_score(&source, record) >= 1000);
    }

    #[test]
    fn refuses_ambiguous_same_filename_records() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("同名歌曲.flac");
        fs::write(&source, b"audio").unwrap();
        let records = vec![
            NeteaseRecord {
                file_name: String::from("同名歌曲.flac"),
                title: String::from("Song A"),
                artist: String::from("Artist A"),
                ..NeteaseRecord::default()
            },
            NeteaseRecord {
                file_name: String::from("同名歌曲.flac"),
                title: String::from("Song B"),
                artist: String::from("Artist B"),
                ..NeteaseRecord::default()
            },
        ];

        assert!(choose_record(&source, &records).is_none());
    }

    #[test]
    fn refuses_a_filename_only_match_without_size_or_path_evidence() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song.flac");
        fs::write(&source, b"audio").unwrap();
        let records = vec![NeteaseRecord {
            file_name: String::from("Song.flac"),
            title: String::from("Song"),
            artist: String::from("Artist"),
            ..NeteaseRecord::default()
        }];

        assert!(choose_record(&source, &records).is_none());
    }

    #[test]
    fn accepts_a_filename_match_when_the_file_size_agrees() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song.flac");
        fs::write(&source, b"audio").unwrap();
        let records = vec![NeteaseRecord {
            file_name: String::from("Song.flac"),
            title: String::from("Song"),
            artist: String::from("Artist"),
            size_bytes: Some(5),
            ..NeteaseRecord::default()
        }];

        assert!(choose_record(&source, &records).is_some());
    }

    #[test]
    fn accepts_a_downloaded_extension_when_database_keeps_ncm_extension() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        fs::write(&source, b"audio").unwrap();
        let records = vec![NeteaseRecord {
            file_name: String::from("Song - Artist.ncm"),
            title: String::from("Song"),
            artist: String::from("Artist"),
            ..NeteaseRecord::default()
        }];

        assert!(choose_record(&source, &records).is_some());
    }

    #[test]
    fn accepts_an_image_blob_stored_in_the_cover_column() {
        let source = Path::new("/music/Song.flac");
        let cover = vec![0xFF, 0xD8, 0xFF, 0x00];
        let record = NeteaseRecord {
            cover_data: Some(cover.clone()),
            ..NeteaseRecord::default()
        };

        assert_eq!(cover_from_record(source, &record), Some(cover));
    }

    #[test]
    fn loads_an_image_blob_from_the_database_cover_column() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song.flac");
        fs::write(&source, b"audio").unwrap();
        let cover = vec![0xFF, 0xD8, 0xFF, 0x00];

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, tid INTEGER, aid INTEGER, cover BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track (file, title, artist, album, tid, aid, cover) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![source.to_string_lossy(), "Song", "Artist", "Album", 42, 7, cover.clone()],
            )
            .unwrap();

        let records = load_records_from_connection(&connection).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(cover_from_record(&source, &records[0]), Some(cover));
    }

    #[test]
    fn extracts_cover_references_from_download_detail_json() {
        let references = cover_references_from_json(
            r#"{"track":{"album":{"picUrl":"https://p1.music.126.net/42.jpg"}}}"#,
        );
        assert_eq!(
            references,
            vec![String::from("https://p1.music.126.net/42.jpg")]
        );
    }

    #[test]
    fn loads_cover_reference_from_download_detail_column() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE web_offline_track (relative_path TEXT, track_name TEXT, artist_name TEXT, album_name TEXT, track_id INTEGER, album_id INTEGER, detail TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO web_offline_track (relative_path, track_name, artist_name, album_name, track_id, album_id, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    "Song.flac",
                    "Song",
                    "Artist",
                    "Album",
                    42,
                    7,
                    r#"{"track":{"album":{"picUrl":"https://p1.music.126.net/42.jpg"}}}"#,
                ],
            )
            .unwrap();

        let records = load_records_from_connection(&connection).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].cover_references,
            vec![String::from("https://p1.music.126.net/42.jpg")]
        );
    }

    #[test]
    fn merges_web_track_cover_metadata_into_the_downloaded_file_record() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        fs::write(&source, b"audio").unwrap();

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, tid INTEGER);\
                 CREATE TABLE web_track (tid INTEGER, version INTEGER, track TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track (file, title, artist, album, tid) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![source.to_string_lossy(), "Song", "Artist", "Album", 42],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO web_track (tid, version, track) VALUES (?1, ?2, ?3)",
                params![
                    42,
                    1,
                    r#"{"id":42,"name":"Song","artists":[{"name":"Artist"}],"album":{"id":7,"name":"Album","picUrl":"https://p1.music.126.net/42.jpg"}}"#,
                ],
            )
            .unwrap();

        let records = load_records_from_connection(&connection).unwrap();
        let record = choose_record(&source, &records).expect("the downloaded file should match");
        assert_eq!(
            record.cover_references,
            vec![String::from("https://p1.music.126.net/42.jpg")]
        );
    }

    #[test]
    fn does_not_merge_same_text_from_different_track_ids() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, tid INTEGER);
                 CREATE TABLE web_track (tid INTEGER, version INTEGER, track TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO track(file,title,artist,album,tid) VALUES (?1,?2,?3,?4,?5)",
                params!["Same Song.mp3", "Same Song", "Same Artist", "Same Album", 1],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO web_track(tid,version,track) VALUES (?1,?2,?3)",
                params![
                    2,
                    1,
                    r#"{"id":2,"name":"Same Song","artists":[{"name":"Same Artist"}],"album":{"id":9,"name":"Same Album"}}"#,
                ],
            )
            .unwrap();

        let records = load_records_from_connection(&connection).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|record| record.track_id == "1"));
        assert!(records.iter().any(|record| record.track_id == "2"));
    }

    #[test]
    fn web_track_replaces_a_partial_artist_list_for_non_ncm_downloads() {
        let directory = tempdir().unwrap();
        let source = directory
            .path()
            .join("SHE DID IT AGAIN - Tyla,Zara Larsson.flac");
        fs::write(&source, b"audio").unwrap();

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE web_offline_track (relative_path TEXT, track_name TEXT, artist_name TEXT, album_name TEXT, track_id INTEGER, album_id INTEGER, size INTEGER, detail TEXT);\
                 CREATE TABLE web_track (tid INTEGER, version INTEGER, track TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO web_offline_track (relative_path, track_name, artist_name, album_name, track_id, album_id, size, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "/SHE DID IT AGAIN - Tyla,Zara Larsson.ncm",
                    "SHE DID IT AGAIN",
                    "Tyla",
                    "A*POP",
                    3409113568_i64,
                    388088002_i64,
                    8_532_680_i64,
                    "{}",
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO web_track (tid, version, track) VALUES (?1, ?2, ?3)",
                params![
                    3409113568_i64,
                    1_i64,
                    r#"{"id":3409113568,"name":"SHE DID IT AGAIN","artists":[{"name":"Tyla"},{"name":"Zara Larsson"}],"album":{"id":388088002,"name":"A*POP"}}"#,
                ],
            )
            .unwrap();

        let records = load_records_from_connection(&connection).unwrap();
        let record = choose_record(&source, &records)
            .expect("the canonical web_track identity should match the MP3/FLAC filename");
        assert_eq!(record.title, "SHE DID IT AGAIN");
        assert_eq!(record.artist, "Tyla, Zara Larsson");
        assert_eq!(record.album, "A*POP");
    }

    #[test]
    fn recovers_a_meta_cover_when_only_web_track_metadata_exists() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        let meta = directory.path().join("meta/42");
        fs::write(&source, b"audio").unwrap();
        fs::create_dir_all(&meta).unwrap();
        fs::write(meta.join("offline-42.jpg"), [0xFF, 0xD8, 0xFF, 0x00]).unwrap();
        fs::write(meta.join("unrelated.jpg"), [0xFF, 0xD8, 0xFF, 0x01]).unwrap();

        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE web_track (tid INTEGER, version INTEGER, track TEXT);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO web_track (tid, version, track) VALUES (?1, ?2, ?3)",
                params![
                    42,
                    1,
                    r#"{"id":42,"name":"Song","artists":[["Artist",99]],"album":{"id":7,"name":"Album","picUrl":"42.jpg"}}"#,
                ],
            )
            .unwrap();

        let records = load_records_from_connection(&connection).unwrap();
        let record = choose_record(&source, &records)
            .expect("a pathless web_track record should match the downloaded filename");
        assert_eq!(record.track_id, "42");
        assert_eq!(record.artist, "Artist");
        assert_eq!(
            find_source_directory_cover(&source, Some(record)),
            Some(vec![0xFF, 0xD8, 0xFF, 0x00])
        );
    }

    #[test]
    fn finds_cover_by_track_id_in_a_known_cache_root() {
        let directory = tempdir().unwrap();
        fs::create_dir_all(directory.path().join("covers")).unwrap();
        fs::write(
            directory.path().join("covers/42.jpg"),
            [0xFF, 0xD8, 0xFF, 0x00],
        )
        .unwrap();

        let cover = find_cover_by_name_in_roots(
            &[String::from("42.jpg")],
            &[directory.path().join("covers")],
        )
        .expect("the track id cover should be found");
        assert_eq!(cover, vec![0xFF, 0xD8, 0xFF, 0x00]);
    }

    #[test]
    fn finds_cover_by_explicit_name_in_a_nested_cache_directory() {
        let directory = tempdir().unwrap();
        let nested = directory.path().join("8a/42");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("42.jpg"), [0xFF, 0xD8, 0xFF, 0x00]).unwrap();

        let cover = find_cover_by_name_in_roots(
            &[String::from("42.jpg")],
            &[directory.path().to_path_buf()],
        )
        .expect("the explicitly named nested cover should be found");
        assert_eq!(cover, vec![0xFF, 0xD8, 0xFF, 0x00]);
    }

    #[test]
    fn finds_a_track_id_cover_in_the_users_netease_meta_directory() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song - Artist.flac");
        let meta = directory.path().join("meta/8a/42");
        fs::write(&source, b"audio").unwrap();
        fs::create_dir_all(&meta).unwrap();
        fs::write(meta.join("offline-42.jpg"), [0xFF, 0xD8, 0xFF, 0x00]).unwrap();
        fs::write(meta.join("unrelated.jpg"), [0xFF, 0xD8, 0xFF, 0x01]).unwrap();

        let record = NeteaseRecord {
            track_id: String::from("42"),
            album_id: String::from("7"),
            ..NeteaseRecord::default()
        };
        let cover = find_source_directory_cover(&source, Some(&record))
            .expect("the source directory meta cover should be found");
        assert_eq!(cover, vec![0xFF, 0xD8, 0xFF, 0x00]);
    }

    #[test]
    fn finds_only_explicit_neighbouring_cover_names() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song.flac");
        fs::write(&source, b"audio").unwrap();
        fs::write(directory.path().join("cover.jpg"), [0xFF, 0xD8, 0xFF, 0x00]).unwrap();
        fs::write(
            directory.path().join("unrelated.jpg"),
            [0xFF, 0xD8, 0xFF, 0x00],
        )
        .unwrap();

        let cover = find_adjacent_cover(&source, &NeteaseRecord::default())
            .expect("cover.jpg should be accepted");
        assert_eq!(cover, vec![0xFF, 0xD8, 0xFF, 0x00]);
    }

    #[test]
    fn accepts_a_single_generated_cover_in_the_music_folder() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("Song.flac");
        fs::write(&source, b"audio").unwrap();
        fs::write(
            directory.path().join("image_8c4a.jpg"),
            [0xFF, 0xD8, 0xFF, 0x00],
        )
        .unwrap();

        let cover = find_adjacent_cover(&source, &NeteaseRecord::default())
            .expect("the only image should be used");
        assert_eq!(cover, vec![0xFF, 0xD8, 0xFF, 0x00]);
    }
}
