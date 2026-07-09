use crate::config::{Config, LosslessFormat, Mode};
use crate::task::{TaskController, TaskSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiView {
    Ready,
    Running,
    Paused,
}

#[derive(Debug, Clone)]
pub struct GuiShell {
    pub source_directory: String,
    pub destination_directory: String,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub task: TaskSnapshot,
    pub view: GuiView,
    pub log_lines: Vec<String>,
    task_controller: TaskController,
}

impl GuiShell {
    pub fn from_config(config: &Config, task: &TaskController) -> Self {
        Self {
            source_directory: config.source.clone(),
            destination_directory: config.destination.clone(),
            mode: config.mode,
            lossless_format: config.lossless_format,
            task: task.snapshot(),
            view: GuiView::Ready,
            log_lines: vec![String::from("GUI shell ready")],
            task_controller: task.clone(),
        }
    }

    pub fn pick_source_directory(&mut self, path: impl Into<String>) {
        self.source_directory = path.into();
        self.push_log("Source directory selected");
    }

    pub fn pick_destination_directory(&mut self, path: impl Into<String>) {
        self.destination_directory = path.into();
        self.push_log("Destination directory selected");
    }

    pub fn choose_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.push_log("Mode updated");
    }

    pub fn choose_lossless_format(&mut self, format: Option<LosslessFormat>) {
        self.lossless_format = format;
        self.push_log("Lossless format updated");
    }

    pub fn start(&mut self, total_files: usize) {
        self.task_controller = TaskController::running(total_files);
        self.refresh_task();
        self.view = GuiView::Running;
        self.push_log("Sync started");
    }

    pub fn pause(&mut self) {
        self.task_controller.request_pause();
        self.refresh_task();
        self.view = GuiView::Paused;
        self.push_log("Pause requested");
    }

    pub fn refresh_task(&mut self) {
        self.task = self.task_controller.snapshot();
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log_lines.push(line.into());
    }

    pub fn status_summary(&self) -> String {
        format!(
            "view={:?}, source='{}', destination='{}', mode={:?}, lossless_format={:?}, completed={}/{}, logs={}",
            self.view,
            self.source_directory,
            self.destination_directory,
            self.mode,
            self.lossless_format,
            self.task.completed,
            self.task.total,
            self.log_lines.len()
        )
    }
}

pub fn launch_shell(config: &Config) -> GuiShell {
    let task = TaskController::running(0);
    GuiShell::from_config(config, &task)
}

pub fn launcher_available() -> bool {
    true
}
