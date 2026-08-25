//! Best-effort recovery of metadata left beside music downloaded by NetEase
//! Cloud Music.
//!
//! This module is intentionally local-only.  It never calls a NetEase API and
//! never tries to download artwork.  It reads the desktop client's local
//! SQLite library, matches a source file conservatively, and looks for an
//! explicitly named neighbouring cover image.  The converter can then merge
//! the recovered values into the output tags.

use rusqlite::{Connection, OpenFlags, Row, types::ValueRef};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

const MAX_COVER_BYTES: u64 = 32 * 1024 * 1024;
const NETEASE_CONTAINER: &str = "com.netease.163music";
const NETEASE_COVER_DIR_ENV: &str = "W4DJ_NETEASE_COVER_DIR";

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NeteaseRecord {
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

#[derive(Debug, Clone, Default)]
struct RecordCache {
    fingerprint: Vec<DatabaseFingerprint>,
    records: Arc<Vec<NeteaseRecord>>,
}

static RECORD_CACHE: OnceLock<Mutex<RecordCache>> = OnceLock::new();

/// Recover local NetEase metadata without contacting the network.
pub(crate) fn recover_local_metadata(source_path: &Path) -> Option<RecoveredMetadata> {
    let records = load_cached_records();
    let record = choose_record(source_path, &records);

    let mut recovered = RecoveredMetadata {
        title: record
            .map(|record| record.title.clone())
            .unwrap_or_default(),
        artist: record
            .map(|record| record.artist.clone())
            .unwrap_or_default(),
        album: record
            .map(|record| record.album.clone())
            .unwrap_or_default(),
        cover: None,
        genre: record
            .map(|record| record.genre.clone())
            .unwrap_or_default(),
        aliases_json: record
            .map(|record| record.aliases_json.clone())
            .unwrap_or_default(),
        copyright_text: record
            .map(|record| record.copyright_text.clone())
            .unwrap_or_default(),
        publish_date: record
            .map(|record| record.publish_date.clone())
            .unwrap_or_default(),
        lyric_plain_text: record
            .map(|record| record.lyric_plain_text.clone())
            .unwrap_or_default(),
        lyric_translated_text: record
            .map(|record| record.lyric_translated_text.clone())
            .unwrap_or_default(),
        lyric_romanized_text: record
            .map(|record| record.lyric_romanized_text.clone())
            .unwrap_or_default(),
        lyric_lrc_text: record
            .map(|record| record.lyric_lrc_text.clone())
            .unwrap_or_default(),
        lyric_language: record
            .map(|record| record.lyric_language.clone())
            .unwrap_or_default(),
        lyric_sync_type: record
            .map(|record| record.lyric_sync_type.clone())
            .unwrap_or_default(),
        lyric_source: record
            .map(|record| record.lyric_source.clone())
            .unwrap_or_default(),
        source: if record.is_some() {
            String::from("网易云本地数据库")
        } else {
            String::new()
        },
    };

    recovered.cover = recover_local_cover(source_path);

    if recovered.title.trim().is_empty()
        && recovered.artist.trim().is_empty()
        && recovered.album.trim().is_empty()
        && recovered.cover.is_none()
        && recovered.genre.trim().is_empty()
        && recovered.lyric_plain_text.trim().is_empty()
        && recovered.lyric_lrc_text.trim().is_empty()
    {
        return None;
    }

    if recovered.cover.is_some() && recovered.source.is_empty() {
        recovered.source = String::from("网易云本地封面");
    } else if recovered.cover.is_some() {
        recovered.source.push_str(" + 本地封面");
    }

    Some(recovered)
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
            && let Ok(mut database_records) = load_records_from_db(path)
        {
            records.append(&mut database_records);
        }
    }

    cache.fingerprint = fingerprint;
    cache.records = Arc::new(records);
    Arc::clone(&cache.records)
}

pub(crate) fn database_candidates() -> Vec<PathBuf> {
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
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    let connection = Connection::open_with_flags(path, flags)?;
    load_records_from_connection(&connection)
}

fn load_records_from_connection(connection: &Connection) -> rusqlite::Result<Vec<NeteaseRecord>> {
    let mut records = Vec::new();
    for table in ["track", "web_offline_track", "web_cloud_track"] {
        if table_exists(connection, table)? {
            records.extend(read_table_records(connection, table)?);
        }
    }
    if table_exists(connection, "web_track")? {
        let web_track_records = read_table_records(connection, "web_track")?;
        merge_track_metadata(&mut records, web_track_records);
    }
    Ok(records)
}

fn table_exists(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn read_table_records(
    connection: &Connection,
    table: &str,
) -> rusqlite::Result<Vec<NeteaseRecord>> {
    let available = table_columns(connection, table)?;
    let select = [
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
        select_expression(&available, &["duration", "duration_ms"], "duration_ms"),
        select_expression(&available, &["tid", "track_id", "id"], "track_id"),
        select_expression(&available, &["album_id", "aid"], "album_id"),
        select_expression(
            &available,
            &["cover_path", "cover", "album_cover", "pic", "picture"],
            "cover_path",
        ),
        select_expression(
            &available,
            &["detail", "track", "source_text", "source_extra"],
            "metadata_json",
        ),
    ]
    .join(", ");
    let sql = format!("SELECT {select} FROM \"{table}\" LIMIT 200000");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], record_from_row)?;

    let records = rows
        .filter_map(|row| row.ok())
        .filter_map(|record| {
            let has_any_value = !record.path.is_empty()
                || !record.file_name.is_empty()
                || !record.title.is_empty()
                || !record.artist.is_empty()
                || !record.album.is_empty();
            has_any_value.then_some(record)
        })
        .collect::<Vec<_>>();
    Ok(records)
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
    let path_value = row_text(row, 0);
    let directory = row_text(row, 1);
    let file_name_value = row_text(row, 2);
    let metadata_json = row_text(row, 11);
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
    let cover_data = row_blob(row, 10).filter(|bytes| is_supported_image(bytes));
    let cover_references = cover_references_from_json(&metadata_json);

    Ok(NeteaseRecord {
        path,
        file_name,
        title: prefer_nonempty(row_text(row, 3), &json_metadata.title),
        artist: prefer_nonempty(row_text(row, 4), &json_metadata.artist),
        album: prefer_nonempty(row_text(row, 5), &json_metadata.album),
        size_bytes: row_u64(row, 6),
        duration_ms: row_u64(row, 7),
        track_id: prefer_nonempty(row_text(row, 8), &json_metadata.track_id),
        album_id: prefer_nonempty(row_text(row, 9), &json_metadata.album_id),
        cover_path: row_text(row, 10),
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
    normalize_text(&left.title) == normalize_text(&right.title)
        && normalize_text(&left.artist) == normalize_text(&right.artist)
        && (!left.album.trim().is_empty()
            && !right.album.trim().is_empty()
            && normalize_text(&left.album) == normalize_text(&right.album))
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
    let mut ranked = records
        .iter()
        .map(|record| (record_match_score(source_path, record), record))
        .filter(|(score, _)| *score >= 500)
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.0.cmp(&left.0));

    let (best_score, best) = ranked.first().copied()?;
    // A filename by itself is not enough: a user can have the same track in
    // multiple folders, and NetEase's database may retain stale entries.
    // Accept an exact/suffix path, a filename plus file size, or a filename
    // whose title/artist pair also agrees (scores 780+).
    if best_score < 780 {
        return None;
    }
    if ranked
        .get(1)
        .is_some_and(|(score, other)| *score == best_score && !same_record_identity(best, other))
    {
        return None;
    }
    Some(best)
}

fn record_match_score(source_path: &Path, record: &NeteaseRecord) -> u32 {
    let source_key = normalized_path(source_path.to_string_lossy().as_ref());
    let record_key = normalized_path(&record.path);
    let source_name = normalized_file_name(source_path.to_string_lossy().as_ref());
    let record_name = normalized_file_name(if record.file_name.is_empty() {
        &record.path
    } else {
        &record.file_name
    });
    let source_size = fs::metadata(source_path)
        .ok()
        .map(|metadata| metadata.len());
    let mut score = 0;

    if !record_key.is_empty() && record_key == source_key {
        score += 1000;
    } else if !record_key.is_empty()
        && (!source_key.is_empty()
            && (source_key.ends_with(&format!("/{record_key}"))
                || record_key.ends_with(&format!("/{source_key}"))))
    {
        score += 820;
    }
    if !source_name.is_empty() && source_name == record_name {
        score += 600;
    } else if same_file_stem(&source_name, &record_name) {
        // NetEase may retain the original .ncm name in its database after the
        // user has exported/decrypted the same track to .mp3 or .flac.
        score += 600;
    }
    if source_size.is_some() && source_size == record.size_bytes {
        score += 220;
    }

    if let Some((left, right)) = split_filename_parts(source_path) {
        let left = normalize_text(&left);
        let right = normalize_text(&right);
        let title = normalize_text(&record.title);
        let artist = normalize_text(&record.artist);
        if !title.is_empty()
            && !artist.is_empty()
            && ((title == left && artist == right) || (title == right && artist == left))
        {
            score += 180;
        }
    }

    score
}

fn same_record_identity(left: &NeteaseRecord, right: &NeteaseRecord) -> bool {
    normalize_text(&left.title) == normalize_text(&right.title)
        && normalize_text(&left.artist) == normalize_text(&right.artist)
        && normalize_text(&left.album) == normalize_text(&right.album)
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

fn normalized_file_name(value: &str) -> String {
    Path::new(&value.replace('\\', "/"))
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalize_text)
        .unwrap_or_default()
}

fn same_file_stem(left: &str, right: &str) -> bool {
    let left = Path::new(left)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(normalize_text)
        .unwrap_or_default();
    let right = Path::new(right)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(normalize_text)
        .unwrap_or_default();
    !left.is_empty() && left == right
}

fn normalize_text(value: &str) -> String {
    value
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
        NeteaseRecord, choose_record, cover_from_record, cover_references_from_json,
        find_adjacent_cover, find_cover_by_name_in_roots, find_source_directory_cover,
        load_records_from_connection, record_match_score,
    };
    use rusqlite::{Connection, params};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

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
