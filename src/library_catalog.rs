//! W4DJ's private, queryable song-library index.
//!
//! The NetEase database is deliberately not opened by this module.  Importers
//! build a [`CatalogSnapshot`] from read-only source data and commit that
//! snapshot to this SQLite database in one transaction.

use crate::analysis::TrackAnalysis;
use crate::library_query::{LibraryPage, LibraryQuery, compile_query};
use rusqlite::{Connection, OptionalExtension, Row, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

pub const CATALOG_SCHEMA_VERSION: i64 = 1;

#[derive(Debug)]
pub enum LibraryError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Invalid(String),
}

impl Display for LibraryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for LibraryError {}

impl From<std::io::Error> for LibraryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for LibraryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

pub type LibraryResult<T> = Result<T, LibraryError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationSource {
    Essentia,
    Measured,
    Netease,
}

impl DurationSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Essentia => "essentia",
            Self::Measured => "measured",
            Self::Netease => "netease",
        }
    }

    fn parse(value: Option<String>) -> Option<Self> {
        match value.as_deref() {
            Some("essentia") => Some(Self::Essentia),
            Some("measured") => Some(Self::Measured),
            Some("netease") => Some(Self::Netease),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalStatus {
    Available,
    Missing,
    Unreadable,
    DatabaseOnly,
}

impl LocalStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::DatabaseOnly => "database_only",
        }
    }

    fn parse(value: String) -> Self {
        match value.as_str() {
            "available" => Self::Available,
            "unreadable" => Self::Unreadable,
            "database_only" => Self::DatabaseOnly,
            _ => Self::Missing,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogTrack {
    pub track_key: String,
    pub netease_track_id: Option<String>,
    pub title: String,
    pub artists: String,
    pub artist_list_json: String,
    pub album: String,
    pub aliases_json: String,
    pub copyright_text: String,
    pub publish_date: String,
    pub netease_genre: String,
    pub essentia_genre: String,
    pub lyric_plain_text: String,
    pub lyric_translated_text: String,
    pub lyric_romanized_text: String,
    pub lyric_lrc_text: String,
    pub lyric_language: String,
    pub lyric_sync_type: String,
    pub lyric_source: String,
    pub cover_path: Option<String>,
    pub cover_available: bool,
    pub local_status: LocalStatus,
    pub preferred_local_file_id: Option<i64>,
    pub db_duration_seconds: Option<f64>,
    pub measured_duration_seconds: Option<f64>,
    pub essentia_duration_seconds: Option<f64>,
    pub effective_duration_seconds: Option<f64>,
    pub duration_source: Option<DurationSource>,
    pub db_format: Option<String>,
    pub measured_format: Option<String>,
    pub effective_format: Option<String>,
    pub db_bitrate_bps: Option<i64>,
    pub measured_bitrate_bps: Option<i64>,
    pub effective_bitrate_bps: Option<i64>,
    pub db_size_bytes: Option<i64>,
    pub measured_size_bytes: Option<i64>,
    pub effective_size_bytes: Option<i64>,
    pub bpm: Option<f64>,
    pub musical_key: Option<String>,
    pub scale: Option<String>,
    pub integrated_loudness_lufs: Option<f64>,
    pub loudness_range_lu: Option<f64>,
    pub energy: Option<f64>,
    pub danceability: Option<f64>,
    pub mood_json: String,
    pub instrument_json: String,
    pub drop_loudness_lufs: Option<f64>,
    pub updated_at_ms: i64,
}

impl Default for CatalogTrack {
    fn default() -> Self {
        Self {
            track_key: String::new(),
            netease_track_id: None,
            title: String::new(),
            artists: String::new(),
            artist_list_json: "[]".to_string(),
            album: String::new(),
            aliases_json: "[]".to_string(),
            copyright_text: String::new(),
            publish_date: String::new(),
            netease_genre: String::new(),
            essentia_genre: String::new(),
            lyric_plain_text: String::new(),
            lyric_translated_text: String::new(),
            lyric_romanized_text: String::new(),
            lyric_lrc_text: String::new(),
            lyric_language: String::new(),
            lyric_sync_type: "none".to_string(),
            lyric_source: String::new(),
            cover_path: None,
            cover_available: false,
            local_status: LocalStatus::DatabaseOnly,
            preferred_local_file_id: None,
            db_duration_seconds: None,
            measured_duration_seconds: None,
            essentia_duration_seconds: None,
            effective_duration_seconds: None,
            duration_source: None,
            db_format: None,
            measured_format: None,
            effective_format: None,
            db_bitrate_bps: None,
            measured_bitrate_bps: None,
            effective_bitrate_bps: None,
            db_size_bytes: None,
            measured_size_bytes: None,
            effective_size_bytes: None,
            bpm: None,
            musical_key: None,
            scale: None,
            integrated_loudness_lufs: None,
            loudness_range_lu: None,
            energy: None,
            danceability: None,
            mood_json: "[]".to_string(),
            instrument_json: "[]".to_string(),
            drop_loudness_lufs: None,
            updated_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogLocalFile {
    pub id: Option<i64>,
    pub track_key: String,
    pub path: PathBuf,
    pub size_bytes: i64,
    pub modified_at_ms: Option<i64>,
    pub measured_format: Option<String>,
    pub measured_bitrate_bps: Option<i64>,
    pub measured_duration_seconds: Option<f64>,
    pub sample_rate_hz: Option<i64>,
    pub channels: Option<i64>,
    pub readable: bool,
    pub probe_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSourceRecord {
    pub track_key: String,
    pub source_table: String,
    pub source_primary_key: String,
    pub source_version: Option<String>,
    pub raw_json: String,
    pub imported_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSource {
    pub database_path: PathBuf,
    pub database_size_bytes: i64,
    pub database_modified_at_ms: Option<i64>,
    pub last_imported_at_ms: i64,
    pub import_status: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CatalogSnapshot {
    pub tracks: Vec<CatalogTrack>,
    pub local_files: Vec<CatalogLocalFile>,
    pub source_records: Vec<CatalogSourceRecord>,
    pub sources: Vec<CatalogSource>,
}

pub struct LibraryCatalog {
    path: PathBuf,
    connection: Connection,
}

impl LibraryCatalog {
    pub fn open(path: &Path) -> LibraryResult<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Self {
            path: path.to_path_buf(),
            connection,
        })
    }

    /// Open the private W4DJ catalog and rebuild it if the catalog itself is
    /// damaged. This method never touches an imported source database; the
    /// recovery boundary is the path supplied by W4DJ only.
    pub fn open_or_recover(path: &Path) -> LibraryResult<(Self, Option<PathBuf>)> {
        let mut catalog = Self::open(path)?;
        match catalog.migrate() {
            Ok(()) => Ok((catalog, None)),
            Err(error) if path.is_file() => {
                drop(catalog);
                let stamp = now_ms();
                let backup = PathBuf::from(format!("{}.corrupt-{stamp}", path.display()));
                fs::rename(path, &backup)?;
                for suffix in ["-wal", "-shm"] {
                    let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
                    if sidecar.is_file() {
                        let sidecar_backup = PathBuf::from(format!("{}{suffix}", backup.display()));
                        fs::rename(sidecar, sidecar_backup)?;
                    }
                }
                let mut rebuilt = Self::open(path)?;
                rebuilt.migrate().map_err(|rebuild_error| {
                    LibraryError::Invalid(format!(
                        "索引库损坏，备份到 {} 后重建失败：{rebuild_error}; 原错误：{error}",
                        backup.display()
                    ))
                })?;
                Ok((rebuilt, Some(backup)))
            }
            Err(error) => Err(error),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn migrate(&mut self) -> LibraryResult<()> {
        self.connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS catalog_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS tracks (
                track_key TEXT PRIMARY KEY,
                netease_track_id TEXT,
                title TEXT NOT NULL DEFAULT '',
                artists TEXT NOT NULL DEFAULT '',
                artist_list_json TEXT NOT NULL DEFAULT '[]',
                album TEXT NOT NULL DEFAULT '',
                aliases_json TEXT NOT NULL DEFAULT '[]',
                copyright_text TEXT NOT NULL DEFAULT '',
                publish_date TEXT NOT NULL DEFAULT '',
                netease_genre TEXT NOT NULL DEFAULT '',
                essentia_genre TEXT NOT NULL DEFAULT '',
                lyric_plain_text TEXT NOT NULL DEFAULT '',
                lyric_translated_text TEXT NOT NULL DEFAULT '',
                lyric_romanized_text TEXT NOT NULL DEFAULT '',
                lyric_lrc_text TEXT NOT NULL DEFAULT '',
                lyric_language TEXT NOT NULL DEFAULT '',
                lyric_sync_type TEXT NOT NULL DEFAULT 'none',
                lyric_source TEXT NOT NULL DEFAULT '',
                cover_path TEXT,
                cover_available INTEGER NOT NULL DEFAULT 0,
                local_status TEXT NOT NULL DEFAULT 'database_only',
                preferred_local_file_id INTEGER,
                db_duration_seconds REAL,
                measured_duration_seconds REAL,
                essentia_duration_seconds REAL,
                effective_duration_seconds REAL,
                duration_source TEXT,
                db_format TEXT,
                measured_format TEXT,
                effective_format TEXT,
                db_bitrate_bps INTEGER,
                measured_bitrate_bps INTEGER,
                effective_bitrate_bps INTEGER,
                db_size_bytes INTEGER,
                measured_size_bytes INTEGER,
                effective_size_bytes INTEGER,
                bpm REAL,
                musical_key TEXT,
                scale TEXT,
                integrated_loudness_lufs REAL,
                loudness_range_lu REAL,
                energy REAL,
                danceability REAL,
                mood_json TEXT NOT NULL DEFAULT '[]',
                instrument_json TEXT NOT NULL DEFAULT '[]',
                drop_loudness_lufs REAL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS tracks_netease_id
                ON tracks (netease_track_id)
                WHERE netease_track_id IS NOT NULL AND netease_track_id <> '';
            CREATE INDEX IF NOT EXISTS tracks_title ON tracks(title COLLATE NOCASE);
            CREATE INDEX IF NOT EXISTS tracks_artists ON tracks(artists COLLATE NOCASE);
            CREATE INDEX IF NOT EXISTS tracks_album ON tracks(album COLLATE NOCASE);
            CREATE INDEX IF NOT EXISTS tracks_local_status ON tracks(local_status);
            CREATE INDEX IF NOT EXISTS tracks_bpm ON tracks(bpm);
            CREATE TABLE IF NOT EXISTS local_files (
                id INTEGER PRIMARY KEY,
                track_key TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE,
                size_bytes INTEGER NOT NULL,
                modified_at_ms INTEGER,
                measured_format TEXT,
                measured_bitrate_bps INTEGER,
                measured_duration_seconds REAL,
                sample_rate_hz INTEGER,
                channels INTEGER,
                readable INTEGER NOT NULL,
                probe_error TEXT,
                FOREIGN KEY(track_key) REFERENCES tracks(track_key) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS local_files_track_key ON local_files(track_key);
            CREATE TABLE IF NOT EXISTS source_records (
                id INTEGER PRIMARY KEY,
                track_key TEXT NOT NULL,
                source_table TEXT NOT NULL,
                source_primary_key TEXT NOT NULL,
                source_version TEXT,
                raw_json TEXT NOT NULL,
                imported_at_ms INTEGER NOT NULL,
                UNIQUE(source_table, source_primary_key),
                FOREIGN KEY(track_key) REFERENCES tracks(track_key) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS source_records_track_key ON source_records(track_key);
            CREATE TABLE IF NOT EXISTS catalog_sources (
                database_path TEXT PRIMARY KEY,
                database_size_bytes INTEGER NOT NULL,
                database_modified_at_ms INTEGER,
                last_imported_at_ms INTEGER NOT NULL,
                import_status TEXT NOT NULL,
                last_error TEXT
            );
            "#,
        )?;
        self.connection.execute(
            "INSERT INTO catalog_meta(key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [CATALOG_SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    pub fn upsert_track(&mut self, track: &CatalogTrack) -> LibraryResult<()> {
        self.connection
            .execute(&track_upsert_sql(), params_from_iter(track_values(track)))?;
        Ok(())
    }

    pub fn upsert_snapshot(&mut self, snapshot: &CatalogSnapshot) -> LibraryResult<()> {
        let transaction = self.connection.transaction()?;
        let track_sql = track_upsert_sql_preserving_analysis();
        let local_sql = local_file_upsert_sql();
        let source_sql = source_record_upsert_sql();
        let catalog_source_sql = catalog_source_upsert_sql();

        for track in &snapshot.tracks {
            transaction.execute(&track_sql, params_from_iter(track_values(track)))?;
        }
        // A catalog refresh is a complete snapshot of the discovered music
        // folder. Mark old paths as missing before applying the current
        // snapshot so deleted or moved files cannot remain falsely available
        // for cover lookup or conversion previews.
        transaction.execute(
            "UPDATE local_files SET readable = 0, probe_error = '文件未在最近扫描中发现'",
            [],
        )?;
        for local_file in &snapshot.local_files {
            transaction.execute(&local_sql, params_from_iter(local_file_values(local_file)))?;
        }
        for record in &snapshot.source_records {
            transaction.execute(&source_sql, params_from_iter(source_record_values(record)))?;
        }
        for source in &snapshot.sources {
            transaction.execute(
                &catalog_source_sql,
                params_from_iter(catalog_source_values(source)),
            )?;
        }
        transaction.execute(
            "UPDATE tracks SET local_status = CASE
                WHEN EXISTS (SELECT 1 FROM local_files lf WHERE lf.track_key = tracks.track_key AND lf.readable = 1)
                    THEN 'available'
                WHEN EXISTS (SELECT 1 FROM local_files lf WHERE lf.track_key = tracks.track_key AND lf.probe_error = '文件未在最近扫描中发现')
                    THEN 'missing'
                WHEN EXISTS (SELECT 1 FROM local_files lf WHERE lf.track_key = tracks.track_key)
                    THEN 'unreadable'
                ELSE 'database_only'
             END",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn track_detail(&self, track_key: &str) -> LibraryResult<Option<CatalogTrack>> {
        Ok(self
            .connection
            .query_row(
                &format!("SELECT {} FROM tracks WHERE track_key = ?1", TRACK_COLUMNS),
                [track_key],
                track_from_row,
            )
            .optional()?)
    }

    pub fn source_records_for_track(
        &self,
        track_key: &str,
    ) -> LibraryResult<Vec<CatalogSourceRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT track_key, source_table, source_primary_key, source_version,
                    raw_json, imported_at_ms
             FROM source_records
             WHERE track_key = ?1
             ORDER BY source_table ASC, source_primary_key ASC",
        )?;
        let rows = statement.query_map([track_key], source_record_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn count_tracks(&self) -> LibraryResult<i64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?)
    }

    pub fn local_file_by_path(&self, path: &Path) -> LibraryResult<Option<CatalogLocalFile>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, track_key, path, size_bytes, modified_at_ms, measured_format,
                        measured_bitrate_bps, measured_duration_seconds, sample_rate_hz,
                        channels, readable, probe_error
                 FROM local_files WHERE path = ?1",
                [path.to_string_lossy().as_ref()],
                local_file_from_row,
            )
            .optional()?)
    }

    pub fn local_files_for_track(&self, track_key: &str) -> LibraryResult<Vec<CatalogLocalFile>> {
        let mut statement = self.connection.prepare(
            "SELECT id, track_key, path, size_bytes, modified_at_ms, measured_format,
                    measured_bitrate_bps, measured_duration_seconds, sample_rate_hz,
                    channels, readable, probe_error
             FROM local_files WHERE track_key = ?1 ORDER BY readable DESC, path ASC",
        )?;
        let rows = statement.query_map([track_key], local_file_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn source_matches(
        &self,
        database_path: &Path,
        database_size_bytes: i64,
        database_modified_at_ms: Option<i64>,
    ) -> LibraryResult<bool> {
        let path = database_path.to_string_lossy();
        Ok(self
            .connection
            .query_row(
                "SELECT database_size_bytes, database_modified_at_ms, import_status
                 FROM catalog_sources WHERE database_path = ?1",
                [path.as_ref()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
            .is_some_and(|(size, modified, status)| {
                size == database_size_bytes
                    && modified == database_modified_at_ms
                    && status == "success"
            }))
    }

    pub fn apply_analysis_entries(&mut self, entries: &[TrackAnalysis]) -> LibraryResult<usize> {
        let transaction = self.connection.transaction()?;
        let mut updated = 0;
        for entry in entries {
            let local_path = Path::new(&entry.path).to_string_lossy();
            let Some(track_key) = transaction
                .query_row(
                    "SELECT track_key FROM local_files WHERE path = ?1",
                    [local_path.as_ref()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            else {
                continue;
            };
            let (duration, source) = effective_duration(
                entry.duration_seconds,
                transaction
                    .query_row(
                        "SELECT measured_duration_seconds FROM tracks WHERE track_key = ?1",
                        [&track_key],
                        |row| row.get::<_, Option<f64>>(0),
                    )
                    .optional()?
                    .flatten(),
                transaction
                    .query_row(
                        "SELECT db_duration_seconds FROM tracks WHERE track_key = ?1",
                        [&track_key],
                        |row| row.get::<_, Option<f64>>(0),
                    )
                    .optional()?
                    .flatten(),
            );
            let genre = entry.genre.trim();
            let high_level = entry.high_level.as_ref();
            let mood = high_level
                .map(|value| {
                    serde_json::to_string(&value.mood).unwrap_or_else(|_| "[]".to_string())
                })
                .unwrap_or_else(|| "[]".to_string());
            let instrument = high_level
                .map(|value| {
                    serde_json::to_string(&value.instrument).unwrap_or_else(|_| "[]".to_string())
                })
                .unwrap_or_else(|| "[]".to_string());
            updated += transaction.execute(
                "UPDATE tracks SET
                    essentia_genre=?1, essentia_duration_seconds=?2,
                    effective_duration_seconds=?3, duration_source=?4,
                    bpm=?5, musical_key=?6, scale=?7,
                    integrated_loudness_lufs=?8, loudness_range_lu=?9,
                    energy=?10, danceability=?11, mood_json=?12,
                    instrument_json=?13, drop_loudness_lufs=?14,
                    updated_at_ms=?15
                 WHERE track_key=?16",
                rusqlite::params![
                    genre,
                    entry.duration_seconds,
                    duration,
                    source.map(DurationSource::as_str),
                    entry.bpm,
                    entry.key,
                    entry.scale,
                    entry.integrated_loudness_lufs,
                    entry.loudness_range_lu,
                    entry.energy,
                    entry.danceability,
                    mood,
                    instrument,
                    entry.drop_loudness_lufs,
                    now_ms(),
                    track_key,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(updated)
    }

    pub fn query(&self, query: &LibraryQuery) -> LibraryResult<LibraryPage> {
        let compiled = compile_query(query)?;
        let count_sql = format!("SELECT COUNT(*) FROM tracks t{}", compiled.where_sql);
        let total: u64 = self
            .connection
            .query_row(
                &count_sql,
                params_from_iter(compiled.values.clone()),
                |row| row.get::<_, i64>(0),
            )?
            .max(0) as u64;
        let sql = format!(
            "SELECT {} FROM tracks t{}{} LIMIT ? OFFSET ?",
            TRACK_COLUMNS
                .replace("track_key", "t.track_key")
                .replace("netease_track_id", "t.netease_track_id")
                .replace("title", "t.title")
                .replace("artists", "t.artists")
                .replace("artist_list_json", "t.artist_list_json")
                .replace("album", "t.album")
                .replace("aliases_json", "t.aliases_json")
                .replace("copyright_text", "t.copyright_text")
                .replace("publish_date", "t.publish_date")
                .replace("netease_genre", "t.netease_genre")
                .replace("essentia_genre", "t.essentia_genre")
                .replace("lyric_plain_text", "t.lyric_plain_text")
                .replace("lyric_translated_text", "t.lyric_translated_text")
                .replace("lyric_romanized_text", "t.lyric_romanized_text")
                .replace("lyric_lrc_text", "t.lyric_lrc_text")
                .replace("lyric_language", "t.lyric_language")
                .replace("lyric_sync_type", "t.lyric_sync_type")
                .replace("lyric_source", "t.lyric_source")
                .replace("cover_path", "t.cover_path")
                .replace("cover_available", "t.cover_available")
                .replace("local_status", "t.local_status")
                .replace("preferred_local_file_id", "t.preferred_local_file_id")
                .replace("db_duration_seconds", "t.db_duration_seconds")
                .replace("measured_duration_seconds", "t.measured_duration_seconds")
                .replace("essentia_duration_seconds", "t.essentia_duration_seconds")
                .replace("effective_duration_seconds", "t.effective_duration_seconds")
                .replace("duration_source", "t.duration_source")
                .replace("db_format", "t.db_format")
                .replace("measured_format", "t.measured_format")
                .replace("effective_format", "t.effective_format")
                .replace("db_bitrate_bps", "t.db_bitrate_bps")
                .replace("measured_bitrate_bps", "t.measured_bitrate_bps")
                .replace("effective_bitrate_bps", "t.effective_bitrate_bps")
                .replace("db_size_bytes", "t.db_size_bytes")
                .replace("measured_size_bytes", "t.measured_size_bytes")
                .replace("effective_size_bytes", "t.effective_size_bytes")
                .replace("bpm", "t.bpm")
                .replace("musical_key", "t.musical_key")
                .replace("scale", "t.scale")
                .replace("integrated_loudness_lufs", "t.integrated_loudness_lufs")
                .replace("loudness_range_lu", "t.loudness_range_lu")
                .replace("energy", "t.energy")
                .replace("danceability", "t.danceability")
                .replace("mood_json", "t.mood_json")
                .replace("instrument_json", "t.instrument_json")
                .replace("drop_loudness_lufs", "t.drop_loudness_lufs")
                .replace("updated_at_ms", "t.updated_at_ms"),
            compiled.where_sql,
            compiled.order_sql
        );
        let limit = compiled.limit;
        let offset = compiled.offset;
        let mut values = compiled.values;
        values.push(Value::Integer(i64::from(limit)));
        values.push(Value::Integer(i64::from(offset)));
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values), track_from_row)?;
        let items = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(LibraryPage {
            items,
            total,
            limit,
            offset,
        })
    }

    pub fn clear(&mut self) -> LibraryResult<()> {
        self.connection.execute_batch(
            "DELETE FROM source_records; DELETE FROM local_files; DELETE FROM tracks;
             DELETE FROM catalog_sources;",
        )?;
        Ok(())
    }
}

pub fn effective_duration(
    essentia: Option<f64>,
    measured: Option<f64>,
    netease: Option<f64>,
) -> (Option<f64>, Option<DurationSource>) {
    valid_positive(essentia)
        .map(|value| (Some(value), Some(DurationSource::Essentia)))
        .or_else(|| {
            valid_positive(measured).map(|value| (Some(value), Some(DurationSource::Measured)))
        })
        .or_else(|| {
            valid_positive(netease).map(|value| (Some(value), Some(DurationSource::Netease)))
        })
        .unwrap_or((None, None))
}

pub fn effective_measured_or_database<T: Clone>(
    measured: Option<T>,
    database: Option<T>,
) -> Option<T> {
    measured.or(database)
}

fn valid_positive(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

const TRACK_COLUMNS: &str = "track_key, netease_track_id, title, artists, artist_list_json, album,
    aliases_json, copyright_text, publish_date, netease_genre, essentia_genre,
    lyric_plain_text, lyric_translated_text, lyric_romanized_text, lyric_lrc_text,
    lyric_language, lyric_sync_type, lyric_source, cover_path, cover_available,
    local_status, preferred_local_file_id, db_duration_seconds, measured_duration_seconds,
    essentia_duration_seconds, effective_duration_seconds, duration_source, db_format,
    measured_format, effective_format, db_bitrate_bps, measured_bitrate_bps,
    effective_bitrate_bps, db_size_bytes, measured_size_bytes, effective_size_bytes,
    bpm, musical_key, scale, integrated_loudness_lufs, loudness_range_lu, energy,
    danceability, mood_json, instrument_json, drop_loudness_lufs, updated_at_ms";

fn track_upsert_sql() -> String {
    let columns = TRACK_COLUMNS.replace('\n', " ");
    let placeholders = (1..=47)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let assignments = columns
        .split(',')
        .map(str::trim)
        .filter(|column| *column != "track_key")
        .map(|column| format!("{column}=excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO tracks ({columns}) VALUES ({placeholders}) ON CONFLICT(track_key) DO UPDATE SET {assignments}"
    )
}

fn track_upsert_sql_preserving_analysis() -> String {
    let columns = TRACK_COLUMNS.replace('\n', " ");
    let placeholders = (1..=47)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let preserved = [
        "essentia_genre",
        "essentia_duration_seconds",
        "effective_duration_seconds",
        "duration_source",
        "bpm",
        "musical_key",
        "scale",
        "integrated_loudness_lufs",
        "loudness_range_lu",
        "energy",
        "danceability",
        "mood_json",
        "instrument_json",
        "drop_loudness_lufs",
    ];
    let assignments = columns
        .split(',')
        .map(str::trim)
        .filter(|column| *column != "track_key" && !preserved.contains(column))
        .map(|column| format!("{column}=excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO tracks ({columns}) VALUES ({placeholders}) ON CONFLICT(track_key) DO UPDATE SET {assignments}"
    )
}

fn track_values(track: &CatalogTrack) -> Vec<Value> {
    vec![
        text(&track.track_key),
        optional_text(track.netease_track_id.as_deref()),
        text(&track.title),
        text(&track.artists),
        text(&track.artist_list_json),
        text(&track.album),
        text(&track.aliases_json),
        text(&track.copyright_text),
        text(&track.publish_date),
        text(&track.netease_genre),
        text(&track.essentia_genre),
        text(&track.lyric_plain_text),
        text(&track.lyric_translated_text),
        text(&track.lyric_romanized_text),
        text(&track.lyric_lrc_text),
        text(&track.lyric_language),
        text(&track.lyric_sync_type),
        text(&track.lyric_source),
        optional_text(track.cover_path.as_deref()),
        Value::Integer(i64::from(track.cover_available)),
        text(track.local_status.as_str()),
        optional_i64(track.preferred_local_file_id),
        optional_f64(track.db_duration_seconds),
        optional_f64(track.measured_duration_seconds),
        optional_f64(track.essentia_duration_seconds),
        optional_f64(track.effective_duration_seconds),
        optional_text(track.duration_source.map(DurationSource::as_str)),
        optional_text(track.db_format.as_deref()),
        optional_text(track.measured_format.as_deref()),
        optional_text(track.effective_format.as_deref()),
        optional_i64(track.db_bitrate_bps),
        optional_i64(track.measured_bitrate_bps),
        optional_i64(track.effective_bitrate_bps),
        optional_i64(track.db_size_bytes),
        optional_i64(track.measured_size_bytes),
        optional_i64(track.effective_size_bytes),
        optional_f64(track.bpm),
        optional_text(track.musical_key.as_deref()),
        optional_text(track.scale.as_deref()),
        optional_f64(track.integrated_loudness_lufs),
        optional_f64(track.loudness_range_lu),
        optional_f64(track.energy),
        optional_f64(track.danceability),
        text(&track.mood_json),
        text(&track.instrument_json),
        optional_f64(track.drop_loudness_lufs),
        Value::Integer(track.updated_at_ms),
    ]
}

fn track_from_row(row: &Row<'_>) -> rusqlite::Result<CatalogTrack> {
    Ok(CatalogTrack {
        track_key: row.get(0)?,
        netease_track_id: row.get(1)?,
        title: row.get(2)?,
        artists: row.get(3)?,
        artist_list_json: row.get(4)?,
        album: row.get(5)?,
        aliases_json: row.get(6)?,
        copyright_text: row.get(7)?,
        publish_date: row.get(8)?,
        netease_genre: row.get(9)?,
        essentia_genre: row.get(10)?,
        lyric_plain_text: row.get(11)?,
        lyric_translated_text: row.get(12)?,
        lyric_romanized_text: row.get(13)?,
        lyric_lrc_text: row.get(14)?,
        lyric_language: row.get(15)?,
        lyric_sync_type: row.get(16)?,
        lyric_source: row.get(17)?,
        cover_path: row.get(18)?,
        cover_available: row.get::<_, i64>(19)? != 0,
        local_status: LocalStatus::parse(row.get(20)?),
        preferred_local_file_id: row.get(21)?,
        db_duration_seconds: row.get(22)?,
        measured_duration_seconds: row.get(23)?,
        essentia_duration_seconds: row.get(24)?,
        effective_duration_seconds: row.get(25)?,
        duration_source: DurationSource::parse(row.get(26)?),
        db_format: row.get(27)?,
        measured_format: row.get(28)?,
        effective_format: row.get(29)?,
        db_bitrate_bps: row.get(30)?,
        measured_bitrate_bps: row.get(31)?,
        effective_bitrate_bps: row.get(32)?,
        db_size_bytes: row.get(33)?,
        measured_size_bytes: row.get(34)?,
        effective_size_bytes: row.get(35)?,
        bpm: row.get(36)?,
        musical_key: row.get(37)?,
        scale: row.get(38)?,
        integrated_loudness_lufs: row.get(39)?,
        loudness_range_lu: row.get(40)?,
        energy: row.get(41)?,
        danceability: row.get(42)?,
        mood_json: row.get(43)?,
        instrument_json: row.get(44)?,
        drop_loudness_lufs: row.get(45)?,
        updated_at_ms: row.get(46)?,
    })
}

fn local_file_from_row(row: &Row<'_>) -> rusqlite::Result<CatalogLocalFile> {
    Ok(CatalogLocalFile {
        id: row.get(0)?,
        track_key: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        size_bytes: row.get(3)?,
        modified_at_ms: row.get(4)?,
        measured_format: row.get(5)?,
        measured_bitrate_bps: row.get(6)?,
        measured_duration_seconds: row.get(7)?,
        sample_rate_hz: row.get(8)?,
        channels: row.get(9)?,
        readable: row.get::<_, i64>(10)? != 0,
        probe_error: row.get(11)?,
    })
}

fn source_record_from_row(row: &Row<'_>) -> rusqlite::Result<CatalogSourceRecord> {
    Ok(CatalogSourceRecord {
        track_key: row.get(0)?,
        source_table: row.get(1)?,
        source_primary_key: row.get(2)?,
        source_version: row.get(3)?,
        raw_json: row.get(4)?,
        imported_at_ms: row.get(5)?,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn local_file_upsert_sql() -> String {
    "INSERT INTO local_files
        (id, track_key, path, size_bytes, modified_at_ms, measured_format,
         measured_bitrate_bps, measured_duration_seconds, sample_rate_hz, channels,
         readable, probe_error)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
     ON CONFLICT(path) DO UPDATE SET
         track_key=excluded.track_key, size_bytes=excluded.size_bytes,
         modified_at_ms=excluded.modified_at_ms, measured_format=excluded.measured_format,
         measured_bitrate_bps=excluded.measured_bitrate_bps,
         measured_duration_seconds=excluded.measured_duration_seconds,
         sample_rate_hz=excluded.sample_rate_hz, channels=excluded.channels,
         readable=excluded.readable, probe_error=excluded.probe_error"
        .to_string()
}

fn local_file_values(file: &CatalogLocalFile) -> Vec<Value> {
    vec![
        optional_i64(file.id),
        text(&file.track_key),
        text(&file.path.to_string_lossy()),
        Value::Integer(file.size_bytes),
        optional_i64(file.modified_at_ms),
        optional_text(file.measured_format.as_deref()),
        optional_i64(file.measured_bitrate_bps),
        optional_f64(file.measured_duration_seconds),
        optional_i64(file.sample_rate_hz),
        optional_i64(file.channels),
        Value::Integer(i64::from(file.readable)),
        optional_text(file.probe_error.as_deref()),
    ]
}

fn source_record_upsert_sql() -> String {
    "INSERT INTO source_records
        (track_key, source_table, source_primary_key, source_version, raw_json, imported_at_ms)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
     ON CONFLICT(source_table, source_primary_key) DO UPDATE SET
        track_key=excluded.track_key, source_version=excluded.source_version,
        raw_json=excluded.raw_json, imported_at_ms=excluded.imported_at_ms"
        .to_string()
}

fn source_record_values(record: &CatalogSourceRecord) -> Vec<Value> {
    vec![
        text(&record.track_key),
        text(&record.source_table),
        text(&record.source_primary_key),
        optional_text(record.source_version.as_deref()),
        text(&record.raw_json),
        Value::Integer(record.imported_at_ms),
    ]
}

fn catalog_source_upsert_sql() -> String {
    "INSERT INTO catalog_sources
        (database_path, database_size_bytes, database_modified_at_ms,
         last_imported_at_ms, import_status, last_error)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
     ON CONFLICT(database_path) DO UPDATE SET
        database_size_bytes=excluded.database_size_bytes,
        database_modified_at_ms=excluded.database_modified_at_ms,
        last_imported_at_ms=excluded.last_imported_at_ms,
        import_status=excluded.import_status, last_error=excluded.last_error"
        .to_string()
}

fn catalog_source_values(source: &CatalogSource) -> Vec<Value> {
    vec![
        text(&source.database_path.to_string_lossy()),
        Value::Integer(source.database_size_bytes),
        optional_i64(source.database_modified_at_ms),
        Value::Integer(source.last_imported_at_ms),
        text(&source.import_status),
        optional_text(source.last_error.as_deref()),
    ]
}

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn optional_text(value: Option<&str>) -> Value {
    value.map_or(Value::Null, text)
}

fn optional_i64(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::Integer)
}

fn optional_f64(value: Option<f64>) -> Value {
    value.map_or(Value::Null, Value::Real)
}
