use rusqlite::{Connection, params};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use w4dj::netease::{
    NeteaseMetadataResolver, database_fingerprint_view, load_locators_from_db_observed,
};
use w4dj::netease::{load_records_from_db_observed, probe_netease_database};
use w4dj::netease_cache::{self, CacheState};

fn create_database(path: &Path, schema: &str) -> Connection {
    let connection = Connection::open(path).expect("database should open");
    connection
        .execute_batch(schema)
        .expect("schema should be created");
    connection
}

#[test]
fn probe_netease_database_counts_rows_without_parsing_json() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("sqlite_storage.sqlite3");
    let connection = create_database(
        &database,
        "CREATE TABLE track (
            file TEXT,
            title TEXT,
            artist TEXT,
            detail TEXT
        );",
    );
    connection
        .execute(
            "INSERT INTO track(file, title, artist, detail) VALUES (?1, ?2, ?3, ?4)",
            params![
                "/Music/网易云音乐/Song.ncm",
                "Song",
                "Artist",
                "{ definitely not valid json"
            ],
        )
        .unwrap();
    drop(connection);
    fs::write(PathBuf::from(format!("{}-wal", database.display())), b"wal").unwrap();

    let summary = probe_netease_database(&database).unwrap();
    assert!(summary.supported);
    assert_eq!(summary.record_count, 1);
    assert_eq!(summary.path, database);
    assert!(summary.fingerprint.main.exists);
    assert!(summary.fingerprint.wal.exists);
}

#[test]
fn probe_netease_database_reports_unsupported_schema() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("wrong.sqlite3");
    create_database(&database, "CREATE TABLE unrelated (id INTEGER);");

    let summary = probe_netease_database(&database).unwrap();
    assert!(!summary.supported);
    assert_eq!(summary.record_count, 0);
}

#[test]
fn probe_netease_database_returns_error_for_missing_file() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("missing.sqlite3");
    assert!(probe_netease_database(&database).is_err());
}

#[test]
fn load_records_from_db_observed_reads_tables_with_bounded_parallelism() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("sqlite_storage.sqlite3");
    let connection = create_database(
        &database,
        "CREATE TABLE track (
            file TEXT,
            title TEXT,
            artist TEXT,
            album TEXT,
            tid TEXT,
            detail TEXT
        );
        CREATE TABLE web_track (
            file TEXT,
            title TEXT,
            artist TEXT,
            album TEXT,
            tid TEXT,
            detail TEXT
        );",
    );
    connection
        .execute(
            "INSERT INTO track(file, title, artist, album, tid, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "/Music/网易云音乐/Song.ncm",
                "Song",
                "Artist",
                "Album",
                "42",
                r#"{"track":{"name":"Song","artists":[{"name":"Artist"}]}}"#
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO web_track(file, title, artist, album, tid, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "/Music/网易云音乐/Song.mp3",
                "",
                "",
                "",
                "42",
                r#"{"track":{"name":"Song","artists":[{"name":"Artist"}],"album":{"name":"Album"}}}"#
            ],
        )
        .unwrap();
    drop(connection);

    let mut observed = Vec::new();
    let records = load_records_from_db_observed(&database, 2, |table, processed, total| {
        observed.push((table.to_string(), processed, total));
    })
    .unwrap();

    assert_eq!(records.len(), 1);
    assert!(
        observed
            .iter()
            .any(|(table, _, total)| table == "track" && *total == 1)
    );
    assert!(
        observed
            .iter()
            .any(|(table, _, total)| table == "web_track" && *total == 1)
    );
}

#[test]
fn lightweight_locator_cache_round_trips_without_raw_metadata_columns() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("sqlite_storage.sqlite3");
    let cache = directory.path().join("library-dashboard.sqlite3");
    let source = directory.path().join("Song - Artist.mp3");
    fs::write(&source, b"audio").unwrap();
    let connection = create_database(
        &database,
        "CREATE TABLE track (file TEXT, title TEXT, artist TEXT, album TEXT, tid INTEGER, filesize INTEGER, detail TEXT);",
    );
    connection.execute(
        "INSERT INTO track(file,title,artist,album,tid,filesize,detail) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![source.to_string_lossy(), "Song", "Artist", "Album", 42, 5, "{\"lyric\":\"must not be cached\"}"],
    ).unwrap();
    drop(connection);

    let locators = load_locators_from_db_observed(&database, |_, _, _| true).unwrap();
    assert_eq!(locators.len(), 1);
    assert_eq!(locators[0].track_id, "42");
    assert_eq!(locators[0].title_key, "song");
    netease_cache::replace_locators(
        &cache,
        &database,
        &database_fingerprint_view(&database),
        &locators,
    )
    .unwrap();
    let summary = netease_cache::read_summary(
        &cache,
        Some(&database),
        Some(&database_fingerprint_view(&database)),
    )
    .unwrap();
    assert_eq!(summary.state, CacheState::Ready);
    let resolver = NeteaseMetadataResolver::from_locators(&database, locators, None);
    let identity = resolver.track_identity(&source).unwrap();
    assert_eq!(identity.title, "Song");
    let cache_connection = Connection::open(&cache).unwrap();
    assert!(!cache_connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('netease_track_locators') WHERE name='raw_json')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap());
}

#[test]
fn lightweight_locator_cache_becomes_stale_when_wal_fingerprint_changes() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("sqlite_storage.sqlite3");
    let cache = directory.path().join("library-dashboard.sqlite3");
    let connection = create_database(&database, "CREATE TABLE track (file TEXT);");
    drop(connection);
    let fingerprint = database_fingerprint_view(&database);
    netease_cache::replace_locators(&cache, &database, &fingerprint, &[]).unwrap();
    fs::write(
        PathBuf::from(format!("{}-wal", database.display())),
        b"changed",
    )
    .unwrap();
    let changed = database_fingerprint_view(&database);
    let summary = netease_cache::read_summary(&cache, Some(&database), Some(&changed)).unwrap();
    assert_eq!(summary.state, CacheState::Stale);
}

#[test]
fn resolver_matches_typographic_filename_to_web_track_identity() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("sqlite_storage.sqlite3");
    let source = directory
        .path()
        .join("Mass Destruction (＂P3＂ + ＂P3F＂ ver.) - 川村ゆみ,Lotus Juice.flac");
    fs::write(&source, b"untagged flac placeholder").unwrap();
    let connection = create_database(
        &database,
        "CREATE TABLE web_track (tid TEXT NOT NULL, version INTEGER NOT NULL DEFAULT 0, track TEXT NOT NULL);",
    );
    connection
        .execute(
            "INSERT INTO web_track(tid,version,track) VALUES (?1,0,?2)",
            params![
                "864433756",
                r#"{"album":{"id":71720241,"name":"『P3D』＆『P5D』フルサウンドトラック"},"artists":[{"name":"川村ゆみ"},{"name":"Lotus Juice"}],"id":864433756,"name":"Mass Destruction (\"P3\" + \"P3F\" ver.)"}"#
            ],
        )
        .unwrap();
    drop(connection);

    let resolver = NeteaseMetadataResolver::load_exact(&database).unwrap();
    let identity = resolver
        .track_identity(&source)
        .expect("filename should match");
    assert_eq!(identity.track_id.as_deref(), Some("864433756"));
    assert_eq!(identity.album_id.as_deref(), Some("71720241"));
    assert_eq!(identity.title, "Mass Destruction (\"P3\" + \"P3F\" ver.)");
    assert_eq!(identity.artists, "川村ゆみ, Lotus Juice");
    assert_eq!(identity.album, "『P3D』＆『P5D』フルサウンドトラック");

    let locators = load_locators_from_db_observed(&database, |_, _, _| true).unwrap();
    assert_eq!(locators.len(), 1);
    assert_eq!(
        locators[0].normalized_file_name,
        "mass destruction (\"p3\" + \"p3f\" ver.) - 川村ゆみ,lotus juice"
    );
    let lazy_resolver = NeteaseMetadataResolver::from_locators(&database, locators, None);
    assert_eq!(lazy_resolver.track_identity(&source), Some(identity));
}

#[test]
fn real_mass_destruction_source_matches_current_netease_database_when_present() {
    let Ok(database) = env::var("W4DJ_REAL_NETEASE_DB") else {
        return;
    };
    let Ok(source) = env::var("W4DJ_REAL_NETEASE_SOURCE") else {
        return;
    };
    let source = PathBuf::from(source);
    if !Path::new(&database).is_file() || !source.is_file() {
        return;
    }
    let resolver = NeteaseMetadataResolver::load_exact(Path::new(&database)).unwrap();
    let identity = resolver
        .track_identity(&source)
        .expect("current Mass Destruction source should match the selected database");
    assert_eq!(identity.track_id.as_deref(), Some("864433756"));
    assert_eq!(identity.album_id.as_deref(), Some("71720241"));
    assert_eq!(identity.title, "Mass Destruction (\"P3\" + \"P3F\" ver.)");
    assert_eq!(identity.artists, "川村ゆみ, Lotus Juice");
    let locators = load_locators_from_db_observed(Path::new(&database), |_, _, _| true).unwrap();
    let lazy_resolver =
        NeteaseMetadataResolver::from_locators(Path::new(&database), locators, None);
    assert_eq!(lazy_resolver.track_identity(&source), Some(identity));
}
