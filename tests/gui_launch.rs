use w4dj::config::{Config, LosslessFormat, Mode};
use w4dj::gui::{GuiView, launch_shell, launcher_available};

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
        conflict_strategy: Default::default(),
        filename_rule: Default::default(),
    };
    let mut shell = launch_shell(&config);

    shell.pick_source_directory("/music/updated-source");
    shell.pick_destination_directory("/music/updated-destination");
    shell.choose_mode(Mode::Compat);
    shell.choose_lossless_format(Some(LosslessFormat::Aiff));
    shell.start(4);
    shell.pause();

    assert_eq!(shell.source_directory, "/music/updated-source");
    assert_eq!(shell.destination_directory, "/music/updated-destination");
    assert!(matches!(shell.mode, Mode::Compat));
    assert!(matches!(shell.lossless_format, Some(LosslessFormat::Aiff)));
    assert!(matches!(shell.view, GuiView::Paused));
    assert!(shell.task.paused);
    assert_eq!(shell.task.total, 4);
    assert!(shell.log_lines.len() >= 6);

    let summary = shell.status_summary();
    assert!(summary.contains("Paused"));
    assert!(summary.contains("/music/updated-source"));
}
