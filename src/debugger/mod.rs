//! Native debug session core (1.5.0 program, debugger design WP-2).
//!
//! This module is the session core shaped for its future home in the
//! split engine core: it is built in-tree first and moves wholesale
//! when the workspace split lands. It owns the vocabulary the rest of
//! the debugger is expressed in — the session gate flag, the typed
//! event stream, the breakpoint table, and the session state
//! machine — and nothing else. In particular it contains:
//!
//! - no terminal or console handling (frontends are views; they live
//!   elsewhere and only translate into session operations),
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
pub mod driver;
pub mod event;
pub mod gate;
pub mod session;

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
pub use driver::{SessionDriver, StopLoop, StopLoopOutcome};
#[allow(unused_imports)]
pub use event::{
    DebugEvent, ExceptionKind, ExceptionSite, FrameReturn, OutputStream, ReturnRoute, SeamKind,
    SourceLocation, StopInfo, StopReason, SynthesizedFrameKind, WatchChange,
};
#[allow(unused_imports)]
pub use gate::{GateFlag, SESSION_ACTIVE};
#[allow(unused_imports)]
pub use session::{DebugSession, SessionError, SessionState, StepArming, Termination};

#[cfg(test)]
mod wiring_tests;
