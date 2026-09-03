//! A small, local-only journal for diagnostics and run reports.
//!
//! The conversion path never waits for a file write.  Events are put on a
//! bounded channel with `try_send`; a dedicated writer owns all file I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

const QUEUE_CAPACITY: usize = 2048;
const COMPENSATION_CAPACITY: usize = 1000;
const RETENTION_DAYS: u64 = 30;
const MAX_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeJournalEvent {
    pub timestamp: String,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    pub details: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeJournalHealth {
    pub queued: u64,
    pub written: u64,
    pub dropped: u64,
    pub merged_progress: u64,
    pub write_errors: u64,
    pub compensation_count: u64,
    pub previous_run_interrupted: bool,
}

enum JournalCommand {
    Event(RuntimeJournalEvent),
    Flush(SyncSender<()>),
    Shutdown,
}

struct JournalInner {
    root: PathBuf,
    sender: SyncSender<JournalCommand>,
    health: Mutex<RuntimeJournalHealth>,
    compensation: Mutex<VecDeque<RuntimeJournalEvent>>,
    merged_progress: Mutex<Option<RuntimeJournalEvent>>,
    stopped: AtomicBool,
    writer: Mutex<Option<JoinHandle<()>>>,
}

/// Non-blocking structured journal.  Cloning the value is cheap and safe to
/// pass to conversion, scan and analysis worker threads.
#[derive(Clone)]
pub struct RuntimeJournal {
    inner: Arc<JournalInner>,
}

impl RuntimeJournal {
    pub fn start(root: impl AsRef<Path>) -> io::Result<(Self, bool)> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        rotate_logs(&root);
        let marker = root.join("active.json");
        let previous_interrupted = marker.exists();
        let marker_payload = serde_json::json!({
            "startedAt": timestamp_string(),
            "pid": std::process::id(),
        });
        fs::write(&marker, serde_json::to_vec(&marker_payload)?)?;

        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let inner = Arc::new(JournalInner {
            root: root.clone(),
            sender,
            health: Mutex::new(RuntimeJournalHealth {
                previous_run_interrupted: previous_interrupted,
                ..RuntimeJournalHealth::default()
            }),
            compensation: Mutex::new(VecDeque::with_capacity(COMPENSATION_CAPACITY)),
            merged_progress: Mutex::new(None),
            stopped: AtomicBool::new(false),
            writer: Mutex::new(None),
        });
        let writer_inner = Arc::clone(&inner);
        let handle = thread::Builder::new()
            .name("w4dj-runtime-journal".to_string())
            .spawn(move || writer_loop(writer_inner, receiver))
            .map_err(|error| io::Error::other(format!("启动运行日志线程失败：{error}")))?;
        *inner.writer.lock().expect("journal writer lock poisoned") = Some(handle);
        Ok((Self { inner }, previous_interrupted))
    }

    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        Self::start(root).map(|(journal, _)| journal)
    }

    pub fn record(&self, mut event: RuntimeJournalEvent) {
        if event.timestamp.is_empty() {
            event.timestamp = timestamp_string();
        }
        if self.inner.stopped.load(Ordering::Relaxed) {
            self.store_compensation(event);
            return;
        }
        match self
            .inner
            .sender
            .try_send(JournalCommand::Event(event.clone()))
        {
            Ok(()) => {
                if let Ok(mut health) = self.inner.health.lock() {
                    health.queued += 1;
                }
            }
            Err(TrySendError::Full(_)) => {
                let is_progress = event.event.contains("progress")
                    || event
                        .stage
                        .as_deref()
                        .is_some_and(|stage| stage == "progress");
                if let Ok(mut health) = self.inner.health.lock() {
                    if is_progress {
                        health.merged_progress += 1;
                    } else {
                        health.dropped += 1;
                    }
                }
                if is_progress {
                    if let Ok(mut latest) = self.inner.merged_progress.lock() {
                        *latest = Some(event);
                    }
                } else {
                    self.store_compensation(event);
                }
            }
            Err(TrySendError::Disconnected(_)) => {
                self.store_compensation(event);
            }
        }
    }

    pub fn record_event(
        &self,
        event: impl Into<String>,
        operation_id: Option<String>,
        stage: Option<String>,
        status: Option<String>,
        details: Value,
        error: Option<String>,
    ) {
        self.record(RuntimeJournalEvent {
            timestamp: timestamp_string(),
            event: event.into(),
            operation_id,
            stage,
            status,
            details,
            error,
        });
    }

    pub fn health(&self) -> RuntimeJournalHealth {
        self.inner
            .health
            .lock()
            .map(|health| health.clone())
            .unwrap_or_default()
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn compensation_snapshot(&self) -> Vec<RuntimeJournalEvent> {
        self.inner
            .compensation
            .lock()
            .map(|events| events.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn events_snapshot(&self) -> Vec<RuntimeJournalEvent> {
        self.flush();
        let mut events = Vec::new();
        let Ok(entries) = fs::read_dir(&self.inner.root) else {
            return events;
        };
        let mut files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            let Ok(file) = File::open(path) else { continue };
            for line in BufReader::new(file).lines().map_while(Result::ok) {
                if let Ok(event) = serde_json::from_str::<RuntimeJournalEvent>(&line) {
                    events.push(event);
                }
            }
        }
        if let Some(progress) = self
            .inner
            .merged_progress
            .lock()
            .ok()
            .and_then(|progress| progress.clone())
        {
            events.push(progress);
        }
        events.extend(self.compensation_snapshot());
        events
    }

    /// Export a consistent JSON snapshot through a temporary file and an
    /// atomic rename.  Events are streamed line by line instead of loading a
    /// whole 200 MB journal into memory.
    pub fn export_full_report(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.export_full_report_with_diagnostics(path, None)
    }

    /// Export the journal together with an optional point-in-time diagnostic
    /// snapshot. The diagnostic value is supplied by the desktop layer so
    /// this generic journal stays independent of W4DJ's database schema.
    pub fn export_full_report_with_diagnostics(
        &self,
        path: impl AsRef<Path>,
        diagnostics: Option<&Value>,
    ) -> io::Result<()> {
        self.flush();
        let path = path.as_ref();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
        let result = (|| {
            let mut file = File::create(&temporary)?;
            file.write_all(b"{\"schemaVersion\":1,\"reportType\":\"fullRuntime\",")?;
            let exported_at = serde_json::to_string(&timestamp_string())?;
            write!(file, "\"exportedAt\":{exported_at},")?;
            file.write_all(b"\"metadata\":")?;
            serde_json::to_writer(
                &mut file,
                &serde_json::json!({
                    "journalRoot": self.root(),
                    "health": self.health(),
                    "retentionDays": RETENTION_DAYS,
                    "maxBytes": MAX_BYTES,
                }),
            )?;
            if let Some(diagnostics) = diagnostics {
                file.write_all(b",\"diagnostics\":")?;
                serde_json::to_writer(&mut file, diagnostics)?;
            }
            file.write_all(b",\"events\":[")?;
            let mut first = true;
            let mut files = fs::read_dir(&self.inner.root)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|candidate| {
                    candidate.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
                })
                .collect::<Vec<_>>();
            files.sort();
            for event_file in files {
                let input = match File::open(&event_file) {
                    Ok(input) => input,
                    Err(_) => continue,
                };
                for line in BufReader::new(input).lines().map_while(Result::ok) {
                    if serde_json::from_str::<Value>(&line).is_err() {
                        continue;
                    }
                    if !first {
                        file.write_all(b",")?;
                    }
                    first = false;
                    file.write_all(line.as_bytes())?;
                }
            }
            for event in self.compensation_snapshot() {
                if !first {
                    file.write_all(b",")?;
                }
                first = false;
                serde_json::to_writer(&mut file, &event)?;
            }
            if let Some(progress) = self
                .inner
                .merged_progress
                .lock()
                .ok()
                .and_then(|progress| progress.clone())
            {
                if !first {
                    file.write_all(b",")?;
                }
                serde_json::to_writer(&mut file, &progress)?;
            }
            file.write_all(b"]}\n")?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok::<(), io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn mark_clean_shutdown(&self) {
        if self.inner.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        // Shutdown is a control message rather than a business event.  Use a
        // blocking send here so a saturated bounded queue can never leave the
        // writer thread running forever while Drop waits to join it.
        let _ = self.inner.sender.send(JournalCommand::Shutdown);
        if let Ok(mut writer) = self.inner.writer.lock()
            && let Some(handle) = writer.take()
        {
            let _ = handle.join();
        }
        let _ = fs::remove_file(self.inner.root.join("active.json"));
    }

    /// Wait until all events queued before this call have reached the writer.
    /// This is used only by report export (never by conversion/scan workers),
    /// so the diagnostic path can provide a consistent up-to-date snapshot
    /// without making business operations wait for disk I/O.
    fn flush(&self) {
        if self.inner.stopped.load(Ordering::Relaxed) {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(0);
        if self
            .inner
            .sender
            .send(JournalCommand::Flush(sender))
            .is_ok()
        {
            let _ = receiver.recv();
        }
    }

    fn store_compensation(&self, event: RuntimeJournalEvent) {
        if let Ok(mut events) = self.inner.compensation.lock() {
            if events.len() == COMPENSATION_CAPACITY {
                events.pop_front();
            }
            events.push_back(event);
            if let Ok(mut health) = self.inner.health.lock() {
                health.compensation_count = events.len() as u64;
            }
        }
    }
}

impl Drop for RuntimeJournal {
    fn drop(&mut self) {
        // The writer owns one additional Arc while it is running.  Shut it
        // down when this is the last public handle (strong count == 2).
        if Arc::strong_count(&self.inner) <= 2 {
            self.mark_clean_shutdown();
        }
    }
}

fn writer_loop(inner: Arc<JournalInner>, receiver: mpsc::Receiver<JournalCommand>) {
    while let Ok(command) = receiver.recv() {
        match command {
            JournalCommand::Shutdown => break,
            JournalCommand::Event(event) => {
                if append_event(&inner.root, &event).is_err() {
                    if let Ok(mut health) = inner.health.lock() {
                        health.write_errors += 1;
                    }
                    if let Ok(mut events) = inner.compensation.lock() {
                        if events.len() == COMPENSATION_CAPACITY {
                            events.pop_front();
                        }
                        events.push_back(event);
                        if let Ok(mut health) = inner.health.lock() {
                            health.compensation_count = events.len() as u64;
                        }
                    }
                } else if let Ok(mut health) = inner.health.lock() {
                    health.written += 1;
                }
                rotate_logs(&inner.root);
            }
            JournalCommand::Flush(ack) => {
                let _ = ack.send(());
            }
        }
    }
}

fn append_event(root: &Path, event: &RuntimeJournalEvent) -> io::Result<()> {
    fs::create_dir_all(root)?;
    let day = event.timestamp.get(..10).unwrap_or("unknown-date");
    let path = root.join(format!("{day}.jsonl"));
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")
}

fn rotate_logs(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    files.sort_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
    let mut total = files
        .iter()
        .filter_map(|path| fs::metadata(path).ok().map(|meta| meta.len()))
        .sum::<u64>();
    for path in files {
        let old = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age.as_secs() > RETENTION_DAYS * 24 * 60 * 60);
        if (old || total > MAX_BYTES)
            && let Ok(size) = fs::metadata(&path).map(|meta| meta.len())
        {
            let _ = fs::remove_file(&path);
            total = total.saturating_sub(size);
        }
    }
}

fn timestamp_string() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_unix_timestamp(seconds)
}

fn format_unix_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let hour = seconds_of_day / 3_600;
    let minute = seconds_of_day % 3_600 / 60;
    let second = seconds_of_day % 60;

    // Civil date conversion without adding another time dependency.  The
    // first ten characters are therefore an actual YYYY-MM-DD day key for
    // daily JSONL files.
    let shifted_days = days + 719_468;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_non_blocking_events_and_exports_json() {
        let root = std::env::temp_dir().join(format!("w4dj-journal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (journal, interrupted) = RuntimeJournal::start(&root).expect("journal");
        assert!(!interrupted);
        journal.record_event("app_started", None, None, None, Value::Null, None);
        journal.flush();
        let daily_files = fs::read_dir(&root)
            .expect("journal root")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".jsonl"))
            .collect::<Vec<_>>();
        assert!(daily_files.iter().any(|name| {
            name.len() == 16
                && name.as_bytes().get(4) == Some(&b'-')
                && name.as_bytes().get(7) == Some(&b'-')
                && name.ends_with(".jsonl")
        }));
        let output = root.join("report.json");
        journal.export_full_report(&output).expect("export");
        let value: Value = serde_json::from_slice(&fs::read(&output).expect("report bytes"))
            .expect("valid report");
        assert_eq!(value["reportType"], "fullRuntime");
        assert!(
            value["events"]
                .as_array()
                .is_some_and(|events| !events.is_empty())
        );
        let diagnostics = serde_json::json!({
            "w4dj": {
                "database": {
                    "available": true,
                    "snapshotBytes": 123,
                },
            },
        });
        journal
            .export_full_report_with_diagnostics(&output, Some(&diagnostics))
            .expect("export with diagnostics");
        let value: Value = serde_json::from_slice(&fs::read(&output).expect("report bytes"))
            .expect("valid report with diagnostics");
        assert_eq!(
            value["diagnostics"]["w4dj"]["database"]["snapshotBytes"],
            123
        );
        journal.mark_clean_shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_interrupted_marker() {
        let root =
            std::env::temp_dir().join(format!("w4dj-journal-interrupted-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (first, interrupted) = RuntimeJournal::start(&root).expect("first journal");
        assert!(!interrupted);
        first.mark_clean_shutdown();
        fs::write(root.join("active.json"), b"stale").expect("stale marker");
        let (journal, interrupted) = RuntimeJournal::start(&root).expect("second journal");
        assert!(interrupted);
        journal.mark_clean_shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn high_volume_progress_events_do_not_block_the_caller() {
        let root =
            std::env::temp_dir().join(format!("w4dj-journal-pressure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (journal, interrupted) = RuntimeJournal::start(&root).expect("journal");
        assert!(!interrupted);
        let started = std::time::Instant::now();
        for index in 0..10_000u32 {
            journal.record_event(
                "analysis_candidate_progress",
                Some(String::from("pressure-test")),
                Some(String::from("analysis")),
                Some(String::from("running")),
                serde_json::json!({"processed": index, "total": 10_000}),
                None,
            );
        }
        // This is deliberately generous to keep the test stable on a busy CI
        // host while still catching accidental synchronous file I/O.
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        journal.mark_clean_shutdown();
        let health = journal.health();
        assert_eq!(
            health.queued + health.merged_progress + health.dropped,
            10_000
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rotation_removes_a_log_when_retention_cap_is_exceeded() {
        let root =
            std::env::temp_dir().join(format!("w4dj-journal-rotation-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("journal root");
        let path = root.join("2020-01-01.jsonl");
        File::create(&path)
            .and_then(|file| file.set_len(MAX_BYTES + 1))
            .expect("sparse oversized log");
        rotate_logs(&root);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }
}
