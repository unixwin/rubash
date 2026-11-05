//! One completion attempt as an explicit, reentrancy-aware object
//! (unixwin/niubash#863 GAP-7).
//!
//! The host's completer used to reach the session through a
//! thread-local lend of the whole shell (`COMPLETION_BRIDGE`):
//! the completer must be `Send`, so it could not hold the
//! session, and a nested completion observed "no bridge" —
//! empty engine faces — instead of live state. This guard is
//! the contract replacement for that lend's role: the host
//! opens one guard per attempt from the `&mut Session` it
//! already holds, drives every execution face of the attempt
//! through the guard, and reads live state through
//! [`CompletionAttempt::session`] for the attempt's duration.
//!
//! Why a guard object, and not a scoped `&self` context (the
//! brief's option (a))? An attempt EXECUTES: it runs compspec
//! functions (`run_completion_function_live` takes `&mut`),
//! captures `COMPREPLY`, mutates the governing-compspec copy
//! through `compopt`, and lets function bodies write session
//! state. A `&self` context cannot own any of that without
//! interior mutability across the whole executor; the reads
//! half already has `&self` faces (`completion_view`,
//! `has_compspec`, `glob_filename`, `has_function`). So the
//! honest shape is (b): an explicit object owning the `&mut`
//! borrow for the attempt, lending `&Session` reads within it.
//!
//! Nesting semantics: nested attempts are repeated method
//! calls on the SAME guard, never a second guard (the borrow
//! forbids it, which is the point — the aliasing hazard the
//! thread-local's `ACTIVE` flag papered over is now a compile
//! error). Each `complete_detailed` call arms the completion
//! window for its own duration through the depth counter, so
//! an inner attempt returning never closes the outer
//! attempt's `compopt` window; reads through [`Self::session`]
//! are live at every depth, never dark. When the guard drops,
//! the window discipline is whatever the individual calls
//! left: a bare `compopt` outside any attempt errors, exactly
//! as before.

use super::capabilities::{Candidate, CompletionOutcome};
use super::diagnostic::SessionError;
use super::session::Session;

/// One completion attempt, borrowed from its session.
pub struct CompletionAttempt<'a> {
    session: &'a mut Session,
}

impl<'a> CompletionAttempt<'a> {
    pub(crate) fn new(session: &'a mut Session) -> Self {
        Self { session }
    }

    /// Candidates for `line` with the cursor at byte `cursor`:
    /// exactly [`Session::complete`] through this attempt.
    pub fn complete(&mut self, line: &str, cursor: usize) -> Vec<Candidate> {
        self.session.complete(line, cursor)
    }

    /// [`Session::complete_detailed`] through this attempt:
    /// candidates plus the attempt's insertion/display policy.
    /// Calling this again on the same guard is a nested
    /// attempt: it sees everything the outer attempt wrote
    /// (functions defined, compspecs registered, variables
    /// set) and its own return never darkens the outer
    /// attempt's reads.
    pub fn complete_detailed(&mut self, line: &str, cursor: usize) -> CompletionOutcome {
        self.session.complete_detailed(line, cursor)
    }

    /// [`Session::call_function_with_env`] through this
    /// attempt: the widget/compdef callback face, with a set
    /// of temporary variable bindings applied for the call's
    /// duration and restored afterwards. The function runs
    /// with the attempt's window armed, so `compgen` and bare
    /// `compopt` inside it behave exactly as inside a
    /// compspec function.
    pub fn call_function_with_env<I, S, E, K, V>(
        &mut self,
        name: &str,
        args: I,
        temporary_env: E,
    ) -> Result<i32, SessionError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
        E: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.session
            .call_function_with_env(name, args, temporary_env)
    }

    /// The session under this attempt, shared: every `&self`
    /// read face (`completion_view`, `has_compspec`,
    /// `glob_filename`, `has_function`, `environment_full`,
    /// `variable_value`) answers from live state at whatever
    /// depth the attempt has reached. This is the nested-read
    /// face the thread-local lend served dark: a nested
    /// completion reading through the guard never observes
    /// "no session".
    pub fn session(&self) -> &Session {
        self.session
    }
}
