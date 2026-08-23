//! Internal lock used by the read/dirty containers.
//!
//! A single, unified implementation based on `parking_lot::Mutex`. Containers
//! never re-enter the lock internally, so a plain (non-reentrant) mutex is
//! sufficient.

/// Lock used by the containers. Behaves like a mutex on `()`.
pub(crate) type SyncLock = parking_lot::Mutex<()>;

/// Guard returned by [`SyncLock::lock`]. Only ever held, never dereferenced.
pub(crate) type SyncLockGuard<'a> = parking_lot::MutexGuard<'a, ()>;
