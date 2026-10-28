//! The typed debug-event stream (WP-1 event catalogue DE-01..=DE-12).
//!
//! The session owns one event stream; frontends are views over it
//! and translate events into their own protocol or transcript
//! shapes — the catalogue ids below are the stable names that
//! translation (and the acceptance transcripts) refer to. The
//! catalogue deliberately excludes job-completion notices and
//! pipeline status: the design promises those as *rendered data*
//! for the inspection surface, not as events.
//!
//! Two properties of the substrate are baked into the payloads:
//!
//! - Stop events carry their command text from the same frozen
//!   source the trap path uses, never a reconstruction (WP-1 pin
//!   P-3), so [`StopInfo::command_text`] is that text, verbatim.
//! - A frame return is an event in its own right, including the two
//!   bypass routes where the frame never ran (entry skipped, entry
//!   force-returned): later work explains "why did the entry stop
//!   not happen" from this event, and return-waiting step arming
//!   matches on it.

use super::breakpoints::{BreakpointId, BreakpointResolution};

/// A source position in the identity the engine itself reports for
/// the running code (the same identity breakpoint targets key on).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    /// File identity as reported by the engine.
    pub file: String,
    /// 1-based line number.
    pub line: u32,
}

impl SourceLocation {
    /// A location from its file identity and 1-based line.
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }
}

/// The internal seam kind tags (WP-1 catalogue section 4).
///
/// Tags drive stepping filters and fixture attribution; they are
/// not admission tiers — the two stop granularities of the design
/// (line, function entry) sit on top of this substrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamKind {
    /// A plain command at the main seam.
    SimpleCommand,
    /// A conditional command at the main seam.
    Conditional,
    /// An arithmetic command at the main seam.
    Arithmetic,
    /// The inner command of a `time` pipeline.
    TimeInner,
    /// A background command body launch.
    BackgroundLaunch,
    /// A `case` head.
    CaseHead,
    /// A `select` head.
    SelectHead,
    /// A `for` head, per iteration.
    ForHead,
    /// The init clause of an arithmetic `for`.
    ArithForInit,
    /// The test clause of an arithmetic `for`.
    ArithForTest,
    /// The update clause of an arithmetic `for`.
    ArithForUpdate,
    /// One pipeline element.
    PipelineElement,
    /// A function entry, before the body.
    FunctionEntry,
    /// A function return site.
    FunctionReturn,
    /// A sourced file's return site.
    SourceReturn,
    /// The per-command boundary poll (async-stop service context).
    BoundaryPoll,
    /// The exit path (session-exit context).
    ExitPath,
}

/// Why the session stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The entry stop: before the first command (design section 14
    /// Q1 ruling — both frontends stop there).
    Entry,
    /// A line breakpoint committed a stop.
    Breakpoint(BreakpointId),
    /// A function breakpoint committed a stop at an entry seam.
    FunctionBreakpoint(BreakpointId),
    /// Step arming was satisfied (including a finish arrival).
    Step,
    /// A watch entry's value changed since the previous stop
    /// (design 5.4 stop-and-diff).
    Watchpoint(BreakpointId),
    /// An asynchronous stop request was serviced at a boundary poll
    /// (design 4.4/9.4).
    Pause,
    /// An exception site whose filter is selected parked the
    /// session before the engine's pending action ran (design
    /// 9.3; PLAN row 12): before the ERR trap action for
    /// [`ExceptionKind::ErrTrap`], before the exit path for
    /// [`ExceptionKind::Errexit`], and before the run ends for
    /// [`ExceptionKind::ParseError`]. The stop is a park, not a
    /// veto: resuming lets the pending action proceed unchanged.
    Exception(ExceptionKind),
}

/// The payload of a stop: everything a frontend needs to render the
/// stop without asking the engine anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopInfo {
    /// Why the session stopped.
    pub reason: StopReason,
    /// Where it stopped, when a seam supplied a position.
    pub location: Option<SourceLocation>,
    /// The seam kind tag of the stop point.
    pub seam: Option<SeamKind>,
    /// Depth of the frame the session stopped in.
    pub frame_depth: usize,
    /// The command text of the stop point, from the same frozen
    /// source the trap path reports — never reconstructed.
    pub command_text: Option<String>,
}

impl StopInfo {
    /// The entry stop record: no seam has fired yet, so there is no
    /// location, tag, or command text.
    pub fn entry() -> Self {
        Self {
            reason: StopReason::Entry,
            location: None,
            seam: None,
            frame_depth: 0,
            command_text: None,
        }
    }
}

/// How a returning frame got to its return site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnRoute {
    /// The body ran and completed (or returned) normally.
    Completed,
    /// The entry decision skipped the body entirely; the frame
    /// returns without having executed.
    EntrySkipped,
    /// A trap action forced the return from the entry point.
    EntryForceReturn,
    /// A sourced file finished and control returns to its sourcer.
    SourcedFile,
}

/// A frame-transition payload (DE-04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameReturn {
    /// The returning function's name, or `None` for a sourced file.
    pub function: Option<String>,
    /// Depth of the frame that is returning.
    pub frame_depth: usize,
    /// The status the frame returns with.
    pub status: i32,
    /// Which route brought the frame here.
    pub route: ReturnRoute,
}

/// A watch change payload (DE-05): the value moved since the
/// previous stop, in rendered form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchChange {
    /// The watch entry that changed.
    pub id: BreakpointId,
    /// The watched name or expression text.
    pub expression: String,
    /// The rendered value at the previous stop (`None`: unset).
    pub old_value: Option<String>,
    /// The rendered value now (`None`: unset).
    pub new_value: Option<String>,
}

/// Which exception surface an [`DebugEvent::ExceptionSite`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionKind {
    /// An error-trap action is about to run.
    ErrTrap,
    /// An errexit decision is about to end the shell.
    Errexit,
    /// A parse error is terminating the run.
    ParseError,
}

/// An exception-site payload (DE-06). The site is reported, and
/// because its filter is selected the session also parks there
/// ([`StopReason::Exception`]); the park is not a veto — resuming
/// lets the action the site announced (the trap action, the exit,
/// the run's end) proceed unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionSite {
    /// Which surface reported.
    pub kind: ExceptionKind,
    /// The status in play at the site.
    pub status: i32,
    /// Where the site is, when known.
    pub location: Option<SourceLocation>,
}

/// Which synthesized frame a DE-12 event concerns. Synthesized
/// frames are snapshot frames: the banner vocabulary names them so
/// nobody mistakes them for the live frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthesizedFrameKind {
    /// A `( ... )` subshell frame.
    Subshell,
    /// A `$( ... )` command-substitution frame.
    CommandSubstitution,
}

/// The output stream an output event belongs to (DE-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    /// The debuggee's standard output.
    Stdout,
    /// The debuggee's standard error.
    Stderr,
    /// Console output (the debugger's own channel in protocol
    /// renderings).
    Console,
}

/// One event on the session's typed stream.
///
/// Variants are annotated with their WP-1 catalogue id; the id is
/// also available programmatically via [`DebugEvent::catalogue_id`]
/// for transcripts and fixture attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugEvent {
    /// DE-01: the session is established. The entry stop follows as
    /// a [`DebugEvent::Stopped`] with [`StopReason::Entry`].
    SessionStarted,
    /// DE-02/DE-03: the session stopped — at a line seam (DE-02) or
    /// a function-entry seam (DE-03); the payload's seam tag and
    /// reason say which.
    Stopped(StopInfo),
    /// DE-04: a frame returned, by any route (see [`ReturnRoute`]).
    FrameReturned(FrameReturn),
    /// DE-05: a watch entry's value changed since the previous stop.
    WatchChanged(WatchChange),
    /// DE-06: an exception site whose filter is selected was
    /// reached; the park at the site follows as a
    /// [`DebugEvent::Stopped`] with [`StopReason::Exception`].
    ExceptionSite(ExceptionSite),
    /// DE-07: an asynchronous stop request was serviced at a
    /// boundary poll, after any signal trap for it had been
    /// dispatched.
    AsyncStopServiced,
    /// DE-08: an untrapped fatal signal killed the debuggee. The
    /// session reports the death; it does not pretend recovery is
    /// possible.
    DebuggeeDied {
        /// The signal number that killed the debuggee.
        signal: i32,
    },
    /// DE-09: the debuggee finished its exit path with a final
    /// status.
    SessionExited {
        /// The debuggee's final exit status.
        status: i32,
    },
    /// DE-10: debuggee output, classified by stream. The engine
    /// side only classifies; the debuggee's stdio is untouched.
    OutputProduced {
        /// Which stream the chunk belongs to.
        stream: OutputStream,
        /// The output chunk.
        text: String,
    },
    /// DE-11: a breakpoint's resolution state changed (verified,
    /// not verified with reason, or a pending function entry
    /// resolved).
    BreakpointResolved {
        /// The entry whose resolution changed.
        id: BreakpointId,
        /// Its new resolution state.
        resolution: BreakpointResolution,
    },
    /// DE-12: a synthesized (snapshot) frame was entered.
    SynthesizedFrameEntered {
        /// Which kind of synthesized frame.
        kind: SynthesizedFrameKind,
        /// Its nesting depth.
        depth: usize,
    },
    /// DE-12: a synthesized (snapshot) frame was exited.
    SynthesizedFrameExited {
        /// Which kind of synthesized frame.
        kind: SynthesizedFrameKind,
        /// Its nesting depth.
        depth: usize,
    },
}

impl DebugEvent {
    /// The WP-1 catalogue id of this event ("DE-01" ..= "DE-12").
    pub fn catalogue_id(&self) -> &'static str {
        match self {
            DebugEvent::SessionStarted => "DE-01",
            DebugEvent::Stopped(info) => match info.seam {
                Some(SeamKind::FunctionEntry) => "DE-03",
                _ => "DE-02",
            },
            DebugEvent::FrameReturned(_) => "DE-04",
            DebugEvent::WatchChanged(_) => "DE-05",
            DebugEvent::ExceptionSite(_) => "DE-06",
            DebugEvent::AsyncStopServiced => "DE-07",
            DebugEvent::DebuggeeDied { .. } => "DE-08",
            DebugEvent::SessionExited { .. } => "DE-09",
            DebugEvent::OutputProduced { .. } => "DE-10",
            DebugEvent::BreakpointResolved { .. } => "DE-11",
            DebugEvent::SynthesizedFrameEntered { .. }
            | DebugEvent::SynthesizedFrameExited { .. } => "DE-12",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DebugEvent, ExceptionKind, ExceptionSite, FrameReturn, OutputStream, ReturnRoute, SeamKind,
        SourceLocation, StopInfo, StopReason, SynthesizedFrameKind, WatchChange,
    };
    use crate::debugger::breakpoints::{BreakpointResolution, BreakpointTable};

    fn stop_at(seam: SeamKind) -> DebugEvent {
        DebugEvent::Stopped(StopInfo {
            reason: StopReason::Step,
            location: Some(SourceLocation::new("a.sh", 3)),
            seam: Some(seam),
            frame_depth: 0,
            command_text: Some("echo hi".to_string()),
        })
    }

    #[test]
    fn entry_stop_info_has_no_seam_facts_yet() {
        let info = StopInfo::entry();
        assert_eq!(info.reason, StopReason::Entry);
        assert_eq!(info.location, None);
        assert_eq!(info.seam, None);
        assert_eq!(info.command_text, None);
    }

    #[test]
    fn stopped_catalogue_id_distinguishes_entry_seams() {
        assert_eq!(stop_at(SeamKind::SimpleCommand).catalogue_id(), "DE-02");
        assert_eq!(stop_at(SeamKind::FunctionEntry).catalogue_id(), "DE-03");
    }

    #[test]
    fn every_catalogue_variant_maps_to_its_id() {
        let mut table = BreakpointTable::new();
        let id = table.insert_line("a.sh", 1);
        let cases: Vec<(DebugEvent, &str)> = vec![
            (DebugEvent::SessionStarted, "DE-01"),
            (
                DebugEvent::FrameReturned(FrameReturn {
                    function: Some("f".to_string()),
                    frame_depth: 1,
                    status: 0,
                    route: ReturnRoute::EntrySkipped,
                }),
                "DE-04",
            ),
            (
                DebugEvent::WatchChanged(WatchChange {
                    id,
                    expression: "x".to_string(),
                    old_value: Some("1".to_string()),
                    new_value: Some("2".to_string()),
                }),
                "DE-05",
            ),
            (
                DebugEvent::ExceptionSite(ExceptionSite {
                    kind: ExceptionKind::ParseError,
                    status: 2,
                    location: None,
                }),
                "DE-06",
            ),
            (DebugEvent::AsyncStopServiced, "DE-07"),
            (DebugEvent::DebuggeeDied { signal: 9 }, "DE-08"),
            (DebugEvent::SessionExited { status: 3 }, "DE-09"),
            (
                DebugEvent::OutputProduced {
                    stream: OutputStream::Stderr,
                    text: "boom".to_string(),
                },
                "DE-10",
            ),
            (
                DebugEvent::BreakpointResolved {
                    id,
                    resolution: BreakpointResolution::Pending,
                },
                "DE-11",
            ),
            (
                DebugEvent::SynthesizedFrameEntered {
                    kind: SynthesizedFrameKind::CommandSubstitution,
                    depth: 2,
                },
                "DE-12",
            ),
            (
                DebugEvent::SynthesizedFrameExited {
                    kind: SynthesizedFrameKind::Subshell,
                    depth: 1,
                },
                "DE-12",
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(event.catalogue_id(), expected, "event: {event:?}");
        }
    }

    #[test]
    fn source_location_equality_is_by_identity_and_line() {
        assert_eq!(
            SourceLocation::new("lib.sh", 20),
            SourceLocation::new("lib.sh", 20)
        );
        assert_ne!(
            SourceLocation::new("lib.sh", 20),
            SourceLocation::new("lib.sh", 21)
        );
        assert_ne!(
            SourceLocation::new("lib.sh", 20),
            SourceLocation::new("other.sh", 20)
        );
    }
}
