//! `std::sync::Mutex` backend (used when the `parking-lot` Cargo feature is
//! disabled). The standard library mutex is fully supported by Miri on every
//! platform, including Windows.

use std::sync::{Mutex, MutexGuard};

/// Lock used by the containers. Behaves like a mutex on `()`.
pub(crate) struct SyncLock {
    inner: Mutex<()>,
}

/// Guard returned by [`SyncLock::lock`]. Only ever held (for its `Drop`),
/// never dereferenced.
pub(crate) struct SyncLockGuard<'a> {
    #[allow(dead_code)]
    inner: MutexGuard<'a, ()>,
}

impl SyncLock {
    pub fn lock(&self) -> SyncLockGuard<'_> {
        SyncLockGuard {
            // The lock is never held across a panic, so it cannot be poisoned.
            inner: self.inner.lock().unwrap(),
        }
    }
}

impl Default for SyncLock {
    fn default() -> Self {
        Self {
            inner: Mutex::new(()),
        }
    }
}
