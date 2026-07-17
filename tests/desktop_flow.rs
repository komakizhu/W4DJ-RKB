use w4dj::config::Mode;
use w4dj::desktop::{DesktopController, DesktopState, DesktopStatus};
use w4dj::history::FailedFile;
use w4dj::preferences::{AppPreferences, SyncSlotPreferences};

#[test]
fn progress_updates_are_reflected_in_desktop_state() {
    let mut controller = test_controller();

    controller.start_sync(0, 3).unwrap();
    controller.record_file_started(0, "track.wav").unwrap();
    controller.complete_current_file(0).unwrap();

    assert!(matches!(
        controller.state().slots[0].status,
        DesktopStatus::Running
    ));
    assert_eq!(controller.state().slots[0].progress_total, 3);
    assert_eq!(controller.state().slots[0].new_tracks, 3);
    assert_eq!(controller.state().slots[0].progress_completed, 1);
    assert_eq!(controller.state().slots[0].current_file, "track.wav");
}

#[test]
fn pause_requests_wait_for_current_file() {
    let mut controller = test_controller();

    controller.start_sync(0, 3).unwrap();
    controller.pause_sync(0).unwrap();

    assert!(matches!(
        controller.state().slots[0].status,
        DesktopStatus::Running
    ));
    assert!(controller.pause_after_current_file(0).unwrap());
    assert_eq!(controller.state().slots[0].progress_total, 3);
    assert_eq!(controller.state().slots[0].progress_completed, 0);
    assert!(!controller.pause_after_current_file(1).unwrap());
}

#[test]
fn starting_slot_two_does_not_change_slot_one() {
    let mut controller = test_controller();

    controller.start_sync(1, 3).unwrap();
    controller.record_file_started(1, "second.wav").unwrap();
    controller.complete_current_file(1).unwrap();

    assert!(matches!(
        controller.state().slots[0].status,
        DesktopStatus::Idle
    ));
    assert_eq!(controller.state().slots[0].progress_completed, 0);
    assert!(matches!(
        controller.state().slots[1].status,
        DesktopStatus::Running
    ));
    assert_eq!(controller.state().slots[1].progress_completed, 1);
    assert_eq!(controller.state().slots[1].current_file, "second.wav");
}

#[test]
fn slot_two_blank_destination_falls_back_to_slot_one_destination() {
    let mut controller = test_controller();
    controller.select_destination_directory(1, "   ").unwrap();

    assert_eq!(
        controller.effective_destination(1).unwrap().as_deref(),
        Some("/music/out-1")
    );
    assert_eq!(controller.state().slots[1].destination_directory, "   ");
}

#[test]
fn slot_two_uses_its_own_destination_when_configured() {
    let controller = test_controller();

    assert_eq!(
        controller.effective_destination(1).unwrap().as_deref(),
        Some("/music/out-2")
    );
}

#[test]
fn slot_two_fallback_does_not_require_slot_one_source() {
    let mut controller = test_controller();
    controller.select_source_directory(0, "").unwrap();
    controller.select_destination_directory(1, "").unwrap();

    assert_eq!(
        controller.effective_destination(1).unwrap().as_deref(),
        Some("/music/out-1")
    );
}

#[test]
fn invalid_slot_indexes_are_rejected() {
    let mut controller = test_controller();

    assert!(controller.select_source_directory(2, "/invalid").is_err());
    assert!(controller.start_sync(2, 1).is_err());
    assert!(controller.effective_destination(2).is_err());
}

#[test]
fn global_start_targets_only_configured_idle_slots() {
    let mut controller = test_controller();
    controller.select_source_directory(0, "   ").unwrap();

    assert_eq!(controller.startable_slot_indexes(), vec![1]);

    controller.start_sync(1, 0).unwrap();
    assert!(controller.startable_slot_indexes().is_empty());
}

#[test]
fn pause_request_keeps_slot_running_until_the_worker_stops() {
    let mut controller = test_controller();
    controller.start_sync(0, 3).unwrap();

    controller.pause_all_running().unwrap();

    assert!(matches!(
        controller.state().slots[0].status,
        DesktopStatus::Running
    ));
    assert!(controller.pause_after_current_file(0).unwrap());
    assert!(matches!(
        controller.state().slots[1].status,
        DesktopStatus::Idle
    ));
}

#[test]
fn cancelling_a_running_slot_stops_new_files_and_finishes_as_cancelled() {
    let mut controller = test_controller();
    controller.start_sync(0, 3).unwrap();

    controller.cancel_sync(0).unwrap();
    let task = controller.task_controller(0).unwrap();
    assert!(task.is_cancelled());
    assert!(!task.should_start_next_file());

    controller.finish_sync(0, task.snapshot()).unwrap();
    assert!(matches!(
        controller.state().slots[0].status,
        DesktopStatus::Cancelled
    ));
}

#[test]
fn confirmed_start_uses_preview_candidate_count() {
    let mut controller = test_controller();
    controller.start_confirmed_sync(0, 3).unwrap();

    assert_eq!(controller.state().slots[0].progress_total, 3);
    assert!(matches!(
        controller.state().slots[0].status,
        DesktopStatus::Running
    ));
}

#[test]
fn starting_a_new_task_discards_logs_from_the_previous_task() {
    let mut controller = test_controller();
    controller.start_confirmed_sync(0, 1).unwrap();
    controller
        .push_log(0, "Old task failed: private-old-path.flac")
        .unwrap();

    controller.start_confirmed_sync(0, 1).unwrap();

    assert_eq!(controller.state().slots[0].logs, vec!["Sync started"]);
}

#[test]
fn preflight_counts_keep_their_meaning_during_conversion() {
    let mut controller = test_controller();
    controller.start_confirmed_sync(0, 3).unwrap();
    controller
        .set_preflight_summary(0, 3, 2, 2, 1, Some(1024))
        .unwrap();

    assert_eq!(controller.state().slots[0].new_tracks, 3);
    assert_eq!(controller.state().slots[0].progress_completed, 0);
    assert_eq!(controller.state().slots[0].existing_tracks, 2);
    assert_eq!(controller.state().slots[0].skipped_tracks, 2);
    assert_eq!(controller.state().slots[0].error_tracks, 1);

    let task = controller.task_controller(0).unwrap();
    task.complete_current_file();
    controller
        .record_file_result(0, "converted", task.snapshot(), None)
        .unwrap();
    assert_eq!(controller.state().slots[0].new_tracks, 3);
    assert_eq!(controller.state().slots[0].progress_completed, 1);
    assert_eq!(controller.state().slots[0].skipped_tracks, 2);
    assert_eq!(controller.state().slots[0].error_tracks, 1);

    controller
        .record_file_result(
            0,
            "failed",
            task.snapshot(),
            Some("conversion failed".into()),
        )
        .unwrap();
    assert_eq!(controller.state().slots[0].new_tracks, 3);
    assert_eq!(controller.state().slots[0].skipped_tracks, 2);
    assert_eq!(controller.state().slots[0].error_tracks, 2);
}

#[test]
fn existing_and_skipped_counts_remain_independent_for_overwrite_plans() {
    let mut controller = test_controller();
    controller.start_confirmed_sync(0, 3).unwrap();
    controller
        .set_preflight_summary(0, 3, 2, 0, 0, Some(1024))
        .unwrap();

    assert_eq!(controller.state().slots[0].new_tracks, 3);
    assert_eq!(controller.state().slots[0].existing_tracks, 2);
    assert_eq!(controller.state().slots[0].skipped_tracks, 0);
}

#[test]
fn failed_result_is_available_for_retry_without_increasing_completed_count() {
    let mut controller = test_controller();
    controller.start_confirmed_sync(0, 1).unwrap();
    let snapshot = controller.task_controller(0).unwrap().snapshot();
    controller
        .record_file_failed(
            0,
            FailedFile {
                name: "song".into(),
                source_path: "/in/song.flac".into(),
                destination_path: "/out/song.mp3".into(),
                message: "conversion failed".into(),
                category: Default::default(),
            },
            snapshot,
        )
        .unwrap();

    assert_eq!(controller.state().slots[0].progress_completed, 0);
    assert_eq!(controller.state().slots[0].failed_files.len(), 1);
}

fn test_controller() -> DesktopController {
    DesktopController::new(DesktopState::from_preferences(AppPreferences {
        slots: [
            SyncSlotPreferences::new("/music/in-1", "/music/out-1"),
            SyncSlotPreferences::new("/music/in-2", "/music/out-2"),
        ],
        mode: Mode::Compat,
        lossless_format: None,
        ..AppPreferences::default()
    }))
}
