//! Internal lock used by the read/dirty containers.
//!
//! The backend is a **single choice**, selected by exactly one Cargo feature:
//!
//! - `parking-lot` (default): [`parking_lot`] module, `parking_lot::Mutex`,
//!   the fastest mutex for the write path.
//! - `std-lock`: [`std_mutex`] module, `std::sync::Mutex`, which Miri supports
//!   on every platform (including Windows), so race/UB tests can run under
//!   Miri on Windows with `--no-default-features --features std-lock`.
//!
//! Exactly one of the two must be enabled: enabling both, or neither, is a
//! compile error.
//!
//! Reads are lock-free anyway (they go through the atomic snapshot), so this
//! lock only guards the write path.

#[cfg(all(feature = "parking-lot", feature = "std-lock"))]
compile_error!("features `parking-lot` and `std-lock` are mutually exclusive; enable exactly one");
#[cfg(all(not(feature = "parking-lot"), not(feature = "std-lock")))]
compile_error!("exactly one of the `parking-lot` or `std-lock` features must be enabled");

#[cfg(feature = "parking-lot")]
mod parking_lot;
#[cfg(all(feature = "std-lock", not(feature = "parking-lot")))]
mod std_mutex;

#[cfg(feature = "parking-lot")]
pub(crate) use parking_lot::{SyncLock, SyncLockGuard};
#[cfg(all(feature = "std-lock", not(feature = "parking-lot")))]
pub(crate) use std_mutex::{SyncLock, SyncLockGuard};
