#![allow(dead_code)]

#[path = "../src/config.rs"]
mod config;
#[path = "../src/metadata.rs"]
mod metadata;
#[path = "../src/sync.rs"]
mod sync;
#[path = "../src/task.rs"]
mod task;
#[path = "../src/main.rs"]
mod w4dj;

#[test]
fn gui_module_exposes_launcher() {
    assert!(w4dj::gui::launcher_available());
}
