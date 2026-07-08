use w4dj::config::{Config, LosslessFormat, Mode};
use w4dj::gui::{launch_shell, launcher_available, GuiView};
use w4dj::task::TaskController;

#[test]
fn gui_module_exposes_launcher() {
    assert!(launcher_available());
}

#[test]
fn gui_shell_tracks_basic_user_choices() {
    let config = Config {
        source: String::from("/music/source"),
        destination: String::from("/music/destination"),
        mode: Mode::Lossless,
        lossless_format: Some(LosslessFormat::Aiff),
    };
    let mut shell = launch_shell(&config);
    let task = TaskController::running(4);

    shell.pick_source_directory("/music/updated-source");
    shell.pick_destination_directory("/music/updated-destination");
    shell.choose_mode(Mode::Compat);
    shell.choose_lossless_format(Some(LosslessFormat::Flac));
    shell.start(&task);
    shell.pause(&task);

    assert_eq!(shell.source_directory, "/music/updated-source");
    assert_eq!(shell.destination_directory, "/music/updated-destination");
    assert!(matches!(shell.mode, Mode::Compat));
    assert!(matches!(shell.lossless_format, Some(LosslessFormat::Flac)));
    assert!(matches!(shell.view, GuiView::Paused));
    assert!(shell.task.paused);
    assert!(shell.log_lines.len() >= 6);
}
