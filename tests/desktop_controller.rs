use w4dj::config::{LosslessFormat, Mode};
use w4dj::desktop::{DesktopController, DesktopState, DesktopStatus};
use w4dj::preferences::{AppPreferences, SyncSlotPreferences};

#[test]
fn desktop_controller_starts_in_idle_state_with_saved_values() {
    let preferences = AppPreferences {
        slots: [
            SyncSlotPreferences::new("/music/in-1", "/music/out-1"),
            SyncSlotPreferences::new("/music/in-2", "/music/out-2"),
        ],
        mode: Mode::Lossless,
        lossless_format: Some(LosslessFormat::Aiff),
    };

    let controller = DesktopController::new(DesktopState::from_preferences(preferences));

    assert_eq!(controller.state().slots[0].source_directory, "/music/in-1");
    assert_eq!(
        controller.state().slots[0].destination_directory,
        "/music/out-1"
    );
    assert_eq!(controller.state().slots[1].source_directory, "/music/in-2");
    assert_eq!(
        controller.state().slots[1].destination_directory,
        "/music/out-2"
    );
    assert!(matches!(controller.state().mode, Mode::Lossless));
    assert!(matches!(
        controller.state().slots[0].status,
        DesktopStatus::Idle
    ));
    assert!(matches!(
        controller.state().slots[1].status,
        DesktopStatus::Idle
    ));
    assert_eq!(controller.state().slots[0].progress_total, 0);
    assert_eq!(controller.state().slots[1].progress_completed, 0);
    assert_eq!(controller.state().slots[1].current_file, "");
}
