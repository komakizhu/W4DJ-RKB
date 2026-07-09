use w4dj::config::Mode;
use w4dj::desktop::{DesktopController, DesktopState, DesktopStatus};
use w4dj::preferences::AppPreferences;

#[test]
fn progress_updates_are_reflected_in_desktop_state() {
    let mut controller = test_controller();

    controller.start_sync(3);
    controller.record_file_started("track.wav");
    controller.complete_current_file();

    assert!(matches!(controller.state().status, DesktopStatus::Running));
    assert_eq!(controller.state().progress_total, 3);
    assert_eq!(controller.state().progress_completed, 1);
    assert_eq!(controller.state().current_file, "track.wav");
}

#[test]
fn pause_requests_wait_for_current_file() {
    let mut controller = test_controller();

    controller.start_sync(3);
    controller.pause_sync();

    assert!(matches!(controller.state().status, DesktopStatus::Paused));
    assert!(controller.pause_after_current_file());
    assert_eq!(controller.state().progress_total, 3);
    assert_eq!(controller.state().progress_completed, 0);
}

fn test_controller() -> DesktopController {
    DesktopController::new(DesktopState::from_preferences(AppPreferences {
        source_directory: "/music/in".into(),
        destination_directory: "/music/out".into(),
        mode: Mode::Compat,
        lossless_format: None,
    }))
}
