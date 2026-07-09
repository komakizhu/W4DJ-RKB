### Task 3: Add shared task-state and pause semantics

**Files:**
- Modify: `/private/tmp/w4dj-review/src/sync.rs`
- Modify: `/private/tmp/w4dj-review/src/main.rs`
- Create: `/private/tmp/w4dj-review/src/task.rs`
- Test: `/private/tmp/w4dj-review/tests/task_state.rs`

**Interfaces:**
- Consumes: file queue and cancellation flag.
- Produces: progress state, pause-after-current-file behavior, and status snapshots for CLI and GUI.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn pause_waits_for_current_file() {
    let mut task = TaskState::running(3);
    task.request_pause();
    assert!(task.pause_after_current_file());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test pause_waits_for_current_file -v`
Expected: FAIL because `TaskState` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

```rust
pub struct TaskState {
    pub total: usize,
    pub completed: usize,
    pub paused: bool,
}

impl TaskState {
    pub fn running(total: usize) -> Self {
        Self { total, completed: 0, paused: false }
    }

    pub fn request_pause(&mut self) {
        self.paused = true;
    }

    pub fn pause_after_current_file(&self) -> bool {
        self.paused
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test pause_waits_for_current_file -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/task.rs src/sync.rs src/main.rs tests/task_state.rs
git commit -m "feat: add task state and pause semantics"
```

