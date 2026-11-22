//! Cooperative cancellation (rubash#2810): the host-held token a
//! [`Session`](super::Session) polls at execution safe points.
//!
//! The token is plain shared memory — no signal, no environment
//! variable, nothing a script can observe: `trap` never sees it, no
//! status is synthesized from it, and the script-visible semantics
//! (including every GNU signal behavior) are unchanged. The host
//! holds the write end ([`CancelToken::cancel`]); the engine holds
//! a read end and polls it with one relaxed atomic load at each
//! safe point: statement boundaries in the main list walker (every
//! `while`/`for`/`until`/`select`/`eval` body funnels through it),
//! the canonical-counter fast `for ((;;))` loop's trap tick, and the
//! engine's own blocking wait loops (the in-process `sleep`
//! emulation, the `read` acquisition's poll slices, `select`'s
//! query). A blocked external child process is deliberately NOT a
//! safe point — preempting one is the host/process layer's business
//! (the issue's non-goal), so an engine wait on a real child does
//! not consult the token.
//!
//! Cancellation is one-way: once [`CancelToken::cancel`] lands, the
//! current run unwinds to a distinct [`Flow::Cancelled`](super::Flow)
//! outcome (never a spoofed SIGINT 130, never a `trap INT` fire), and
//! the session stays consistent — variables, functions and cwd keep
//! their last-completed state — until the host calls
//! [`CancelToken::reset`] and runs the next script.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A cloneable cooperative-cancellation handle (rubash#2810).
///
/// The host clones it freely and may call [`CancelToken::cancel`]
/// from any thread while a run is in progress; the engine's safe
/// points only ever [`CancelToken::is_cancelled`]-load it.
/// `Send + Sync` comes free from the `Arc<AtomicBool>` carrier.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    /// A fresh token in the unset state.
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    /// Request cancellation of the run this token is installed on.
    /// Takes effect at the next engine safe point; idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Clear the cancellation request so the session can run the
    /// next script. The host calls this after the cancelled run's
    /// outcome reported [`Flow::Cancelled`](super::Flow) — resetting
    /// mid-run is legal (a later cancel re-arms it) but a run that
    /// observed the request already ended either way.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested (one relaxed load —
    /// the same read the engine's safe points perform).
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}
