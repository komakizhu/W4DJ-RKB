use std::fs;
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
fn select_source_directory(path: String, state: tauri::State<'_, AppState>) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.select_source_directory(path);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
}

#[tauri::command]
fn select_destination_directory(path: String, state: tauri::State<'_, AppState>) -> DesktopState {
    let snapshot = {
        let mut controller = state.controller.lock().expect("desktop lock poisoned");
        controller.select_destination_directory(path);
        controller.state().clone()
    };
    persist_preferences(&state);
    snapshot
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
fn start_sync(total_files: usize, state: tauri::State<'_, AppState>) -> DesktopState {
    let controller = Arc::clone(&state.controller);
    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        if controller.is_running() {
            return controller.state().clone();
        }

        controller.start_sync(total_files);
        controller.push_log("Scanning folders");
    }

    thread::spawn(move || run_sync_task(controller));

    state
        .controller
        .lock()
        .expect("desktop lock poisoned")
        .state()
        .clone()
}

#[tauri::command]
fn pause_sync(state: tauri::State<'_, AppState>) -> DesktopState {
    let mut controller = state.controller.lock().expect("desktop lock poisoned");
    controller.pause_sync();
    controller.state().clone()
}

fn main() {
    let controller = DesktopController::new(DesktopState::from_preferences(AppPreferences::default()));

    tauri::Builder::default()
        .manage(AppState {
            controller: Arc::new(Mutex::new(controller)),
            preferences_path: Arc::new(Mutex::new(PathBuf::new())),
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_desktop_state,
            select_source_directory,
            select_destination_directory,
            choose_mode,
            choose_lossless_format,
            start_sync,
            pause_sync
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

fn run_sync_task(controller: Arc<Mutex<DesktopController>>) {
    let (source, destination, mode, lossless_format, task_controller) = {
        let controller = controller.lock().expect("desktop lock poisoned");
        let state = controller.state();
        (
            state.source_directory.clone(),
            state.destination_directory.clone(),
            state.mode,
            state.lossless_format,
            controller.task_controller(),
        )
    };

    if source.trim().is_empty() {
        fail_sync(&controller, "请选择原始目录");
        return;
    }

    if destination.trim().is_empty() {
        fail_sync(&controller, "请选择输出目录");
        return;
    }

    if !Path::new(&source).exists() {
        fail_sync(&controller, format!("原始目录不存在：{}", source));
        return;
    }

    if let Err(error) = fs::create_dir_all(&destination) {
        fail_sync(&controller, format!("无法创建输出目录：{}", error));
        return;
    }

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller.push_log(format!("Scanning source: {}", source));
    }
    let source_files = get_music_dict(&source);

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller.push_log(format!("Scanning destination: {}", destination));
    }
    let destination_files = get_music_dict(&destination);
    let queued_files = compare_music_dicts(&source_files, &destination_files, &mode, lossless_format);

    {
        let mut controller = controller.lock().expect("desktop lock poisoned");
        controller.set_progress_total(queued_files.len());
        controller.push_log(format!("Found {} songs to sync", queued_files.len()));

        if queued_files.is_empty() {
            controller.finish_sync(task_controller.snapshot());
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
            controller.record_file_completed(name, task.snapshot());
        },
    );

    let mut controller = controller.lock().expect("desktop lock poisoned");
    match result {
        Ok(snapshot) => controller.finish_sync(snapshot),
        Err(error) => controller.fail_sync(format!("导出失败：{}", error)),
    }
}

fn fail_sync(controller: &Arc<Mutex<DesktopController>>, message: impl Into<String>) {
    let mut controller = controller.lock().expect("desktop lock poisoned");
    controller.fail_sync(message);
}
