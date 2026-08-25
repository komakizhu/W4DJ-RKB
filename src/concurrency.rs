//! Shared, cancellable concurrency budget used by scans and conversions.
//!
//! The budget is deliberately independent from either task slot.  A permit
//! is held only for the file-level operation that is doing work, and its RAII
//! drop releases the capacity even when a worker returns an error or panics.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const MIN_CONCURRENCY_LIMIT: usize = 1;
pub const MAX_CONCURRENCY_LIMIT: usize = 10;
pub const DEFAULT_CONCURRENCY_LIMIT: usize = 2;

pub fn normalize_concurrency_limit(value: usize) -> usize {
    value.clamp(MIN_CONCURRENCY_LIMIT, MAX_CONCURRENCY_LIMIT)
}

#[derive(Debug)]
struct BudgetState {
    limit: usize,
    active: usize,
}

/// An application-wide permit pool.  Waiting is interruptible so a cancelled
/// scan/conversion does not leave a worker blocked forever on a permit.
#[derive(Debug, Clone)]
pub struct GlobalConcurrencyBudget {
    state: Arc<(Mutex<BudgetState>, Condvar)>,
}

#[derive(Debug)]
pub struct ConcurrencyPermit {
    state: Arc<(Mutex<BudgetState>, Condvar)>,
}

impl GlobalConcurrencyBudget {
    pub fn new(limit: usize) -> Self {
        Self {
            state: Arc::new((
                Mutex::new(BudgetState {
                    limit: normalize_concurrency_limit(limit),
                    active: 0,
                }),
                Condvar::new(),
            )),
        }
    }

    pub fn limit(&self) -> usize {
        self.state
            .0
            .lock()
            .expect("concurrency budget lock poisoned")
            .limit
    }

    pub fn active_count(&self) -> usize {
        self.state
            .0
            .lock()
            .expect("concurrency budget lock poisoned")
            .active
    }

    pub fn acquire<F>(&self, should_cancel: F) -> Option<ConcurrencyPermit>
    where
        F: Fn() -> bool,
    {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().expect("concurrency budget lock poisoned");
        loop {
            if should_cancel() {
                return None;
            }
            if state.active < state.limit {
                state.active += 1;
                return Some(ConcurrencyPermit {
                    state: Arc::clone(&self.state),
                });
            }
            let (next, _) = wake
                .wait_timeout(state, Duration::from_millis(50))
                .expect("concurrency budget condvar poisoned");
            state = next;
        }
    }
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.active = state.active.saturating_sub(1);
            wake.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn normalizes_limit_to_the_product_range() {
        assert_eq!(normalize_concurrency_limit(0), 1);
        assert_eq!(normalize_concurrency_limit(2), 2);
        assert_eq!(normalize_concurrency_limit(99), 10);
    }

    #[test]
    fn permits_are_shared_and_never_exceed_limit() {
        let budget = Arc::new(GlobalConcurrencyBudget::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let threads = (0..6)
            .map(|_| {
                let budget = Arc::clone(&budget);
                let active = Arc::clone(&active);
                let peak = Arc::clone(&peak);
                thread::spawn(move || {
                    let permit = budget.acquire(|| false).expect("permit");
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(5));
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().expect("worker should finish");
        }
        assert!(peak.load(Ordering::SeqCst) <= 2);
        assert_eq!(budget.active_count(), 0);
    }

    #[test]
    fn waiting_acquire_is_cancellable() {
        let budget = GlobalConcurrencyBudget::new(1);
        let _permit = budget.acquire(|| false).expect("first permit");
        let cancelled = AtomicBool::new(true);
        assert!(
            budget
                .acquire(|| cancelled.load(Ordering::SeqCst))
                .is_none()
        );
    }
}
