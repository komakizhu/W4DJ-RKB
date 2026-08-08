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

const MAX_COVER_BYTES: u64 = 32 * 1024 * 1024;
const NETEASE_CONTAINER: &str = "com.netease.163music";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RecoveredMetadata {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
    pub(crate) cover: Option<Vec<u8>>,
    pub(crate) source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NeteaseRecord {
    path: String,
    file_name: String,
    title: String,
    artist: String,
    album: String,
    size_bytes: Option<u64>,
    duration_ms: Option<u64>,
    track_id: String,
    album_id: String,
    cover_path: String,
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
    let record = choose_record(source_path, &records)?;

    let mut recovered = RecoveredMetadata {
        title: record.title.clone(),
        artist: record.artist.clone(),
        album: record.album.clone(),
        cover: None,
        source: String::from("网易云本地数据库"),
    };

    recovered.cover =
        cover_from_record(source_path, record).or_else(|| find_adjacent_cover(source_path, record));

    if recovered.title.trim().is_empty()
        && recovered.artist.trim().is_empty()
        && recovered.album.trim().is_empty()
        && recovered.cover.is_none()
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

fn load_cached_records() -> Arc<Vec<NeteaseRecord>> {
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

fn database_candidates() -> Vec<PathBuf> {
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

fn load_records_from_db(path: &Path) -> rusqlite::Result<Vec<NeteaseRecord>> {
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
    let path = combine_path(&path_value, &directory);
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
        .unwrap_or(file_name_value);

    Ok(NeteaseRecord {
        path,
        file_name,
        title: row_text(row, 3),
        artist: row_text(row, 4),
        album: row_text(row, 5),
        size_bytes: row_u64(row, 6),
        duration_ms: row_u64(row, 7),
        track_id: row_text(row, 8),
        album_id: row_text(row, 9),
        cover_path: row_text(row, 10),
    })
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

fn choose_record<'a>(
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

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

fn cover_from_record(source_path: &Path, record: &NeteaseRecord) -> Option<Vec<u8>> {
    let raw = record.cover_path.trim();
    if raw.is_empty() {
        return None;
    }

    let candidate = PathBuf::from(raw);
    let candidates = [
        candidate.clone(),
        source_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&candidate),
        home_dir().unwrap_or_default().join(&candidate),
    ];
    candidates.iter().find_map(|path| read_cover_file(path))
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
        NeteaseRecord, choose_record, find_adjacent_cover, load_records_from_connection,
        record_match_score,
    };
    use rusqlite::{Connection, params};
    use std::fs;
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
