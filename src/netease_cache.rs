//! Persistent, lightweight NetEase locator cache.
//!
//! The cache deliberately stores only matching keys and source row locators.
//! It never stores the source JSON, lyrics, cover bytes, or a Dashboard song
//! projection.  The source SQLite database remains read-only; a complete
//! record is fetched only after a conversion candidate has been selected.

use crate::netease::{DatabaseFingerprintView, NeteaseTrackLocator};
use rusqlite::{Connection, OpenFlags, params};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CACHE_SCHEMA_VERSION: &str = "2";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheState {
    #[default]
    Idle,
    Ready,
    Stale,
    Building,
    Cancelling,
    Cancelled,
    Error,
}

impl CacheState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Ready => "ready",
            Self::Stale => "stale",
            Self::Building => "building",
            Self::Cancelling => "cancelling",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CacheSummary {
    pub state: CacheState,
    pub record_count: usize,
    pub database_path: Option<PathBuf>,
    pub fingerprint: Option<DatabaseFingerprintView>,
    pub last_error: Option<String>,
}

pub fn ensure_schema(path: &Path) -> rusqlite::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS netease_cache_meta (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS netease_track_locators (
             locator_key TEXT PRIMARY KEY,
             track_id TEXT NOT NULL DEFAULT '',
             source_table TEXT NOT NULL,
             source_primary_key TEXT NOT NULL,
             source_version TEXT,
             normalized_path TEXT NOT NULL DEFAULT '',
             normalized_file_name TEXT NOT NULL DEFAULT '',
             size_bytes INTEGER,
             title_key TEXT NOT NULL DEFAULT '',
             artist_key TEXT NOT NULL DEFAULT '',
             album_key TEXT NOT NULL DEFAULT ''
         );
         CREATE INDEX IF NOT EXISTS netease_locator_track_id
             ON netease_track_locators(track_id);
         CREATE INDEX IF NOT EXISTS netease_locator_path
             ON netease_track_locators(normalized_path);
         CREATE INDEX IF NOT EXISTS netease_locator_file_name
             ON netease_track_locators(normalized_file_name);",
    )
}

pub fn read_summary(
    path: &Path,
    database_path: Option<&Path>,
    current_fingerprint: Option<&DatabaseFingerprintView>,
) -> rusqlite::Result<CacheSummary> {
    if !path.is_file() {
        return Ok(CacheSummary::default());
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_meta: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='netease_cache_meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_meta {
        return Ok(CacheSummary::default());
    }
    let value = |key: &str| -> rusqlite::Result<Option<String>> {
        connection
            .query_row(
                "SELECT value FROM netease_cache_meta WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
    };
    let stored_fingerprint = value("fingerprint")?
        .and_then(|raw| serde_json::from_str::<DatabaseFingerprintView>(&raw).ok());
    let stored_path = value("databasePath")?.map(PathBuf::from);
    let schema_is_current = value("schemaVersion")?.as_deref() == Some(CACHE_SCHEMA_VERSION);
    let state = if !schema_is_current {
        CacheState::Stale
    } else {
        match value("status")?.as_deref() {
            Some("ready")
                if stored_path == database_path.map(Path::to_path_buf)
                    && current_fingerprint.is_some_and(|fingerprint| {
                        Some(fingerprint) == stored_fingerprint.as_ref()
                    }) =>
            {
                CacheState::Ready
            }
            Some("ready") => CacheState::Stale,
            Some("building") => CacheState::Building,
            Some("cancelling") => CacheState::Cancelling,
            Some("cancelled") => CacheState::Cancelled,
            Some("error") => CacheState::Error,
            _ => CacheState::Idle,
        }
    };
    let record_count = connection
        .query_row("SELECT COUNT(*) FROM netease_track_locators", [], |row| {
            row.get::<_, usize>(0)
        })
        .unwrap_or_default();
    Ok(CacheSummary {
        state,
        record_count,
        database_path: stored_path,
        fingerprint: stored_fingerprint,
        last_error: value("lastError")?,
    })
}

pub fn read_locators(path: &Path) -> rusqlite::Result<Vec<NeteaseTrackLocator>> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection.prepare(
        "SELECT track_id, source_table, source_primary_key, source_version,
                normalized_path, normalized_file_name, size_bytes, title_key,
                artist_key, album_key
           FROM netease_track_locators",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(NeteaseTrackLocator {
            track_id: row.get(0)?,
            source_table: row.get(1)?,
            source_primary_key: row.get(2)?,
            source_version: row.get(3)?,
            normalized_path: row.get(4)?,
            normalized_file_name: row.get(5)?,
            size_bytes: row
                .get::<_, Option<i64>>(6)?
                .and_then(|value| u64::try_from(value).ok()),
            title_key: row.get(7)?,
            artist_key: row.get(8)?,
            album_key: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub fn mark_state(path: &Path, state: CacheState, error: Option<&str>) -> rusqlite::Result<()> {
    ensure_schema(path)?;
    let connection = Connection::open(path)?;
    connection.execute(
        "INSERT INTO netease_cache_meta(key,value) VALUES ('status',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [state.as_str()],
    )?;
    if let Some(error) = error {
        connection.execute(
            "INSERT INTO netease_cache_meta(key,value) VALUES ('lastError',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [error],
        )?;
    }
    Ok(())
}

pub fn replace_locators(
    path: &Path,
    database_path: &Path,
    fingerprint: &DatabaseFingerprintView,
    locators: &[NeteaseTrackLocator],
) -> rusqlite::Result<()> {
    ensure_schema(path)?;
    let mut connection = Connection::open(path)?;
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM netease_track_locators", [])?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO netease_track_locators(
                locator_key, track_id, source_table, source_primary_key,
                source_version, normalized_path, normalized_file_name,
                size_bytes, title_key, artist_key, album_key
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        )?;
        for locator in locators {
            let key = format!("{}:{}", locator.source_table, locator.source_primary_key);
            statement.execute(params![
                key,
                locator.track_id,
                locator.source_table,
                locator.source_primary_key,
                locator.source_version,
                locator.normalized_path,
                locator.normalized_file_name,
                locator
                    .size_bytes
                    .map(|value| value.min(i64::MAX as u64) as i64),
                locator.title_key,
                locator.artist_key,
                locator.album_key,
            ])?;
        }
    }
    for (key, value) in [
        ("schemaVersion", CACHE_SCHEMA_VERSION.to_string()),
        ("status", CacheState::Ready.as_str().to_string()),
        ("databasePath", database_path.to_string_lossy().into_owned()),
        (
            "fingerprint",
            serde_json::to_string(fingerprint).unwrap_or_default(),
        ),
        ("recordCount", locators.len().to_string()),
        ("lastError", String::new()),
        ("updatedAtMs", now_ms().to_string()),
    ] {
        transaction.execute(
            "INSERT INTO netease_cache_meta(key,value) VALUES (?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
    }
    transaction.commit()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
