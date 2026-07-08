use w4dj::config::{LosslessFormat, Mode};
use w4dj::desktop::{DesktopController, DesktopState, DesktopStatus};
use w4dj::preferences::AppPreferences;

#[test]
fn desktop_controller_starts_in_idle_state_with_saved_values() {
    let preferences = AppPreferences {
        source_directory: "/music/in".into(),
        destination_directory: "/music/out".into(),
        mode: Mode::Lossless,
        lossless_format: Some(LosslessFormat::Aiff),
    };

    let controller = DesktopController::new(DesktopState::from_preferences(preferences));

    assert_eq!(controller.state().source_directory, "/music/in");
    assert_eq!(controller.state().destination_directory, "/music/out");
    assert!(matches!(controller.state().mode, Mode::Lossless));
    assert!(matches!(controller.state().status, DesktopStatus::Idle));
    assert_eq!(controller.state().progress_total, 0);
    assert_eq!(controller.state().progress_completed, 0);
    assert_eq!(controller.state().current_file, "");
}
