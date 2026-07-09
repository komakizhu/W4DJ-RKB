use crate::config::{LosslessFormat, Mode};
use crate::preferences::AppPreferences;
use crate::task::{TaskController, TaskSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopStatus {
    Idle,
    Running,
    Paused,
    Completed,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopState {
    pub source_directory: String,
    pub destination_directory: String,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub status: DesktopStatus,
    pub progress_total: usize,
    pub progress_completed: usize,
    pub current_file: String,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DesktopController {
    state: DesktopState,
    task_controller: TaskController,
}

impl DesktopState {
    pub fn from_preferences(preferences: AppPreferences) -> Self {
        Self {
            source_directory: preferences.source_directory,
            destination_directory: preferences.destination_directory,
            mode: preferences.mode,
            lossless_format: preferences.lossless_format,
            status: DesktopStatus::Idle,
            progress_total: 0,
            progress_completed: 0,
            current_file: String::new(),
            logs: vec![String::from("Desktop shell ready")],
        }
    }

    pub fn preferences(&self) -> AppPreferences {
        AppPreferences {
            source_directory: self.source_directory.clone(),
            destination_directory: self.destination_directory.clone(),
            mode: self.mode,
            lossless_format: self.lossless_format,
        }
    }
}

impl DesktopController {
    pub fn new(state: DesktopState) -> Self {
        Self {
            state,
            task_controller: TaskController::running(0),
        }
    }

    pub fn apply_preferences(&mut self, preferences: AppPreferences) {
        self.state.source_directory = preferences.source_directory;
        self.state.destination_directory = preferences.destination_directory;
        self.state.mode = preferences.mode;
        self.state.lossless_format = preferences.lossless_format;
    }

    pub fn state(&self) -> &DesktopState {
        &self.state
    }

    pub fn select_source_directory(&mut self, path: impl Into<String>) {
        self.state.source_directory = path.into();
        self.push_log("Source directory selected");
    }

    pub fn select_destination_directory(&mut self, path: impl Into<String>) {
        self.state.destination_directory = path.into();
        self.push_log("Destination directory selected");
    }

    pub fn choose_mode(&mut self, mode: Mode) {
        self.state.mode = mode;
        self.push_log("Mode updated");
    }

    pub fn choose_lossless_format(&mut self, format: Option<LosslessFormat>) {
        self.state.lossless_format = format;
        self.push_log("Lossless format updated");
    }

    pub fn start_sync(&mut self, total_files: usize) {
        self.task_controller = TaskController::running(total_files);
        self.state.status = DesktopStatus::Running;
        self.state.progress_total = total_files;
        self.state.progress_completed = 0;
        self.state.current_file.clear();
        self.push_log("Sync started");
    }

    pub fn task_controller(&self) -> TaskController {
        self.task_controller.clone()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state.status, DesktopStatus::Running)
    }

    pub fn set_progress_total(&mut self, total_files: usize) {
        self.task_controller.set_total(total_files);
        self.state.progress_total = total_files;
        self.state.progress_completed = 0;
    }

    pub fn pause_sync(&mut self) {
        self.task_controller.request_pause();
        self.state.status = DesktopStatus::Paused;
        self.push_log("Pause requested");
    }

    pub fn record_file_started(&mut self, file_name: impl Into<String>) {
        self.state.current_file = file_name.into();
        self.push_log(format!("Processing {}", self.state.current_file));
    }

    pub fn complete_current_file(&mut self) {
        self.task_controller.complete_current_file();
        let snapshot = self.task_controller.snapshot();
        self.state.progress_completed = snapshot.completed;

        if snapshot.completed >= snapshot.total && snapshot.total > 0 {
            self.state.status = DesktopStatus::Completed;
            self.push_log("Sync completed");
        }
    }

    pub fn record_file_completed(&mut self, file_name: impl Into<String>, snapshot: TaskSnapshot) {
        self.state.current_file = file_name.into();
        self.state.progress_completed = snapshot.completed;
        self.push_log(format!("Processed {}", self.state.current_file));
    }

    pub fn finish_sync(&mut self, snapshot: TaskSnapshot) {
        self.state.progress_total = snapshot.total;
        self.state.progress_completed = snapshot.completed;

        if snapshot.paused {
            self.state.status = DesktopStatus::Paused;
            self.push_log("Sync paused after current file");
        } else {
            self.state.status = DesktopStatus::Completed;
            self.push_log("Sync completed");
        }
    }

    pub fn fail_sync(&mut self, message: impl Into<String>) {
        self.state.status = DesktopStatus::Error;
        self.push_log(message);
    }

    pub fn pause_after_current_file(&self) -> bool {
        self.task_controller.pause_after_current_file()
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.state.logs.push(line.into());
    }
}
