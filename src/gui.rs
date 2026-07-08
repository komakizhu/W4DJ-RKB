use crate::config::{Config, LosslessFormat, Mode};
use crate::task::{TaskController, TaskSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiShell {
    pub source_directory: String,
    pub destination_directory: String,
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub task: TaskSnapshot,
    pub log_lines: Vec<String>,
}

impl GuiShell {
    pub fn from_config(config: &Config, task: &TaskController) -> Self {
        Self {
            source_directory: config.source.clone(),
            destination_directory: config.destination.clone(),
            mode: config.mode,
            lossless_format: config.lossless_format,
            task: task.snapshot(),
            log_lines: Vec::new(),
        }
    }

    pub fn refresh_task(&mut self, task: &TaskController) {
        self.task = task.snapshot();
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log_lines.push(line.into());
    }
}

pub fn launcher_available() -> bool {
    true
}
