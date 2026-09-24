//! fd layer dispatcher (Q12, docs/TASKBOARD Q12).
//!
//! Two engine landings behind one consumer surface:
//!   - `windows_impl`: the raw-HANDLE table (governance doc 3.6, layer 3).
//!     HANDLE = win32 handle; handle inheritance is opt-in per handle.
//!   - `unix`: POSIX translation. HANDLE = RawFd; fork inherits everything,
//!     `dup` shares the open file description, so the table's core invariant
//!     ("duplicate shares the file offset") is kernel-native.
//!
//! Consumers import `crate::fd::*` unchanged on both platforms.

#[cfg(windows)]
pub(crate) mod windows_impl;

#[cfg(windows)]
pub use windows_impl::*;

#[cfg(unix)]
pub(crate) mod unix;

#[cfg(unix)]
pub use unix::*;
