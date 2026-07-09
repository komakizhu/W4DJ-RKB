#![allow(dead_code)]

#[path = "../src/config.rs"]
mod config;
#[path = "../src/metadata.rs"]
mod metadata;
#[path = "../src/sync.rs"]
mod sync;
#[path = "../src/task.rs"]
mod task;

use config::Mode;
use std::collections::HashMap;
use std::fs;
use task::{TaskController, TaskState};

#[test]
fn pause_waits_for_current_file() {
    let task = TaskController::running(3);
    task.request_pause();
    assert!(task.pause_after_current_file());
}

#[test]
fn running_initializes_progress() {
    let task = TaskState::running(3);
    assert_eq!(task.total, 3);
    assert_eq!(task.completed, 0);
    assert!(!task.paused);
    assert!(!task.cancelled);
}

#[test]
fn completion_does_not_exceed_total() {
    let mut task = TaskState::running(1);
    task.complete_current_file();
    task.complete_current_file();
    assert_eq!(task.completed, 1);
}

#[test]
fn cancel_prevents_new_files_from_starting() {
    let task = TaskController::running(2);
    task.request_cancel();

    assert!(!task.should_start_next_file());
    assert!(task.snapshot().cancelled);
}

#[test]
fn snapshot_reports_remaining_work() {
    let task = TaskController::running(3);
    task.complete_current_file();

    let snapshot = task.snapshot();

    assert_eq!(snapshot.total, 3);
    assert_eq!(snapshot.completed, 1);
    assert_eq!(snapshot.remaining, 2);
    assert!(!snapshot.paused);
}

#[test]
fn sync_entry_stops_before_next_file_when_pause_requested() {
    let name = String::from("song");
    let info = (String::from("1"), String::from("/definitely/missing.mp3"));
    let mut owned_files = HashMap::new();
    owned_files.insert(name, info);
    let queued_files = owned_files.iter().collect();
    let task = TaskController::running(1);

    task.request_pause();

    let snapshot = sync::sync_music_library_with_task(
        &queued_files,
        "/unused-destination",
        &Mode::Compat,
        None,
        &task,
    )
    .unwrap();

    assert_eq!(snapshot.completed, 0);
    assert!(snapshot.paused);
    assert_eq!(snapshot.remaining, 1);
}

#[test]
fn pause_requested_after_current_file_stops_before_next_file() {
    let temp_dir = std::env::temp_dir().join(format!("w4dj-task-state-{}", std::process::id()));
    let source = temp_dir.join("source");
    let dest = temp_dir.join("dest");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&dest).unwrap();
    let first_file = source.join("first.mp3");
    let second_file = source.join("second.mp3");
    fs::write(&first_file, b"first").unwrap();
    fs::write(&second_file, b"second").unwrap();

    let first_name = String::from("first");
    let second_name = String::from("second");
    let first_info = (String::from("5"), first_file.to_string_lossy().into_owned());
    let second_info = (
        String::from("6"),
        second_file.to_string_lossy().into_owned(),
    );
    let mut owned_files = HashMap::new();
    owned_files.insert(first_name, first_info);
    owned_files.insert(second_name, second_info);
    let queued_files = owned_files.iter().collect();
    let task = TaskController::running(2);

    let snapshot = sync::sync_music_library_with_observer(
        &queued_files,
        dest.to_str().unwrap(),
        &Mode::Compat,
        None,
        &task,
        |_, task| task.request_pause(),
    )
    .unwrap();

    assert_eq!(snapshot.completed, 1);
    assert!(snapshot.paused);
    assert_eq!(snapshot.remaining, 1);
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn failed_file_does_not_increment_completed_count() {
    let name = String::from("missing");
    let info = (String::from("1"), String::from("/definitely/missing.mp3"));
    let mut owned_files = HashMap::new();
    owned_files.insert(name, info);
    let queued_files = owned_files.iter().collect();
    let task = TaskController::running(1);

    let result = sync::sync_music_library_with_task(
        &queued_files,
        "/unused-destination",
        &Mode::Compat,
        None,
        &task,
    );

    assert!(result.is_err());
    assert_eq!(task.snapshot().completed, 0);
}

#[test]
fn policy_entry_returns_status_snapshot() {
    let owned_files: HashMap<String, (String, String)> = HashMap::new();
    let queued_files = owned_files.iter().collect();

    let snapshot = sync::sync_music_library_with_policy(
        &queued_files,
        "/unused-destination",
        &Mode::Compat,
        None,
    )
    .unwrap();

    assert_eq!(snapshot.total, 0);
    assert_eq!(snapshot.completed, 0);
    assert_eq!(snapshot.remaining, 0);
}
