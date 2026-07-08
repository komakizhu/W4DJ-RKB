use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskState {
    pub total: usize,
    pub completed: usize,
    pub paused: bool,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSnapshot {
    pub total: usize,
    pub completed: usize,
    pub paused: bool,
    pub cancelled: bool,
    pub remaining: usize,
}

#[derive(Debug, Clone)]
pub struct TaskController {
    state: Arc<Mutex<TaskState>>,
}

impl TaskState {
    pub fn running(total: usize) -> Self {
        Self {
            total,
            completed: 0,
            paused: false,
            cancelled: false,
        }
    }

    pub fn pause_after_current_file(&self) -> bool {
        self.paused
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn complete_current_file(&mut self) {
        if self.completed < self.total {
            self.completed += 1;
        }
    }

    pub fn should_start_next_file(&self) -> bool {
        !self.paused && !self.cancelled && self.completed < self.total
    }

    pub fn snapshot(&self) -> TaskSnapshot {
        TaskSnapshot {
            total: self.total,
            completed: self.completed,
            paused: self.paused,
            cancelled: self.cancelled,
            remaining: self.total.saturating_sub(self.completed),
        }
    }
}

impl TaskController {
    pub fn running(total: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(TaskState::running(total))),
        }
    }

    pub fn request_pause(&self) {
        self.with_state(|state| state.paused = true);
    }

    pub fn request_cancel(&self) {
        self.with_state(|state| state.cancelled = true);
    }

    pub fn should_start_next_file(&self) -> bool {
        self.with_state(|state| state.should_start_next_file())
    }

    pub fn pause_after_current_file(&self) -> bool {
        self.with_state(|state| state.pause_after_current_file())
    }

    pub fn is_cancelled(&self) -> bool {
        self.with_state(|state| state.is_cancelled())
    }

    pub fn complete_current_file(&self) {
        self.with_state(|state| state.complete_current_file());
    }

    pub fn snapshot(&self) -> TaskSnapshot {
        self.with_state(|state| state.snapshot())
    }

    fn with_state<T>(&self, action: impl FnOnce(&mut TaskState) -> T) -> T {
        let mut state = self.state.lock().expect("task state lock poisoned");
        action(&mut state)
    }
}
