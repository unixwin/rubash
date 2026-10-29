//! The 1.5.0 Session API — the engine's published contract.
//!
//! This module is Tier A of the API design
//! (`docs/design/1.5.0-session-api.md`, rubash#595), published
//! in-tree for 1.5.0: the public Session contract the design
//! froze — one explicit single-threaded [`Session`] object per
//! shell — plus the tier-B syntax views of design Q4. The
//! workspace split (PLAN row 4, post-1.5.0) moves this module
//! wholesale to the split core crate; its shape does not change
//! when it moves. The API ledger (`docs/api-ledger.md`) governs
//! the consumed surface around it: what niubash consumes is
//! frozen there, and this contract is an addition to that
//! surface, not a replacement for it — the `Executor` and driver
//! items niubash consumes today keep working as recorded.
//!
//! What a host gets here:
//!
//! - [`Session`] / [`SessionBuilder`] — build a session
//!   (configuration plus capability injection), then drive it:
//!   `run`, `run_line`, `run_stdin`, `run_interactive_stdin`,
//!   `parse`, `prepare_line`, `complete`, `expand_prompt`,
//!   `call_function`, `interrupt` / `inject_signal`,
//!   `source_startup_files`, and the `close` finish contract.
//! - [`RunOutcome`] / [`Flow`] — how a run ended, as values the
//!   host acts on (`exit` is a flow, not a process exit).
//! - The §5.3 state views ([`StateViews`], [`VariablesView`],
//!   [`OptionsView`]) — the session's live state, read and
//!   written through the engine's own machinery.
//! - The capability traits a host implements:
//!   [`HistoryProvider`] (the design's unchanged trait),
//!   [`ExternalCommandHandler`], [`CompletionProvider`],
//!   [`DiagnosticSink`], [`RcPolicy`], and on Windows
//!   `ElevationHandler`; plus the observer trait
//!   [`SessionEvents`].
//! - Diagnostics as values: [`Diagnostic`], [`ParseFailure`],
//!   [`SessionError`].
//! - One recorded exception to the rule above: the tier-C
//!   [`Session::executor`] / [`Session::executor_mut`] seam,
//!   which hands a host the session's executor for the faces
//!   the contract does not cover (the API ledger names them).
//!   It is not part of the tier-A stability promise.
//! - Tier-B syntax: [`SyntaxView`] and friends, produced by
//!   `Session::parse` — owned view types only. The internal
//!   `Ast` is not part of Tier A (design Q4) and is not
//!   re-exported from here.
//!
//! Threading (design Q5): a [`Session`] is explicitly
//! single-threaded — neither `Send` nor `Sync`, by
//! construction. One thread owns one session; concurrency is
//! multiple sessions on multiple threads. Sessions coexist in
//! one process: executor state is session-scoped and the
//! engine's remaining process-global surfaces are internally
//! serialized, so hosts create sessions through the builder
//! and never share one across threads.
//!
//! One rule governs the whole module, from the debugger design
//! §7: **inspection is a rendering of state the engine already
//! maintains; there is no shadow bookkeeping.** A snapshot is
//! captured from the engine's own stores and views at the
//! moment it is taken, and its declaration text is produced by
//! the same printer `declare -p` uses, so the API and the shell
//! can never disagree about what a variable looks like.

pub mod capabilities;
pub mod diagnostic;
pub mod events;
pub mod session;
pub mod syntax;
pub mod views;

#[cfg(windows)]
pub use capabilities::ElevationHandler;
pub use capabilities::{
    Candidate, CompletionProvider, DiagnosticSink, ExternalCommandHandler, RcPolicy, StartupClass,
    StartupContext,
};
pub use diagnostic::{Diagnostic, DiagnosticKind, ParseFailure, SessionError};
pub use events::{CommandEvent, JobInfo, SessionEvents};
pub use session::{
    Flow, ParsedUnit, PreparedLine, RunOptions, RunOutcome, Session, SessionBuilder,
};
pub use syntax::{FunctionSite, LineClass, SyntaxView};
pub use views::{OptionState, OptionsView, StateViews, VariableSnapshot, VariablesView};

pub use crate::executor::HostExternalCommandOutput;
pub use crate::history::{HistoryProvider, SharedHistoryProvider};

#[cfg(test)]
mod consumer_tests;
