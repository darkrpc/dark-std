//! `parking_lot::Mutex` backend (default, enabled by the `parking-lot`
//! Cargo feature).

/// Lock used by the containers. Behaves like a mutex on `()`.
pub(crate) type SyncLock = parking_lot::Mutex<()>;

/// Guard returned by [`SyncLock::lock`]. Only ever held, never dereferenced.
pub(crate) type SyncLockGuard<'a> = parking_lot::MutexGuard<'a, ()>;
