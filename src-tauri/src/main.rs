#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::Manager;
use w4dj::config::{LosslessFormat, Mode};
use w4dj::desktop::{DesktopController, DesktopState};
use w4dj::preferences::{AppPreferences, load_preferences, save_preferences};
use w4dj::sync::{compare_music_dicts, get_music_dict, sync_music_library_with_observer};

#[cfg(target_os = "macos")]
use window_vibrancy::{NSVisualEffectMaterial, NSVisualEffectState, apply_vibrancy};

struct AppState {
    controller: Arc<Mutex<DesktopController>>,
    preferences_path: Arc<Mutex<PathBuf>>,
    destination_coordinator: DestinationCoordinator,
}

#[derive(Clone, Default)]
struct DestinationCoordinator {
    locks: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>,
}

impl DestinationCoordinator {
    fn lock_for(&self, destination: &Path) -> Arc<Mutex<()>> {
        let key = fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
        let mut locks = self.locks.lock().expect("destination lock map poisoned");
        Arc::clone(
            locks
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }
}

#[tauri::command]
fn load_desktop_state(state: tauri::State<'_, AppState>) -> DesktopState {
    state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone()
}

#[tauri::command]
fn select_source_directory(
    slot_index: usize,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.select_source_directory(slot_index, path)?;
        controller.state().clone()
    };
    persist_preferences(&state);
    Ok(snapshot)
}

#[tauri::command]
fn select_destination_directory(
    slot_index: usize,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.select_destination_directory(slot_index, path)?;
        controller.state().clone()
    };
    persist_preferences(&state);
    Ok(snapshot)
}

#[tauri::command]
fn choose_mode(mode: Mode, state: tauri::State<'_, AppState>) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_mode(mode);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn choose_lossless_format(
    format: Option<LosslessFormat>,
    state: tauri::State<'_, AppState>,
) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.choose_lossless_format(format);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn start_sync(
    slot_index: usize,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let controller = Arc::clone(&state.controller);
    let destination_coordinator = state.destination_coordinator.clone();
    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        if controller.is_running(slot_index)? {
            return Ok(controller.state().clone());
        }

        controller.start_sync(slot_index, 0)?;
        controller.push_log(slot_index, "Scanning folders")?;
    }

    thread::spawn(move || run_sync_task(controller, destination_coordinator, slot_index));

    Ok(state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone())
}

#[tauri::command]
fn pause_sync(
    slot_index: usize,
    state: tauri::State<'_, AppState>,
) -> Result<DesktopState, String> {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.pause_sync(slot_index)?;
    Ok(controller.state().clone())
}

#[tauri::command]
fn start_all_sync(state: tauri::State<'_, AppState>) -> Result<DesktopState, String> {
    let controller = Arc::clone(&state.controller);
    let destination_coordinator = state.destination_coordinator.clone();
    let slot_indexes = {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        let slot_indexes = controller.startable_slot_indexes();

        if slot_indexes.is_empty() {
            if controller.state().slots.iter().any(|slot| {
                !slot.source_directory.trim().is_empty()
                    && matches!(slot.status, w4dj::desktop::DesktopStatus::Running)
            }) {
                return Ok(controller.state().clone());
            }
            return Err(String::from("请至少选择一个歌曲下载目录"));
        }

        for &slot_index in &slot_indexes {
            controller.start_sync(slot_index, 0)?;
            controller.push_log(slot_index, "Scanning folders")?;
        }

        slot_indexes
    };

    for slot_index in slot_indexes {
        let controller = Arc::clone(&controller);
        let destination_coordinator = destination_coordinator.clone();
        thread::spawn(move || run_sync_task(controller, destination_coordinator, slot_index));
    }

    Ok(state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone())
}

#[tauri::command]
fn pause_all_sync(state: tauri::State<'_, AppState>) -> Result<DesktopState, String> {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.pause_all_running()?;
    Ok(controller.state().clone())
}

fn main() {
    let controller = DesktopController::new(DesktopState::from_preferences(AppPreferences::default()));

    tauri::Builder::default()
        .manage(AppState {
            controller: Arc::new(Mutex::new(controller)),
            preferences_path: Arc::new(Mutex::new(PathBuf::new())),
            destination_coordinator: DestinationCoordinator::default(),
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_desktop_state,
            select_source_directory,
            select_destination_directory,
            choose_mode,
            choose_lossless_format,
            start_sync,
            pause_sync,
            start_all_sync,
            pause_all_sync
        ])
        .setup(|app| {
            let preferences_path = app
                .path()
                .app_config_dir()
                .expect("failed to resolve app config directory")
                .join("preferences.json");

            {
                let state = app.state::<AppState>();
                let mut path_guard = state
                    .preferences_path
                    .lock()
                    .expect("preferences path lock poisoned");
                *path_guard = preferences_path.clone();
            }

            {
                let preferences = load_preferences(&preferences_path)
                    .unwrap_or_else(|_| AppPreferences::default());
                let state = app.state::<AppState>();
                let mut controller = state
                    .controller
                    .lock()
                    .expect("desktop lock poisoned");
                controller.apply_preferences(preferences);
            }

            #[cfg(target_os = "macos")]
            {
                let window = app
                    .get_webview_window("main")
                    .expect("main window should exist");

                apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::HudWindow,
                    Some(NSVisualEffectState::Active),
                    Some(18.0),
                )
                .expect("failed to apply macOS vibrancy");

                window.center().expect("failed to center main window");
                window.show().expect("failed to show main window");
                window.set_focus().expect("failed to focus main window");
            }

            #[cfg(not(target_os = "macos"))]
            {
                let window = app
                    .get_webview_window("main")
                    .expect("main window should exist");

                window.center().expect("failed to center main window");
                window.show().expect("failed to show main window");
                window.set_focus().expect("failed to focus main window");
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run W4DJ desktop shell");
}

fn persist_preferences(state: &tauri::State<'_, AppState>) {
    let preferences = {
        let controller = state.controller.lock().expect("desktop lock poisoned");
        controller.state().preferences()
    };

    let preferences_path = state
        .preferences_path
        .lock()
        .expect("preferences path lock poisoned")
        .clone();

    if preferences_path.as_os_str().is_empty() {
        return;
    }

    if let Err(error) = save_preferences(&preferences_path, &preferences) {
        eprintln!("Failed to save preferences: {}", error);
    }
}

fn run_sync_task(
    controller: Arc<Mutex<DesktopController>>,
    destination_coordinator: DestinationCoordinator,
    slot_index: usize,
) {
    let (source, destination, using_fallback, mode, lossless_format, task_controller) = {
        let controller = controller.lock().expect("desktop lock poisoned");
        let state = controller.state();
        let slot = &state.slots[slot_index];
        let destination = controller
            .effective_destination(slot_index)
            .expect("sync slot index validated before worker start")
            .unwrap_or_default();
        (
            slot.source_directory.clone(),
            destination.clone(),
            slot_index == 1
                && slot.destination_directory.trim().is_empty()
                && !destination.trim().is_empty(),
            state.mode,
            state.lossless_format,
            controller
                .task_controller(slot_index)
                .expect("sync slot index validated before worker start"),
        )
    };

    if source.trim().is_empty() {
        fail_sync(&controller, slot_index, "请选择歌曲下载目录");
        return;
    }

    if destination.trim().is_empty() {
        fail_sync(&controller, slot_index, "请选择输出目录");
        return;
    }

    if !Path::new(&source).exists() {
        fail_sync(
            &controller,
            slot_index,
            format!("歌曲下载目录不存在：{}", source),
        );
        return;
    }

    if let Err(error) = fs::create_dir_all(&destination) {
        fail_sync(
            &controller,
            slot_index,
            format!("无法创建输出目录：{}", error),
        );
        return;
    }

    let destination_lock = destination_coordinator.lock_for(Path::new(&destination));
    let _destination_guard = destination_lock
        .lock()
        .expect("destination sync lock poisoned");

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        if using_fallback {
            controller
                .push_log(
                    slot_index,
                    format!("Using output directory 1 fallback: {}", destination),
                )
                .expect("sync slot index validated before worker start");
        }
        controller
            .push_log(slot_index, format!("Scanning source: {}", source))
            .expect("sync slot index validated before worker start");
    }
    let source_files = get_music_dict(&source);

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller
            .push_log(
                slot_index,
                format!("Scanning destination: {}", destination),
            )
            .expect("sync slot index validated before worker start");
    }
    let destination_files = get_music_dict(&destination);
    let queued_files = compare_music_dicts(&source_files, &destination_files, &mode, lossless_format);

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller
            .set_progress_total(slot_index, queued_files.len())
            .expect("sync slot index validated before worker start");
        controller
            .push_log(
                slot_index,
                format!("Found {} songs to sync", queued_files.len()),
            )
            .expect("sync slot index validated before worker start");

        if queued_files.is_empty() {
            controller
                .finish_sync(slot_index, task_controller.snapshot())
                .expect("sync slot index validated before worker start");
            return;
        }
    }

    let result = sync_music_library_with_observer(
        &queued_files,
        &destination,
        &mode,
        lossless_format,
        &task_controller,
        |name, task| {
            let mut controller = controller.lock().expect("desktop lock poisoned");
            controller
                .record_file_completed(slot_index, name, task.snapshot())
                .expect("sync slot index validated before worker start");
        },
    );

    let mut controller = controller.lock().expect("desktop lock poisoned");
    match result {
        Ok(snapshot) => controller
            .finish_sync(slot_index, snapshot)
            .expect("sync slot index validated before worker start"),
        Err(error) => controller
            .fail_sync(slot_index, format!("导出失败：{}", error))
            .expect("sync slot index validated before worker start"),
    }
}

fn fail_sync(
    controller: &Arc<Mutex<DesktopController>>,
    slot_index: usize,
    message: impl Into<String>,
) {
    let mut controller = controller.lock().expect("desktop lock poisoned");
    controller
        .fail_sync(slot_index, message)
        .expect("sync slot index validated before worker start");
}

#[cfg(test)]
mod tests {
    use super::DestinationCoordinator;
    use std::path::Path;
    use std::sync::Arc;

    #[test]
    fn coordinator_reuses_a_lock_for_the_same_destination() {
        let coordinator = DestinationCoordinator::default();

        let first = coordinator.lock_for(Path::new("/music/output-a"));
        let second = coordinator.lock_for(Path::new("/music/output-a"));
        let other = coordinator.lock_for(Path::new("/music/output-b"));

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));
    }
}
