use crate::config::{ConflictStrategy, FilenameRule, LosslessFormat, Mode};
use crate::history::FailedFile;
use crate::preferences::{AppPreferences, SYNC_SLOT_COUNT, SyncSlotPreferences};
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
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSlotState {
    pub source_directory: String,
    pub destination_directory: String,
    pub status: DesktopStatus,
    pub progress_total: usize,
    pub progress_completed: usize,
    pub new_tracks: usize,
    pub skipped_tracks: usize,
    pub existing_tracks: usize,
    pub error_tracks: usize,
    pub estimated_output_bytes: Option<u64>,
    pub failed_files: Vec<FailedFile>,
    pub current_file: String,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopState {
    pub slots: [SyncSlotState; SYNC_SLOT_COUNT],
    pub mode: Mode,
    pub lossless_format: Option<LosslessFormat>,
    pub conflict_strategy: ConflictStrategy,
    pub filename_rule: FilenameRule,
}

#[derive(Debug, Clone)]
pub struct DesktopController {
    state: DesktopState,
    task_controllers: [TaskController; SYNC_SLOT_COUNT],
}

impl SyncSlotState {
    fn from_preferences(preferences: SyncSlotPreferences) -> Self {
        Self {
            source_directory: preferences.source_directory,
            destination_directory: preferences.destination_directory,
            status: DesktopStatus::Idle,
            progress_total: 0,
            progress_completed: 0,
            new_tracks: 0,
            skipped_tracks: 0,
            existing_tracks: 0,
            error_tracks: 0,
            estimated_output_bytes: None,
            failed_files: Vec::new(),
            current_file: String::new(),
            logs: vec![String::from("Desktop shell ready")],
        }
    }
}

impl DesktopState {
    pub fn from_preferences(preferences: AppPreferences) -> Self {
        let AppPreferences {
            slots,
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
        } = preferences;

        Self {
            slots: slots.map(SyncSlotState::from_preferences),
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
        }
    }

    pub fn preferences(&self) -> AppPreferences {
        AppPreferences {
            slots: std::array::from_fn(|index| SyncSlotPreferences {
                source_directory: self.slots[index].source_directory.clone(),
                destination_directory: self.slots[index].destination_directory.clone(),
            }),
            mode: self.mode,
            lossless_format: self.lossless_format,
            conflict_strategy: self.conflict_strategy,
            filename_rule: self.filename_rule,
        }
    }
}

impl DesktopController {
    pub fn new(state: DesktopState) -> Self {
        Self {
            state,
            task_controllers: [TaskController::running(0), TaskController::running(0)],
        }
    }

    pub fn apply_preferences(&mut self, preferences: AppPreferences) {
        let AppPreferences {
            slots,
            mode,
            lossless_format,
            conflict_strategy,
            filename_rule,
        } = preferences;

        for (state_slot, preferences_slot) in self.state.slots.iter_mut().zip(slots) {
            state_slot.source_directory = preferences_slot.source_directory;
            state_slot.destination_directory = preferences_slot.destination_directory;
        }
        self.state.mode = mode;
        self.state.lossless_format = lossless_format;
        self.state.conflict_strategy = conflict_strategy;
        self.state.filename_rule = filename_rule;
    }

    pub fn state(&self) -> &DesktopState {
        &self.state
    }

    pub fn select_source_directory(
        &mut self,
        slot_index: usize,
        path: impl Into<String>,
    ) -> Result<(), String> {
        let slot = self.slot_mut(slot_index)?;
        slot.source_directory = path.into();
        slot.logs.push(String::from("Source directory selected"));
        Ok(())
    }

    pub fn select_destination_directory(
        &mut self,
        slot_index: usize,
        path: impl Into<String>,
    ) -> Result<(), String> {
        let slot = self.slot_mut(slot_index)?;
        slot.destination_directory = path.into();
        slot.logs
            .push(String::from("Destination directory selected"));
        Ok(())
    }

    pub fn choose_mode(&mut self, mode: Mode) {
        self.state.mode = mode;
        self.push_log_to_all("Mode updated");
    }

    pub fn choose_lossless_format(&mut self, format: Option<LosslessFormat>) {
        self.state.lossless_format = format;
        self.push_log_to_all("Lossless format updated");
    }

    pub fn choose_conflict_strategy(&mut self, strategy: ConflictStrategy) {
        self.state.conflict_strategy = strategy;
        self.push_log_to_all("Conflict strategy updated");
    }

    pub fn choose_filename_rule(&mut self, rule: FilenameRule) {
        self.state.filename_rule = rule;
        self.push_log_to_all("Filename rule updated");
    }

    pub fn start_sync(&mut self, slot_index: usize, total_files: usize) -> Result<(), String> {
        self.validate_slot_index(slot_index)?;
        self.task_controllers[slot_index] = TaskController::running(total_files);

        let slot = &mut self.state.slots[slot_index];
        slot.status = DesktopStatus::Running;
        slot.progress_total = total_files;
        slot.progress_completed = 0;
        slot.new_tracks = total_files;
        slot.skipped_tracks = 0;
        slot.existing_tracks = 0;
        slot.error_tracks = 0;
        slot.estimated_output_bytes = None;
        slot.failed_files.clear();
        slot.current_file.clear();
        slot.logs.clear();
        slot.logs.push(String::from("Sync started"));
        Ok(())
    }

    pub fn start_confirmed_sync(
        &mut self,
        slot_index: usize,
        total_files: usize,
    ) -> Result<(), String> {
        self.start_sync(slot_index, total_files)
    }

    pub fn set_preflight_summary(
        &mut self,
        slot_index: usize,
        new_tracks: usize,
        existing_tracks: usize,
        skipped_tracks: usize,
        error_tracks: usize,
        estimated_output_bytes: Option<u64>,
    ) -> Result<(), String> {
        let slot = self.slot_mut(slot_index)?;
        slot.new_tracks = new_tracks;
        slot.existing_tracks = existing_tracks;
        slot.skipped_tracks = skipped_tracks;
        slot.error_tracks = error_tracks;
        slot.estimated_output_bytes = estimated_output_bytes;
        Ok(())
    }

    pub fn task_controller(&self, slot_index: usize) -> Result<TaskController, String> {
        self.validate_slot_index(slot_index)?;
        Ok(self.task_controllers[slot_index].clone())
    }

    pub fn is_running(&self, slot_index: usize) -> Result<bool, String> {
        Ok(matches!(
            self.slot(slot_index)?.status,
            DesktopStatus::Running
        ))
    }

    pub fn startable_slot_indexes(&self) -> Vec<usize> {
        self.state
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot_index, slot)| {
                (!slot.source_directory.trim().is_empty()
                    && !matches!(slot.status, DesktopStatus::Running))
                .then_some(slot_index)
            })
            .collect()
    }

    pub fn set_progress_total(
        &mut self,
        slot_index: usize,
        total_files: usize,
    ) -> Result<(), String> {
        let task_controller = self.task_controller(slot_index)?;
        task_controller.set_total(total_files);

        let slot = self.slot_mut(slot_index)?;
        slot.progress_total = total_files;
        slot.progress_completed = 0;
        slot.new_tracks = total_files;
        Ok(())
    }

    pub fn pause_sync(&mut self, slot_index: usize) -> Result<(), String> {
        let task_controller = self.task_controller(slot_index)?;
        task_controller.request_pause();

        let slot = self.slot_mut(slot_index)?;
        slot.logs.push(String::from(
            "Pause requested; waiting for the current song to finish",
        ));
        Ok(())
    }

    pub fn pause_all_running(&mut self) -> Result<(), String> {
        let running_slots: Vec<usize> = self
            .state
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot_index, slot)| {
                matches!(slot.status, DesktopStatus::Running).then_some(slot_index)
            })
            .collect();

        for slot_index in running_slots {
            self.pause_sync(slot_index)?;
        }

        Ok(())
    }

    pub fn cancel_sync(&mut self, slot_index: usize) -> Result<(), String> {
        let task_controller = self.task_controller(slot_index)?;
        task_controller.request_cancel();

        let slot = self.slot_mut(slot_index)?;
        slot.logs.push(String::from(
            "Cancel requested; the current song will finish safely before stopping",
        ));
        Ok(())
    }

    pub fn cancel_all_running(&mut self) -> Result<(), String> {
        let running_slots: Vec<usize> = self
            .state
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot_index, slot)| {
                matches!(slot.status, DesktopStatus::Running).then_some(slot_index)
            })
            .collect();

        for slot_index in running_slots {
            self.cancel_sync(slot_index)?;
        }

        Ok(())
    }

    pub fn record_file_started(
        &mut self,
        slot_index: usize,
        file_name: impl Into<String>,
    ) -> Result<(), String> {
        let file_name = file_name.into();
        let slot = self.slot_mut(slot_index)?;
        slot.current_file = file_name.clone();
        slot.logs.push(format!("Processing {file_name}"));
        Ok(())
    }

    pub fn complete_current_file(&mut self, slot_index: usize) -> Result<(), String> {
        let task_controller = self.task_controller(slot_index)?;
        task_controller.complete_current_file();
        let snapshot = task_controller.snapshot();

        let slot = self.slot_mut(slot_index)?;
        slot.progress_completed = snapshot.completed;
        if snapshot.completed >= snapshot.total && snapshot.total > 0 {
            slot.status = DesktopStatus::Completed;
            slot.logs.push(String::from("Sync completed"));
        }
        Ok(())
    }

    pub fn record_file_completed(
        &mut self,
        slot_index: usize,
        file_name: impl Into<String>,
        snapshot: TaskSnapshot,
    ) -> Result<(), String> {
        let file_name = file_name.into();
        let slot = self.slot_mut(slot_index)?;
        slot.current_file = file_name.clone();
        slot.progress_completed = snapshot.completed;
        slot.logs.push(format!("Processed {file_name}"));
        Ok(())
    }

    pub fn record_file_result(
        &mut self,
        slot_index: usize,
        file_name: impl Into<String>,
        snapshot: TaskSnapshot,
        error: Option<String>,
    ) -> Result<(), String> {
        let file_name = file_name.into();
        let slot = self.slot_mut(slot_index)?;
        slot.current_file = file_name.clone();
        slot.progress_completed = snapshot.completed;

        match error {
            Some(error) => {
                slot.error_tracks += 1;
                slot.logs.push(format!("Failed {file_name}: {error}"));
            }
            None => {
                slot.logs.push(format!("Processed {file_name}"));
            }
        }

        Ok(())
    }

    pub fn record_file_failed(
        &mut self,
        slot_index: usize,
        failed_file: FailedFile,
        snapshot: TaskSnapshot,
    ) -> Result<(), String> {
        let slot = self.slot_mut(slot_index)?;
        slot.current_file = failed_file.name.clone();
        slot.progress_completed = snapshot.completed;
        slot.error_tracks += 1;
        slot.failed_files.push(failed_file.clone());
        slot.logs.push(format!(
            "Failed {}: {}",
            failed_file.name, failed_file.message
        ));
        Ok(())
    }

    pub fn finish_sync(&mut self, slot_index: usize, snapshot: TaskSnapshot) -> Result<(), String> {
        let slot = self.slot_mut(slot_index)?;
        slot.progress_total = snapshot.total;
        slot.progress_completed = snapshot.completed;

        if snapshot.cancelled {
            slot.status = DesktopStatus::Cancelled;
            slot.logs.push(String::from(
                "Sync cancelled; unfinished songs can be resumed later",
            ));
        } else if snapshot.paused {
            slot.status = DesktopStatus::Paused;
            slot.logs
                .push(String::from("Sync paused after current file"));
        } else if !slot.failed_files.is_empty() {
            slot.status = DesktopStatus::Error;
            slot.logs.push(String::from("Sync completed with errors"));
        } else {
            slot.status = DesktopStatus::Completed;
            slot.logs.push(String::from("Sync completed"));
        }
        Ok(())
    }

    pub fn fail_sync(
        &mut self,
        slot_index: usize,
        message: impl Into<String>,
    ) -> Result<(), String> {
        let slot = self.slot_mut(slot_index)?;
        slot.status = DesktopStatus::Error;
        slot.logs.push(message.into());
        Ok(())
    }

    pub fn pause_after_current_file(&self, slot_index: usize) -> Result<bool, String> {
        Ok(self.task_controller(slot_index)?.pause_after_current_file())
    }

    pub fn push_log(&mut self, slot_index: usize, line: impl Into<String>) -> Result<(), String> {
        self.slot_mut(slot_index)?.logs.push(line.into());
        Ok(())
    }

    pub fn effective_destination(&self, slot_index: usize) -> Result<Option<String>, String> {
        let configured = &self.slot(slot_index)?.destination_directory;
        if !configured.trim().is_empty() {
            return Ok(Some(configured.clone()));
        }

        if slot_index == 1 {
            let fallback = &self.state.slots[0].destination_directory;
            if !fallback.trim().is_empty() {
                return Ok(Some(fallback.clone()));
            }
        }

        Ok(None)
    }

    fn slot(&self, slot_index: usize) -> Result<&SyncSlotState, String> {
        self.state
            .slots
            .get(slot_index)
            .ok_or_else(|| invalid_slot_index(slot_index))
    }

    fn slot_mut(&mut self, slot_index: usize) -> Result<&mut SyncSlotState, String> {
        self.state
            .slots
            .get_mut(slot_index)
            .ok_or_else(|| invalid_slot_index(slot_index))
    }

    fn validate_slot_index(&self, slot_index: usize) -> Result<(), String> {
        self.slot(slot_index).map(|_| ())
    }

    fn push_log_to_all(&mut self, line: &str) {
        for slot in &mut self.state.slots {
            slot.logs.push(line.to_string());
        }
    }
}

fn invalid_slot_index(slot_index: usize) -> String {
    format!("Invalid sync slot index: {slot_index}")
}
