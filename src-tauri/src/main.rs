use std::sync::Mutex;
use w4dj::config::{LosslessFormat, Mode};
use w4dj::desktop::{DesktopController, DesktopState};
use w4dj::preferences::AppPreferences;

struct AppState {
    controller: Mutex<DesktopController>,
}

#[tauri::command]
fn load_desktop_state(state: tauri::State<'_, AppState>) -> DesktopState {
    state.controller.lock().expect("desktop lock poisoned").state().clone()
}

#[tauri::command]
fn select_source_directory(path: String, state: tauri::State<'_, AppState>) -> DesktopState {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.select_source_directory(path);
    controller.state().clone()
}

#[tauri::command]
fn select_destination_directory(path: String, state: tauri::State<'_, AppState>) -> DesktopState {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.select_destination_directory(path);
    controller.state().clone()
}

#[tauri::command]
fn choose_mode(mode: Mode, state: tauri::State<'_, AppState>) -> DesktopState {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.choose_mode(mode);
    controller.state().clone()
}

#[tauri::command]
fn choose_lossless_format(
    format: Option<LosslessFormat>,
    state: tauri::State<'_, AppState>,
) -> DesktopState {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.choose_lossless_format(format);
    controller.state().clone()
}

#[tauri::command]
fn start_sync(total_files: usize, state: tauri::State<'_, AppState>) -> DesktopState {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.start_sync(total_files);
    controller.state().clone()
}

#[tauri::command]
fn pause_sync(state: tauri::State<'_, AppState>) -> DesktopState {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.pause_sync();
    controller.state().clone()
}

fn main() {
    let preferences = AppPreferences::default();
    let controller = DesktopController::new(DesktopState::from_preferences(preferences));

    tauri::Builder::default()
        .manage(AppState {
            controller: Mutex::new(controller),
        })
        .invoke_handler(tauri::generate_handler![
            load_desktop_state,
            select_source_directory,
            select_destination_directory,
            choose_mode,
            choose_lossless_format,
            start_sync,
            pause_sync
        ])
        .run(tauri::generate_context!())
        .expect("failed to run W4DJ desktop shell");
}
