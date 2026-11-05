//! Layer 1–2 of the Session API (API design §5.1–§5.2): the
//! [`Session`] object and its builder, the run path with its
//! outcome value, and the session-level queries (parse,
//! completeness, prompt, completion, startup sourcing, close).
//!
//! Everything here is a façade over seams the engine already
//! has — the script drivers, the executor's accessor surface,
//! the §5.3 state views. No execution semantics live in this
//! module: a state write goes through the machinery every other
//! write uses, and a run is driven by the same driver entries
//! the shipped hosts drive.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use super::bind::BindView;
#[cfg(windows)]
use super::capabilities::ElevationHandler;
use super::capabilities::{
    Candidate, CompletionOutcome, CompletionProvider, DiagnosticSink, ExternalCommandHandler,
    RcPolicy, StartupClass, StartupContext,
};
use super::completion_attempt::CompletionAttempt;
use super::completion_view::{full_env_sorted, CompletionView};
use super::diagnostic::{Diagnostic, DiagnosticKind, ParseFailure, SessionError};
use super::events::{CommandEvent, SessionEvents};
use super::syntax::SyntaxView;
use super::views::{OptionsView, StateViews, VariablesView};
use crate::executor::Executor;
use crate::history::{SessionHistory, SharedHistoryProvider};
use crate::script_driver;

/// How a run ended, as a value the host acts on (§5.2): an
/// embedding host decides whether an exit request closes the
/// session, the pane, or nothing. This replaces the exit-jump
/// flags hosts used to have to poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Flow {
    /// The run completed; the session continues.
    Completed,
    /// The run asked the shell to exit with this status (the
    /// `exit` builtin's jump, or a fatal function error's). The
    /// status is the same value [`RunOutcome::status`] carries.
    ExitRequested(i32),
    /// A top-level `return` unwound the run with this status.
    /// The engine's drivers fold a top-level return into the
    /// run's status, so `run()` does not produce this variant in
    /// this publication step; it is part of the contract's shape
    /// for the sourced-unit paths.
    Return(i32),
    /// Q6 arm B: the run ended in an errexit break — GNU's
    /// ERREXIT unwind (evalstring.c:387-393), kept apart from
    /// both [`Flow::Completed`] (arm A folds the break there,
    /// which is what forces hosts to re-derive it) and
    /// [`Flow::ExitRequested`] (no `exit` was requested). A host
    /// that adjudicates stops from `flow` alone now sees the
    /// break; the status is the same value
    /// [`RunOutcome::status`] carries.
    ErrexitBreak(i32),
}

/// The options a single run carries (§5.2). Construct through
/// [`RunOptions::default`] and set the fields — the struct is
/// `#[non_exhaustive]` so the option set can grow within a
/// minor version.
///
/// There is deliberately no `capture` field in this step:
/// §5.4's output-sink takeover has no engine seam short of new
/// instrumentation, and an option the engine cannot honor
/// would be a fake contract. Runs inherit the session's
/// streams, as every shipped host's runs do today.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunOptions {
    /// Diagnostic prefix / $0-ish identity of this source (the
    /// document the host is feeding from). Defaults to the
    /// session's builder `argv0` when unset.
    pub source_name: Option<String>,
    /// The document line the source's first line is (REPL hosts
    /// submit line-by-line from a larger document).
    pub line_offset: usize,
    /// Interactive-shaped run: history recording, alias gates,
    /// job notices behave as they do for an interactive reader.
    pub interactive: bool,
}

impl RunOptions {
    /// All defaults: no source name, offset 0, non-interactive.
    pub fn new() -> RunOptions {
        RunOptions::default()
    }
}

/// Where a session's process shape came from (rubash#2130): a
/// fresh invocation, or an engine respawn — this process/session
/// is a fork-shaped child (process substitution, coproc, async
/// wrapper) continuing a parent session's post-startup state.
/// GNU's forked children never run the startup chain
/// (execute_cmd.c:1576 `execute_in_subshell`); a respawn-shaped
/// session declares that here, as a construction fact, instead of
/// the transport layer guessing from inherited environment marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RespawnSource {
    /// A fresh invocation: the startup chain is this session's
    /// own to run.
    #[default]
    Fresh,
    /// An engine respawn continuing a parent session: the startup
    /// chain must not run (the parent already ran it, and this
    /// session inherits the post-startup state).
    EngineChild,
}

/// The session's invocation shape, declared at construction time
/// (rubash#2130). GNU decides startup behavior from the invocation
/// itself — shell.c:540-549 derives `interactive_shell` from the
/// process's own argv/tty shape and shell.c:497-503 sets
/// `login_shell` only from `-l`/`--login` or the argv[0] `-` mark —
/// and keeps both in process-local globals, never in the
/// environment. The shape is therefore a construction parameter
/// of the session, not a set of environment seeds: the builder
/// applies it to the executor's typed invocation state after the
/// environment seeds, so the structured declaration is
/// authoritative for the fields it names. The
/// `__RUBASH_INTERACTIVE` / `__RUBASH_LOGIN_SHELL` env-table
/// entries continue to exist as the state's write-through
/// projection for the faces that render them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct InvocationShape {
    /// Interactive shell (the `-i` / no-operand-tty shape).
    pub interactive: bool,
    /// Login shell (the `-l` / argv[0] `-` shape).
    pub login: bool,
    /// Whether this session is an engine respawn.
    pub respawn: RespawnSource,
}

impl InvocationShape {
    /// The plain shape: non-interactive, non-login, fresh.
    pub fn new() -> InvocationShape {
        InvocationShape::default()
    }

    /// An interactive fresh session.
    pub fn interactive() -> InvocationShape {
        InvocationShape {
            interactive: true,
            ..InvocationShape::default()
        }
    }

    /// An interactive login session (the desktop-login shape).
    pub fn login_interactive() -> InvocationShape {
        InvocationShape {
            interactive: true,
            login: true,
            ..InvocationShape::default()
        }
    }
}

/// Which top-level unwind ended a run, as the engine's driver
/// adjudicated it (unixwin/niubash#863 GAP-1). GNU's
/// parse_and_execute catches both unwind classes at its own
/// setjmp and re-raises them into the reader's top level
/// (evalstring.c:396-403, :618-619); a host driving a live
/// shell must honor both, but they are not the same class —
/// [`Flow`] folds the errexit break into
/// [`Flow::Completed`], so this decomposition is the only
/// contract surface that separates them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunUnwind {
    /// A real `exit` jump (or a fatal function error's)
    /// ended the run; [`RunOutcome::status`] is the jump's
    /// own status. A jump riding a parse error is a reader
    /// discard, not this class (evalstring.c:585-601).
    ExitJump,
    /// The errexit break ended the run: a command failed
    /// under `set -e` outside every exemption — including a
    /// failed parse status tripping the same predicate.
    /// [`RunOutcome::status`] is the failing status.
    ErrexitBreak,
}

/// How a run was classified by the engine's drivers, as
/// values (unixwin/niubash#863 GAP-1): the classification
/// hosts used to re-derive from the executor's pendings
/// (`take_parse_error`, `take_exit_jump_pending`,
/// `take_signal_discard_pending`, `last_command_inverted`)
/// after every run. Every field is a verdict the driver
/// itself computed for this run — the session layer reports
/// them, it does not recompute them.
///
/// `#[non_exhaustive]` so the reported set can grow within a
/// minor version, exactly as [`RunOutcome`]'s can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RunClassification {
    /// The run's terminal unit ended on a parse error. On
    /// its own this is GNU's DISCARD class — the failing unit
    /// is dropped and the session lives (evalstring.c:585-601)
    /// — unless the errexit verdict also fired (`unwind`).
    pub parse_error: bool,
    /// A signal DISCARD was the run's terminal class: an
    /// untrapped signal death whose status the run already
    /// carries, consumed by the session at the run's end so
    /// it cannot leak into the next run's accounting.
    /// Populated by `run`/`run_line`; the stdin routes leave
    /// the mark to their callers and report `false`.
    pub signal_discard: bool,
    /// A real `exit`/FatalFunctionError jump ended the run's
    /// terminal unit (Q6 arm B: corrected by the executor's
    /// errexit-unwind mark — an errexit break no longer reads
    /// as a jump; arm A reported the raw pending here, which
    /// made a pure errexit break indistinguishable from a
    /// real exit). It can stand alongside `parse_error`, in
    /// which case the jump was a reader discard and did not
    /// count — `unwind` carries the adjudicated form.
    pub exit_jump: bool,
    /// The exit-jump pending exactly as the terminal unit
    /// armed it, before the correction `exit_jump` applies
    /// (GAP-2): either unwind class arms it — a real `exit`
    /// and an errexit break alike (evalstring.c:396-403
    /// re-raises both through the jump machinery) — so a
    /// consumer whose own rule is stated over the pending
    /// itself (the reader-batch feeder's stop rule, a
    /// PROMPT_COMMAND tail read) takes the raw fact as a
    /// value instead of polling the executor's pending by
    /// hand. `exit_jump` remains the adjudicated read; this
    /// is the carrier both reads come from. Populated on
    /// every route that populates `exit_jump`, and by
    /// [`Session::take_classification`]'s drain.
    pub exit_jump_raw: bool,
    /// The errexit verdict for the terminal unit: nonzero
    /// status, errexit in effect, and the `!` inversion
    /// exemption (execute_cmd.c:652-656, CMD_IGNORE_RETURN)
    /// not applying — computed once by
    /// `script_driver::errexit_break_verdict` and populated
    /// on every route (Q6 arm B; arm A's line-offset route
    /// reported `false` unconditionally). A signal discard
    /// does not preempt the verdict: a discarded unit whose
    /// status trips the verdict reports `true` (the discard
    /// abandoned the line; the verdict ends the session).
    pub errexit_break: bool,
    /// The run's last command carried the `!` inversion
    /// (the input to the execute_cmd.c:652-656 exemption).
    pub last_inverted: bool,
    /// Which unwind ended the run, if one did — the grouped
    /// driver's own decomposition of its stop bool. The
    /// reader-batch host rule is: end the session iff
    /// `unwind.is_some()`, with [`RunOutcome::status`].
    pub unwind: Option<RunUnwind>,
}

/// What a run produced (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunOutcome {
    /// The run's final status, including 128 + signal for
    /// signal deaths.
    pub status: i32,
    pub flow: Flow,
    /// Value-form diagnostics the run produced. Parse
    /// diagnostics are produced on every run route (GAP-2):
    /// the same values are delivered to the session's
    /// diagnostic sink as they are collected. The remaining
    /// kinds stay within [`Diagnostic`]'s production
    /// boundary.
    pub diagnostics: Vec<Diagnostic>,
    /// The run's classification (GAP-1): the driver's own
    /// verdicts, including the exit-jump vs errexit-break
    /// decomposition [`Flow`] folds away.
    pub classification: RunClassification,
}

/// What [`Session::close_with_report`] reports (the close
/// boundary as a value, rubash#2039): the exit path's final
/// status plus the session state at the close boundary, read
/// **after** the exit path (EXIT trap, logout rules, the finish
/// machinery's stream-end duties) completed.
///
/// A host whose contract reads session state after close takes
/// it from this value instead of snapshotting before
/// [`Session::close`] — a pre-close snapshot cannot see the
/// EXIT trap's own effects, which is the structural premise the
/// niu cwd funnel of rubash#668 tripped over. `#[non_exhaustive]`
/// so the reported state set can grow within a minor version,
/// exactly as [`RunOutcome`]'s can.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CloseReport {
    /// The final status — the same value [`Session::close`]
    /// returns: the close-time status as the exit path resolved
    /// it. GNU trap.c:1027 `run_exit_trap()` saves the entry
    /// status into `trap_saved_exit_value` (trap.c:1036) and
    /// returns it (trap.c:1052, 1094) — a bare `exit` inside the
    /// EXIT trap keeps the saved value.
    pub status: i32,
    /// The session's working directory at the close boundary,
    /// read after the exit path ran: the same value the §5.3
    /// [`Session::cwd`] face reports, at the one point a
    /// pre-close read cannot reach (the EXIT trap's own `cd` is
    /// the session's last cwd write).
    pub cwd: PathBuf,
}

/// The result of [`Session::parse`]: the tier-B syntax view of
/// the unit (Q4: owned view types, never the internal `Ast`),
/// plus whether the source is incomplete as fed.
#[derive(Debug, Clone)]
pub struct ParsedUnit {
    syntax: SyntaxView,
    needs_more_input: bool,
}

impl ParsedUnit {
    /// The owned syntax view of the parsed unit.
    pub fn syntax(&self) -> &SyntaxView {
        &self.syntax
    }

    /// The dead-line judgment for one line of the unit's file.
    pub fn classify_line(&self, line: u32) -> super::syntax::LineClass {
        self.syntax.classify_line(line)
    }

    /// The definition site of a function the unit defines.
    pub fn function_site(&self, name: &str) -> Option<&super::syntax::FunctionSite> {
        self.syntax.function_site(name)
    }

    /// Whether the source, as fed, is incomplete (an unclosed
    /// quote, compound, or heredoc) — the host should feed more
    /// before running it.
    pub fn needs_more_input(&self) -> bool {
        self.needs_more_input
    }
}

/// One accepted interactive line, after
/// [`Session::prepare_line`] (§8: the tier-A form of
/// `pre_process_interactive_line`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreparedLine {
    /// Execute this text: the history expansion when expansion
    /// modified the line, otherwise the input verbatim.
    Execute(String),
    /// Do not execute the line (expansion error, or the `:p`
    /// print-only modifier). The diagnostic or the printed
    /// expansion has already gone to the error stream and the
    /// history list was fixed the way GNU records it.
    Discard,
}

/// A shell session: the unit a host embeds (API design §5.1).
///
/// Threading contract (Q5, decided 2026-10-25): a `Session` is
/// **single-threaded by construction** — it is `!Send` and
/// `!Sync`, and the API says so in the types instead of letting
/// hosts discover it. Hosts that need sessions on several
/// threads create one session per thread. Multiple sessions
/// per process are supported and independent, except for
/// documented process-global resources (the signal mailbox),
/// where the engine's existing arbitration applies: the first
/// session to enable kernel signal delivery owns it.
///
/// Process-global effects are opt-in (§3.3): a library session
/// is built on the engine's library-role executor; the
/// process-exit role is the builder's explicit flag.
///
/// Closing: [`Session::close`] runs the exit path (EXIT trap,
/// logout rules, the finish machinery's stream-end duties) and
/// returns the final status; [`Session::close_with_report`] is
/// the same close reporting the close-boundary state as a
/// value, for host contracts that would otherwise have to read
/// state after close (rubash#2039). Dropping a session without
/// `close` is defined: no EXIT trap runs and no session
/// finalization happens; the executor's own drop-time
/// bookkeeping (signal-mailbox unregister, and the journaled
/// restore of the session's own process-environment writes —
/// the engine's rubash#182 / G-03 semantics) is unchanged
/// engine behavior, not a session effect.
pub struct Session {
    executor: Executor,
    events: Option<Box<dyn SessionEvents>>,
    diagnostic_sink: Option<Box<dyn DiagnosticSink>>,
    completion_provider: Option<Box<dyn CompletionProvider>>,
    rc_policy: Option<Box<dyn RcPolicy>>,
    rc_override: Option<String>,
    argv0: Option<String>,
    interactive: bool,
    /// The invocation shape this session was built with
    /// (rubash#2130). The executor's typed invocation state is the
    /// live copy engine decisions read; this is the as-built
    /// record the host reads back.
    shape: InvocationShape,
    /// Q5 made literal: whatever the executor's own auto traits
    /// do, a `Session` never becomes `Send`/`Sync` by accident.
    _single_thread: PhantomData<Rc<()>>,
}

impl Session {
    /// Start building a session (§5.1).
    pub fn builder() -> SessionBuilder {
        SessionBuilder::default()
    }

    // ----------------------------------------------------------
    // Execution (§5.2)
    // ----------------------------------------------------------

    /// Run source text and report the outcome as a value.
    ///
    /// The run is driven by the engine's sanctioned driver
    /// entries, routed the way the shipped hosts route: an
    /// unclosed/heredoc batch takes the line-offset source
    /// route; an interactive run, or one whose text uses
    /// history or live aliases, takes the grouped history
    /// driver; anything else takes the line-offset source
    /// route. Parse→transform→execute is **not** part of this
    /// contract: `run` takes source text (the design's §5.2
    /// shape), and a host that must rewrite between parse and
    /// execute has no tier-A seam — the recorded API gap of
    /// the consumption audit (§3.2), unchanged by this
    /// publication.
    pub fn run(&mut self, source: &str, options: &RunOptions) -> Result<RunOutcome, SessionError> {
        let source_name = options.source_name.clone().or_else(|| self.argv0.clone());
        let event = CommandEvent {
            text: source.to_string(),
            source_name: source_name.clone(),
            line_offset: options.line_offset,
        };
        let observing = self.events.is_some();
        let cwd_before = if observing {
            Some(self.executor.state_cwd_owned())
        } else {
            None
        };
        let env_before = if observing {
            Some(self.executor.view_environment())
        } else {
            None
        };
        if let Some(events) = self.events.as_mut() {
            events.before_command(&event);
        }
        let started = Instant::now();

        // GAP-2: the run's parse diagnostics are captured as
        // values from here on; both routes below drain the
        // capture into the outcome (and the diagnostic sink).
        self.executor.arm_run_diagnostics();
        let posix = self.posix_mode();
        let unclosed = crate::lexer::has_unclosed_input_syntax_posix(source, posix)
            || script_driver::heredoc_delimiter_unclosed_quote(source).is_some();
        let alias_live =
            self.executor.alias_expansion_enabled() && !self.executor.aliases_snapshot().is_empty();
        let (status, flow, mut classification, diagnostics) = if !unclosed
            && (options.interactive
                || script_driver::script_uses_history(source)
                || script_driver::script_uses_aliases(source, alias_live))
        {
            let (status, _unwound, facts) =
                script_driver::run_script_with_history_at_line_offset_classified(
                    &mut self.executor,
                    source,
                    options.line_offset,
                    None,
                );
            // Q6 arm B: the flow is a pure map of the driver's
            // adjudicated stop — no Session-layer re-derivation
            // (arm A re-read the errexit predicate here, with a
            // documented boundary bug: a real nonzero `exit`
            // under armed errexit resolved to the errexit arm
            // and folded to Completed). The driver's stop bool
            // (`_unwound`) is the same fact as `facts.stop` and
            // is no longer consulted separately.
            let flow = match facts.stop {
                Some(script_driver::GroupedRunStop::ExitJump) => Flow::ExitRequested(status),
                Some(script_driver::GroupedRunStop::ErrexitBreak) => Flow::ErrexitBreak(status),
                None => Flow::Completed,
            };
            let classification = RunClassification {
                parse_error: facts.parse_error,
                signal_discard: facts.signal_discard,
                exit_jump: facts.exit_jump,
                exit_jump_raw: facts.exit_jump_raw,
                errexit_break: facts.errexit_stop,
                last_inverted: facts.last_inverted,
                unwind: match facts.stop {
                    Some(script_driver::GroupedRunStop::ExitJump) => Some(RunUnwind::ExitJump),
                    Some(script_driver::GroupedRunStop::ErrexitBreak) => {
                        Some(RunUnwind::ErrexitBreak)
                    }
                    None => None,
                },
            };
            let diagnostics = self.drain_run_diagnostics(source_name.as_deref());
            (status, flow, classification, diagnostics)
        } else {
            let status = script_driver::run_source_with_line_offset(
                &mut self.executor,
                source,
                options.interactive,
                options.line_offset,
                None,
                source_name.as_deref(),
            );
            let outcome = self.outcome_after_driver(status, source_name.as_deref());
            (
                outcome.status,
                outcome.flow,
                outcome.classification,
                outcome.diagnostics,
            )
        };
        // A signal DISCARD the run ended on is the run's own
        // bookkeeping (its status already reflects it); consume
        // it so it cannot leak into the next run's accounting —
        // and record it on the outcome's classification (GAP-1),
        // the value hosts used to take from the executor by hand.
        // Q6 arm B: the routes now surface the discard through
        // the classification itself; combine with (never
        // overwrite by) any mark the Session layer still owns.
        classification.signal_discard =
            classification.signal_discard || self.executor.take_signal_discard_pending();
        let elapsed = started.elapsed();

        if let Some(events) = self.events.as_mut() {
            events.after_command(&event, status, elapsed);
            if let Some(before) = cwd_before {
                let now = self.executor.state_cwd_owned();
                if now != before {
                    events.cwd_changed(&now);
                }
            }
            if let Some(before) = env_before {
                let now = self.executor.view_environment();
                let before_map: HashMap<&str, &str> = before
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str()))
                    .collect();
                let now_map: HashMap<&str, &str> = now
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str()))
                    .collect();
                for (name, value) in &now {
                    if before_map.get(name.as_str()) != Some(&value.as_str()) {
                        events.exported_variable_changed(name, Some(value));
                    }
                }
                for (name, _) in &before {
                    if !now_map.contains_key(name.as_str()) {
                        events.exported_variable_changed(name, None);
                    }
                }
            }
            if let Flow::ExitRequested(status) = flow {
                events.exit_requested(status);
            }
        }
        Ok(RunOutcome {
            status,
            flow,
            diagnostics,
            classification,
        })
    }

    /// The interactive-shaped sugar of §5.2: history-expand one
    /// accepted line and, unless the expansion discards it, run
    /// it as an interactive submission.
    pub fn run_line(&mut self, line: &str) -> Result<RunOutcome, SessionError> {
        match self.prepare_line(line) {
            PreparedLine::Discard => Ok(RunOutcome {
                status: self.executor.last_exit_code(),
                flow: Flow::Completed,
                diagnostics: Vec::new(),
                // A history-expansion discard: nothing ran, so
                // no run classification was adjudicated.
                classification: RunClassification::default(),
            }),
            PreparedLine::Execute(text) => self.run(
                &text,
                &RunOptions {
                    source_name: None,
                    line_offset: 0,
                    interactive: true,
                },
            ),
        }
    }

    /// Drive the session from the process's standard input
    /// (the non-interactive stdin driver both binaries use).
    pub fn run_stdin(&mut self) -> Result<RunOutcome, SessionError> {
        self.executor.arm_run_diagnostics();
        let status = script_driver::run_stdin_script(&mut self.executor);
        let source_name = self.argv0.clone();
        Ok(self.outcome_after_driver(status, source_name.as_deref()))
    }

    /// Drive the session interactively from a piped standard
    /// input (the `run_interactive_stdin` route, with its
    /// interactive-history preparation). Marks the session
    /// interactive for [`Session::close`]'s finish duties.
    pub fn run_interactive_stdin(&mut self) -> Result<RunOutcome, SessionError> {
        script_driver::prepare_interactive_history(&mut self.executor);
        self.interactive = true;
        self.executor.arm_run_diagnostics();
        let status = script_driver::run_interactive_stdin(&mut self.executor);
        let source_name = self.argv0.clone();
        Ok(self.outcome_after_driver(status, source_name.as_deref()))
    }

    /// History-expand one interactive line before execution
    /// (§8's tier-A form of `pre_process_interactive_line`).
    pub fn prepare_line(&mut self, line: &str) -> PreparedLine {
        match script_driver::pre_process_interactive_line(&mut self.executor, line) {
            script_driver::InteractiveExpansion::Execute(text) => PreparedLine::Execute(text),
            script_driver::InteractiveExpansion::Discard => PreparedLine::Discard,
        }
    }

    /// Whether `source`, as fed, needs more input before it
    /// can run (§6.5: the `stdin_source_needs_more*` family as
    /// a session query, in the session's posix mode).
    pub fn needs_more_input(&self, source: &str) -> bool {
        script_driver::stdin_source_needs_more_posix(source, self.posix_mode())
    }

    /// Parse source without executing it (§5.2): the returned
    /// unit is the tier-B syntax surface (Q4), not the internal
    /// `Ast`. A source carrying a syntax error fails with the
    /// first error as a [`Diagnostic`] value, which is also
    /// delivered to the session's diagnostic sink.
    pub fn parse(
        &mut self,
        source: &str,
        source_name: Option<&str>,
    ) -> Result<ParsedUnit, ParseFailure> {
        let tokens = crate::lexer::tokenize_with_initial_posix(source, self.posix_mode());
        let ast = crate::parser::parse(&tokens);
        if let Some(node) = ast
            .commands
            .iter()
            .find_map(|command| script_driver::first_parse_error_node(command))
        {
            let diagnostic = diagnostic_from_error_node(&node, source_name);
            if let Some(sink) = self.diagnostic_sink.as_mut() {
                sink.diagnostic(&diagnostic);
            }
            return Err(ParseFailure::Syntax(diagnostic));
        }
        Ok(ParsedUnit {
            syntax: SyntaxView::from_ast(source_name.unwrap_or(""), &ast),
            needs_more_input: script_driver::stdin_source_needs_more(source),
        })
    }

    /// Read-only word expansion (§5.2's expansion helper, in
    /// the shape the engine's own read-only entry has — the
    /// same entry `$BASH_ENV` expansion uses): the expanded
    /// text of one word against live session state. Expansions
    /// whose semantics mutate state are not this entry's
    /// contract; run them.
    pub fn expand_word(&self, word: &str) -> String {
        self.executor.expand_word(word)
    }

    /// The expansion tail of GNU `shell-expand-line` (bashline.c:2982)
    /// after the host's history/alias legs: expand the line's words,
    /// field-split unquoted results on IFS, and rejoin with single
    /// spaces (`expand_word` + `string_list`). No filename globbing —
    /// GNU does not glob word/string expansion. The quoted form of a
    /// variable keeps its spacing; the unquoted form splits.
    pub fn expand_shell_words(&self, line: &str) -> String {
        self.executor.expand_shell_words(line)
    }

    /// Expand a prompt template and deliver the result through
    /// the event observer's prompt channel (§5.4: prompts are
    /// produced here and rendered by the host, never
    /// interleaved into the output stream).
    pub fn expand_prompt(&mut self, template: &str) -> String {
        let prompt = self.executor.expand_prompt_string_mut(template);
        if let Some(events) = self.events.as_mut() {
            events.prompt_ready(&prompt);
        }
        prompt
    }

    /// Run the session's `PROMPT_COMMAND` through the engine's
    /// prompt-command semantics; the command's status when one
    /// ran.
    pub fn run_prompt_command(&mut self) -> Option<i32> {
        self.executor.execute_prompt_command()
    }

    /// Drain the executor's classification pendings as one
    /// value (unixwin/niubash#863 GAP-1), for the session
    /// operations that return no [`RunOutcome`] — above all
    /// [`Session::run_prompt_command`], whose text runner
    /// re-arms the exit jump (rubash#433) for the reader to
    /// honor, and which hosts used to poll through the
    /// executor's `take_exit_jump_pending` by hand.
    ///
    /// This is the raw flag set exactly as the executor holds
    /// it — the same takes, lifted onto the contract; nothing
    /// is adjudicated here. `errexit_break` and `unwind` are
    /// driver verdicts and are only populated on
    /// [`RunOutcome::classification`]; a drain reports them
    /// unset.
    pub fn take_classification(&mut self) -> RunClassification {
        // Q6 arm B: the drained jump read gets the same
        // correction as the outcome routes (an errexit unwind
        // is not a jump). This drain stays a raw fact read —
        // it adjudicates no unwind (the caller's own context
        // owns the tail it drains), as in arm A.
        let exit_jump_raw = self.executor.take_exit_jump_pending();
        let errexit_unwind = self.executor.take_errexit_unwind_armed();
        RunClassification {
            parse_error: self.executor.take_parse_error(),
            signal_discard: self.executor.take_signal_discard_pending(),
            exit_jump: exit_jump_raw && !errexit_unwind,
            exit_jump_raw,
            errexit_break: false,
            last_inverted: self.executor.last_command_inverted(),
            unwind: None,
        }
    }

    // ----------------------------------------------------------
    // Key bindings (niubash#863 GAP-3)
    // ----------------------------------------------------------

    /// The key-binding registry as one owned snapshot
    /// (unixwin/niubash#863 GAP-3): the registry's generation,
    /// editing mode and readline variable table, plus every
    /// registered entry with its canonical bytes (the engine's
    /// own keyseq translation), written form, kind, resolved
    /// keymap and GNU dump line.
    ///
    /// This is the tier-A form of the reads hosts used to make
    /// through the tier-C executor seam (`bind_registry_snapshot`
    /// / `bind_registry_generation` / `bind_editing_mode` /
    /// `bind_readline_variable`) and then re-decode by hand. A
    /// pure read: the view is captured from the registry itself
    /// and taking it mutates nothing — the generation it reports
    /// is the generation it leaves behind.
    pub fn bind_view(&self) -> BindView {
        let registry = self.executor.bind_registry();
        let view = BindView::capture(&registry.borrow());
        view
    }

    // ----------------------------------------------------------
    // Completion (§6.2)
    // ----------------------------------------------------------

    /// Candidates for `line` with the cursor at byte `cursor`:
    /// the engine's programmable completion, with the injected
    /// [`CompletionProvider`]'s candidates merged after (a text
    /// the engine already produced is not duplicated).
    pub fn complete(&mut self, line: &str, cursor: usize) -> Vec<Candidate> {
        self.complete_detailed(line, cursor).candidates
    }

    /// [`Session::complete`] with the attempt's insertion/display
    /// policy (rubash#2573): the governing compspec's `-o` policy
    /// options (nospace / noquote / filenames / fullquote / nosort,
    /// including `compopt` mutations the completion function made)
    /// reach the host as a [`CompletionPolicy`] — the
    /// pcomp_set_readline_variables faces — instead of being dropped
    /// at the engine boundary. Provider candidates merge exactly as
    /// in `complete`; the policy describes the ENGINE's attempt (the
    /// provider has no compspec), so a host applying it to provider
    /// candidates too is making its own choice.
    pub fn complete_detailed(&mut self, line: &str, cursor: usize) -> CompletionOutcome {
        let (texts, policy) = self.executor.complete_line_detailed(line, cursor);
        let mut candidates: Vec<Candidate> = texts.into_iter().map(Candidate::new).collect();
        if let Some(provider) = self.completion_provider.as_mut() {
            for candidate in provider.candidates(line, cursor) {
                if !candidates.iter().any(|known| known.text == candidate.text) {
                    candidates.push(candidate);
                }
            }
        }
        CompletionOutcome { candidates, policy }
    }

    /// Whether a compspec is registered for `command`.
    pub fn has_compspec(&self, command: &str) -> bool {
        self.executor.has_compspec(command)
    }

    /// Open one completion attempt as an explicit object
    /// (unixwin/niubash#863 GAP-7): the contract replacement
    /// for the host's thread-local shell lend. The host opens
    /// a guard per attempt from the `&mut Session` it holds,
    /// drives the attempt's execution faces through the guard,
    /// and reads live state through the guard's [`Session`]
    /// lend for the attempt's duration. Nested attempts are
    /// repeated calls on the same guard; a second guard
    /// cannot exist while this one lives, so the aliasing
    /// hazard the thread-local's reentrancy flag papered over
    /// is a compile error instead of a dark face at runtime.
    pub fn begin_completion_attempt(&mut self) -> CompletionAttempt<'_> {
        CompletionAttempt::new(self)
    }

    /// GNU `pathexp.c:442 shell_glob_filename` over one pattern, for the
    /// readline glob-completion entry point (`bashline.c:4053`
    /// `glob_complete_word` → `bash_glob_completion_internal`). The
    /// executor's pathname expansion supplies the matches, so every glob
    /// option and `GLOBIGNORE` behaves exactly as it does for a command
    /// word. No match (including `failglob`) is an empty list.
    pub fn glob_filename(&self, pattern: &str) -> Vec<String> {
        self.executor.glob_filename(pattern)
    }

    /// One capture of everything a completer reads
    /// (unixwin/niubash#863 GAP-4): the session cwd, the whole
    /// variable table in the executor snapshot's own rendering
    /// — attribute pseudo-entries included — with a content
    /// fingerprint, the function names, and the open fd
    /// numbers, as a [`CompletionView`].
    ///
    /// This is the tier-A form of the reads hosts used to make
    /// through the tier-C executor seam (`env_vars_snapshot` /
    /// `functions_snapshot` / `open_fds` / `session_cwd`) and
    /// then mirror into a snapshot struct of their own,
    /// re-synchronized after every line. A pure read through
    /// `&self`: the view is captured from the executor's own
    /// snapshot functions and taking it mutates nothing.
    pub fn completion_view(&self) -> CompletionView {
        CompletionView::capture(&self.executor)
    }

    // ----------------------------------------------------------
    // Path resolution (§5.2; niubash#863 GAP-5)
    // ----------------------------------------------------------

    /// Resolve a shell-visible path in this session's
    /// namespace (niubash#863 GAP-5): the tier-A form of the
    /// read hosts used to make through the tier-C seam
    /// (`executor().resolve_shell_path(...)`) — the
    /// completer's `/tmp` answer, the GAP-4 view's deliberate
    /// omission, is this face's first consumer. A pure read
    /// through `&self`: the resolution is the executor's own,
    /// fed the session's environment, and the ruled path
    /// contract does not move (bare `/` is the payload root;
    /// the drive spellings are as the engine resolves them).
    pub fn resolve_shell_path(&self, path: &str) -> PathBuf {
        self.executor.resolve_shell_path(path)
    }

    /// Resolve a shell-visible path against an explicit
    /// environment map (niubash#863 GAP-5): the associated
    /// form, for host layers that hold an environment
    /// snapshot but do not own the session (process-plugin
    /// working-directory setup is the recorded case). The
    /// same-source twin of
    /// `Executor::resolve_shell_path_from_env`: both
    /// delegate to the one resolver in `executor::path`, so
    /// the contract and the tier-C form cannot disagree
    /// about a spelling.
    pub fn resolve_shell_path_from_env(path: &str, env_vars: &HashMap<String, String>) -> PathBuf {
        Executor::resolve_shell_path_from_env(path, env_vars)
    }

    /// Resolve one shell PATH entry into the native
    /// directories a child process should receive
    /// (niubash#863 GAP-5): the associated form of
    /// `Executor::resolve_shell_path_process_entries_from_env`,
    /// delegating to the same `executor::path` function —
    /// logical command directories resolve to their real
    /// backing directory below the configured shell root.
    pub fn resolve_shell_path_process_entries_from_env(
        path: &str,
        env_vars: &HashMap<String, String>,
    ) -> Vec<PathBuf> {
        Executor::resolve_shell_path_process_entries_from_env(path, env_vars)
    }

    /// The display-domain form of a path (niubash#863
    /// GAP-5): the spelling `$PWD`, script identities and
    /// dirname compositions share, as one owned String. The
    /// tier-A form of the module function hosts used to call
    /// directly at `rubash::executor::shell_pwd_display_path`;
    /// a pure function of the spelling, no session state.
    pub fn shell_pwd_display_path(path: &str) -> String {
        crate::executor::shell_pwd_display_path(path)
    }

    // ----------------------------------------------------------
    // Functions (§5.2 / §5.3)
    // ----------------------------------------------------------

    /// Call a session function by name with arguments; the
    /// function's exit status.
    pub fn call_function<I, S>(&mut self, name: &str, args: I) -> Result<i32, SessionError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.executor
            .call_function(name, args)
            .map_err(|error| SessionError::from_execute_error(&error))
    }

    /// Call a session function by name with arguments, with a
    /// set of temporary variable bindings applied for the
    /// call's duration (the env channel §8's mapping row covers
    /// for this pair of faces, rubash#2001).
    ///
    /// Each named variable is bound to the given value while
    /// the function runs and restored to its previous state
    /// when the call returns — its old value, or unset if it
    /// was unset — whatever the function itself did to it in
    /// the meantime; the function's other effects on session
    /// state persist, exactly as with [`Session::call_function`].
    /// A name bound to the empty string is set-but-empty for
    /// the call (`${name+x}` expands to `x`), distinct from a
    /// name the channel does not mention, which the call
    /// leaves untouched. This is the shape a host's
    /// completion / widget callbacks use to hand the function
    /// its input variables (niubash's compdef injection of
    /// `NIU_COMP_WORDS` / `NIU_COMP_CWORD`) on the contract
    /// surface itself, without the tier-C seam.
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
        self.executor
            .call_function_with_env(name, args, temporary_env)
            .map_err(|error| SessionError::from_execute_error(&error))
    }

    // ----------------------------------------------------------
    // Cancellation and signals (§6.4)
    // ----------------------------------------------------------

    /// Cooperative cancellation: queue SIGINT for this process
    /// through the engine's delivery machinery — the call a
    /// host's console/Ctrl+C path makes while a run executes.
    ///
    /// Adjudication is the engine's own, and it is GNU's: the
    /// boundary poll of a run in progress dispatches **trapped**
    /// signals to their actions; an interactive session routes
    /// SIGINT through its handler pin (rubash#1662), where an
    /// untrapped delivery unwinds the current input as a
    /// DISCARD and the run ends with status 130 while the
    /// session continues. An untrapped fatal signal in a
    /// non-interactive session is adjudicated by kernel
    /// disposition, never by this queue — a queued copy is
    /// dropped as stale, because its kernel form is what ends
    /// the process. The delivery queue is process-level, so the
    /// injecting call may sit on a peer session; a signal queued
    /// while no run is in progress is consumed by the
    /// idle-input DISCARD discipline and does not carry into
    /// the next run.
    pub fn interrupt(&mut self) {
        self.executor.interrupt();
    }

    /// Parity with an external signal: queue `sig` through the
    /// same delivery machinery as a `kill` to the shell's own
    /// pid, under the adjudication [`Session::interrupt`]
    /// documents — traps dispatch at command boundaries, and
    /// signal deaths that the queue does adjudicate surface in
    /// [`RunOutcome::status`] as 128 + signal.
    pub fn inject_signal(&mut self, sig: i32) {
        self.executor.inject_signal(sig);
    }

    // ----------------------------------------------------------
    // Streams and terminal (§5.4)
    // ----------------------------------------------------------

    /// Bind the process's standard input as this session's
    /// input source (the formal form of the executor's
    /// process-stdin inheritance; also settable at build time).
    pub fn inherit_process_stdin(&mut self) {
        self.executor.inherit_process_stdin();
    }

    /// Tell the session the terminal size changed: `LINES` /
    /// `COLUMNS` rebind to the given dimensions.
    pub fn notify_terminal_size(&mut self, columns: i32, rows: i32) {
        script_driver::bind_terminal_size(&mut self.executor, rows, columns);
    }

    // ----------------------------------------------------------
    // State views (§5.3)
    // ----------------------------------------------------------

    /// All of the §5.3 state views over this session.
    pub fn views(&mut self) -> StateViews<'_> {
        self.executor.state_views()
    }

    /// The variables view: get / set / export / unset /
    /// attributes, with array access through typed cells.
    pub fn variables(&mut self) -> VariablesView<'_> {
        self.executor.variables_view()
    }

    /// The options view: `set -o` and `shopt` options by name,
    /// plus the flag-only shell options (`restricted`), with
    /// validity checked and reported.
    pub fn options(&mut self) -> OptionsView<'_> {
        self.executor.options_view()
    }

    /// What children will inherit: the exported scalar
    /// variables with their values, sorted by name.
    pub fn environment(&self) -> Vec<(String, String)> {
        self.executor.view_environment()
    }

    /// The whole variable table, not just the exported subset
    /// (niubash#863 GAP-6): every variable with its value, plus
    /// the attribute pseudo-entries the executor's snapshot
    /// materializes, presented sorted by name. The `&self`
    /// form of the full-table read hosts used to make through
    /// the tier-C seam (`executor().env_vars_snapshot()` —
    /// the process-PATH synchronization reads the table as
    /// conversion context, exported or not). One source with
    /// [`CompletionView`]'s `env` component — the shared
    /// rendering in `completion_view::full_env_sorted` over
    /// the executor's own snapshot — so this face, the
    /// completion capture and the snapshot itself can never
    /// disagree about the table.
    pub fn environment_full(&self) -> Vec<(String, String)> {
        full_env_sorted(&self.executor)
    }

    /// The stored value of one variable (niubash#863 GAP-6):
    /// the `&self` form of the raw table read hosts made
    /// through the tier-C seam (`executor().get_env(name)`),
    /// for the `&self` host methods (prompt, title and mode
    /// probes) that cannot take the `&mut` variables view.
    /// This is the table's stored string, deliberately NOT
    /// the `$name` expansion read: dynamic getters, nameref
    /// resolution and the typed-cell layer are
    /// [`VariablesView::resolved_value`]'s semantics, and the
    /// two reads differ exactly there (the GAP-6 consumer
    /// pins record both readings per shape).
    pub fn variable_value(&self, name: &str) -> Option<String> {
        self.executor.get_env(name).map(str::to_string)
    }

    /// The session's current working directory.
    /// The invocation shape this session was built with
    /// (rubash#2130), as declared through
    /// [`SessionBuilder::invocation_shape`] /
    /// [`SessionBuilder::login_shell`] /
    /// [`SessionBuilder::interactive`].
    pub fn invocation_shape(&self) -> InvocationShape {
        self.shape
    }

    pub fn cwd(&self) -> &Path {
        self.executor.session_cwd()
    }

    /// Move the session's working directory through the `cd`
    /// machinery; false when the move failed, with the session
    /// left where it was.
    pub fn set_cwd(&mut self, path: &Path) -> bool {
        self.executor.state_views().set_cwd(path)
    }

    /// The session's positional parameters (`$@` as a list).
    pub fn positional(&self) -> Vec<String> {
        self.executor.positional_params()
    }

    /// Replace the positional parameters (the `set --` path).
    pub fn set_positional(&mut self, args: Vec<String>) {
        self.executor.set_positional_params(args);
    }

    /// Alias snapshot: (name, value) pairs, sorted by name.
    pub fn aliases(&self) -> Vec<(String, String)> {
        let mut aliases: Vec<(String, String)> =
            self.executor.aliases_snapshot().into_iter().collect();
        aliases.sort();
        aliases
    }

    /// Function names currently defined, sorted.
    pub fn functions(&self) -> Vec<String> {
        self.executor.functions_snapshot()
    }

    /// Whether a shell function is currently defined, by name
    /// (unixwin/niubash#863 GAP-4b): the `&self` form of the
    /// probe hosts used to make through the tier-C seam
    /// (`executor().has_function(name)`), the only other
    /// contract form being `StateViews::has_function` behind
    /// `views()`'s `&mut self`. Same source either way: the
    /// executor's own function-table lookup.
    pub fn has_function(&self, name: &str) -> bool {
        self.executor.has_function(name)
    }

    /// `$?`, named plainly.
    pub fn last_status(&self) -> i32 {
        self.executor.last_exit_code()
    }

    /// Set `$?`.
    pub fn set_last_status(&mut self, status: i32) {
        self.executor.set_last_exit_code(status);
    }

    // ----------------------------------------------------------
    // Session history (the list, distinct from the provider)
    // ----------------------------------------------------------

    /// The session history list's entries, oldest first, when
    /// a list is installed.
    pub fn history_entries(&self) -> Option<Vec<String>> {
        self.executor
            .get_session_history()
            .map(|history| history.borrow().entries.clone())
    }

    /// Install a session history list holding `entries`
    /// (oldest first), replacing any list the session has.
    pub fn set_history_entries(&mut self, entries: Vec<String>) {
        let mut history = SessionHistory::new();
        history.timestamps = vec![String::new(); entries.len()];
        history.entries = entries;
        self.executor
            .set_session_history(Some(Rc::new(RefCell::new(history))));
    }

    // ----------------------------------------------------------
    // Startup files (the RcPolicy face, §5.5 / §6.6)
    // ----------------------------------------------------------

    /// Source one startup-file class, in order, with the
    /// engine's startup-file semantics: the file list comes
    /// from the injected [`RcPolicy`], or — without one — from
    /// the engine's own invocation lists (the login chain is
    /// empty on Windows, exactly as the shipped binary's is;
    /// the rc chain runs on every platform; the env file is
    /// `$BASH_ENV` resolved). A parse error aborts only its
    /// file; an `exit` in a file ends the chain and is
    /// reported as the outcome's [`Flow::ExitRequested`].
    pub fn source_startup_files(
        &mut self,
        class: StartupClass,
    ) -> Result<RunOutcome, SessionError> {
        // rubash#2130: a session constructed as an engine respawn
        // inherits its parent's post-startup state and runs no
        // startup chain — the API form of the CLI respawn door in
        // main.rs's run_startup_files (execute_cmd.c:1576: a
        // forked subshell never re-sources). The declared shape
        // decides, not an inherited environment mark.
        if self.shape.respawn == RespawnSource::EngineChild {
            return Ok(RunOutcome {
                status: self.executor.last_exit_code(),
                flow: Flow::Completed,
                diagnostics: Vec::new(),
                classification: RunClassification::default(),
            });
        }
        let files = self.startup_files_for(class);
        let mut status = self.executor.last_exit_code();
        // GAP-2: the chain's parse diagnostics accumulate as
        // values across the files, each stamped with its own
        // file's name at its per-file drain.
        self.executor.arm_run_diagnostics();
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        for file in &files {
            status = self.source_one_startup_file(&file.to_string_lossy())?;
            diagnostics.extend(self.drain_run_diagnostics(Some(&file.to_string_lossy())));
            // GAP-1: decompose the file's raw jump with the
            // executor's errexit-unwind record, exactly as the
            // run routes do — a startup file whose last command
            // fails under an errexit it armed itself ends the
            // chain in an errexit break, not an exit jump (the
            // raw read mislabeled that kind; Q6 arm B
            // approximation (iv), resolved). The armed record
            // is drained either way so it cannot leak into the
            // next file's accounting.
            let exit_jump_raw = self.executor.take_exit_jump_pending();
            let errexit_unwind = self.executor.take_errexit_unwind_armed();
            if exit_jump_raw && errexit_unwind {
                return Ok(RunOutcome {
                    status,
                    flow: Flow::ErrexitBreak(status),
                    diagnostics,
                    classification: RunClassification {
                        parse_error: false,
                        signal_discard: false,
                        exit_jump: false,
                        exit_jump_raw,
                        errexit_break: true,
                        last_inverted: self.executor.last_command_inverted(),
                        unwind: Some(RunUnwind::ErrexitBreak),
                    },
                });
            }
            if exit_jump_raw {
                return Ok(RunOutcome {
                    status,
                    flow: Flow::ExitRequested(status),
                    diagnostics,
                    // The chain's own adjudication, recorded as
                    // it is made: the file's jump stood and ends
                    // the chain (per-file parse errors were
                    // consumed as discards inside
                    // source_one_startup_file and are not this
                    // outcome's terminal class).
                    classification: RunClassification {
                        parse_error: false,
                        signal_discard: false,
                        exit_jump: true,
                        exit_jump_raw,
                        errexit_break: false,
                        last_inverted: self.executor.last_command_inverted(),
                        unwind: Some(RunUnwind::ExitJump),
                    },
                });
            }
        }
        Ok(RunOutcome {
            status,
            flow: Flow::Completed,
            diagnostics,
            classification: RunClassification::default(),
        })
    }

    // ----------------------------------------------------------
    // Close (§5.1)
    // ----------------------------------------------------------

    /// Close the session and report the close boundary as a
    /// value (rubash#2039): run the exit path — EXIT trap,
    /// logout rules, and the finish machinery's stream-end
    /// duties (session history write, end-of-stream job
    /// notification flush, persistent process-substitution
    /// close — the duties whose absence in a host was the
    /// consumption audit's data-loss finding) — then return the
    /// final status plus the close-boundary state as a
    /// [`CloseReport`], read after the exit path ran. Consumes
    /// the session, exactly as [`Session::close`] does. This is
    /// the face for host contracts that read session state
    /// after close: a pre-close snapshot cannot see the EXIT
    /// trap's own effects (the rubash#668 cwd funnel's
    /// structural premise).
    pub fn close_with_report(mut self) -> CloseReport {
        let status = self.executor.last_exit_code();
        let status = script_driver::finish_shell(&mut self.executor, status, self.interactive);
        CloseReport {
            status,
            cwd: self.executor.session_cwd().to_path_buf(),
        }
    }

    /// Close the session: run the exit path — EXIT trap,
    /// logout rules, and the finish machinery's stream-end
    /// duties (session history write, end-of-stream job
    /// notification flush, persistent process-substitution
    /// close — the duties whose absence in a host was the
    /// consumption audit's data-loss finding) — and return
    /// the final status. Consumes the session. Hosts that also
    /// need session state after close use
    /// [`Session::close_with_report`] (rubash#2039) instead of
    /// snapshotting before this call.
    pub fn close(self) -> i32 {
        self.close_with_report().status
    }

    // ----------------------------------------------------------
    // Tier-C internals access (NOT part of the tier-A contract)
    // ----------------------------------------------------------

    /// The session's executor, read-only: a **tier-C** seam,
    /// recorded in the API ledger (`docs/api-ledger.md`) and
    /// explicitly outside the tier-A contract's stability
    /// promise. It exists for the faces the contract does not
    /// cover, each recorded in the ledger with its reason: the
    /// tier-C bind registry behind the host's gated bind
    /// surface, the parse→transform→execute routes of the
    /// consumption audit's §3.2 gap (the contract takes source
    /// text only), and the post-build history-provider install
    /// the interactive single-stream discipline requires.
    /// Hosts must prefer the contract faces (the run methods,
    /// the §5.3 views, the capability traits) wherever one
    /// exists; this seam is the recorded remainder, and its
    /// consumer set is expected to shrink, never grow, as the
    /// contract's coverage grows.
    pub fn executor(&self) -> &Executor {
        &self.executor
    }

    /// The mutable form of [`Session::executor`]: the same
    /// tier-C seam, for the recorded faces that must drive or
    /// configure the executor directly.
    pub fn executor_mut(&mut self) -> &mut Executor {
        &mut self.executor
    }

    // ----------------------------------------------------------
    // Internals
    // ----------------------------------------------------------

    fn posix_mode(&self) -> bool {
        self.executor.view_option_enabled("posix").unwrap_or(false)
    }

    /// The post-run flag accounting both shipped hosts poll by
    /// hand, folded into the outcome value: a parse-flagged
    /// run's pending jump is the reader discard, not an exit
    /// (the startup-file rule of rubash#1158, applied to every
    /// driver route); a real exit jump becomes
    /// [`Flow::ExitRequested`]. The outcome's classification
    /// (GAP-1) records the adjudicated reads — `exit_jump` is
    /// the corrected jump (Q6 arm B: an errexit unwind is not a
    /// jump), and `unwind` carries the adjudicated stop kind.
    /// Q6 arm B: this route now computes the same errexit
    /// verdict as the grouped driver through
    /// `script_driver::errexit_break_verdict` (arm A reported
    /// `errexit_break: false` here unconditionally — the
    /// line-route hole); the stdin feeder's own stop rule can
    /// now read the adjudicated fields instead of re-deriving.
    fn outcome_after_driver(&mut self, status: i32, source_name: Option<&str>) -> RunOutcome {
        let parse_error = self.executor.take_parse_error();
        // Q6 arm B: same fact reads and the same adjudication
        // order as the grouped driver — the raw jump corrected
        // by the executor's errexit-unwind mark, the discard
        // record surfaced as a field, and the shared errexit
        // verdict computed (arm A hardcoded `errexit_break:
        // false` on this route: the line-route errexit hole).
        // Precedence, identical to the grouped loop: a real
        // jump that counts (not riding a parse abort) outranks
        // the verdict; a parse abort discards the jump but an
        // armed verdict on the parse status still adjudicates
        // the break (GNU's interactive reader exits on a syntax
        // error under `set -e`); a signal discard does NOT
        // preempt the verdict (it abandons the line; the
        // verdict on the interrupted status decides the
        // session — Q6 pty evidence on GNU 5.3.20).
        let exit_jump_raw = self.executor.take_exit_jump_pending();
        let errexit_unwind = self.executor.take_errexit_unwind_armed();
        let exit_jump = exit_jump_raw && !errexit_unwind;
        let discarded = self.executor.take_unit_signal_discard();
        let errexit_break = script_driver::errexit_break_verdict(&self.executor, status);
        let unwind = if exit_jump && !parse_error {
            Some(RunUnwind::ExitJump)
        } else if errexit_break {
            Some(RunUnwind::ErrexitBreak)
        } else {
            None
        };
        let flow = match unwind {
            Some(RunUnwind::ExitJump) => Flow::ExitRequested(status),
            Some(RunUnwind::ErrexitBreak) => Flow::ErrexitBreak(status),
            None => Flow::Completed,
        };
        RunOutcome {
            status,
            flow,
            diagnostics: self.drain_run_diagnostics(source_name),
            classification: RunClassification {
                parse_error,
                signal_discard: discarded,
                exit_jump,
                exit_jump_raw,
                errexit_break,
                last_inverted: self.executor.last_command_inverted(),
                unwind,
            },
        }
    }

    /// GAP-2: drain the executor's armed run-diagnostics
    /// capture into contract values — one [`Diagnostic`] per
    /// recorded parse diagnostic, stamped with the run's
    /// source name, each also delivered to the session's
    /// diagnostic sink (the delivery [`Session::parse`]
    /// already performs for its own production). An unarmed
    /// or silent run drains empty.
    fn drain_run_diagnostics(&mut self, source_name: Option<&str>) -> Vec<Diagnostic> {
        let records = self.executor.take_run_diagnostics();
        let mut diagnostics = Vec::with_capacity(records.len());
        for record in records {
            let diagnostic = Diagnostic {
                kind: DiagnosticKind::Parse,
                message: record.message,
                source_name: source_name.map(str::to_string),
                line: record.line,
                status: None,
            };
            if let Some(sink) = self.diagnostic_sink.as_mut() {
                sink.diagnostic(&diagnostic);
            }
            diagnostics.push(diagnostic);
        }
        diagnostics
    }

    fn startup_files_for(&mut self, class: StartupClass) -> Vec<PathBuf> {
        if let Some(policy) = self.rc_policy.as_mut() {
            let context = StartupContext {
                home: self.executor.get_env("HOME").map(str::to_string),
                rc_override: self.rc_override.clone(),
                bash_env: crate::invocation::bash_env_file(&self.executor),
            };
            return policy.startup_files(class, &context);
        }
        match class {
            StartupClass::LoginProfile => self.default_login_profile_files(),
            StartupClass::InteractiveRc => {
                let home = self.executor.get_env("HOME").unwrap_or_default();
                if home.is_empty() {
                    return Vec::new();
                }
                crate::invocation::nonlogin_interactive_files(
                    home,
                    self.rc_override.as_deref(),
                    |path| std::fs::metadata(self.executor.resolve_shell_path(path)).is_ok(),
                )
                .into_iter()
                .map(PathBuf::from)
                .collect()
            }
            StartupClass::EnvFile => crate::invocation::bash_env_file(&self.executor)
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        }
    }

    /// The default login chain: the engine's
    /// `invocation::login_profile_files` list.
    #[cfg(not(windows))]
    fn default_login_profile_files(&self) -> Vec<PathBuf> {
        let home = self.executor.get_env("HOME").unwrap_or_default();
        if home.is_empty() {
            return Vec::new();
        }
        crate::invocation::login_profile_files(home, |path| {
            std::fs::metadata(self.executor.resolve_shell_path(path)).is_ok()
        })
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    /// The login chain is a Unix requirement; on Windows there
    /// is no real login concept and the shipped binary's branch
    /// sources nothing (kept deliberately, rubash#1158).
    #[cfg(windows)]
    fn default_login_profile_files(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// One startup file, with the binary's `run_init_file`
    /// semantics (src/main.rs): the sourced-file context is
    /// established for the run and restored afterwards; a
    /// parse error consumes the discard jump so a broken file
    /// cannot end startup, while a real `exit` stays pending
    /// for the caller to see.
    fn source_one_startup_file(&mut self, init_file: &str) -> Result<i32, SessionError> {
        let path = self.executor.resolve_shell_path(init_file);
        let contents =
            script_driver::read_script_bytes(&path).map_err(|error| SessionError::Execution {
                status: Some(1),
                message: format!("{init_file}: {}", crate::posix_errors::message(&error)),
            })?;
        self.executor.set_env("__RUBASH_INTERACTIVE_FLAG_OFF", "1");
        let old_in_source = self
            .executor
            .get_env("__RUBASH_IN_SOURCE")
            .map(str::to_string);
        let old_script_name = self
            .executor
            .get_env("__RUBASH_SCRIPT_NAME")
            .map(str::to_string);
        self.executor.set_env("__RUBASH_IN_SOURCE", "1");
        self.executor.set_env("__RUBASH_SCRIPT_NAME", init_file);
        let _ = self.executor.take_parse_error();
        let status = script_driver::run_source(&mut self.executor, &contents, false);
        if self.executor.take_parse_error() {
            self.executor.take_exit_jump_pending();
        }
        match old_in_source {
            Some(value) => self.executor.set_env("__RUBASH_IN_SOURCE", &value),
            None => self.executor.remove_env("__RUBASH_IN_SOURCE"),
        }
        match old_script_name {
            Some(value) => self.executor.set_env("__RUBASH_SCRIPT_NAME", &value),
            None => self.executor.remove_env("__RUBASH_SCRIPT_NAME"),
        }
        self.executor.remove_env("__RUBASH_INTERACTIVE_FLAG_OFF");
        Ok(status)
    }
}

/// Extract the §6.3 value from the parser's error node: the
/// message is the node's stored marker text (the plain marker
/// verbatim; the near-token and EOF-compound markers in the
/// same wording the text form's report renders from those
/// fields), the line the node's own stamp.
fn diagnostic_from_error_node(
    node: &crate::parser::CommandNode,
    source_name: Option<&str>,
) -> Diagnostic {
    let sep = crate::executor::markers::PARSE_ERROR_FIELD_SEP;
    let (message, line) = if let Some(message) = node.get_assignment("__RUBASH_PARSE_ERROR__") {
        (message.clone(), node.line)
    } else if let Some(spec) = node.get_assignment("__RUBASH_PARSE_ERROR_NEAR__") {
        let mut fields = spec.split(sep);
        let token = fields.next().unwrap_or_default();
        let line = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .or(node.line);
        (
            format!("syntax error near unexpected token `{token}'"),
            line,
        )
    } else if node.get_assignment("__RUBASH_PARSE_ERROR_EOF__").is_some() {
        // Plain end-of-input error (dangling `|`/`&&`/`||`): GNU parse.y
        // yyerror "syntax error: unexpected end of file" at the line after
        // the node's line (deep-audit B6-F1).
        (
            "syntax error: unexpected end of file".to_string(),
            node.line.map(|line| line + 1).or(node.line),
        )
    } else if let Some(spec) = node.get_assignment("__RUBASH_PARSE_ERROR_EOF_COMPOUND__") {
        let mut fields = spec.split(sep);
        let name = fields.next().unwrap_or("(");
        let open_line = fields
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        let eof_line = fields.next().and_then(|value| value.parse::<usize>().ok());
        (
            format!(
                "syntax error: unexpected end of file from `{name}' command on line {open_line}"
            ),
            eof_line.or(node.line),
        )
    } else {
        ("syntax error".to_string(), node.line)
    };
    Diagnostic {
        kind: DiagnosticKind::Parse,
        message,
        source_name: source_name.map(str::to_string),
        line,
        status: None,
    }
}

/// The session builder (§5.1): consuming setters and a fallible
/// `build()` — startup can fail (bad cwd, bad provider) and
/// hosts deserve a `Result`, not a half-initialized executor.
#[derive(Default)]
pub struct SessionBuilder {
    environment: Vec<(String, String)>,
    working_directory: Option<PathBuf>,
    argv0: Option<String>,
    history_provider: Option<SharedHistoryProvider>,
    external_command_handler: Option<Box<dyn ExternalCommandHandler>>,
    #[cfg(windows)]
    elevation_handler: Option<Box<dyn ElevationHandler>>,
    events: Option<Box<dyn SessionEvents>>,
    diagnostic_sink: Option<Box<dyn DiagnosticSink>>,
    completion_provider: Option<Box<dyn CompletionProvider>>,
    rc_policy: Option<Box<dyn RcPolicy>>,
    rc_override: Option<String>,
    process_exit_role: bool,
    login_shell: bool,
    interactive: bool,
    /// The declared invocation shape (rubash#2130); `login_shell`
    /// / `interactive` above write its fields, and `build()`
    /// applies it to the executor after the environment seeds.
    /// `shape_declared` records whether the host used the
    /// structured face at all — undeclared keeps the legacy
    /// per-flag behavior bit for bit.
    shape: InvocationShape,
    shape_declared: bool,
    inherit_process_stdin: bool,
    shell_root: Option<PathBuf>,
    winuxcmd_path: Option<PathBuf>,
    compatible_shell_path: Option<PathBuf>,
    disabled_builtins: Vec<String>,
}

impl SessionBuilder {
    /// Seed a variable (the designed form of the census's
    /// post-hoc `set_env` calls: builder initialization
    /// instead). Later seeds of the same name replace earlier
    /// ones, in order.
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> SessionBuilder {
        self.environment.push((name.into(), value.into()));
        self
    }

    /// Seed a whole environment at once.
    pub fn environment<I, K, V>(mut self, vars: I) -> SessionBuilder
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.environment
            .extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// The session's initial working directory, entered
    /// through the `cd` machinery at build time; a directory
    /// the machinery cannot enter fails the build.
    pub fn working_directory(mut self, path: impl Into<PathBuf>) -> SessionBuilder {
        self.working_directory = Some(path.into());
        self
    }

    /// The session's `$0` / diagnostic-prefix identity.
    pub fn argv0(mut self, name: impl Into<String>) -> SessionBuilder {
        self.argv0 = Some(name.into());
        self
    }

    /// The history capability (the existing trait, unchanged).
    pub fn history_provider(mut self, provider: SharedHistoryProvider) -> SessionBuilder {
        self.history_provider = Some(provider);
        self
    }

    /// The external-command capability (§5.5).
    pub fn external_command_handler(
        mut self,
        handler: Box<dyn ExternalCommandHandler>,
    ) -> SessionBuilder {
        self.external_command_handler = Some(handler);
        self
    }

    /// The elevation capability (Windows; §5.5).
    #[cfg(windows)]
    pub fn elevation_handler(mut self, handler: Box<dyn ElevationHandler>) -> SessionBuilder {
        self.elevation_handler = Some(handler);
        self
    }

    /// The event observer (§5.6).
    pub fn events(mut self, observer: Box<dyn SessionEvents>) -> SessionBuilder {
        self.events = Some(observer);
        self
    }

    /// The diagnostic sink (§6.3).
    pub fn diagnostic_sink(mut self, sink: Box<dyn DiagnosticSink>) -> SessionBuilder {
        self.diagnostic_sink = Some(sink);
        self
    }

    /// The completion capability (§5.5).
    pub fn completion_provider(mut self, provider: Box<dyn CompletionProvider>) -> SessionBuilder {
        self.completion_provider = Some(provider);
        self
    }

    /// The rc policy (§5.5): the host decides startup
    /// sourcing. Without one, the engine's own invocation
    /// lists decide.
    pub fn rc_policy(mut self, policy: Box<dyn RcPolicy>) -> SessionBuilder {
        self.rc_policy = Some(policy);
        self
    }

    /// The `--rcfile` replacement for the personal-rc slot,
    /// consulted by the default startup lists.
    pub fn rc_file(mut self, path: impl Into<String>) -> SessionBuilder {
        self.rc_override = Some(path.into());
        self
    }

    /// The process-exit role (§5.1, §3.3): this session is
    /// the process's outermost shell — its executor takes the
    /// process-exit form (no birth environment snapshot, the
    /// drop-time restore and mailbox-marker removal skipped
    /// because the process dies with it). Default off:
    /// library sessions are inert to the process.
    pub fn process_exit_role(mut self, role: bool) -> SessionBuilder {
        self.process_exit_role = role;
        self
    }

    /// Mark the session a login shell at build time (the
    /// builder-time form of the session-configuration family).
    pub fn login_shell(mut self, login: bool) -> SessionBuilder {
        self.login_shell = login;
        self.shape.login = login;
        self.shape_declared = true;
        self
    }

    /// The session is an interactive shell: the flag
    /// [`Session::close`] passes to the finish machinery.
    pub fn interactive(mut self, interactive: bool) -> SessionBuilder {
        self.interactive = interactive;
        self.shape.interactive = interactive;
        self.shape_declared = true;
        self
    }

    /// Declare the session's invocation shape as one structured
    /// construction parameter (rubash#2130): interactivity, login
    /// shape, and respawn source as construction facts, applied
    /// to the executor's typed invocation state at `build()` —
    /// after the environment seeds, so the declaration, not a
    /// stray seed of the `__RUBASH_INTERACTIVE` /
    /// `__RUBASH_LOGIN_SHELL` carriers, is authoritative. This
    /// supersedes the per-flag [`SessionBuilder::login_shell`] /
    /// [`SessionBuilder::interactive`] methods (it also sets
    /// them, so `close` semantics and
    /// [`Session::invocation_shape`] read back the same shape);
    /// hosts should use one face or the other, not mix them.
    pub fn invocation_shape(mut self, shape: InvocationShape) -> SessionBuilder {
        self.shape = shape;
        self.shape_declared = true;
        self.login_shell = shape.login;
        self.interactive = shape.interactive;
        self
    }

    /// Bind the process's standard input as the session's
    /// input source at build time (§5.4's stream binding, in
    /// the form of the executor's existing inheritance face).
    pub fn inherit_process_stdin(mut self, inherit: bool) -> SessionBuilder {
        self.inherit_process_stdin = inherit;
        self
    }

    /// Session configuration, builder-time (the API ledger's
    /// session-configuration family): the shell root the
    /// session's path resolution anchors to.
    pub fn shell_root(mut self, root: impl Into<PathBuf>) -> SessionBuilder {
        self.shell_root = Some(root.into());
        self
    }

    /// Session configuration: the WinuxCmd bundle path.
    pub fn winuxcmd_path(mut self, path: impl Into<PathBuf>) -> SessionBuilder {
        self.winuxcmd_path = Some(path.into());
        self
    }

    /// Session configuration: the compatible shell path.
    pub fn compatible_shell_path(mut self, path: impl Into<PathBuf>) -> SessionBuilder {
        self.compatible_shell_path = Some(path.into());
        self
    }

    /// Session configuration: a builtin the session disables
    /// (e.g. a host that routes `sudo` itself).
    pub fn disable_builtin(mut self, name: impl Into<String>) -> SessionBuilder {
        self.disabled_builtins.push(name.into());
        self
    }

    /// Build the session. Fails when a builder input is one
    /// the engine refuses (today: a working directory the
    /// `cd` machinery cannot enter).
    pub fn build(self) -> Result<Session, SessionError> {
        let mut executor = if self.process_exit_role {
            Executor::new_process_exit()
        } else {
            Executor::new()
        };
        if let Some(argv0) = &self.argv0 {
            executor.set_env("__RUBASH_SCRIPT_NAME", argv0);
        }
        for (name, value) in &self.environment {
            executor.set_env(name, value);
        }
        if let Some(root) = &self.shell_root {
            executor.set_shell_root(root);
        }
        if let Some(path) = &self.winuxcmd_path {
            executor.set_winuxcmd_path(path);
        }
        if let Some(path) = &self.compatible_shell_path {
            executor.set_compatible_shell_path(path);
        }
        for name in &self.disabled_builtins {
            executor.set_builtin_disabled(name, true);
        }
        if self.login_shell {
            executor.mark_login_shell();
        }
        if self.shape_declared {
            // rubash#2130: the structured declaration is applied
            // last — it is authoritative for the executor's typed
            // invocation state and its projection carriers.
            executor.set_invocation_shape(self.shape.interactive, self.shape.login);
        }
        if let Some(provider) = self.history_provider {
            executor.set_history_provider(provider);
        }
        if let Some(handler) = self.external_command_handler {
            let handler = RefCell::new(handler);
            executor.set_host_external_command_handler(move |argv, env| {
                handler.borrow_mut().handle_command(argv, env)
            });
        }
        #[cfg(windows)]
        if let Some(handler) = self.elevation_handler {
            let handler = RefCell::new(handler);
            executor.set_elevation_handler(move |request| handler.borrow_mut().elevate(request));
        }
        if self.inherit_process_stdin {
            executor.inherit_process_stdin();
        }
        if let Some(dir) = &self.working_directory {
            if !executor.state_views().set_cwd(dir) {
                return Err(SessionError::build(format!(
                    "cannot enter working directory {}",
                    dir.display()
                )));
            }
        }
        Ok(Session {
            executor,
            events: self.events,
            diagnostic_sink: self.diagnostic_sink,
            completion_provider: self.completion_provider,
            rc_policy: self.rc_policy,
            rc_override: self.rc_override,
            argv0: self.argv0,
            interactive: self.interactive,
            shape: self.shape,
            _single_thread: PhantomData,
        })
    }
}
