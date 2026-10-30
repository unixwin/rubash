//! Native debug session core (1.5.0 program, debugger design WP-2).
//!
//! This module is the session core shaped for its future home in the
//! split engine core: it is built in-tree first and moves wholesale
//! when the workspace split lands. It owns the vocabulary the rest of
//! the debugger is expressed in — the session gate flag, the typed
//! event stream, the breakpoint table, and the session state
//! machine — and nothing else. In particular it contains:
//!
//! - no debugging semantics in the frontends: the `--dap`
//!   protocol frontend lives in this module (PLAN row 10) as a
//!   pure view — it translates between framed protocol messages
//!   and session operations and renders the session's own
//!   snapshots, and owns no state machine of its own. The
//!   interactive `--debug` view (row 11) follows the same rule,
//! - no platform windowing or process facilities,
//! - no execution logic: the executor consults the session through
//!   the DEBUG-trap funnel (WP-3 wiring, in `executor::trap_exec`);
//!   nothing here reaches back into executor internals. With no
//!   session attached the gate flag stays disarmed and this module
//!   costs one atomic load.
//!
//! The single-threaded session contract applies throughout: one
//! thread owns one session; the only cross-thread surface is the
//! atomic gate and stop-request flags.

pub mod breakpoints;
pub mod cli;
pub mod dap;
pub mod driver;
pub mod evaluation;
pub mod event;
pub mod gate;
pub mod inspect;
pub mod json;
pub mod resolution;
pub mod session;
pub mod stepping;

// The re-exports below are the module's deliberate facade: the names are
// the debugger vocabulary for consumers in later work packages, so most
// have no in-tree consumer yet. Path dependents that build this crate
// with `-D warnings` (niubash CI does, via its toolchain action's default
// RUSTFLAGS, and path dependencies get no `--cap-lints` leniency) must
// not be billed for that, so each facade group carries its own
// `#[allow(unused_imports)]` instead of a module-wide allow.
#[allow(unused_imports)]
pub use breakpoints::{
    Breakpoint, BreakpointId, BreakpointKind, BreakpointResolution, BreakpointTable,
    BreakpointTarget, HitFlow, HitOutcome, WatchSnapshot,
};
#[allow(unused_imports)]
pub use cli::{run_debug_cli, run_debug_cli_with_stream, DebugCliPreset};
#[allow(unused_imports)]
pub use dap::{run_dap_server, run_dap_stdio, DapLaunchPreset};
#[allow(unused_imports)]
pub use driver::{SessionDriver, StopLoop, StopLoopOutcome};
#[allow(unused_imports)]
pub use evaluation::{
    check_implicit, parse_failure, single_expansion_word, DiagnosticKind, EvaluationDiagnostic,
    EvaluationOutcome, EvaluationRequest, EvaluationResult, PowerMode, RejectionKind,
    WhitelistRejection,
};
#[allow(unused_imports)]
pub use event::{
    DebugEvent, ExceptionKind, ExceptionSite, FrameReturn, OutputStream, ReturnRoute, SeamKind,
    SourceLocation, StopInfo, StopReason, SynthesizedFrameKind, WatchChange,
};
#[allow(unused_imports)]
pub use gate::{GateFlag, SESSION_ACTIVE};
#[allow(unused_imports)]
pub use inspect::{
    render_caller, render_caller_bare, render_dynamic_state, render_info_args, render_pipestatus,
    render_variables, render_watch_value, render_where, VariablePartition,
};
pub(crate) use resolution::file_identity;
#[allow(unused_imports)]
pub use session::{
    exception_filter_id, DebugSession, SessionError, SessionState, StepArming, Termination,
    EXCEPTION_FILTER_IDS,
};
#[allow(unused_imports)]
pub use stepping::{
    arming_stops_at_frame_return, arming_stops_at_seam, arming_stops_at_seam_in_file,
};

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod dap_tests;
#[cfg(test)]
mod driver_tests;
#[cfg(test)]
mod evaluation_tests;
#[cfg(test)]
mod s4_states_tests;
#[cfg(test)]
mod stepping_tests;
#[cfg(test)]
mod wiring_tests;
