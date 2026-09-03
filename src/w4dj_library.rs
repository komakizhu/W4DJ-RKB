//! Output-owned W4DJ song library.
//!
//! The older library_catalog module is kept as a compatibility query/DTO
//! layer. This module owns the new w4dj.sqlite3 lifecycle: rows are created
//! from successfully committed output files, not from NetEase database rows.

use crate::analysis::{TrackAnalysis, load_analysis_file, read_embedded_track_metadata};
use crate::dj_playlist::{
    DjPlaylistImportWarning, ImportedDjPlaylist, ImportedDjPlaylistSummary, ImportedDjPlaylistTrack,
};
use crate::dj_playlist_match::{
    DjOutputCandidate, DjPlaylistMatchKind, DjPlaylistMatchReport, DjPlaylistTrackMatch,
    candidate_filename, identity_key_for, match_imported_playlist_with_priority,
};
use crate::history::HistoryStatus;
use crate::library_catalog::{
    CatalogLocalFile, CatalogTrack, LibraryCatalog, LibraryError, LocalStatus,
};
use crate::library_query::{LibraryPage, LibraryQuery};
use crate::media_probe::probe_local_audio;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

pub const W4DJ_SCHEMA_VERSION: i64 = 4;
pub const OUTPUT_IDENTITY_MANIFEST_FILE_NAME: &str = ".w4dj-output-identities.json";
const OUTPUT_IDENTITY_MANIFEST_FORMAT: &str = "w4dj-output-identities";
const OUTPUT_IDENTITY_MANIFEST_VERSION: u32 = 1;
static OUTPUT_IDENTITY_MANIFEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommittedOutputFacts {
    pub source_size_bytes: Option<u64>,
    pub source_modified_at_ms: Option<u64>,
    pub conversion_mode: Option<String>,
    pub lossless_format: Option<String>,
    pub filename_rule: Option<String>,
    pub netease_filename_format: Option<String>,
    pub filename_normalization_policy: Option<String>,
    /// Identifier for the successful conversion operation that produced the
    /// output. It is provenance for playlist matching, never song identity.
    pub conversion_batch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedOutputBinding {
    pub source_path: String,
    pub destination_path: String,
    pub output_root: String,
    pub slot_index: usize,
    pub source_size_bytes: Option<u64>,
    pub source_modified_at_ms: Option<u64>,
    pub mode: Option<String>,
    pub lossless_format: Option<String>,
    pub filename_rule: Option<String>,
    pub netease_filename_format: Option<String>,
    pub filename_normalization_policy: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputReconcileSummary {
    pub invalidated_paths: Vec<PathBuf>,
    pub removed_paths: Vec<PathBuf>,
}

/// Durable provenance for output files. SQLite remains the fast, rebuildable
/// index; this small sidecar is materialized only by an explicit W4DJ playlist
/// operation and is the recovery source when that index is cleared or
/// recreated for that operation. It intentionally stores no audio hash and is
/// kept beside the output root so copying the output folder keeps the matching
/// identity.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct OutputIdentityManifest {
    format: String,
    format_version: u32,
    outputs: Vec<OutputIdentityManifestEntry>,
    #[serde(default)]
    playlists: Vec<OutputPlaylistManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct OutputIdentityManifestEntry {
    relative_path: String,
    #[serde(default)]
    source_path: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist_display: String,
}

/// Active playlist bindings live beside output identities so a rebuilt W4DJ
/// SQLite index can recover a user's export choices. This is a private sidecar
/// contract, not the public `.w4dj` protocol, and intentionally has no NetEase
/// identifiers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct OutputPlaylistManifest {
    playlist_id: String,
    tracks: Vec<OutputPlaylistManifestTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct OutputPlaylistManifestTrack {
    position: u64,
    relative_path: String,
    title: String,
    artist_display: String,
    #[serde(default)]
    score: Option<i32>,
    #[serde(default)]
    match_method: Option<String>,
    #[serde(default)]
    candidate_source: Option<String>,
}

/// Filesystem facts captured during the scan walk.  Reconciliation consumes
/// these values directly instead of issuing a second `stat` for every output.
/// A zero-sized or otherwise invalid snapshot is rejected just like the
/// legacy path-based reconciliation API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutputFileSnapshot {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnalysisStatus {
    NotAnalyzed,
    Failed,
    Completed,
}

impl AnalysisStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAnalyzed => "notAnalyzed",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct W4djLibraryStats {
    pub total: u64,
    pub available: u64,
    pub invalid: u64,
    pub not_analyzed: u64,
    pub analysis_failed: u64,
    pub analysis_completed: u64,
}

#[derive(Debug)]
pub enum W4djLibraryError {
    Library(LibraryError),
    Invalid(String),
}

impl std::fmt::Display for W4djLibraryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Library(error) => error.fmt(formatter),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for W4djLibraryError {}

impl From<LibraryError> for W4djLibraryError {
    fn from(error: LibraryError) -> Self {
        Self::Library(error)
    }
}

impl From<rusqlite::Error> for W4djLibraryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Library(error.into())
    }
}

impl From<std::io::Error> for W4djLibraryError {
    fn from(error: std::io::Error) -> Self {
        Self::Library(error.into())
    }
}

pub type W4djResult<T> = Result<T, W4djLibraryError>;

pub struct W4djLibrary {
    catalog: LibraryCatalog,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmotionEvaluationManifest {
    pub schema_version: u32,
    pub session_id: String,
    pub seed: u64,
    pub sample_size: usize,
    pub clip_policy: String,
    pub tracks: Vec<EmotionEvaluationTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmotionEvaluationTrack {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub relative_path: String,
    pub duration_seconds: Option<f64>,
    pub clip_start_seconds: f64,
    pub clip_duration_seconds: f64,
    pub clip_selection: String,
    pub legacy_mood: Value,
    pub emomusic: Value,
    pub muse: Value,
    pub mirex: Value,
}

impl W4djLibrary {
    pub fn open(path: &Path) -> W4djResult<Self> {
        let (catalog, _) = LibraryCatalog::open_or_recover(path)?;
        let mut library = Self { catalog };
        library.migrate()?;
        Ok(library)
    }

    /// Open an existing output catalog for diagnostics without running the
    /// normal migration/recovery path. This keeps report export read-only.
    pub fn open_read_only(path: &Path) -> W4djResult<Self> {
        Ok(Self {
            catalog: LibraryCatalog::open_read_only(path)?,
        })
    }

    pub fn open_or_recover(path: &Path) -> W4djResult<(Self, Option<PathBuf>)> {
        let (catalog, backup) = LibraryCatalog::open_or_recover(path)?;
        let mut library = Self { catalog };
        library.migrate()?;
        Ok((library, backup))
    }

    pub fn path(&self) -> &Path {
        self.catalog.path()
    }

    /// Return a consistent SQLite online-backup snapshot of the catalog.
    /// Copying only `w4dj.sqlite3` can miss committed WAL pages, so the
    /// snapshot is made through SQLite and can be embedded in a report.
    pub fn sqlite_snapshot_bytes(&self) -> W4djResult<Vec<u8>> {
        let directory = tempfile::tempdir()?;
        let snapshot_path = directory.path().join("w4dj.sqlite3");
        let mut destination = rusqlite::Connection::open(&snapshot_path)?;
        rusqlite::backup::Backup::new(self.catalog.connection(), &mut destination)?
            .run_to_completion(100, Duration::from_millis(10), None)?;
        Ok(fs::read(snapshot_path)?)
    }

    /// Load all successful source→output bindings in one SQLite query. Scan
    /// code uses this snapshot to classify outputs without per-track lookups.
    pub fn committed_output_bindings(&self) -> W4djResult<Vec<CommittedOutputBinding>> {
        let mut normalized_roots = HashMap::<String, String>::new();
        let mut statement = self.catalog.connection().prepare(
            "SELECT source_path,destination_path,output_root,slot_index,
                    source_size_bytes,source_modified_at_ms,conversion_mode,
                    lossless_format,filename_rule,netease_filename_format,
                    filename_normalization_policy
             FROM w4dj_track_meta WHERE status='available' AND source_path IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| {
            let source_path: String = row.get(0)?;
            let destination_path: String = row.get(1)?;
            let output_root: String = row.get(2)?;
            let normalized_root = normalized_roots
                .entry(output_root.clone())
                .or_insert_with(|| normalize_path(Path::new(&output_root)))
                .clone();
            Ok(CommittedOutputBinding {
                source_path,
                destination_path,
                output_root: normalized_root,
                slot_index: row.get::<_, i64>(3)? as usize,
                source_size_bytes: row.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                source_modified_at_ms: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                mode: row.get(6)?,
                lossless_format: row.get(7)?,
                filename_rule: row.get(8)?,
                netease_filename_format: row.get(9)?,
                filename_normalization_policy: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn migrate(&mut self) -> W4djResult<()> {
        self.catalog.connection().execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS library_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS output_roots (
                root_path TEXT PRIMARY KEY,
                first_seen_at_ms INTEGER NOT NULL,
                last_seen_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS slot_output_roots (
                slot_index INTEGER PRIMARY KEY,
                root_path TEXT NOT NULL,
                applied_at_ms INTEGER NOT NULL,
                FOREIGN KEY(root_path) REFERENCES output_roots(root_path)
            );
            CREATE TABLE IF NOT EXISTS w4dj_track_meta (
                track_key TEXT PRIMARY KEY,
                source_path TEXT,
                destination_path TEXT NOT NULL UNIQUE,
                slot_index INTEGER NOT NULL,
                output_root TEXT NOT NULL,
                status TEXT NOT NULL,
                analysis_status TEXT NOT NULL DEFAULT 'notAnalyzed',
                analysis_error TEXT,
                measured_duration_seconds REAL,
                source_size_bytes INTEGER,
                source_modified_at_ms INTEGER,
                conversion_mode TEXT,
                lossless_format TEXT,
                filename_rule TEXT,
                netease_filename_format TEXT,
                filename_normalization_policy TEXT,
                conversion_batch_id TEXT,
                committed_at_ms INTEGER,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                FOREIGN KEY(track_key) REFERENCES tracks(track_key) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS w4dj_track_meta_source_path
                ON w4dj_track_meta(source_path);
            CREATE INDEX IF NOT EXISTS w4dj_track_meta_status
                ON w4dj_track_meta(status);
            CREATE TABLE IF NOT EXISTS analysis_results (
                track_key TEXT PRIMARY KEY,
                destination_path TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                error TEXT,
                analysis_json TEXT NOT NULL,
                analyzed_at_ms INTEGER NOT NULL,
                FOREIGN KEY(track_key) REFERENCES tracks(track_key) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS w4dj_output_identities (
                destination_path TEXT PRIMARY KEY,
                netease_track_id TEXT,
                netease_album_id TEXT,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS imported_dj_playlists (
                playlist_id TEXT PRIMARY KEY,
                format_version INTEGER NOT NULL,
                name TEXT NOT NULL,
                output_mode TEXT,
                scenario TEXT,
                target_region TEXT,
                platform_priority_json TEXT NOT NULL,
                source_path TEXT,
                created_at TEXT,
                imported_at_ms INTEGER NOT NULL,
                warnings_json TEXT NOT NULL,
                claimed_batch_id TEXT
            );
            CREATE TABLE IF NOT EXISTS imported_dj_playlist_tracks (
                playlist_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                record_id TEXT,
                title TEXT NOT NULL,
                artist_display TEXT NOT NULL,
                artists_json TEXT NOT NULL,
                album_or_ep TEXT,
                duration_seconds INTEGER,
                bpm TEXT,
                musical_key TEXT,
                platform_refs_json TEXT NOT NULL,
                dedupe_key TEXT NOT NULL,
                expected_filename_hint TEXT,
                netease_track_id TEXT,
                netease_import_line TEXT NOT NULL,
                PRIMARY KEY (playlist_id, position),
                UNIQUE (playlist_id, dedupe_key),
                FOREIGN KEY (playlist_id) REFERENCES imported_dj_playlists(playlist_id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS imported_dj_playlist_matches (
                playlist_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                track_key TEXT,
                status TEXT NOT NULL CHECK (status IN ('matched', 'unmatched', 'ambiguous', 'missing')),
                match_method TEXT,
                score INTEGER,
                candidates_json TEXT NOT NULL,
                reason TEXT NOT NULL DEFAULT '',
                candidate_source TEXT,
                confirmed INTEGER NOT NULL DEFAULT 0,
                excluded INTEGER NOT NULL DEFAULT 0,
                matched_at_ms INTEGER NOT NULL,
                PRIMARY KEY (playlist_id, position),
                FOREIGN KEY (playlist_id, position)
                    REFERENCES imported_dj_playlist_tracks(playlist_id, position) ON DELETE CASCADE,
                FOREIGN KEY (track_key) REFERENCES tracks(track_key) ON DELETE SET NULL
            );
            CREATE INDEX IF NOT EXISTS imported_dj_playlist_tracks_playlist
                ON imported_dj_playlist_tracks(playlist_id, position);
            CREATE INDEX IF NOT EXISTS imported_dj_playlist_matches_playlist
                ON imported_dj_playlist_matches(playlist_id, position);
            "#,
        )?;
        self.ensure_committed_output_columns()?;
        // Add the v4 nullable facts before any legacy table rebuild so the
        // migration can copy them forward instead of silently dropping them.
        self.migrate_track_meta_without_root_foreign_key()?;
        let claimed_batch_id_exists: bool = self.catalog.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('imported_dj_playlists') WHERE name='claimed_batch_id')",
            [],
            |row| row.get(0),
        )?;
        if !claimed_batch_id_exists {
            self.catalog.connection().execute(
                "ALTER TABLE imported_dj_playlists ADD COLUMN claimed_batch_id TEXT",
                [],
            )?;
        }
        let netease_track_id_exists: bool = self.catalog.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('imported_dj_playlist_tracks') WHERE name='netease_track_id')",
            [],
            |row| row.get(0),
        )?;
        if !netease_track_id_exists {
            self.catalog.connection().execute(
                "ALTER TABLE imported_dj_playlist_tracks ADD COLUMN netease_track_id TEXT",
                [],
            )?;
        }
        let match_reason_exists: bool = self.catalog.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('imported_dj_playlist_matches') WHERE name='reason')",
            [],
            |row| row.get(0),
        )?;
        if !match_reason_exists {
            self.catalog.connection().execute(
                "ALTER TABLE imported_dj_playlist_matches ADD COLUMN reason TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        let match_candidate_source_exists: bool = self.catalog.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('imported_dj_playlist_matches') WHERE name='candidate_source')",
            [],
            |row| row.get(0),
        )?;
        if !match_candidate_source_exists {
            self.catalog.connection().execute(
                "ALTER TABLE imported_dj_playlist_matches ADD COLUMN candidate_source TEXT",
                [],
            )?;
        }
        let match_confirmed_exists: bool = self.catalog.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('imported_dj_playlist_matches') WHERE name='confirmed')",
            [],
            |row| row.get(0),
        )?;
        if !match_confirmed_exists {
            self.catalog.connection().execute(
                "ALTER TABLE imported_dj_playlist_matches ADD COLUMN confirmed INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        let match_excluded_exists: bool = self.catalog.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('imported_dj_playlist_matches') WHERE name='excluded')",
            [],
            |row| row.get(0),
        )?;
        if !match_excluded_exists {
            self.catalog.connection().execute(
                "ALTER TABLE imported_dj_playlist_matches ADD COLUMN excluded INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        self.catalog.connection().execute_batch(
            "CREATE INDEX IF NOT EXISTS w4dj_track_meta_batch
                ON w4dj_track_meta(conversion_batch_id, committed_at_ms);
             CREATE INDEX IF NOT EXISTS imported_dj_playlists_claimed_batch
                ON imported_dj_playlists(claimed_batch_id);",
        )?;
        // These columns are retained solely so older databases can migrate
        // without a destructive table rewrite. W4DJ must never use legacy
        // NetEase IDs as playlist/output identity, so scrub any historical
        // values at the migration boundary. The shared catalog keeps its
        // ordinary NetEase metadata for conversion retrieval; only rows owned
        // by the W4DJ output projection are cleared here.
        self.catalog.connection().execute_batch(
            "UPDATE imported_dj_playlist_tracks SET netease_track_id=NULL;
             UPDATE w4dj_output_identities
                SET netease_track_id=NULL, netease_album_id=NULL;
             UPDATE tracks SET netease_track_id=NULL
              WHERE track_key IN (SELECT track_key FROM w4dj_track_meta);",
        )?;
        self.catalog.connection().execute(
            "INSERT INTO library_meta(key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [W4DJ_SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    fn ensure_committed_output_columns(&mut self) -> W4djResult<()> {
        let columns = [
            ("source_size_bytes", "INTEGER"),
            ("source_modified_at_ms", "INTEGER"),
            ("conversion_mode", "TEXT"),
            ("lossless_format", "TEXT"),
            ("filename_rule", "TEXT"),
            ("netease_filename_format", "TEXT"),
            ("filename_normalization_policy", "TEXT"),
            ("conversion_batch_id", "TEXT"),
            ("committed_at_ms", "INTEGER"),
        ];
        for (name, ty) in columns {
            let exists: bool = self.catalog.connection().query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('w4dj_track_meta') WHERE name=?1)",
                [name],
                |row| row.get(0),
            )?;
            if !exists {
                self.catalog.connection().execute(
                    &format!("ALTER TABLE w4dj_track_meta ADD COLUMN {name} {ty}"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    /// Older W4DJ builds coupled each output row to the output-root scope
    /// state machine through a foreign key.  The lightweight index no longer
    /// maintains that state.  Rebuild the small compatibility table once so
    /// new registrations can store the root for display/relative paths
    /// without inserting or updating `output_roots`.
    fn migrate_track_meta_without_root_foreign_key(&mut self) -> W4djResult<()> {
        let has_root_foreign_key: bool = self.catalog.connection().query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_foreign_key_list('w4dj_track_meta')
                WHERE \"table\"='output_roots'
            )",
            [],
            |row| row.get(0),
        )?;
        if !has_root_foreign_key {
            return Ok(());
        }
        self.catalog.connection().execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE w4dj_track_meta_light (
                track_key TEXT PRIMARY KEY,
                source_path TEXT,
                destination_path TEXT NOT NULL UNIQUE,
                slot_index INTEGER NOT NULL,
                output_root TEXT NOT NULL,
                status TEXT NOT NULL,
                analysis_status TEXT NOT NULL DEFAULT 'notAnalyzed',
                analysis_error TEXT,
                measured_duration_seconds REAL,
                source_size_bytes INTEGER,
                source_modified_at_ms INTEGER,
                conversion_mode TEXT,
                lossless_format TEXT,
                filename_rule TEXT,
                netease_filename_format TEXT,
                filename_normalization_policy TEXT,
                conversion_batch_id TEXT,
                committed_at_ms INTEGER,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                FOREIGN KEY(track_key) REFERENCES tracks(track_key) ON DELETE CASCADE
             );
             INSERT INTO w4dj_track_meta_light(
                track_key,source_path,destination_path,slot_index,output_root,
                status,analysis_status,analysis_error,measured_duration_seconds,
                source_size_bytes,source_modified_at_ms,conversion_mode,lossless_format,
                filename_rule,netease_filename_format,filename_normalization_policy,
                conversion_batch_id,committed_at_ms,
                created_at_ms,updated_at_ms
             ) SELECT track_key,source_path,destination_path,slot_index,output_root,
                status,analysis_status,analysis_error,measured_duration_seconds,
                source_size_bytes,source_modified_at_ms,conversion_mode,lossless_format,
                filename_rule,netease_filename_format,filename_normalization_policy,
                conversion_batch_id,committed_at_ms,
                created_at_ms,updated_at_ms
             FROM w4dj_track_meta;
             DROP TABLE w4dj_track_meta;
             ALTER TABLE w4dj_track_meta_light RENAME TO w4dj_track_meta;
             CREATE INDEX IF NOT EXISTS w4dj_track_meta_source_path
                ON w4dj_track_meta(source_path);
             CREATE INDEX IF NOT EXISTS w4dj_track_meta_status
                ON w4dj_track_meta(status);
             COMMIT;",
        )?;
        Ok(())
    }

    pub fn upsert_imported_dj_playlist(&mut self, playlist: &ImportedDjPlaylist) -> W4djResult<()> {
        if playlist.playlist_id.trim().is_empty() || playlist.name.trim().is_empty() {
            return Err(W4djLibraryError::Invalid(
                "DJ 歌单 ID 和名称不能为空".to_string(),
            ));
        }
        let warnings_json = serde_json::to_string(&playlist.warnings)
            .map_err(|error| W4djLibraryError::Invalid(format!("序列化导入警告失败：{error}")))?;
        let imported_at_ms = playlist.imported_at_ms.unwrap_or_else(now_ms);
        let source_path = playlist
            .source_path
            .as_ref()
            .map(|path| normalize_path(path));
        let transaction = self.catalog.connection_mut().transaction()?;
        transaction.execute(
            "INSERT INTO imported_dj_playlists(
                playlist_id,format_version,name,output_mode,scenario,target_region,
                platform_priority_json,source_path,created_at,imported_at_ms,warnings_json
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL,?9,?10)
             ON CONFLICT(playlist_id) DO UPDATE SET
                format_version=excluded.format_version,
                name=excluded.name,
                output_mode=excluded.output_mode,
                scenario=excluded.scenario,
                target_region=excluded.target_region,
                platform_priority_json=excluded.platform_priority_json,
                source_path=excluded.source_path,
                imported_at_ms=excluded.imported_at_ms,
                warnings_json=excluded.warnings_json",
            params![
                playlist.playlist_id,
                playlist.format_version,
                playlist.name,
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                "[]",
                source_path,
                imported_at_ms,
                warnings_json,
            ],
        )?;
        transaction.execute(
            "DELETE FROM imported_dj_playlist_matches WHERE playlist_id=?1",
            [&playlist.playlist_id],
        )?;
        transaction.execute(
            "DELETE FROM imported_dj_playlist_tracks WHERE playlist_id=?1",
            [&playlist.playlist_id],
        )?;
        for track in &playlist.tracks {
            let artists_json =
                serde_json::to_string(&[track.artist_display.as_str()]).map_err(|error| {
                    W4djLibraryError::Invalid(format!("序列化歌手列表失败：{error}"))
                })?;
            let platform_refs_json = "[]";
            // Older local database schemas made dedupe_key unique per playlist.
            // Keep the normalized key in memory, but suffix the storage key by
            // position so repeated positions remain representable without a
            // destructive database migration.
            let stored_dedupe_key = format!("{}:position:{}", track.dedupe_key, track.position);
            transaction.execute(
                "INSERT INTO imported_dj_playlist_tracks(
                    playlist_id,position,record_id,title,artist_display,artists_json,
                    album_or_ep,duration_seconds,bpm,musical_key,platform_refs_json,
                    dedupe_key,expected_filename_hint,netease_track_id,netease_import_line
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                params![
                    playlist.playlist_id,
                    track.position,
                    Option::<String>::None,
                    track.title,
                    track.artist_display,
                    artists_json,
                    Option::<String>::None,
                    Option::<u64>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    platform_refs_json,
                    stored_dedupe_key,
                    Option::<String>::None,
                    Option::<String>::None,
                    track.netease_import_line,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_imported_dj_playlists(&self) -> W4djResult<Vec<ImportedDjPlaylistSummary>> {
        let mut statement = self.catalog.connection().prepare(
            "SELECT p.playlist_id,p.name,COUNT(t.position),
                    json_array_length(p.warnings_json),p.imported_at_ms,p.source_path
             FROM imported_dj_playlists p
             LEFT JOIN imported_dj_playlist_tracks t ON t.playlist_id=p.playlist_id
             GROUP BY p.playlist_id
             ORDER BY p.imported_at_ms DESC, p.playlist_id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ImportedDjPlaylistSummary {
                playlist_id: row.get(0)?,
                name: row.get(1)?,
                track_count: row.get::<_, i64>(2)?.max(0) as usize,
                warning_count: row.get::<_, i64>(3)?.max(0) as usize,
                imported_at_ms: row.get(4)?,
                source_path: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_imported_dj_playlist(
        &self,
        playlist_id: &str,
    ) -> W4djResult<Option<ImportedDjPlaylist>> {
        let Some((format_version, name, source_path, imported_at_ms, warnings_json)) = self
            .catalog
            .connection()
            .query_row(
                "SELECT format_version,name,source_path,imported_at_ms,warnings_json
             FROM imported_dj_playlists WHERE playlist_id=?1",
                [playlist_id],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut statement = self.catalog.connection().prepare(
            "SELECT position,title,artist_display,dedupe_key,netease_import_line
             FROM imported_dj_playlist_tracks WHERE playlist_id=?1 ORDER BY position",
        )?;
        let rows = statement.query_map([playlist_id], |row| {
            let position: u64 = row.get(0)?;
            let stored_dedupe_key: String = row.get(3)?;
            Ok(ImportedDjPlaylistTrack {
                position,
                title: row.get(1)?,
                artist_display: row.get(2)?,
                dedupe_key: restore_dedupe_key(&stored_dedupe_key, position),
                netease_import_line: row.get(4)?,
            })
        })?;
        let tracks = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let warnings: Vec<DjPlaylistImportWarning> = serde_json::from_str(&warnings_json)
            .map_err(|error| W4djLibraryError::Invalid(format!("读取导入警告失败：{error}")))?;
        Ok(Some(ImportedDjPlaylist {
            playlist_id: playlist_id.to_string(),
            format_version,
            name,
            source_path: source_path.map(PathBuf::from),
            imported_at_ms: Some(imported_at_ms),
            tracks,
            warnings,
        }))
    }

    /// Return every output owned by the lightweight W4DJ index. Availability
    /// flags are returned as stored compatibility hints, but they are not
    /// used to decide whether a song can be matched; the exporter checks only
    /// the selected paths. No filesystem or NetEase lookup happens here.
    pub fn available_dj_output_candidates(&self) -> W4djResult<Vec<DjOutputCandidate>> {
        let mut statement = self.catalog.connection().prepare(
            "SELECT m.track_key,t.title,t.artists,
                    COALESCE(m.measured_duration_seconds,t.effective_duration_seconds),
                    m.destination_path,m.status,
                    COALESCE((SELECT readable FROM local_files lf
                              WHERE lf.path=m.destination_path LIMIT 1), 1),
                    m.conversion_batch_id,m.committed_at_ms
             FROM w4dj_track_meta m
             JOIN tracks t ON t.track_key=m.track_key
             WHERE m.status='available'
             ORDER BY COALESCE(m.committed_at_ms, 0) DESC, m.destination_path, m.track_key",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(DjOutputCandidate {
                track_key: row.get(0)?,
                title: row.get(1)?,
                artist_display: row.get(2)?,
                duration_seconds: row.get(3)?,
                destination_path: PathBuf::from(row.get::<_, String>(4)?),
                status: row.get(5)?,
                readable: row.get::<_, i64>(6)? != 0,
                conversion_batch_id: row.get(7)?,
                committed_at_ms: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Atomically reserve the newest successful conversion batch for a W4DJ
    /// playlist. A batch is claimed at most once, which prevents a later
    /// playlist import from stealing the outputs intended for an earlier one.
    pub fn claim_latest_conversion_batch(
        &mut self,
        playlist_id: &str,
    ) -> W4djResult<Option<String>> {
        let transaction = self.catalog.connection_mut().transaction()?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT claimed_batch_id FROM imported_dj_playlists WHERE playlist_id=?1",
                [playlist_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if let Some(current_id) = current.as_deref()
            && transaction.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM w4dj_track_meta
                     WHERE status='available' AND conversion_batch_id=?1
                 )",
                [current_id],
                |row| row.get::<_, bool>(0),
            )?
        {
            transaction.commit()?;
            return Ok(current);
        }
        if current.is_some() {
            transaction.execute(
                "UPDATE imported_dj_playlists SET claimed_batch_id=NULL WHERE playlist_id=?1",
                [playlist_id],
            )?;
        }
        let latest: Option<String> = transaction
            .query_row(
                "SELECT conversion_batch_id
                 FROM w4dj_track_meta
                 WHERE status='available'
                   AND conversion_batch_id IS NOT NULL
                   AND TRIM(conversion_batch_id)<>''
                 GROUP BY conversion_batch_id
                 ORDER BY MAX(COALESCE(committed_at_ms, 0)) DESC, conversion_batch_id DESC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let Some(batch_id) = latest else {
            transaction.commit()?;
            return Ok(None);
        };
        let already_claimed: bool = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM imported_dj_playlists
                 WHERE claimed_batch_id=?1 AND playlist_id<>?2
             )",
            params![batch_id, playlist_id],
            |row| row.get(0),
        )?;
        if already_claimed {
            transaction.commit()?;
            return Ok(None);
        }
        let updated = transaction.execute(
            "UPDATE imported_dj_playlists SET claimed_batch_id=?1
             WHERE playlist_id=?2 AND claimed_batch_id IS NULL",
            params![batch_id, playlist_id],
        )?;
        transaction.commit()?;
        Ok((updated == 1).then_some(batch_id))
    }

    pub fn dj_output_candidates_for_batch(
        &self,
        batch_id: &str,
    ) -> W4djResult<Vec<DjOutputCandidate>> {
        Ok(self
            .available_dj_output_candidates()?
            .into_iter()
            .filter(|candidate| candidate.conversion_batch_id.as_deref() == Some(batch_id))
            .collect())
    }

    pub fn has_imported_dj_playlist_matches(&self, playlist_id: &str) -> W4djResult<bool> {
        Ok(self.catalog.connection().query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM imported_dj_playlist_matches WHERE playlist_id=?1
             )",
            [playlist_id],
            |row| row.get(0),
        )?)
    }

    pub fn compute_imported_dj_playlist_matches(
        &mut self,
        playlist_id: &str,
    ) -> W4djResult<DjPlaylistMatchReport> {
        let playlist = self
            .get_imported_dj_playlist(playlist_id)?
            .ok_or_else(|| W4djLibraryError::Invalid("未找到指定 DJ 歌单".to_string()))?;
        let candidates = self.available_dj_output_candidates()?;
        let recent = self
            .claim_latest_conversion_batch(playlist_id)?
            .map(|batch_id| {
                candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.conversion_batch_id.as_deref() == Some(batch_id.as_str())
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(match_imported_playlist_with_priority(
            &playlist,
            &recent,
            &candidates,
        ))
    }

    pub fn replace_imported_dj_playlist_matches(
        &mut self,
        playlist_id: &str,
        report: &DjPlaylistMatchReport,
    ) -> W4djResult<()> {
        if report.playlist_id != playlist_id {
            return Err(W4djLibraryError::Invalid(
                "匹配报告与歌单 ID 不一致".to_string(),
            ));
        }
        let playlist = self
            .get_imported_dj_playlist(playlist_id)?
            .ok_or_else(|| W4djLibraryError::Invalid("未找到指定 DJ 歌单".to_string()))?;
        let candidates = self.available_dj_output_candidates()?;
        let mut effective_report = report.clone();
        let available_by_key = candidates
            .iter()
            .map(|candidate| (candidate.track_key.as_str(), candidate))
            .collect::<std::collections::HashMap<_, _>>();
        for row in &mut effective_report.matches {
            if row.status != "matched" {
                row.destination_path = None;
                row.candidate_source = None;
                row.confirmed = false;
                continue;
            }
            let Some(track_key) = row.track_key.as_deref() else {
                row.status = "unmatched".to_string();
                row.kind = DjPlaylistMatchKind::Unmatched;
                row.destination_path = None;
                row.candidate_source = None;
                row.confirmed = false;
                continue;
            };
            let Some(candidate) = available_by_key.get(track_key) else {
                row.status = "missing".to_string();
                row.kind = DjPlaylistMatchKind::Missing;
                row.track_key = None;
                row.destination_path = None;
                row.candidate_source = None;
                row.confirmed = false;
                row.score = None;
                row.reason = "匹配的输出已不可用".to_string();
                continue;
            };
            row.destination_path = Some(candidate.destination_path.clone());
            row.confirmed = candidate.readable;
            if row.candidate_source.is_none() {
                row.candidate_source = Some(if row.manual {
                    "manual".to_string()
                } else {
                    "library".to_string()
                });
            }
        }
        validate_match_report(&playlist, &effective_report, &candidates)?;
        let transaction = self.catalog.connection_mut().transaction()?;
        transaction.execute(
            "DELETE FROM imported_dj_playlist_matches WHERE playlist_id=?1",
            [playlist_id],
        )?;
        let matched_at_ms = now_ms();
        for row in &effective_report.matches {
            let candidates_json = serde_json::to_string(&row.candidates).map_err(|error| {
                W4djLibraryError::Invalid(format!("序列化歌单匹配候选失败：{error}"))
            })?;
            transaction.execute(
                "INSERT INTO imported_dj_playlist_matches(
                    playlist_id,position,track_key,status,match_method,score,
                    candidates_json,reason,candidate_source,confirmed,excluded,matched_at_ms
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    playlist_id,
                    row.position,
                    row.track_key,
                    row.status,
                    row.match_method,
                    row.score,
                    candidates_json,
                    row.reason,
                    row.candidate_source,
                    i64::from(row.confirmed),
                    i64::from(row.excluded),
                    matched_at_ms,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get_imported_dj_playlist_match_report(
        &self,
        playlist_id: &str,
    ) -> W4djResult<DjPlaylistMatchReport> {
        let playlist = self
            .get_imported_dj_playlist(playlist_id)?
            .ok_or_else(|| W4djLibraryError::Invalid("未找到指定 DJ 歌单".to_string()))?;
        let mut statement = self.catalog.connection().prepare(
            "SELECT position,track_key,status,match_method,score,candidates_json,reason,
                    candidate_source,confirmed,excluded
             FROM imported_dj_playlist_matches WHERE playlist_id=?1 ORDER BY position",
        )?;
        let rows = statement.query_map([playlist_id], |row| {
            let position: u64 = row.get(0)?;
            let track = playlist
                .tracks
                .iter()
                .find(|track| track.position == position)
                .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
            let candidates_json: String = row.get(5)?;
            let reason: String = row.get(6)?;
            let status: String = row.get(2)?;
            let match_method: Option<String> = row.get(3)?;
            let candidate_source: Option<String> = row.get(7)?;
            let confirmed: bool = row.get::<_, i64>(8)? != 0;
            let excluded: bool = row.get::<_, i64>(9)? != 0;
            let manual = match_method.as_deref() == Some("manual");
            Ok(DjPlaylistTrackMatch {
                position,
                dedupe_key: track.dedupe_key.clone(),
                title: track.title.clone(),
                artist_display: track.artist_display.clone(),
                kind: persisted_match_kind(&status, match_method.as_deref()),
                status,
                track_key: row.get(1)?,
                match_method,
                score: row.get(4)?,
                reason,
                candidates: serde_json::from_str(&candidates_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        candidates_json.len(),
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                manual,
                destination_path: None,
                candidate_source,
                confirmed,
                excluded,
            })
        })?;
        let stored = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if stored.len() == playlist.tracks.len() {
            let candidates = self.available_dj_output_candidates()?;
            let available_by_key = candidates
                .iter()
                .map(|candidate| (candidate.track_key.as_str(), candidate))
                .collect::<std::collections::HashMap<_, _>>();
            let mut refreshed = stored;
            for row in &mut refreshed {
                if row.status != "matched" {
                    if row.reason.trim().is_empty() {
                        row.reason = match row.status.as_str() {
                            "ambiguous" => "存在多个同等精确候选，需要手动选择".to_string(),
                            "missing" => "匹配的输出已不可用".to_string(),
                            _ => "没有达到自动匹配条件，请从候选中手动选择".to_string(),
                        };
                    }
                    continue;
                }
                let Some(track_key) = row.track_key.clone() else {
                    row.status = "missing".to_string();
                    row.match_method = row
                        .match_method
                        .take()
                        .or_else(|| Some("stored".to_string()));
                    row.score = None;
                    row.reason = "已保存匹配缺少输出引用".to_string();
                    continue;
                };
                if !available_by_key.contains_key(track_key.as_str()) {
                    row.status = "missing".to_string();
                    row.track_key = None;
                    row.score = None;
                    row.reason = if row.manual {
                        "此前的手动输出已不可用".to_string()
                    } else {
                        "此前匹配的输出已不可用".to_string()
                    };
                    row.destination_path = None;
                    row.candidate_source = None;
                    row.confirmed = false;
                } else if row.reason.trim().is_empty() {
                    row.reason = "标题、歌手和可用时长均匹配".to_string();
                }
                if row.status == "matched"
                    && let Some(candidate) = available_by_key.get(track_key.as_str())
                {
                    row.destination_path = Some(candidate.destination_path.clone());
                    row.confirmed = candidate.readable;
                }
            }
            return Ok(build_match_report(playlist_id, refreshed));
        }
        Ok(crate::dj_playlist_match::match_imported_playlist(
            &playlist,
            &self.available_dj_output_candidates()?,
        ))
    }

    pub fn set_imported_dj_playlist_match(
        &mut self,
        playlist_id: &str,
        position: u64,
        track_key: &str,
    ) -> W4djResult<()> {
        let mut report = self.get_imported_dj_playlist_match_report(playlist_id)?;
        let candidates = self.available_dj_output_candidates()?;
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.track_key == track_key)
            .ok_or_else(|| W4djLibraryError::Invalid("只能选择当前可用且可读的输出".to_string()))?;
        let duplicate_rows = report
            .matches
            .iter()
            .filter(|row| row.position != position && row.track_key.as_deref() == Some(track_key))
            .collect::<Vec<_>>();
        let playlist = self
            .get_imported_dj_playlist(playlist_id)?
            .ok_or_else(|| W4djLibraryError::Invalid("未找到指定 DJ 歌单".to_string()))?;
        let selected_track = playlist
            .tracks
            .iter()
            .find(|track| track.position == position)
            .ok_or_else(|| W4djLibraryError::Invalid("未找到歌单位置".to_string()))?;
        if duplicate_rows.iter().any(|row| {
            crate::dj_playlist_match::identity_key_for(&row.title, &row.artist_display)
                != crate::dj_playlist_match::identity_key_for(
                    &selected_track.title,
                    &selected_track.artist_display,
                )
        }) {
            return Err(W4djLibraryError::Invalid(
                "同一个输出只能复用于标题和歌手相同的歌单歌曲".to_string(),
            ));
        }
        let row = report
            .matches
            .iter_mut()
            .find(|row| row.position == position)
            .ok_or_else(|| W4djLibraryError::Invalid("未找到歌单位置".to_string()))?;
        row.status = "matched".to_string();
        row.kind = DjPlaylistMatchKind::Manual;
        row.track_key = Some(track_key.to_string());
        row.match_method = Some("manual".to_string());
        row.score = Some(100);
        row.reason = "用户手动确认".to_string();
        row.manual = true;
        row.destination_path = Some(candidate.destination_path.clone());
        row.candidate_source = Some("manual".to_string());
        row.confirmed = true;
        row.excluded = false;
        if !row
            .candidates
            .iter()
            .any(|item| item.track_key == track_key)
        {
            row.candidates
                .push(crate::dj_playlist_match::DjPlaylistMatchCandidate {
                    track_key: candidate.track_key.clone(),
                    title: candidate.title.clone(),
                    artist_display: candidate.artist_display.clone(),
                    duration_seconds: candidate.duration_seconds,
                    destination_filename: candidate
                        .destination_path
                        .file_name()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    score: 100,
                    reason: "用户手动确认".to_string(),
                });
        }
        self.replace_imported_dj_playlist_matches(playlist_id, &report)
    }

    /// Bind a playlist row to an explicitly selected local audio file. The
    /// file is indexed as a W4DJ output when it is not already known, so a
    /// previously converted song can be recovered without guessing an ID.
    pub fn set_imported_dj_playlist_match_by_path(
        &mut self,
        playlist_id: &str,
        position: u64,
        destination_path: &Path,
    ) -> W4djResult<()> {
        let metadata = fs::symlink_metadata(destination_path)
            .map_err(|error| W4djLibraryError::Invalid(format!("无法读取所选本地歌曲：{error}")))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            return Err(W4djLibraryError::Invalid(
                "所选路径必须是非空的本地音频文件，不能是符号链接".to_string(),
            ));
        }
        if !is_audio_path(destination_path) {
            return Err(W4djLibraryError::Invalid(
                "所选文件不是支持的音频格式".to_string(),
            ));
        }
        let normalized = normalize_path(destination_path);
        let candidate = self
            .available_dj_output_candidates()?
            .into_iter()
            .find(|candidate| normalize_path(&candidate.destination_path) == normalized);
        if candidate.is_none() {
            let root = destination_path.parent().unwrap_or_else(|| Path::new("."));
            self.upsert_output_file(0, root, None, destination_path)?;
        }
        let candidate = self
            .available_dj_output_candidates()?
            .into_iter()
            .find(|candidate| normalize_path(&candidate.destination_path) == normalized)
            .ok_or_else(|| W4djLibraryError::Invalid("所选本地歌曲无法登记".to_string()))?;
        self.set_imported_dj_playlist_match(playlist_id, position, &candidate.track_key)
    }

    /// Compatibility endpoint for older clients. The current review flow treats
    /// a valid binding as accepted by default; the exporter does not gate on
    /// this legacy flag.
    pub fn set_imported_dj_playlist_match_confirmed(
        &mut self,
        playlist_id: &str,
        position: u64,
        confirmed: bool,
    ) -> W4djResult<()> {
        let mut report = self.get_imported_dj_playlist_match_report(playlist_id)?;
        let row = report
            .matches
            .iter_mut()
            .find(|row| row.position == position)
            .ok_or_else(|| W4djLibraryError::Invalid("未找到歌单位置".to_string()))?;
        if confirmed {
            let Some(path) = row.destination_path.as_deref() else {
                return Err(W4djLibraryError::Invalid(
                    "请先为该歌单歌曲选择本地输出".to_string(),
                ));
            };
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                W4djLibraryError::Invalid(format!("复核的本地歌曲不可读取：{error}"))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
                return Err(W4djLibraryError::Invalid(
                    "复核的本地歌曲已不存在或不是有效文件".to_string(),
                ));
            }
            row.status = "matched".to_string();
            row.confirmed = true;
        } else {
            row.confirmed = false;
        }
        self.replace_imported_dj_playlist_matches(playlist_id, &report)
    }

    pub fn clear_imported_dj_playlist_match(
        &mut self,
        playlist_id: &str,
        position: u64,
    ) -> W4djResult<()> {
        let mut report = self.get_imported_dj_playlist_match_report(playlist_id)?;
        let row = report
            .matches
            .iter_mut()
            .find(|row| row.position == position)
            .ok_or_else(|| W4djLibraryError::Invalid("未找到歌单位置".to_string()))?;
        row.status = "unmatched".to_string();
        row.kind = DjPlaylistMatchKind::Unmatched;
        row.track_key = None;
        row.match_method = None;
        row.score = None;
        row.reason = "已清除手动匹配，等待下一次识别".to_string();
        row.manual = false;
        row.destination_path = None;
        row.candidate_source = None;
        row.confirmed = false;
        row.excluded = false;
        self.replace_imported_dj_playlist_matches(playlist_id, &report)
    }

    /// Add or remove rows from this playlist's export list. This is a review
    /// decision only: it never deletes an output file or a W4DJ library row.
    pub fn set_imported_dj_playlist_matches_excluded(
        &mut self,
        playlist_id: &str,
        positions: &[u64],
        excluded: bool,
    ) -> W4djResult<()> {
        if positions.is_empty() {
            return Err(W4djLibraryError::Invalid(
                "至少需要选择一首歌曲".to_string(),
            ));
        }
        let requested = positions.iter().copied().collect::<HashSet<_>>();
        if requested.len() != positions.len() {
            return Err(W4djLibraryError::Invalid("歌单位置不能重复".to_string()));
        }
        let mut report = self.get_imported_dj_playlist_match_report(playlist_id)?;
        if requested
            .iter()
            .any(|position| !report.matches.iter().any(|row| row.position == *position))
        {
            return Err(W4djLibraryError::Invalid(
                "包含不存在的歌单位置".to_string(),
            ));
        }
        for row in &mut report.matches {
            if requested.contains(&row.position) {
                row.excluded = excluded;
                if row.status == "matched" {
                    row.confirmed = true;
                }
            }
        }
        self.replace_imported_dj_playlist_matches(playlist_id, &report)
    }

    pub fn is_initial_import_done(&self) -> W4djResult<bool> {
        Ok(self
            .catalog
            .connection()
            .query_row(
                "SELECT value FROM library_meta WHERE key='initial_import_done'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_some_and(|value| value == "1"))
    }

    pub fn mark_initial_import_done(&mut self) -> W4djResult<()> {
        self.catalog.connection().execute(
            "INSERT INTO library_meta(key,value) VALUES ('initial_import_done','1')
             ON CONFLICT(key) DO UPDATE SET value='1'",
            [],
        )?;
        Ok(())
    }

    /// Import only files already present below successful conversion
    /// destinations. NetEase database-only records are never consulted.
    pub fn import_initial_history(
        &mut self,
        history_path: &Path,
        analysis_path: &Path,
    ) -> W4djResult<usize> {
        if self.is_initial_import_done()? {
            return Ok(0);
        }
        let history = crate::history::load_history(history_path)
            .map_err(|error| W4djLibraryError::Invalid(format!("读取转换历史失败：{error}")))?;
        let analyses = load_analysis_file(analysis_path).map_err(W4djLibraryError::Invalid)?;
        let mut analysis_by_path = std::collections::HashMap::new();
        for entry in analyses {
            analysis_by_path.insert(entry.path.clone(), entry);
        }
        let mut imported = 0;
        for entry in history.iter().filter(|entry| {
            matches!(
                entry.status,
                HistoryStatus::Completed | HistoryStatus::Partial
            )
        }) {
            let root = Path::new(&entry.destination_directory);
            if !root.is_dir() {
                continue;
            }
            let files = WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .flatten()
                .filter(|item| item.file_type().is_file() && is_audio_path(item.path()))
                .map(|item| item.path().to_path_buf())
                .collect::<Vec<_>>();
            for destination in files {
                let source = entry
                    .analysis_reports
                    .iter()
                    .find(|report| report.destination_path == destination.to_string_lossy())
                    .map(|report| report.source_path.clone());
                self.upsert_output_file(
                    entry.slot_index,
                    root,
                    source.as_deref().map(Path::new),
                    &destination,
                )?;
                if let Some(source) = source.as_deref()
                    && let Some(analysis) = analysis_by_path.get(source)
                {
                    self.apply_analysis_for_destination(&destination, analysis)?;
                }
                imported += 1;
            }
        }
        self.mark_initial_import_done()?;
        Ok(imported)
    }

    /// Register an output using only facts already known by the conversion
    /// preview.  This is the production ingestion path for new conversions:
    /// it deliberately does not stat, probe, open, or read either the source
    /// or destination file.  The safe-commit callback has already proved that
    /// the destination exists; the index stores that fact as a lightweight
    /// local binding and defers all expensive metadata work to the explicit
    /// analysis/maintenance commands.
    ///
    /// The legacy `w4dj_track_meta` table still carries an output-root column
    /// for relative-path rendering, but its old foreign key/state machine is
    /// migrated away.  This path does not touch `output_roots` or
    /// `slot_output_roots`, and never transitions a previous row to
    /// `outOfScope`.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_lightweight_output(
        &mut self,
        slot_index: usize,
        source_path: Option<&Path>,
        destination_path: &Path,
        title: &str,
        artist: &str,
    ) -> W4djResult<String> {
        self.upsert_lightweight_output_inner(
            slot_index,
            source_path,
            destination_path,
            None,
            title,
            artist,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_lightweight_output_inner(
        &mut self,
        slot_index: usize,
        source_path: Option<&Path>,
        destination_path: &Path,
        output_root: Option<&Path>,
        title: &str,
        artist: &str,
        facts: Option<&CommittedOutputFacts>,
    ) -> W4djResult<String> {
        let destination = normalize_index_path(destination_path);
        if destination.is_empty() {
            return Err(W4djLibraryError::Invalid("输出路径不能为空".to_string()));
        }
        let output_root = output_root
            .map(normalize_index_path)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                Path::new(&destination)
                    .parent()
                    .map(normalize_index_path)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| ".".to_string())
            });
        let stored_database_source: Option<String> = self
            .catalog
            .connection()
            .query_row(
                "SELECT source_path FROM w4dj_track_meta
                 WHERE destination_path=?1 LIMIT 1",
                [&destination],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let database_source = stored_database_source;
        let requested_source = source_path.map(normalize_index_path);
        let source = requested_source.clone().or_else(|| database_source.clone());
        let title = title.trim();
        let artist = artist.trim();
        let fallback_title = Path::new(&destination)
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| destination.clone());
        let title = if !title.is_empty() {
            title
        } else {
            fallback_title.as_str()
        };
        let artist_list_json = serde_json::to_string(
            &artist
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        let now = now_ms();

        let transaction = self.catalog.connection_mut().transaction()?;

        // Resolve stable output identity from local source/path provenance
        // only. NetEase IDs are intentionally absent from this path.
        let mut existing_key: Option<String> = None;
        if existing_key.is_none()
            && let Some(source) = source.as_deref()
        {
            existing_key = transaction
                .query_row(
                    "SELECT track_key FROM w4dj_track_meta WHERE source_path=?1 LIMIT 1",
                    [source],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
        }
        if existing_key.is_none() {
            existing_key = transaction
                .query_row(
                    "SELECT track_key FROM w4dj_track_meta WHERE destination_path=?1 LIMIT 1",
                    [&destination],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
        }
        let track_key = existing_key.unwrap_or_else(|| {
            source
                .as_deref()
                .map(|value| format!("source:{value}"))
                .unwrap_or_else(|| format!("output:{destination}"))
        });

        // A destination can only belong to one output.  If a new identity is
        // committed over an old row, remove that stale row inside this same
        // transaction; no audio file is touched.
        if let Some(destination_key) = transaction
            .query_row(
                "SELECT track_key FROM w4dj_track_meta WHERE destination_path=?1",
                [&destination],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            && destination_key != track_key
        {
            transaction.execute("DELETE FROM tracks WHERE track_key=?1", [&destination_key])?;
        }

        transaction.execute(
            "INSERT INTO tracks(
                track_key, title, artists, artist_list_json,
                local_status, updated_at_ms
             ) VALUES (?1,?2,?3,?4,'available',?5)
             ON CONFLICT(track_key) DO UPDATE SET
                title=CASE WHEN excluded.title<>'' THEN excluded.title ELSE tracks.title END,
                artists=CASE WHEN excluded.artists<>'' THEN excluded.artists ELSE tracks.artists END,
                artist_list_json=CASE WHEN excluded.artists<>'' THEN excluded.artist_list_json ELSE tracks.artist_list_json END,
                local_status='available', updated_at_ms=excluded.updated_at_ms",
            params![track_key, title, artist, artist_list_json, now],
        )?;

        transaction.execute("DELETE FROM local_files WHERE track_key=?1", [&track_key])?;
        transaction.execute(
            "INSERT INTO local_files(
                track_key,path,size_bytes,modified_at_ms,measured_format,
                measured_bitrate_bps,measured_duration_seconds,sample_rate_hz,
                channels,readable,probe_error
             ) VALUES (?1,?2,0,NULL,NULL,NULL,NULL,NULL,NULL,1,NULL)",
            params![track_key, destination],
        )?;
        transaction.execute(
            "INSERT INTO w4dj_track_meta(
                track_key,source_path,destination_path,slot_index,output_root,
                status,analysis_status,analysis_error,measured_duration_seconds,
                source_size_bytes,source_modified_at_ms,conversion_mode,lossless_format,
                filename_rule,netease_filename_format,filename_normalization_policy,
                conversion_batch_id,committed_at_ms,created_at_ms,updated_at_ms
             ) VALUES (?1,?2,?3,?4,?5,'available','notAnalyzed',NULL,NULL,
                       ?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)
             ON CONFLICT(track_key) DO UPDATE SET
                source_path=excluded.source_path,
                destination_path=excluded.destination_path,
                slot_index=excluded.slot_index,
                output_root=excluded.output_root,
                status='available',
                analysis_status='notAnalyzed',
                analysis_error=NULL,
                measured_duration_seconds=NULL,
                source_size_bytes=COALESCE(excluded.source_size_bytes,w4dj_track_meta.source_size_bytes),
                source_modified_at_ms=COALESCE(excluded.source_modified_at_ms,w4dj_track_meta.source_modified_at_ms),
                conversion_mode=COALESCE(excluded.conversion_mode,w4dj_track_meta.conversion_mode),
                lossless_format=COALESCE(excluded.lossless_format,w4dj_track_meta.lossless_format),
                filename_rule=COALESCE(excluded.filename_rule,w4dj_track_meta.filename_rule),
                netease_filename_format=COALESCE(excluded.netease_filename_format,w4dj_track_meta.netease_filename_format),
                filename_normalization_policy=COALESCE(excluded.filename_normalization_policy,w4dj_track_meta.filename_normalization_policy),
                conversion_batch_id=COALESCE(excluded.conversion_batch_id,w4dj_track_meta.conversion_batch_id),
                committed_at_ms=COALESCE(excluded.committed_at_ms,w4dj_track_meta.committed_at_ms),
                updated_at_ms=excluded.updated_at_ms",
            params![
                track_key,
                source,
                destination,
                slot_index as i64,
                output_root,
                facts.and_then(|facts| facts.source_size_bytes.map(|value| value as i64)),
                facts.and_then(|facts| facts.source_modified_at_ms.map(|value| value as i64)),
                facts.and_then(|facts| facts.conversion_mode.as_deref()),
                facts.and_then(|facts| facts.lossless_format.as_deref()),
                facts.and_then(|facts| facts.filename_rule.as_deref()),
                facts.and_then(|facts| facts.netease_filename_format.as_deref()),
                facts.and_then(|facts| facts.filename_normalization_policy.as_deref()),
                facts.and_then(|facts| facts.conversion_batch_id.as_deref()),
                facts.map(|_| now),
                now,
            ],
        )?;
        // The bytes behind this output were just safely committed. Any
        // previous projection therefore belongs to the old bytes, even when
        // the destination path itself did not change.
        transaction.execute(
            "DELETE FROM analysis_results WHERE track_key=?1",
            [&track_key],
        )?;
        transaction.execute(
            "UPDATE tracks SET essentia_genre='', essentia_duration_seconds=NULL,
                preferred_local_file_id=NULL,
                measured_duration_seconds=NULL, effective_duration_seconds=NULL,
                duration_source=NULL, measured_format=NULL, effective_format=NULL,
                measured_bitrate_bps=NULL, effective_bitrate_bps=NULL,
                measured_size_bytes=NULL, effective_size_bytes=NULL,
                bpm=NULL, musical_key=NULL, scale=NULL,
                integrated_loudness_lufs=NULL, loudness_range_lu=NULL,
                energy=NULL, danceability=NULL, mood_json='[]', instrument_json='[]',
                style_json='[]', discogs_mood_theme_json='[]',
                discogs_approachability_json='{}', discogs_instrumentation_json='[]',
                discogs_timbre_json='{}', discogs_danceability_json='{}',
                drop_loudness_lufs=NULL, updated_at_ms=?1 WHERE track_key=?2",
            params![now, track_key],
        )?;
        transaction.commit()?;
        Ok(track_key)
    }

    /// Register a safely committed output and persist the source fingerprint
    /// and naming context used to produce it in the same SQLite transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_committed_output(
        &mut self,
        slot_index: usize,
        source_path: Option<&Path>,
        destination_path: &Path,
        title: &str,
        artist: &str,
        facts: &CommittedOutputFacts,
    ) -> W4djResult<String> {
        self.upsert_lightweight_output_inner(
            slot_index,
            source_path,
            destination_path,
            None,
            title,
            artist,
            Some(facts),
        )
    }

    /// Register a committed output while preserving the configured output
    /// root so a later explicit W4DJ operation can materialize its recovery
    /// sidecar. This matters when a filename is emitted below a nested
    /// subdirectory: the identity manifest belongs beside the root, not beside
    /// the individual file.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_committed_output_in_root(
        &mut self,
        slot_index: usize,
        output_root: &Path,
        source_path: Option<&Path>,
        destination_path: &Path,
        title: &str,
        artist: &str,
        facts: &CommittedOutputFacts,
    ) -> W4djResult<String> {
        self.upsert_lightweight_output_inner(
            slot_index,
            source_path,
            destination_path,
            Some(output_root),
            title,
            artist,
            Some(facts),
        )
    }

    /// Register one output only after its final safe commit has completed.
    pub fn upsert_output_file(
        &mut self,
        slot_index: usize,
        output_root: &Path,
        source_path: Option<&Path>,
        destination_path: &Path,
    ) -> W4djResult<String> {
        let metadata = fs::metadata(destination_path)
            .map_err(|error| W4djLibraryError::Invalid(format!("输出文件无法登记：{error}")))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(W4djLibraryError::Invalid(
                "输出文件不是有效音频文件".to_string(),
            ));
        }
        let destination = normalize_path(destination_path);
        let root = normalize_path(output_root);
        let requested_source = source_path.map(normalize_path);
        let track_key = format!("output:{destination}");
        let now = now_ms();
        let measured_format = destination_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        // Output registration is deliberately tolerant of environments where
        // the bundled FFmpeg sidecar is unavailable.  In that case the
        // extension/size facts still remain authoritative and a later invalid
        // scan or analysis pass can enrich the missing probe fields.
        let probe = probe_local_audio(destination_path).ok();
        let measured_format = probe
            .as_ref()
            .map(|facts| facts.format.clone())
            .or(measured_format);
        let metadata_values = read_embedded_track_metadata(destination_path);
        let existing_source = self
            .catalog
            .connection()
            .query_row(
                "SELECT source_path FROM w4dj_track_meta WHERE track_key=?1 LIMIT 1",
                [&track_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let known_database_source = existing_source;
        let source = requested_source.clone().or(known_database_source);
        let mut track = self
            .catalog
            .track_detail(&track_key)?
            .unwrap_or_else(CatalogTrack::default);
        track.track_key = track_key.clone();
        // This is an output-library record, not a NetEase retrieval record.
        // Do not carry an old source ID into W4DJ just because the same path
        // was indexed by an earlier build.
        track.netease_track_id = None;
        if !metadata_values.title.trim().is_empty() {
            track.title = metadata_values.title;
        } else if track.title.trim().is_empty() {
            track.title = destination_path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| destination.clone());
        }
        if !metadata_values.artist.trim().is_empty() {
            track.artists = metadata_values.artist;
        }
        if !metadata_values.album.trim().is_empty() {
            track.album = metadata_values.album;
        }
        if !metadata_values.genre.trim().is_empty() {
            track.netease_genre = metadata_values.genre;
        }
        track.artist_list_json = serde_json::to_string(
            &track
                .artists
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        track.local_status = LocalStatus::Available;
        track.measured_format = measured_format.clone();
        track.effective_format = measured_format;
        track.measured_size_bytes = Some(
            probe
                .as_ref()
                .map(|facts| facts.size_bytes)
                .unwrap_or_else(|| metadata.len().min(i64::MAX as u64) as i64),
        );
        track.effective_size_bytes = track.measured_size_bytes;
        track.measured_duration_seconds = probe.as_ref().and_then(|facts| facts.duration_seconds);
        track.effective_duration_seconds = track.measured_duration_seconds;
        track.duration_source = track
            .measured_duration_seconds
            .map(|_| crate::library_catalog::DurationSource::Measured);
        track.measured_bitrate_bps = probe.as_ref().and_then(|facts| facts.average_bitrate_bps);
        track.effective_bitrate_bps = track.measured_bitrate_bps;
        track.updated_at_ms = now;
        self.catalog.upsert_track(&track)?;
        self.catalog.upsert_local_file(&CatalogLocalFile {
            id: None,
            track_key: track_key.clone(),
            path: PathBuf::from(&destination),
            size_bytes: probe
                .as_ref()
                .map(|facts| facts.size_bytes)
                .unwrap_or_else(|| metadata.len().min(i64::MAX as u64) as i64),
            modified_at_ms: modified_at_ms(&metadata),
            measured_format: track.effective_format.clone(),
            measured_bitrate_bps: track.measured_bitrate_bps,
            measured_duration_seconds: track.measured_duration_seconds,
            sample_rate_hz: probe.as_ref().and_then(|facts| facts.sample_rate_hz),
            channels: probe.as_ref().and_then(|facts| facts.channels),
            readable: true,
            probe_error: None,
        })?;
        let connection = self.catalog.connection();
        connection.execute(
            "INSERT INTO output_roots(root_path, first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?2)
             ON CONFLICT(root_path) DO UPDATE SET last_seen_at_ms=excluded.last_seen_at_ms",
            params![root, now],
        )?;
        let previous_root = connection
            .query_row(
                "SELECT root_path FROM slot_output_roots WHERE slot_index=?1",
                [slot_index as i64],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        connection.execute(
            "INSERT INTO slot_output_roots(slot_index, root_path, applied_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(slot_index) DO UPDATE SET root_path=excluded.root_path, applied_at_ms=excluded.applied_at_ms",
            params![slot_index as i64, root, now],
        )?;
        connection.execute(
            "INSERT INTO w4dj_track_meta(
                track_key, source_path, destination_path, slot_index, output_root,
                status, analysis_status, measured_duration_seconds, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'available',
                COALESCE((SELECT analysis_status FROM w4dj_track_meta WHERE track_key=?1), 'notAnalyzed'),
                NULL, COALESCE((SELECT created_at_ms FROM w4dj_track_meta WHERE track_key=?1), ?6), ?6)
             ON CONFLICT(track_key) DO UPDATE SET
                source_path=excluded.source_path,
                destination_path=excluded.destination_path,
                slot_index=excluded.slot_index,
                output_root=excluded.output_root,
                status='available',
                updated_at_ms=excluded.updated_at_ms",
            params![track_key, source, destination, slot_index as i64, root, now],
        )?;
        if let Some(previous_root) = previous_root
            && previous_root != root
            && !connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM slot_output_roots WHERE root_path=?1)",
                [&previous_root],
                |row| row.get::<_, bool>(0),
            )?
        {
            connection.execute(
                "UPDATE w4dj_track_meta SET status='outOfScope'
                 WHERE output_root=?1 AND track_key<>?2",
                params![previous_root, track_key],
            )?;
            connection.execute(
                "UPDATE tracks SET local_status='out_of_scope', updated_at_ms=?1
                 WHERE track_key IN (SELECT track_key FROM w4dj_track_meta WHERE status='outOfScope')",
                [now],
            )?;
        }
        Ok(track_key)
    }

    pub fn apply_analysis_for_destination(
        &mut self,
        destination_path: &Path,
        entry: &TrackAnalysis,
    ) -> W4djResult<bool> {
        let destination = normalize_path(destination_path);
        let Some(track_key) = self
            .catalog
            .connection()
            .query_row(
                "SELECT track_key FROM w4dj_track_meta WHERE destination_path=?1",
                [&destination],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        self.apply_analysis_for_track(&track_key, &destination, entry)
    }

    /// Marks a newly committed output as needing analysis. Conversion can
    /// replace the bytes behind an existing destination path; any previous
    /// Essentia projection then belongs to the old file and must not remain
    /// visible in the Dashboard.
    pub fn invalidate_analysis_for_destination(
        &mut self,
        destination_path: &Path,
    ) -> W4djResult<bool> {
        let destination = normalize_path(destination_path);
        let Some(track_key) = self
            .catalog
            .connection()
            .query_row(
                "SELECT track_key FROM w4dj_track_meta WHERE destination_path=?1",
                [&destination],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let now = now_ms();
        let connection = self.catalog.connection();
        connection.execute(
            "DELETE FROM analysis_results WHERE track_key=?1",
            [&track_key],
        )?;
        connection.execute(
            "UPDATE w4dj_track_meta SET analysis_status='notAnalyzed', analysis_error=NULL, updated_at_ms=?1 WHERE track_key=?2",
            params![now, track_key],
        )?;
        connection.execute(
            "UPDATE tracks SET essentia_genre='', essentia_duration_seconds=NULL,
                effective_duration_seconds=measured_duration_seconds,
                duration_source=CASE WHEN measured_duration_seconds IS NOT NULL THEN 'measured' ELSE NULL END,
                bpm=NULL, musical_key=NULL, scale=NULL, integrated_loudness_lufs=NULL,
                loudness_range_lu=NULL, energy=NULL, danceability=NULL, mood_json='[]',
                instrument_json='[]', discogs_mood_theme_json='[]',
                discogs_approachability_json='{}', discogs_instrumentation_json='[]',
                discogs_timbre_json='{}', discogs_danceability_json='{}',
                drop_loudness_lufs=NULL, updated_at_ms=?1
             WHERE track_key=?2",
            params![now, track_key],
        )?;
        Ok(true)
    }

    pub fn mark_analysis_failed_for_destination(
        &mut self,
        destination_path: &Path,
        error: &str,
    ) -> W4djResult<bool> {
        let destination = normalize_path(destination_path);
        let Some(track_key) = self
            .catalog
            .connection()
            .query_row(
                "SELECT track_key FROM w4dj_track_meta WHERE destination_path=?1",
                [&destination],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let now = now_ms();
        let connection = self.catalog.connection();
        let existing_status = connection
            .query_row(
                "SELECT status FROM analysis_results WHERE track_key=?1",
                [&track_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_status.as_deref() == Some(AnalysisStatus::Completed.as_str()) {
            // A failed re-analysis must not destroy the last known-good
            // projection. The caller still records the failed attempt in its
            // conversion report, while Dashboard keeps the completed value.
            return Ok(true);
        }
        connection.execute(
            "INSERT INTO analysis_results(track_key,destination_path,status,error,analysis_json,analyzed_at_ms)
             VALUES (?1,?2,'failed',?3,'{}',?4)
             ON CONFLICT(track_key) DO UPDATE SET destination_path=excluded.destination_path,
                status='failed',error=excluded.error,analysis_json='{}',analyzed_at_ms=excluded.analyzed_at_ms",
            params![track_key, destination, error, now],
        )?;
        connection.execute(
            "UPDATE w4dj_track_meta SET analysis_status='failed', analysis_error=?1, updated_at_ms=?2 WHERE track_key=?3",
            params![error, now, track_key],
        )?;
        Ok(true)
    }

    pub fn apply_analysis_entries(&mut self, entries: &[TrackAnalysis]) -> W4djResult<usize> {
        let connection = self.catalog.connection();
        let mut statement = connection
            .prepare("SELECT track_key, destination_path, source_path FROM w4dj_track_meta")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mappings = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut updated = 0;
        for entry in entries {
            if let Some((track_key, destination, _)) =
                mappings.iter().find(|(_, destination, source)| {
                    source.as_deref().is_some_and(|value| {
                        normalize_path(Path::new(value)) == normalize_path(Path::new(&entry.path))
                    }) || destination == &normalize_path(Path::new(&entry.path))
                })
                && self.apply_analysis_for_track(track_key, destination, entry)?
            {
                updated += 1;
            }
        }
        Ok(updated)
    }

    fn apply_analysis_for_track(
        &mut self,
        track_key: &str,
        destination: &str,
        entry: &TrackAnalysis,
    ) -> W4djResult<bool> {
        let status = analysis_status(entry);
        let error = entry
            .high_level
            .as_ref()
            .and_then(|value| value.reason.clone());
        let analysis_json = serde_json::to_string(entry)
            .map_err(|error| W4djLibraryError::Invalid(format!("序列化分析结果失败：{error}")))?;
        let connection = self.catalog.connection();
        let genre = analysis_genre(entry);
        let mood = labels_json(entry.high_level.as_ref().map(|value| &value.mood));
        let instrument = labels_json(entry.high_level.as_ref().map(|value| &value.instrument));
        let style = labels_json(entry.high_level.as_ref().map(|value| &value.style));
        let existing_discogs =
            connection_query_discogs_projection(self.catalog.connection(), track_key)?;
        let mut discogs = existing_discogs;
        if let Some(analysis) = entry
            .high_level
            .as_ref()
            .and_then(|value| value.discogs_effnet.as_ref())
        {
            for (index, id) in [
                "moodTheme",
                "approachability",
                "instrumentation",
                "timbre",
                "danceability",
            ]
            .iter()
            .enumerate()
            {
                if let Some(head) = analysis.heads.get(*id)
                    && head.status == "completed"
                {
                    discogs[index] =
                        serde_json::to_string(head).unwrap_or_else(|_| discogs[index].clone());
                }
            }
        }
        let now = now_ms();
        let changed = connection.execute(
            "UPDATE tracks SET essentia_genre=?1, essentia_duration_seconds=?2,
                effective_duration_seconds=?2, duration_source=CASE WHEN ?2 IS NOT NULL THEN 'essentia' ELSE duration_source END,
                bpm=?3, musical_key=?4, scale=?5, integrated_loudness_lufs=?6,
                loudness_range_lu=?7, energy=?8, danceability=?9, mood_json=?10,
                instrument_json=?11, style_json=?12, discogs_mood_theme_json=?13,
                discogs_approachability_json=?14, discogs_instrumentation_json=?15,
                discogs_timbre_json=?16, discogs_danceability_json=?17,
                drop_loudness_lufs=?18, updated_at_ms=?19
             WHERE track_key=?20",
            params![
                genre,
                entry.duration_seconds,
                entry.bpm,
                entry.key,
                entry.scale,
                entry.integrated_loudness_lufs,
                entry.loudness_range_lu,
                entry.energy,
                entry.danceability,
                mood,
                instrument,
                style,
                discogs[0],
                discogs[1],
                discogs[2],
                discogs[3],
                discogs[4],
                entry.drop_loudness_lufs,
                now,
                track_key,
            ],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        connection.execute(
            "INSERT INTO analysis_results(track_key,destination_path,status,error,analysis_json,analyzed_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(track_key) DO UPDATE SET destination_path=excluded.destination_path,
                status=excluded.status,error=excluded.error,analysis_json=excluded.analysis_json,
                analyzed_at_ms=excluded.analyzed_at_ms",
            params![track_key, destination, status.as_str(), error, analysis_json, now],
        )?;
        connection.execute(
            "UPDATE w4dj_track_meta SET analysis_status=?1, analysis_error=?2, updated_at_ms=?3 WHERE track_key=?4",
            params![status.as_str(), error, now, track_key],
        )?;
        Ok(true)
    }

    pub fn clear_analyses(&mut self) -> W4djResult<()> {
        let connection = self.catalog.connection();
        connection.execute("DELETE FROM analysis_results", [])?;
        connection.execute(
            "UPDATE w4dj_track_meta SET analysis_status='notAnalyzed', analysis_error=NULL, updated_at_ms=?1",
            [now_ms()],
        )?;
        connection.execute(
            "UPDATE tracks SET essentia_genre='', essentia_duration_seconds=NULL,
                effective_duration_seconds=measured_duration_seconds,
                duration_source=CASE WHEN measured_duration_seconds IS NOT NULL THEN 'measured' ELSE NULL END,
                bpm=NULL, musical_key=NULL, scale=NULL, integrated_loudness_lufs=NULL,
                loudness_range_lu=NULL, energy=NULL, danceability=NULL, mood_json='[]',
                instrument_json='[]', style_json='[]', discogs_mood_theme_json='[]',
                discogs_approachability_json='{}', discogs_instrumentation_json='[]',
                discogs_timbre_json='{}', discogs_danceability_json='{}',
                drop_loudness_lufs=NULL, updated_at_ms=?1",
            [now_ms()],
        )?;
        Ok(())
    }

    /// Reconcile one or more scanned output roots with the files that were
    /// actually observed on disk.  Existing rows (and their analysis) are
    /// retained when the destination path remains present; rows that
    /// disappeared from a participating root are removed atomically together
    /// with their analysis projection.  Roots not included in `roots` are
    /// deliberately untouched.
    pub fn reconcile_output_roots(
        &mut self,
        roots: &[(usize, PathBuf, Vec<PathBuf>)],
    ) -> W4djResult<OutputReconcileSummary> {
        let mut snapshots = Vec::with_capacity(roots.len());
        for (slot_index, root_path, paths) in roots {
            let root = normalize_path(root_path);
            if root.is_empty() {
                continue;
            }
            let mut destinations = HashSet::with_capacity(paths.len());
            let mut files = Vec::with_capacity(paths.len());
            for path in paths {
                let metadata = fs::metadata(path).map_err(|error| {
                    W4djLibraryError::Invalid(format!("输出文件无法登记：{error}"))
                })?;
                if !metadata.is_file() || metadata.len() == 0 {
                    return Err(W4djLibraryError::Invalid(
                        "输出扫描发现无效音频文件".to_string(),
                    ));
                }
                let destination = normalize_path(path);
                if !destinations.insert(destination.clone()) {
                    continue;
                }
                files.push(ScannedOutputFile {
                    destination,
                    title: path
                        .file_stem()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string()),
                    size_bytes: metadata.len().min(i64::MAX as u64) as i64,
                    modified_at_ms: modified_at_ms(&metadata),
                    measured_format: path
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(str::to_ascii_lowercase),
                    identity: None,
                });
            }
            snapshots.push(ScannedOutputRoot {
                slot_index: *slot_index,
                root,
                destinations,
                files,
            });
        }

        self.reconcile_scanned_output_roots(snapshots, true)
    }

    /// Reconcile output roots from the snapshots collected by the scan walk.
    /// This is the production path: it performs no filesystem metadata reads
    /// and therefore cannot turn reconciliation into a second output scan.
    pub fn reconcile_output_snapshots(
        &mut self,
        roots: &[(usize, PathBuf, Vec<OutputFileSnapshot>)],
    ) -> W4djResult<OutputReconcileSummary> {
        let mut snapshots = Vec::with_capacity(roots.len());
        for (slot_index, root_path, files) in roots {
            let root = normalize_path(root_path);
            if root.is_empty() {
                continue;
            }
            let mut destinations = HashSet::with_capacity(files.len());
            let mut scanned_files = Vec::with_capacity(files.len());
            for file in files {
                if file.size_bytes == 0 {
                    return Err(W4djLibraryError::Invalid(
                        "输出扫描发现无效音频文件".to_string(),
                    ));
                }
                // The walk already captured the filesystem facts.  Rebuild a
                // stable absolute key from the normalized root and the
                // snapshot's relative path instead of canonicalizing/stat'ing
                // each output again during reconciliation.
                let destination = snapshot_path_key(root_path, &root, &file.path);
                if !destinations.insert(destination.clone()) {
                    continue;
                }
                scanned_files.push(ScannedOutputFile {
                    destination,
                    title: file
                        .path
                        .file_stem()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| file.path.display().to_string()),
                    size_bytes: file.size_bytes.min(i64::MAX as u64) as i64,
                    modified_at_ms: file
                        .modified_at_ms
                        .map(|value| value.min(i64::MAX as u64) as i64),
                    measured_format: file
                        .path
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(str::to_ascii_lowercase),
                    identity: None,
                });
            }
            snapshots.push(ScannedOutputRoot {
                slot_index: *slot_index,
                root,
                destinations,
                files: scanned_files,
            });
        }

        self.reconcile_scanned_output_roots(snapshots, true)
    }

    /// Rehydrate output rows from the durable sidecars without treating the
    /// sidecar as a complete filesystem snapshot. This is used at the playlist
    /// matching boundary after the derived SQLite index was cleared: it adds
    /// existing sidecar-backed files and never removes unrelated rows.
    fn output_identity_roots_with_stored(
        &self,
        roots: &[(usize, PathBuf)],
    ) -> W4djResult<Vec<(usize, PathBuf)>> {
        let mut roots_to_use = roots.to_vec();
        let mut known_roots = roots
            .iter()
            .map(|(_, root)| normalize_index_path(root))
            .collect::<HashSet<_>>();
        let mut add_root = |slot_index: usize, root: String| {
            let root = normalize_index_path(Path::new(&root));
            if !root.is_empty() && known_roots.insert(root.clone()) {
                roots_to_use.push((slot_index, PathBuf::from(root)));
            }
        };
        let stored_slot_roots = {
            let mut statement = self.catalog.connection().prepare(
                "SELECT slot_index,root_path FROM slot_output_roots
                 ORDER BY slot_index",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, usize>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (slot_index, root) in stored_slot_roots {
            add_root(slot_index, root);
        }
        let stored_roots = {
            let mut statement = self
                .catalog
                .connection()
                .prepare("SELECT root_path FROM output_roots ORDER BY root_path")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for root in stored_roots {
            add_root(0, root);
        }
        Ok(roots_to_use)
    }

    /// Materialize the current output identities and active playlist
    /// bindings into the adjacent recovery sidecars. This is intentionally an
    /// explicit W4DJ boundary: ordinary conversion, scanning, and library
    /// maintenance update SQLite only. Output entries are merged so a
    /// temporary or partially rebuilt index cannot erase recovery data;
    /// playlist entries are replaced per known playlist so a changed manual
    /// binding cannot resurrect an obsolete path.
    pub fn persist_output_identity_manifests(
        &self,
        roots: &[(usize, PathBuf)],
    ) -> W4djResult<usize> {
        let roots = self.output_identity_roots_with_stored(roots)?;
        if roots.is_empty() {
            return Ok(0);
        }
        let rows = {
            let mut statement = self.catalog.connection().prepare(
                "SELECT m.destination_path,m.source_path,t.title,t.artists
                 FROM w4dj_track_meta m
                 JOIN tracks t ON t.track_key=m.track_key
                 WHERE m.destination_path IS NOT NULL
                 ORDER BY m.destination_path",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut entries_by_root = HashMap::<String, Vec<OutputIdentityManifestEntry>>::new();
        for (destination, source, title, artists) in rows {
            let Some(root) = roots
                .iter()
                .map(|(_, root)| normalize_index_path(root))
                .filter(|root| {
                    Path::new(&destination)
                        .strip_prefix(Path::new(root))
                        .is_ok()
                })
                .max_by_key(String::len)
            else {
                continue;
            };
            let Some(relative_path) = safe_manifest_relative_path(&relative_manifest_path(
                Path::new(&destination),
                Path::new(&root),
            )) else {
                continue;
            };
            entries_by_root
                .entry(root)
                .or_default()
                .push(OutputIdentityManifestEntry {
                    relative_path,
                    source_path: source.filter(|value| !value.trim().is_empty()),
                    title: title.trim().to_string(),
                    artist_display: artists.trim().to_string(),
                });
        }

        let playlist_ids = {
            let mut statement = self
                .catalog
                .connection()
                .prepare("SELECT playlist_id FROM imported_dj_playlists")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut playlist_updates_by_root =
            HashMap::<String, HashMap<String, Vec<OutputPlaylistManifestTrack>>>::new();
        let mut statement = self.catalog.connection().prepare(
            "SELECT p.playlist_id,m.position,o.destination_path,p_track.title,
                    p_track.artist_display,m.score,m.match_method,m.candidate_source,
                    o.status
             FROM imported_dj_playlists p
             JOIN imported_dj_playlist_matches m ON m.playlist_id=p.playlist_id
             JOIN imported_dj_playlist_tracks p_track
               ON p_track.playlist_id=m.playlist_id AND p_track.position=m.position
             JOIN w4dj_track_meta o ON o.track_key=m.track_key
             WHERE m.status='matched' AND m.excluded=0
             ORDER BY p.playlist_id,m.position",
        )?;
        let reviewed_rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i32>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;
        for row in reviewed_rows {
            let row = row?;
            let (
                playlist_id,
                position,
                destination,
                title,
                artist_display,
                score,
                match_method,
                candidate_source,
                status,
            ) = (
                row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8,
            );
            if status != "available" {
                continue;
            }
            let Some(root) = roots
                .iter()
                .map(|(_, root)| normalize_index_path(root))
                .filter(|root| {
                    Path::new(&destination)
                        .strip_prefix(Path::new(root))
                        .is_ok()
                })
                .max_by_key(String::len)
            else {
                continue;
            };
            let Some(relative_path) = safe_manifest_relative_path(&relative_manifest_path(
                Path::new(&destination),
                Path::new(&root),
            )) else {
                continue;
            };
            playlist_updates_by_root
                .entry(root)
                .or_default()
                .entry(playlist_id)
                .or_default()
                .push(OutputPlaylistManifestTrack {
                    position,
                    relative_path,
                    title,
                    artist_display,
                    score,
                    match_method,
                    candidate_source,
                });
        }

        let mut persisted = 0;
        for (_, root_path) in roots {
            let root = normalize_index_path(&root_path);
            let entries = entries_by_root.remove(&root).unwrap_or_default();
            let updates = playlist_updates_by_root
                .remove(&root)
                .unwrap_or_default()
                .into_iter()
                .map(|(playlist_id, mut tracks)| {
                    tracks.sort_unstable_by_key(|track| track.position);
                    OutputPlaylistManifest {
                        playlist_id,
                        tracks,
                    }
                })
                .collect::<Vec<_>>();
            let manifest_exists = output_identity_manifest_path(Path::new(&root)).is_file();
            if entries.is_empty()
                && updates.is_empty()
                && (!manifest_exists || playlist_ids.is_empty())
            {
                continue;
            }
            persisted += entries.len();
            persist_output_identity_manifest_updates(
                Path::new(&root),
                entries,
                updates,
                &playlist_ids,
            )?;
        }
        Ok(persisted)
    }

    /// Restore active playlist bindings from the explicit W4DJ sidecars
    /// after the output index has been rebuilt. Bindings are accepted only
    /// when their stored title/artist still identifies the current playlist
    /// row and their relative path resolves to a real local audio file.
    pub fn restore_imported_dj_playlist_review_manifests(
        &mut self,
        playlist_id: &str,
        roots: &[(usize, PathBuf)],
    ) -> W4djResult<usize> {
        if playlist_id.trim().is_empty() {
            return Err(W4djLibraryError::Invalid("DJ 歌单 ID 不能为空".to_string()));
        }
        let roots_to_restore = self.output_identity_roots_with_stored(roots)?;
        let mut bindings = Vec::new();
        for (_, root_path) in roots_to_restore {
            let root = normalize_path(&root_path);
            if root.is_empty() {
                continue;
            }
            let manifest = load_output_identity_manifest(Path::new(&root));
            for playlist in manifest
                .playlists
                .into_iter()
                .filter(|playlist| playlist.playlist_id == playlist_id)
            {
                for binding in playlist.tracks {
                    let Some(relative_path) = safe_manifest_relative_path(&binding.relative_path)
                    else {
                        continue;
                    };
                    let destination = Path::new(&root).join(relative_path);
                    let Ok(metadata) = fs::symlink_metadata(&destination) else {
                        continue;
                    };
                    if metadata.file_type().is_symlink()
                        || !metadata.is_file()
                        || metadata.len() == 0
                        || !is_audio_path(&destination)
                    {
                        continue;
                    }
                    bindings.push((root.clone(), destination, binding));
                }
            }
        }
        if bindings.is_empty() {
            return Ok(0);
        }

        let mut report = self.get_imported_dj_playlist_match_report(playlist_id)?;
        let mut candidates = self.available_dj_output_candidates()?;
        let mut restored = 0;
        for (root, destination, binding) in bindings {
            let normalized_destination = normalize_path(&destination);
            if !candidates.iter().any(|candidate| {
                normalize_path(&candidate.destination_path) == normalized_destination
            }) {
                self.upsert_output_file(0, Path::new(&root), None, &destination)?;
                candidates = self.available_dj_output_candidates()?;
            }
            let Some(candidate) = candidates.iter().find(|candidate| {
                normalize_path(&candidate.destination_path) == normalized_destination
            }) else {
                continue;
            };
            let Some(row) = report
                .matches
                .iter_mut()
                .find(|row| row.position == binding.position)
            else {
                continue;
            };
            if identity_key_for(&row.title, &row.artist_display)
                != identity_key_for(&binding.title, &binding.artist_display)
            {
                continue;
            }
            let manual = binding.match_method.as_deref() == Some("manual");
            row.kind = if manual {
                DjPlaylistMatchKind::Manual
            } else if binding.match_method.as_deref() == Some("recentBm25f") {
                DjPlaylistMatchKind::RecentBm25f
            } else {
                DjPlaylistMatchKind::LibraryBm25f
            };
            row.status = "matched".to_string();
            row.track_key = Some(candidate.track_key.clone());
            row.match_method = binding.match_method.clone();
            row.score = binding.score;
            row.reason = "从 W4DJ 隐藏复核清单恢复".to_string();
            row.manual = manual;
            row.destination_path = Some(candidate.destination_path.clone());
            row.candidate_source = binding.candidate_source.clone();
            row.confirmed = true;
            if !row
                .candidates
                .iter()
                .any(|item| item.track_key == candidate.track_key)
            {
                row.candidates
                    .push(crate::dj_playlist_match::DjPlaylistMatchCandidate {
                        track_key: candidate.track_key.clone(),
                        title: candidate.title.clone(),
                        artist_display: candidate.artist_display.clone(),
                        duration_seconds: candidate.duration_seconds,
                        destination_filename: candidate_filename(candidate),
                        score: binding.score.unwrap_or(100),
                        reason: "从 W4DJ 隐藏复核清单恢复".to_string(),
                    });
            }
            restored += 1;
        }
        if restored > 0 {
            self.replace_imported_dj_playlist_matches(playlist_id, &report)?;
        }
        Ok(restored)
    }

    pub fn restore_output_identity_manifests(
        &mut self,
        roots: &[(usize, PathBuf)],
    ) -> W4djResult<usize> {
        let roots_to_restore = self.output_identity_roots_with_stored(roots)?;

        let mut snapshots = Vec::with_capacity(roots_to_restore.len());
        let mut restored = 0;
        for (slot_index, root_path) in &roots_to_restore {
            let root = normalize_path(root_path);
            if root.is_empty() {
                continue;
            }
            let manifest = load_output_identity_manifest(Path::new(&root));
            let mut destinations = HashSet::new();
            let mut files = Vec::new();
            for identity in manifest.outputs {
                let Some(relative_path) = safe_manifest_relative_path(&identity.relative_path)
                else {
                    continue;
                };
                let destination_path = Path::new(&root).join(&relative_path);
                let Ok(metadata) = fs::metadata(&destination_path) else {
                    continue;
                };
                if !metadata.is_file() || metadata.len() == 0 {
                    continue;
                }
                let destination = normalize_path(&destination_path);
                if !destinations.insert(destination.clone()) {
                    continue;
                }
                files.push(ScannedOutputFile {
                    destination,
                    title: destination_path
                        .file_stem()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| destination_path.display().to_string()),
                    size_bytes: metadata.len().min(i64::MAX as u64) as i64,
                    modified_at_ms: modified_at_ms(&metadata),
                    measured_format: destination_path
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(str::to_ascii_lowercase),
                    identity: Some(OutputIdentityManifestEntry {
                        relative_path,
                        ..identity
                    }),
                });
                restored += 1;
            }
            snapshots.push(ScannedOutputRoot {
                slot_index: *slot_index,
                root,
                destinations,
                files,
            });
        }
        self.reconcile_scanned_output_roots(snapshots, false)?;
        Ok(restored)
    }

    fn reconcile_scanned_output_roots(
        &mut self,
        snapshots: Vec<ScannedOutputRoot>,
        prune_missing: bool,
    ) -> W4djResult<OutputReconcileSummary> {
        let transaction = self.catalog.connection_mut().transaction()?;
        let now = now_ms();
        let mut summary = OutputReconcileSummary::default();
        for snapshot in snapshots {
            transaction.execute(
                "INSERT INTO output_roots(root_path,first_seen_at_ms,last_seen_at_ms)
                 VALUES (?1,?2,?2)
                 ON CONFLICT(root_path) DO UPDATE SET last_seen_at_ms=excluded.last_seen_at_ms",
                params![snapshot.root, now],
            )?;
            transaction.execute(
                "INSERT INTO slot_output_roots(slot_index,root_path,applied_at_ms)
                 VALUES (?1,?2,?3)
                 ON CONFLICT(slot_index) DO UPDATE SET root_path=excluded.root_path, applied_at_ms=excluded.applied_at_ms",
                params![snapshot.slot_index as i64, snapshot.root, now],
            )?;

            for file in snapshot.files {
                let existing_key = transaction
                    .query_row(
                        "SELECT track_key FROM w4dj_track_meta WHERE destination_path=?1 LIMIT 1",
                        [&file.destination],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                let previous_fingerprint = if let Some(track_key) = existing_key.as_deref() {
                    transaction
                        .query_row(
                            "SELECT size_bytes,modified_at_ms FROM local_files
                             WHERE track_key=?1 AND path=?2 LIMIT 1",
                            params![track_key, file.destination],
                            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
                        )
                        .optional()?
                } else {
                    None
                };
                let fingerprint_unchanged =
                    previous_fingerprint.is_some_and(|(size_bytes, modified_at_ms)| {
                        size_bytes == file.size_bytes
                            && modified_at_ms.is_some()
                            && modified_at_ms == file.modified_at_ms
                    });
                let track_key = existing_key
                    .clone()
                    .unwrap_or_else(|| format!("output:{}", file.destination));
                let stored_track_metadata = existing_key
                    .as_deref()
                    .map(|track_key| {
                        transaction
                            .query_row(
                                "SELECT title,artists
                                 FROM tracks WHERE track_key=?1 LIMIT 1",
                                [track_key],
                                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                            )
                            .optional()
                    })
                    .transpose()?
                    .flatten();
                let stored_source_path = existing_key
                    .as_deref()
                    .map(|track_key| {
                        transaction
                            .query_row(
                                "SELECT source_path
                                 FROM w4dj_track_meta WHERE track_key=?1 LIMIT 1",
                                [track_key],
                                |row| row.get::<_, Option<String>>(0),
                            )
                            .optional()
                    })
                    .transpose()?
                    .flatten()
                    .flatten();
                let title = file
                    .identity
                    .as_ref()
                    .map(|identity| identity.title.trim())
                    .filter(|value| !value.is_empty())
                    .or_else(|| {
                        stored_track_metadata
                            .as_ref()
                            .map(|metadata| metadata.0.trim())
                            .filter(|value| !value.is_empty())
                    })
                    .unwrap_or(file.title.as_str())
                    .to_string();
                let artists = file
                    .identity
                    .as_ref()
                    .map(|identity| identity.artist_display.trim())
                    .filter(|value| !value.is_empty())
                    .or_else(|| {
                        stored_track_metadata
                            .as_ref()
                            .map(|metadata| metadata.1.trim())
                            .filter(|value| !value.is_empty())
                    })
                    .unwrap_or("")
                    .to_string();
                let artist_list_json = serde_json::to_string(
                    &artists
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_else(|_| "[]".to_string());
                let source_path = file
                    .identity
                    .as_ref()
                    .and_then(|identity| identity.source_path.clone());
                let source_path = source_path.or(stored_source_path);
                transaction.execute(
                    "INSERT INTO tracks(
                        track_key,title,artists,artist_list_json,
                        local_status,updated_at_ms
                     ) VALUES (?1,?2,?3,?4,'available',?5)
                     ON CONFLICT(track_key) DO UPDATE SET
                        local_status='available',updated_at_ms=excluded.updated_at_ms",
                    params![track_key, title, artists, artist_list_json, now],
                )?;
                if let Some(identity) = file.identity.as_ref()
                    && (!identity.title.trim().is_empty()
                        || !identity.artist_display.trim().is_empty())
                {
                    transaction.execute(
                        "UPDATE tracks SET
                            title=CASE WHEN TRIM(?1)<>'' THEN ?1 ELSE title END,
                            artists=CASE WHEN TRIM(?2)<>'' THEN ?2 ELSE artists END,
                            artist_list_json=CASE WHEN TRIM(?2)<>'' THEN ?3 ELSE artist_list_json END
                         WHERE track_key=?4",
                        params![
                            identity.title.trim(),
                            identity.artist_display.trim(),
                            artist_list_json,
                            track_key
                        ],
                    )?;
                }
                transaction.execute(
                    "INSERT INTO local_files(track_key,path,size_bytes,modified_at_ms,measured_format,readable,probe_error)
                     VALUES (?1,?2,?3,?4,?5,1,NULL)
                     ON CONFLICT(path) DO UPDATE SET track_key=excluded.track_key,
                        size_bytes=excluded.size_bytes,modified_at_ms=excluded.modified_at_ms,
                        measured_format=excluded.measured_format,readable=1,probe_error=NULL",
                    params![
                        track_key,
                        file.destination,
                        file.size_bytes,
                        file.modified_at_ms,
                        file.measured_format,
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO w4dj_track_meta(
                        track_key,source_path,destination_path,slot_index,output_root,status,
                        analysis_status,analysis_error,created_at_ms,updated_at_ms
                     ) VALUES (?1,?2,?3,?4,?5,'available','notAnalyzed',NULL,?6,?6)
                     ON CONFLICT(track_key) DO UPDATE SET source_path=COALESCE(excluded.source_path,w4dj_track_meta.source_path),
                        destination_path=excluded.destination_path,
                        slot_index=excluded.slot_index,output_root=excluded.output_root,
                        status='available',updated_at_ms=excluded.updated_at_ms",
                    params![
                        track_key,
                        source_path,
                        file.destination,
                        snapshot.slot_index as i64,
                        snapshot.root,
                        now
                    ],
                )?;
                if existing_key.is_some() && !fingerprint_unchanged {
                    invalidate_analysis_projection(&transaction, &track_key, now)?;
                    summary
                        .invalidated_paths
                        .push(PathBuf::from(&file.destination));
                }
            }

            if prune_missing {
                let stale = {
                    let mut statement = transaction.prepare(
                        "SELECT track_key,destination_path FROM w4dj_track_meta WHERE output_root=?1",
                    )?;
                    statement
                        .query_map([snapshot.root.as_str()], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                for (track_key, destination) in stale {
                    if !snapshot.destinations.contains(&destination) {
                        transaction
                            .execute("DELETE FROM tracks WHERE track_key=?1", [&track_key])?;
                        summary.removed_paths.push(PathBuf::from(destination));
                    }
                }
            }
        }
        transaction.commit()?;
        Ok(summary)
    }

    /// Remove all output-owned tracks and analyses while leaving imported
    /// playlists, history, preferences and source scan caches untouched.
    pub fn clear_output_library(&mut self) -> W4djResult<()> {
        let transaction = self.catalog.connection_mut().transaction()?;
        // Output identities are durable provenance, not a rebuildable index.
        // Keep them (and the adjacent sidecars) so a later scan can restore
        // NetEase IDs after the derived SQLite rows have been cleared. Keep
        // the last known output roots as discovery hints as well; the normal
        // UI configuration remains authoritative when it is available.
        transaction.execute("DELETE FROM tracks WHERE EXISTS (SELECT 1 FROM w4dj_track_meta WHERE w4dj_track_meta.track_key=tracks.track_key)", [])?;
        transaction.execute("DELETE FROM w4dj_track_meta", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn query(&self, query: &LibraryQuery) -> W4djResult<LibraryPage> {
        Ok(self.catalog.query_with_extra_predicate(
            query,
            "EXISTS (SELECT 1 FROM w4dj_track_meta wm WHERE wm.track_key=t.track_key)",
        )?)
    }

    pub fn track_detail(&self, track_key: &str) -> W4djResult<Option<CatalogTrack>> {
        let owned = self.catalog.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM w4dj_track_meta WHERE track_key=?1)",
            [track_key],
            |row| row.get::<_, bool>(0),
        )?;
        if !owned {
            return Ok(None);
        }
        Ok(self.catalog.track_detail(track_key)?)
    }

    pub fn source_records_for_track(
        &self,
        track_key: &str,
    ) -> W4djResult<Vec<crate::library_catalog::CatalogSourceRecord>> {
        let _ = track_key;
        Ok(Vec::new())
    }

    pub fn local_files_for_track(&self, track_key: &str) -> W4djResult<Vec<CatalogLocalFile>> {
        Ok(self.catalog.local_files_for_track(track_key)?)
    }

    pub fn relocate_analyzed_track(&mut self, track_key: &str, path: &Path) -> W4djResult<()> {
        self.catalog.relocate_analyzed_track(track_key, path)?;
        let destination = normalize_path(path);
        self.catalog.connection().execute(
            "UPDATE w4dj_track_meta SET destination_path=?1, output_root=?2, status='available', updated_at_ms=?3 WHERE track_key=?4",
            params![
                destination,
                normalize_path(path.parent().unwrap_or_else(|| Path::new("."))),
                now_ms(),
                track_key
            ],
        )?;
        self.catalog.connection().execute(
            "UPDATE analysis_results SET destination_path=?1 WHERE track_key=?2",
            params![destination, track_key],
        )?;
        Ok(())
    }

    pub fn remove_analyzed_track(&mut self, track_key: &str) -> W4djResult<bool> {
        // Dashboard rows include outputs that have not been analyzed yet, so
        // removal is intentionally based on W4DJ ownership rather than the
        // legacy catalog's "completed analysis" predicate.
        let transaction = self.catalog.connection_mut().transaction()?;
        let removed = transaction.execute(
            "DELETE FROM tracks WHERE track_key=?1
             AND EXISTS (SELECT 1 FROM w4dj_track_meta WHERE track_key=?1)",
            [track_key],
        )?;
        transaction.commit()?;
        Ok(removed > 0)
    }

    pub fn readable_local_files(&self) -> W4djResult<Vec<CatalogLocalFile>> {
        // The local-file rows are the lightweight index.  Do not consult the
        // legacy availability state machine here; a selected path is checked
        // only when the caller actually reads/analyzes/exports it.
        Ok(self.catalog.readable_local_files()?)
    }

    /// Remove rows for W4DJ's own temporary/AppleDouble artifacts without
    /// touching the corresponding files on disk.  These files are never
    /// formal output tracks and may have been recorded by an older build
    /// before the scan filter was applied at ingestion time.
    pub fn remove_internal_temp_records(&mut self) -> W4djResult<u64> {
        let keys = {
            let mut statement = self
                .catalog
                .connection()
                .prepare("SELECT track_key, destination_path FROM w4dj_track_meta")?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(Result::ok)
                .filter(|(_, destination)| is_internal_temp_path(Path::new(destination)))
                .map(|(track_key, _)| track_key)
                .collect::<Vec<_>>()
        };
        if keys.is_empty() {
            return Ok(0);
        }
        let transaction = self.catalog.connection_mut().transaction()?;
        let mut removed = 0_u64;
        for track_key in keys {
            removed += transaction.execute(
                "DELETE FROM tracks WHERE track_key=?1
                     AND EXISTS (SELECT 1 FROM w4dj_track_meta WHERE track_key=?1)",
                [track_key],
            )? as u64;
        }
        transaction.commit()?;
        Ok(removed)
    }

    pub fn stats(&self) -> W4djResult<W4djLibraryStats> {
        let connection = self.catalog.connection();
        let count = |sql: &str| -> Result<u64, rusqlite::Error> {
            Ok(connection
                .query_row(sql, [], |row| row.get::<_, i64>(0))?
                .max(0) as u64)
        };
        Ok(W4djLibraryStats {
            total: count("SELECT COUNT(*) FROM w4dj_track_meta")?,
            available: count("SELECT COUNT(*) FROM w4dj_track_meta WHERE status='available'")?,
            invalid: count(
                "SELECT COUNT(*) FROM w4dj_track_meta WHERE status IN ('outOfScope','missing','unreadable')",
            )?,
            not_analyzed: count(
                "SELECT COUNT(*) FROM w4dj_track_meta WHERE analysis_status='notAnalyzed'",
            )?,
            analysis_failed: count(
                "SELECT COUNT(*) FROM w4dj_track_meta WHERE analysis_status='failed'",
            )?,
            analysis_completed: count(
                "SELECT COUNT(*) FROM w4dj_track_meta WHERE analysis_status='completed'",
            )?,
        })
    }

    /// Build a read-only manifest for the workspace emotion-evaluation tool.
    /// The manifest is deliberately based on W4DJ output ownership and never
    /// enumerates NetEase database rows.
    pub fn emotion_evaluation_manifest(
        &self,
        count: usize,
        seed: u64,
    ) -> W4djResult<EmotionEvaluationManifest> {
        let connection = self.catalog.connection();
        let mut statement = connection.prepare(
            "SELECT m.track_key, t.title, t.artists, t.album,
                    m.destination_path, m.output_root,
                    COALESCE(m.measured_duration_seconds, t.effective_duration_seconds),
                    ar.status, ar.analysis_json
             FROM w4dj_track_meta m
             JOIN tracks t ON t.track_key=m.track_key
             LEFT JOIN analysis_results ar ON ar.track_key=m.track_key
             ORDER BY m.destination_path",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ManifestRow {
                track_id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                destination_path: row.get(4)?,
                output_root: row.get(5)?,
                duration_seconds: row.get(6)?,
                analysis_status: row.get(7)?,
                analysis_json: row.get(8)?,
            })
        })?;
        let mut records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        shuffle_manifest_rows(&mut records, seed);
        if count > 0 {
            records.truncate(count.min(records.len()));
        }

        let tracks = records
            .into_iter()
            .map(|row| {
                let analysis = row
                    .analysis_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<Value>(json).ok());
                let duration = row
                    .duration_seconds
                    .filter(|value| value.is_finite() && *value > 0.0);
                let (clip_start, clip_duration, clip_selection) = evaluation_clip_window(
                    Path::new(&row.destination_path),
                    duration,
                    analysis.as_ref(),
                );
                EmotionEvaluationTrack {
                    track_id: row.track_id,
                    title: row.title,
                    artist: row.artist,
                    album: row.album,
                    relative_path: relative_manifest_path(
                        Path::new(&row.destination_path),
                        Path::new(&row.output_root),
                    ),
                    duration_seconds: duration,
                    clip_start_seconds: clip_start,
                    clip_duration_seconds: clip_duration,
                    clip_selection,
                    legacy_mood: evaluation_model_value(
                        analysis.as_ref(),
                        row.analysis_status.as_deref(),
                        EvaluationModel::LegacyMood,
                    ),
                    emomusic: evaluation_model_value(
                        analysis.as_ref(),
                        row.analysis_status.as_deref(),
                        EvaluationModel::Emomusic,
                    ),
                    muse: evaluation_model_value(
                        analysis.as_ref(),
                        row.analysis_status.as_deref(),
                        EvaluationModel::Muse,
                    ),
                    mirex: evaluation_model_value(
                        analysis.as_ref(),
                        row.analysis_status.as_deref(),
                        EvaluationModel::Mirex,
                    ),
                }
            })
            .collect::<Vec<_>>();

        Ok(EmotionEvaluationManifest {
            schema_version: 1,
            session_id: format!("emotion-eval-{}", now_ms()),
            seed,
            sample_size: tracks.len(),
            clip_policy: "peak-energy-10s-with-drop-preference".to_string(),
            tracks,
        })
    }

    pub fn remove_invalid(&mut self) -> W4djResult<u64> {
        let transaction = self.catalog.connection_mut().transaction()?;
        let removed = transaction.execute(
            "DELETE FROM tracks WHERE track_key IN (
                SELECT track_key FROM w4dj_track_meta WHERE status IN ('outOfScope','missing','unreadable')
            )",
            [],
        )?;
        transaction.commit()?;
        Ok(removed as u64)
    }

    pub fn scan_invalid<Cancel, Observe>(
        &mut self,
        mut cancelled: Cancel,
        mut observe: Observe,
    ) -> W4djResult<W4djLibraryStats>
    where
        Cancel: FnMut() -> bool,
        Observe: FnMut(usize, usize, &str),
    {
        let connection = self.catalog.connection();
        let mut statement = connection.prepare(
            "SELECT track_key, destination_path, status FROM w4dj_track_meta ORDER BY destination_path",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let total = records.len();
        let mut updates = Vec::with_capacity(total);
        for (index, (track_key, destination, status)) in records.into_iter().enumerate() {
            if cancelled() {
                return Err(W4djLibraryError::Invalid("失效歌曲扫描已取消".to_string()));
            }
            let next_status = if status == "outOfScope" {
                status
            } else {
                match fs::metadata(&destination) {
                    Ok(metadata) if metadata.is_file() && metadata.len() > 0 => {
                        "available".to_string()
                    }
                    Ok(_) => "unreadable".to_string(),
                    Err(_) => "missing".to_string(),
                }
            };
            let current = Path::new(&destination)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(destination);
            updates.push((track_key, next_status));
            observe(index + 1, total, &current);
        }
        if cancelled() {
            return Err(W4djLibraryError::Invalid("失效歌曲扫描已取消".to_string()));
        }
        let transaction = self.catalog.connection_mut().transaction()?;
        let now = now_ms();
        for (track_key, status) in updates {
            transaction.execute(
                "UPDATE w4dj_track_meta SET status=?1, updated_at_ms=?2 WHERE track_key=?3",
                params![status, now, track_key],
            )?;
            transaction.execute(
                "UPDATE tracks SET local_status=?1, updated_at_ms=?2 WHERE track_key=?3",
                params![status_to_track_status(&status), now, track_key],
            )?;
        }
        transaction.commit()?;
        self.stats()
    }
}

#[derive(Debug)]
struct ManifestRow {
    track_id: String,
    title: String,
    artist: String,
    album: String,
    destination_path: String,
    output_root: String,
    duration_seconds: Option<f64>,
    analysis_status: Option<String>,
    analysis_json: Option<String>,
}

#[derive(Debug)]
struct ScannedOutputFile {
    destination: String,
    title: String,
    size_bytes: i64,
    modified_at_ms: Option<i64>,
    measured_format: Option<String>,
    identity: Option<OutputIdentityManifestEntry>,
}

#[derive(Debug)]
struct ScannedOutputRoot {
    slot_index: usize,
    root: String,
    destinations: HashSet<String>,
    files: Vec<ScannedOutputFile>,
}

#[derive(Debug, Clone, Copy)]
enum EvaluationModel {
    LegacyMood,
    Emomusic,
    Muse,
    Mirex,
}

fn shuffle_manifest_rows(rows: &mut [ManifestRow], seed: u64) {
    let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    for index in (1..rows.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let target = (state as usize) % (index + 1);
        rows.swap(index, target);
    }
}

fn relative_manifest_path(destination: &Path, root: &Path) -> String {
    destination
        .strip_prefix(root)
        .unwrap_or_else(|_| {
            destination
                .file_name()
                .map(Path::new)
                .unwrap_or(destination)
        })
        .to_string_lossy()
        .replace('\\', "/")
}

fn output_identity_manifest_path(root: &Path) -> PathBuf {
    root.join(OUTPUT_IDENTITY_MANIFEST_FILE_NAME)
}

fn empty_output_identity_manifest() -> OutputIdentityManifest {
    OutputIdentityManifest {
        format: OUTPUT_IDENTITY_MANIFEST_FORMAT.to_string(),
        format_version: OUTPUT_IDENTITY_MANIFEST_VERSION,
        outputs: Vec::new(),
        playlists: Vec::new(),
    }
}

/// An output identity sidecar is optional recovery data. A missing or damaged
/// sidecar must never prevent a normal output scan; the current audio files
/// can still be indexed by their filesystem facts.
fn load_output_identity_manifest(root: &Path) -> OutputIdentityManifest {
    let path = output_identity_manifest_path(root);
    let Ok(contents) = fs::read(path) else {
        return empty_output_identity_manifest();
    };
    let Ok(manifest) = serde_json::from_slice::<OutputIdentityManifest>(&contents) else {
        return empty_output_identity_manifest();
    };
    if manifest.format != OUTPUT_IDENTITY_MANIFEST_FORMAT
        || manifest.format_version != OUTPUT_IDENTITY_MANIFEST_VERSION
    {
        return empty_output_identity_manifest();
    }
    manifest
}

fn safe_manifest_relative_path(value: &str) -> Option<String> {
    let value = value.trim().replace('\\', "/");
    let value = value.trim_start_matches("./");
    if value.is_empty() {
        return None;
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    Some(value.to_string())
}

fn merge_output_identity_manifest_entry(
    manifest: &mut OutputIdentityManifest,
    incoming: OutputIdentityManifestEntry,
) {
    if let Some(existing) = manifest
        .outputs
        .iter_mut()
        .find(|entry| entry.relative_path == incoming.relative_path)
    {
        let source_changed = match (
            existing.source_path.as_deref(),
            incoming.source_path.as_deref(),
        ) {
            (Some(existing), Some(incoming)) => !indexed_paths_equal(existing, incoming),
            _ => false,
        };
        let metadata_changed_without_source = existing.source_path.is_none()
            && incoming.source_path.is_some()
            && ((!existing.title.trim().is_empty()
                && !incoming.title.trim().is_empty()
                && !existing
                    .title
                    .trim()
                    .eq_ignore_ascii_case(incoming.title.trim()))
                || (!existing.artist_display.trim().is_empty()
                    && !incoming.artist_display.trim().is_empty()
                    && !existing
                        .artist_display
                        .trim()
                        .eq_ignore_ascii_case(incoming.artist_display.trim())));
        if source_changed || metadata_changed_without_source {
            *existing = incoming;
            return;
        }
        if incoming.source_path.is_some() {
            existing.source_path = incoming.source_path;
        }
        if !incoming.title.is_empty() {
            existing.title = incoming.title;
        }
        if !incoming.artist_display.is_empty() {
            existing.artist_display = incoming.artist_display;
        }
    } else {
        manifest.outputs.push(incoming);
    }
}

fn write_output_identity_manifest(
    root: &Path,
    mut manifest: OutputIdentityManifest,
) -> W4djResult<()> {
    manifest
        .outputs
        .sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));
    manifest
        .playlists
        .sort_unstable_by(|left, right| left.playlist_id.cmp(&right.playlist_id));
    for playlist in &mut manifest.playlists {
        playlist.tracks.sort_unstable_by_key(|track| track.position);
    }
    let contents = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| W4djLibraryError::Invalid(format!("序列化输出身份清单失败：{error}")))?;
    let manifest_path = output_identity_manifest_path(root);
    let temp_path = root.join(format!(
        "{OUTPUT_IDENTITY_MANIFEST_FILE_NAME}.tmp-{}",
        std::process::id()
    ));
    let result = (|| {
        fs::write(&temp_path, contents)?;
        fs::rename(&temp_path, manifest_path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn persist_output_identity_manifest_updates<I, J>(
    root: &Path,
    entries: I,
    playlists: J,
    replace_playlist_ids: &[String],
) -> W4djResult<()>
where
    I: IntoIterator<Item = OutputIdentityManifestEntry>,
    J: IntoIterator<Item = OutputPlaylistManifest>,
{
    let _lock = OUTPUT_IDENTITY_MANIFEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| W4djLibraryError::Invalid("输出身份清单锁已损坏".to_string()))?;
    fs::create_dir_all(root)?;
    let mut manifest = load_output_identity_manifest(root);
    for entry in entries {
        merge_output_identity_manifest_entry(&mut manifest, entry);
    }
    for playlist_id in replace_playlist_ids {
        manifest
            .playlists
            .retain(|playlist| playlist.playlist_id != *playlist_id);
    }
    for playlist in playlists {
        if !playlist.tracks.is_empty() {
            manifest.playlists.push(playlist);
        }
    }
    write_output_identity_manifest(root, manifest)
}

fn evaluation_clip_window(
    destination: &Path,
    duration: Option<f64>,
    analysis: Option<&Value>,
) -> (f64, f64, String) {
    let clip_duration = duration.map_or(10.0, |value| value.min(10.0));
    let max_start = duration.map_or(0.0, |value| (value - clip_duration).max(0.0));
    let drop_start = analysis
        .and_then(|value| value_field(value, &["dropAnalysis", "drop_analysis"]))
        .and_then(|value| value_field(value, &["segmentStartSeconds", "segment_start_seconds"]))
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite());
    if let Some(start) = drop_start {
        return (
            start.clamp(0.0, max_start),
            clip_duration,
            "drop".to_string(),
        );
    }
    if max_start <= 0.0 {
        return (0.0, clip_duration, "fullTrack".to_string());
    }
    if let Some(start) = peak_energy_clip_start(destination, clip_duration, max_start) {
        return (start, clip_duration, "peakEnergy".to_string());
    }
    (0.0, clip_duration, "startFallback".to_string())
}

/// Find a deterministic high-energy window for subjective listening. This is
/// deliberately best-effort: manifest export must still work when a bundled
/// ffmpeg sidecar is unavailable or an output is no longer decodable. Existing
/// Drop analysis remains the stronger cue and is selected before this scan.
fn peak_energy_clip_start(destination: &Path, clip_duration: f64, max_start: f64) -> Option<f64> {
    let ffmpeg = crate::sync::find_ffmpeg()?;
    let output = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-i",
            destination.to_str()?,
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "1",
            "-ar",
            "8000",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() < 4 {
        return None;
    }
    const SAMPLE_RATE: usize = 8_000;
    let window = (clip_duration * SAMPLE_RATE as f64).round() as usize;
    if window == 0 || output.stdout.len() / 4 < window {
        return None;
    }
    let samples = output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    let hop = SAMPLE_RATE;
    let max_sample_start = ((max_start * SAMPLE_RATE as f64).round() as usize)
        .min(samples.len().saturating_sub(window));
    let mut best_start = 0usize;
    let mut best_energy = f64::NEG_INFINITY;
    let mut start = 0usize;
    while start <= max_sample_start {
        let sum = samples[start..start + window]
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>();
        let energy = sum / window as f64;
        if energy.is_finite() && energy > best_energy {
            best_energy = energy;
            best_start = start;
        }
        if start == max_sample_start {
            break;
        }
        start = (start + hop).min(max_sample_start);
    }
    best_energy
        .is_finite()
        .then_some(best_start as f64 / SAMPLE_RATE as f64)
}

fn evaluation_model_value(
    analysis: Option<&Value>,
    analysis_status: Option<&str>,
    model: EvaluationModel,
) -> Value {
    let Some(root) = analysis else {
        let status = match model {
            EvaluationModel::LegacyMood => "missing",
            EvaluationModel::Emomusic | EvaluationModel::Muse | EvaluationModel::Mirex => {
                "model_missing"
            }
        };
        return json!({"status": status});
    };
    let high_level = value_field(root, &["highLevel", "high_level"]);
    let high_level_status = high_level
        .and_then(|value| value_field(value, &["status"]))
        .and_then(Value::as_str)
        .or(analysis_status)
        .unwrap_or("missing");
    match model {
        EvaluationModel::LegacyMood => {
            let Some(labels) = high_level.and_then(|value| value_field(value, &["mood"])) else {
                return json!({"status": "model_missing"});
            };
            let labels = labels.clone();
            let heads = high_level
                .and_then(|value| value_field(value, &["heads"]))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let status = match high_level_status {
                "completed" => "completed",
                "failed" => "failed",
                "cancelled" => "cancelled",
                "model_missing" => "model_missing",
                _ => "missing",
            };
            json!({"status": status, "labels": labels, "heads": heads})
        }
        EvaluationModel::Emomusic | EvaluationModel::Muse => {
            let key = match model {
                EvaluationModel::Emomusic => "emomusic",
                EvaluationModel::Muse => "muse",
                _ => unreachable!(),
            };
            let candidate = high_level
                .and_then(|value| value_field(value, &["emotionCandidates", "emotion_candidates"]))
                .and_then(|value| value_field(value, &[key]));
            candidate
                .cloned()
                .map(|value| ensure_model_status(value, high_level_status))
                .unwrap_or_else(|| json!({"status": "model_missing"}))
        }
        EvaluationModel::Mirex => {
            let candidate =
                high_level.and_then(|value| value_field(value, &["moodCluster", "mood_cluster"]));
            let status = high_level
                .and_then(|value| value_field(value, &["moodClusterStatus", "mood_cluster_status"]))
                .and_then(Value::as_str)
                .unwrap_or(if candidate.is_some() {
                    high_level_status
                } else {
                    "model_missing"
                });
            candidate
                .cloned()
                .map(|labels| json!({"status": status, "labels": labels}))
                .unwrap_or_else(|| json!({"status": status}))
        }
    }
}

fn ensure_model_status(value: Value, fallback: &str) -> Value {
    let Value::Object(mut object) = value else {
        return json!({"status": fallback});
    };
    object
        .entry("status".to_string())
        .or_insert_with(|| Value::String(fallback.to_string()));
    Value::Object(object)
}

fn value_field<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    let object = value.as_object()?;
    names.iter().find_map(|name| object.get(*name))
}

pub fn write_emotion_evaluation_manifest(
    path: &Path,
    manifest: &EmotionEvaluationManifest,
) -> W4djResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp_name = format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("emotion-evaluation-manifest.json")
    );
    let temp_path = parent.join(temp_name);
    let contents = serde_json::to_vec_pretty(manifest).map_err(|error| {
        W4djLibraryError::Invalid(format!("序列化情绪验收 manifest 失败：{error}"))
    })?;
    fs::write(&temp_path, contents)?;
    fs::rename(temp_path, path)?;
    Ok(())
}

fn status_to_track_status(status: &str) -> &str {
    match status {
        "outOfScope" => "out_of_scope",
        "unreadable" => "unreadable",
        "missing" => "missing",
        _ => "available",
    }
}

fn analysis_status(entry: &TrackAnalysis) -> AnalysisStatus {
    // Basic values remain queryable for partial results, but the Dashboard's
    // completed count represents the strict enhanced-analysis contract.
    if crate::analysis::is_complete_analysis(entry) {
        AnalysisStatus::Completed
    } else {
        AnalysisStatus::Failed
    }
}

fn analysis_genre(entry: &TrackAnalysis) -> String {
    entry
        .high_level
        .as_ref()
        .map(|value| {
            value
                .genre
                .iter()
                .map(|label| label.label.trim())
                .filter(|label| !label.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
}

fn labels_json(labels: Option<&Vec<crate::analysis::AnalysisLabel>>) -> String {
    labels
        .map(|values| serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_else(|| "[]".to_string())
}

fn connection_query_discogs_projection(
    connection: &rusqlite::Connection,
    track_key: &str,
) -> W4djResult<[String; 5]> {
    Ok(connection.query_row(
        "SELECT discogs_mood_theme_json, discogs_approachability_json,
                discogs_instrumentation_json, discogs_timbre_json,
                discogs_danceability_json
         FROM tracks WHERE track_key=?1",
        [track_key],
        |row| {
            Ok([
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ])
        },
    )?)
}

fn build_match_report(
    playlist_id: &str,
    matches: Vec<DjPlaylistTrackMatch>,
) -> DjPlaylistMatchReport {
    let matched_count = matches.iter().filter(|row| row.status == "matched").count();
    let ambiguous_count = matches
        .iter()
        .filter(|row| row.status == "ambiguous")
        .count();
    let unmatched_count = matches
        .iter()
        .filter(|row| row.status == "unmatched")
        .count();
    let missing_count = matches.iter().filter(|row| row.status == "missing").count();
    DjPlaylistMatchReport {
        playlist_id: playlist_id.to_string(),
        total: matches.len(),
        matched_count,
        ambiguous_count,
        unmatched_count,
        missing_count,
        matches,
    }
}

fn persisted_match_kind(status: &str, method: Option<&str>) -> DjPlaylistMatchKind {
    match (status, method) {
        ("matched", Some("manual")) => DjPlaylistMatchKind::Manual,
        ("matched", Some("recentBm25f")) => DjPlaylistMatchKind::RecentBm25f,
        ("matched", Some("libraryBm25f")) => DjPlaylistMatchKind::LibraryBm25f,
        // Legacy methods are interpreted as ordinary title/artist matches;
        // their old IDs are never loaded or used.
        ("matched", Some(_)) => DjPlaylistMatchKind::LibraryBm25f,
        ("ambiguous", _) => DjPlaylistMatchKind::Ambiguous,
        ("missing", _) => DjPlaylistMatchKind::Missing,
        _ => DjPlaylistMatchKind::Unmatched,
    }
}

fn validate_match_report(
    playlist: &ImportedDjPlaylist,
    report: &DjPlaylistMatchReport,
    candidates: &[DjOutputCandidate],
) -> W4djResult<()> {
    if report.total != playlist.tracks.len() || report.matches.len() != playlist.tracks.len() {
        return Err(W4djLibraryError::Invalid(
            "匹配报告必须覆盖歌单中的每个位置".to_string(),
        ));
    }
    let available = candidates
        .iter()
        .map(|candidate| candidate.track_key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let expected = playlist
        .tracks
        .iter()
        .map(|track| track.position)
        .collect::<std::collections::HashSet<_>>();
    let mut seen_positions = std::collections::HashSet::new();
    let mut seen_keys = std::collections::HashMap::<String, (String, Vec<String>)>::new();
    for row in &report.matches {
        if !expected.contains(&row.position) || !seen_positions.insert(row.position) {
            return Err(W4djLibraryError::Invalid(
                "匹配报告包含重复或未知的位置".to_string(),
            ));
        }
        if !matches!(
            row.status.as_str(),
            "matched" | "unmatched" | "ambiguous" | "missing"
        ) {
            return Err(W4djLibraryError::Invalid("匹配状态不受支持".to_string()));
        }
        if row.status == "matched" {
            let Some(track_key) = row.track_key.as_deref() else {
                return Err(W4djLibraryError::Invalid(
                    "matched 行必须包含 track_key".to_string(),
                ));
            };
            if !candidates
                .iter()
                .any(|candidate| candidate.track_key == track_key)
            {
                return Err(W4djLibraryError::Invalid(
                    "匹配引用了不可用的输出".to_string(),
                ));
            }
            let playlist_track = playlist
                .tracks
                .iter()
                .find(|track| track.position == row.position)
                .ok_or_else(|| W4djLibraryError::Invalid("匹配位置不存在".to_string()))?;
            let identity = crate::dj_playlist_match::identity_key_for(
                &playlist_track.title,
                &playlist_track.artist_display,
            );
            if !available.contains(track_key) {
                return Err(W4djLibraryError::Invalid(
                    "匹配只能引用当前可用输出；只有标题和歌手相同的重复 position 可以复用"
                        .to_string(),
                ));
            }
            if !row.excluded
                && seen_keys
                    .get(track_key)
                    .is_some_and(|previous_identity| previous_identity != &identity)
            {
                return Err(W4djLibraryError::Invalid(
                    "匹配只能引用当前可用输出；只有标题和歌手相同的重复 position 可以复用"
                        .to_string(),
                ));
            }
            if !row.excluded {
                seen_keys.entry(track_key.to_string()).or_insert(identity);
            }
        } else if row.track_key.is_some() {
            return Err(W4djLibraryError::Invalid(
                "未完成匹配状态不能携带 track_key".to_string(),
            ));
        }
    }
    if seen_positions.len() != expected.len() {
        return Err(W4djLibraryError::Invalid(
            "匹配报告缺少歌单位置".to_string(),
        ));
    }
    Ok(())
}

fn normalize_path(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Normalize a path for the post-commit index without touching the
/// filesystem.  Conversion candidates are absolute paths already; keeping
/// this operation lexical is what makes lightweight registration safe during
/// startup and avoids an implicit stat/canonicalize scan.
fn normalize_index_path(path: &Path) -> String {
    path.to_string_lossy().trim().to_string()
}

fn indexed_paths_equal(left: &str, right: &str) -> bool {
    normalize_index_path(Path::new(left)) == normalize_index_path(Path::new(right))
}

fn snapshot_path_key(root_path: &Path, normalized_root: &str, path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(root_path) {
        return Path::new(normalized_root)
            .join(relative)
            .to_string_lossy()
            .into_owned();
    }
    if let Ok(relative) = path.strip_prefix(Path::new(normalized_root)) {
        return Path::new(normalized_root)
            .join(relative)
            .to_string_lossy()
            .into_owned();
    }
    normalize_index_path(path)
}

fn restore_dedupe_key(stored: &str, position: u64) -> String {
    let suffix = format!(":position:{position}");
    stored.strip_suffix(&suffix).unwrap_or(stored).to_string()
}

fn modified_at_ms(metadata: &fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
}

fn invalidate_analysis_projection(
    transaction: &rusqlite::Transaction<'_>,
    track_key: &str,
    now: i64,
) -> rusqlite::Result<()> {
    transaction.execute(
        "DELETE FROM analysis_results WHERE track_key=?1",
        [track_key],
    )?;
    transaction.execute(
        "UPDATE local_files SET measured_bitrate_bps=NULL, measured_duration_seconds=NULL,
            sample_rate_hz=NULL, channels=NULL WHERE track_key=?1",
        [track_key],
    )?;
    transaction.execute(
        "UPDATE w4dj_track_meta SET analysis_status='notAnalyzed', analysis_error=NULL,
            measured_duration_seconds=NULL, updated_at_ms=?1 WHERE track_key=?2",
        params![now, track_key],
    )?;
    transaction.execute(
        "UPDATE tracks SET essentia_genre='', essentia_duration_seconds=NULL,
            preferred_local_file_id=NULL, measured_duration_seconds=NULL,
            effective_duration_seconds=NULL, duration_source=NULL, measured_format=NULL,
            effective_format=NULL, measured_bitrate_bps=NULL, effective_bitrate_bps=NULL,
            measured_size_bytes=NULL, effective_size_bytes=NULL, bpm=NULL,
            musical_key=NULL, scale=NULL, integrated_loudness_lufs=NULL,
            loudness_range_lu=NULL, energy=NULL, danceability=NULL, mood_json='[]',
            instrument_json='[]', style_json='[]', discogs_mood_theme_json='[]',
            discogs_approachability_json='{}', discogs_instrumentation_json='[]',
            discogs_timbre_json='{}', discogs_danceability_json='{}',
            drop_loudness_lufs=NULL, updated_at_ms=?1 WHERE track_key=?2",
        params![now, track_key],
    )?;
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn is_audio_path(path: &Path) -> bool {
    if is_internal_temp_path(path) {
        return false;
    }
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "mp3" | "flac" | "wav" | "aif" | "aiff" | "m4a"
            )
        })
}

fn is_internal_temp_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.starts_with(".w4dj-") || name.starts_with("._"))
}

#[cfg(test)]
mod tests {
    use super::{AnalysisStatus, W4djLibrary, is_audio_path, normalize_path};
    use crate::analysis::{HighLevelAnalysis, TrackAnalysis};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn output_database_has_independent_schema_and_only_committed_outputs() {
        let directory = tempdir().unwrap();
        let output_root = directory.path().join("out");
        fs::create_dir_all(&output_root).unwrap();
        let output = output_root.join("song.mp3");
        fs::write(&output, b"audio").unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        let key = library
            .upsert_output_file(0, &output_root, None, &output)
            .unwrap();
        assert!(key.starts_with("output:"));
        assert_eq!(library.stats().unwrap().total, 1);
        assert_eq!(library.stats().unwrap().available, 1);
        let tables = library
            .catalog
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='analysis_results'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(tables, 1);
    }

    #[test]
    fn temporary_and_appledouble_audio_files_are_not_importable_outputs() {
        assert!(!is_audio_path(std::path::Path::new(".w4dj-analysis.wav")));
        assert!(!is_audio_path(std::path::Path::new("._song.mp3")));
        assert!(!is_audio_path(std::path::Path::new("source.ncm")));
        assert!(is_audio_path(std::path::Path::new("song.mp3")));
        assert!(is_audio_path(std::path::Path::new("song.m4a")));
    }

    #[test]
    fn removes_stale_internal_rows_without_deleting_files() {
        let directory = tempdir().unwrap();
        let output_root = directory.path().join("out");
        fs::create_dir_all(&output_root).unwrap();
        let regular = output_root.join("song.mp3");
        let temporary = output_root.join(".w4dj-analysis.wav");
        fs::write(&regular, b"audio").unwrap();
        fs::write(&temporary, b"audio").unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        library
            .upsert_output_file(0, &output_root, None, &regular)
            .unwrap();
        library
            .upsert_output_file(0, &output_root, None, &temporary)
            .unwrap();
        assert_eq!(library.stats().unwrap().total, 2);

        assert_eq!(library.remove_internal_temp_records().unwrap(), 1);
        assert_eq!(library.stats().unwrap().total, 1);
        assert!(temporary.is_file());
        assert!(
            library
                .local_files_for_track(&format!("output:{}", normalize_path(&temporary)))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn invalid_scan_marks_missing_without_deleting_rows() {
        let directory = tempdir().unwrap();
        let output_root = directory.path().join("out");
        fs::create_dir_all(&output_root).unwrap();
        let output = output_root.join("song.mp3");
        fs::write(&output, b"audio").unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        library
            .upsert_output_file(0, &output_root, None, &output)
            .unwrap();
        fs::remove_file(&output).unwrap();
        let stats = library.scan_invalid(|| false, |_, _, _| {}).unwrap();
        assert_eq!(stats.invalid, 1);
        assert_eq!(library.stats().unwrap().total, 1);
        assert_eq!(library.remove_invalid().unwrap(), 1);
        assert_eq!(library.stats().unwrap().total, 0);
    }

    #[test]
    fn analysis_status_values_are_stable() {
        assert_eq!(AnalysisStatus::NotAnalyzed.as_str(), "notAnalyzed");
        assert_eq!(AnalysisStatus::Completed.as_str(), "completed");
        assert_eq!(AnalysisStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn reconcile_output_roots_keeps_present_rows_and_removes_missing_rows() {
        let directory = tempdir().unwrap();
        let output_root = directory.path().join("out");
        fs::create_dir_all(&output_root).unwrap();
        let kept = output_root.join("kept.mp3");
        let removed = output_root.join("removed.mp3");
        let added = output_root.join("added.mp3");
        for path in [&kept, &removed, &added] {
            fs::write(path, b"audio").unwrap();
        }
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        library
            .upsert_output_file(0, &output_root, None, &kept)
            .unwrap();
        library
            .upsert_output_file(0, &output_root, None, &removed)
            .unwrap();
        library
            .reconcile_output_roots(&[(0, output_root.clone(), vec![kept.clone(), added.clone()])])
            .unwrap();
        assert_eq!(library.stats().unwrap().total, 2);
        assert!(
            library
                .track_detail(&format!("output:{}", normalize_path(&kept)))
                .unwrap()
                .is_some()
        );
        assert!(
            library
                .track_detail(&format!("output:{}", normalize_path(&removed)))
                .unwrap()
                .is_none()
        );
        assert!(
            library
                .track_detail(&format!("output:{}", normalize_path(&added)))
                .unwrap()
                .is_some()
        );
        library.clear_output_library().unwrap();
        assert_eq!(library.stats().unwrap().total, 0);
        assert!(kept.is_file() && added.is_file());
    }

    #[test]
    fn reconcile_output_roots_reuses_the_existing_destination_identity() {
        let directory = tempdir().unwrap();
        let output_root = directory.path().join("out");
        fs::create_dir_all(&output_root).unwrap();
        let source = directory.path().join("source.ncm");
        let output = output_root.join("song.mp3");
        fs::write(&output, b"audio").unwrap();
        let output = fs::canonicalize(output).unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        let original_key = library
            .upsert_lightweight_output(0, Some(&source), &output, "Song", "Artist")
            .unwrap();
        assert!(original_key.starts_with("source:"));

        library
            .reconcile_output_roots(&[(0, output_root, vec![output.clone()])])
            .unwrap();

        assert_eq!(library.stats().unwrap().total, 1);
        assert!(library.track_detail(&original_key).unwrap().is_some());
        assert!(
            library
                .track_detail(&format!("output:{}", normalize_path(&output)))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reconcile_output_roots_invalidates_analysis_only_when_fingerprint_changes() {
        let directory = tempdir().unwrap();
        let output_root = directory.path().join("out");
        fs::create_dir_all(&output_root).unwrap();
        let output = output_root.join("song.mp3");
        fs::write(&output, b"audio").unwrap();
        let output = fs::canonicalize(output).unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        library
            .upsert_output_file(0, &output_root, None, &output)
            .unwrap();
        library
            .mark_analysis_failed_for_destination(&output, "fixture failure")
            .unwrap();

        library
            .reconcile_output_roots(&[(0, output_root.clone(), vec![output.clone()])])
            .unwrap();
        assert_eq!(library.stats().unwrap().analysis_failed, 1);

        fs::write(&output, b"replacement audio").unwrap();
        let summary = library
            .reconcile_output_roots(&[(0, output_root, vec![output])])
            .unwrap();
        assert_eq!(summary.invalidated_paths.len(), 1);
        let stats = library.stats().unwrap();
        assert_eq!(stats.analysis_failed, 0);
        assert_eq!(stats.not_analyzed, 1);
    }

    #[test]
    fn reconcile_output_roots_rolls_back_all_roots_when_one_snapshot_is_invalid() {
        let directory = tempdir().unwrap();
        let first_root = directory.path().join("first");
        let second_root = directory.path().join("second");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&second_root).unwrap();
        let first_old = first_root.join("old.mp3");
        let first_new = first_root.join("new.mp3");
        let second = second_root.join("invalid.mp3");
        fs::write(&first_old, b"audio").unwrap();
        fs::write(&first_new, b"audio").unwrap();
        fs::write(&second, b"").unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        library
            .upsert_output_file(0, &first_root, None, &first_old)
            .unwrap();

        assert!(
            library
                .reconcile_output_roots(&[
                    (0, first_root, vec![first_new.clone()]),
                    (1, second_root, vec![second]),
                ])
                .is_err()
        );
        assert_eq!(library.stats().unwrap().total, 1);
        assert!(
            library
                .track_detail(&format!("output:{}", normalize_path(&first_old)))
                .unwrap()
                .is_some()
        );
        assert!(
            library
                .track_detail(&format!("output:{}", normalize_path(&first_new)))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reconcile_output_roots_leaves_unparticipating_roots_and_analysis_unchanged() {
        let directory = tempdir().unwrap();
        let participating_root = directory.path().join("participating");
        let untouched_root = directory.path().join("untouched");
        fs::create_dir_all(&participating_root).unwrap();
        fs::create_dir_all(&untouched_root).unwrap();
        let removed = participating_root.join("removed.mp3");
        let untouched = untouched_root.join("untouched.mp3");
        fs::write(&removed, b"audio").unwrap();
        fs::write(&untouched, b"audio").unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        library
            .upsert_output_file(0, &participating_root, None, &removed)
            .unwrap();
        library
            .upsert_output_file(1, &untouched_root, None, &untouched)
            .unwrap();
        library
            .mark_analysis_failed_for_destination(&untouched, "fixture failure")
            .unwrap();

        let summary = library
            .reconcile_output_roots(&[(0, participating_root, Vec::new())])
            .unwrap();
        assert_eq!(
            summary.removed_paths,
            vec![PathBuf::from(normalize_path(&removed))]
        );
        let stats = library.stats().unwrap();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.analysis_failed, 1);
        assert!(
            library
                .track_detail(&format!("output:{}", normalize_path(&untouched)))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn high_level_failure_keeps_basic_values_but_is_not_counted_as_completed() {
        let directory = tempdir().unwrap();
        let output_root = directory.path().join("out");
        fs::create_dir_all(&output_root).unwrap();
        let output = output_root.join("song.mp3");
        fs::write(&output, b"audio").unwrap();
        let mut library = W4djLibrary::open(&directory.path().join("w4dj.sqlite3")).unwrap();
        let key = library
            .upsert_output_file(0, &output_root, None, &output)
            .unwrap();
        let analysis = TrackAnalysis {
            path: output.display().to_string(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: String::new(),
            genre: "网易云 Genre".into(),
            duration_seconds: Some(1.0),
            bpm: Some(120.0),
            key: Some("C".into()),
            scale: Some("major".into()),
            key_strength: None,
            integrated_loudness_lufs: None,
            loudness_range_lu: None,
            energy: Some(0.5),
            danceability: Some(1.1),
            beat_positions: Vec::new(),
            analyzed_at: String::new(),
            analyzer: "Essentia.js".into(),
            analysis_version: "0.2.0".into(),
            source_size_bytes: None,
            source_modified_at: None,
            source_filename_format: None,
            drop_loudness_lufs: None,
            drop_analysis: None,
            high_level: Some(HighLevelAnalysis {
                status: "failed".into(),
                model_version: None,
                reason: Some("模型不可用".into()),
                genre: Vec::new(),
                style: Vec::new(),
                mood: Vec::new(),
                instrument: Vec::new(),
                emotion_candidates: None,
                mood_cluster: Vec::new(),
                mood_cluster_status: None,
                mood_cluster_reason: None,
                filtered: Vec::new(),
                discogs_effnet: None,
            }),
        };

        assert!(
            library
                .apply_analysis_for_destination(&output, &analysis)
                .unwrap()
        );
        assert_eq!(library.stats().unwrap().analysis_completed, 0);
        assert_eq!(library.stats().unwrap().analysis_failed, 1);
        let detail = library.track_detail(&key).unwrap().unwrap();
        assert_eq!(detail.danceability, Some(1.1));
        assert!(detail.essentia_genre.is_empty());

        assert!(
            library
                .mark_analysis_failed_for_destination(&output, "temporary write failure")
                .unwrap()
        );
        assert_eq!(library.stats().unwrap().analysis_completed, 0);
        assert_eq!(library.stats().unwrap().analysis_failed, 1);
        assert_eq!(
            library.track_detail(&key).unwrap().unwrap().danceability,
            Some(1.1)
        );

        assert!(
            library
                .invalidate_analysis_for_destination(&output)
                .unwrap()
        );
        let stats = library.stats().unwrap();
        assert_eq!(stats.analysis_completed, 0);
        assert_eq!(stats.not_analyzed, 1);
        assert_eq!(
            library.track_detail(&key).unwrap().unwrap().danceability,
            None
        );
    }
}
