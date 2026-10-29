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

#[cfg(windows)]
use super::capabilities::ElevationHandler;
use super::capabilities::{
    Candidate, CompletionProvider, DiagnosticSink, ExternalCommandHandler, RcPolicy, StartupClass,
    StartupContext,
};
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

/// What a run produced (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunOutcome {
    /// The run's final status, including 128 + signal for
    /// signal deaths.
    pub status: i32,
    pub flow: Flow,
    /// Value-form diagnostics the run produced. In this
    /// publication step this is empty for `run()` — see
    /// [`Diagnostic`]'s production boundary.
    pub diagnostics: Vec<Diagnostic>,
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
/// returns the final status. Dropping a session without
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

        let posix = self.posix_mode();
        let unclosed = crate::lexer::has_unclosed_input_syntax_posix(source, posix)
            || script_driver::heredoc_delimiter_unclosed_quote(source).is_some();
        let alias_live =
            self.executor.alias_expansion_enabled() && !self.executor.aliases_snapshot().is_empty();
        let (status, flow) = if !unclosed
            && (options.interactive
                || script_driver::script_uses_history(source)
                || script_driver::script_uses_aliases(source, alias_live))
        {
            let (status, unwound) = script_driver::run_script_with_history_at_line_offset_tracked(
                &mut self.executor,
                source,
                options.line_offset,
                None,
            );
            // The grouped driver consumes the jump flags and
            // reports one stop bool that conflates a real exit
            // with the errexit break; separate them with the
            // driver's own published ingredients (its errexit
            // predicate, re-read at the run's end). Known
            // boundary, inherited from the driver's
            // classification: a run that both requests exit
            // with a nonzero status and satisfies the errexit
            // predicate resolves to the errexit arm.
            let errexit_stop = status != 0
                && script_driver::stdin_script_errexit_enabled(&self.executor)
                && !self.executor.last_command_inverted();
            let flow = if unwound && !errexit_stop {
                Flow::ExitRequested(status)
            } else {
                Flow::Completed
            };
            (status, flow)
        } else {
            let status = script_driver::run_source_with_line_offset(
                &mut self.executor,
                source,
                options.interactive,
                options.line_offset,
                None,
                source_name.as_deref(),
            );
            let outcome = self.outcome_after_driver(status);
            (outcome.status, outcome.flow)
        };
        // A signal DISCARD the run ended on is the run's own
        // bookkeeping (its status already reflects it); consume
        // it so it cannot leak into the next run's accounting.
        let _ = self.executor.take_signal_discard_pending();
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
            diagnostics: Vec::new(),
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
        let status = script_driver::run_stdin_script(&mut self.executor);
        Ok(self.outcome_after_driver(status))
    }

    /// Drive the session interactively from a piped standard
    /// input (the `run_interactive_stdin` route, with its
    /// interactive-history preparation). Marks the session
    /// interactive for [`Session::close`]'s finish duties.
    pub fn run_interactive_stdin(&mut self) -> Result<RunOutcome, SessionError> {
        script_driver::prepare_interactive_history(&mut self.executor);
        self.interactive = true;
        let status = script_driver::run_interactive_stdin(&mut self.executor);
        Ok(self.outcome_after_driver(status))
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

    // ----------------------------------------------------------
    // Completion (§6.2)
    // ----------------------------------------------------------

    /// Candidates for `line` with the cursor at byte `cursor`:
    /// the engine's programmable completion, with the injected
    /// [`CompletionProvider`]'s candidates merged after (a text
    /// the engine already produced is not duplicated).
    pub fn complete(&mut self, line: &str, cursor: usize) -> Vec<Candidate> {
        let mut candidates: Vec<Candidate> = self
            .executor
            .complete_line(line, cursor)
            .into_iter()
            .map(Candidate::new)
            .collect();
        if let Some(provider) = self.completion_provider.as_mut() {
            for candidate in provider.candidates(line, cursor) {
                if !candidates.iter().any(|known| known.text == candidate.text) {
                    candidates.push(candidate);
                }
            }
        }
        candidates
    }

    /// Whether a compspec is registered for `command`.
    pub fn has_compspec(&self, command: &str) -> bool {
        self.executor.has_compspec(command)
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
    /// with validity checked and reported.
    pub fn options(&mut self) -> OptionsView<'_> {
        self.executor.options_view()
    }

    /// What children will inherit: the exported scalar
    /// variables with their values, sorted by name.
    pub fn environment(&self) -> Vec<(String, String)> {
        self.executor.view_environment()
    }

    /// The session's current working directory.
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
        let files = self.startup_files_for(class);
        let mut status = self.executor.last_exit_code();
        for file in &files {
            status = self.source_one_startup_file(&file.to_string_lossy())?;
            if self.executor.take_exit_jump_pending() {
                return Ok(RunOutcome {
                    status,
                    flow: Flow::ExitRequested(status),
                    diagnostics: Vec::new(),
                });
            }
        }
        Ok(RunOutcome {
            status,
            flow: Flow::Completed,
            diagnostics: Vec::new(),
        })
    }

    // ----------------------------------------------------------
    // Close (§5.1)
    // ----------------------------------------------------------

    /// Close the session: run the exit path — EXIT trap,
    /// logout rules, and the finish machinery's stream-end
    /// duties (session history write, end-of-stream job
    /// notification flush, persistent process-substitution
    /// close — the duties whose absence in a host was the
    /// consumption audit's data-loss finding) — and return
    /// the final status. Consumes the session.
    pub fn close(mut self) -> i32 {
        let status = self.executor.last_exit_code();
        script_driver::finish_shell(&mut self.executor, status, self.interactive)
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
    /// [`Flow::ExitRequested`].
    fn outcome_after_driver(&mut self, status: i32) -> RunOutcome {
        let flow = if self.executor.take_parse_error() {
            self.executor.take_exit_jump_pending();
            Flow::Completed
        } else if self.executor.take_exit_jump_pending() {
            Flow::ExitRequested(status)
        } else {
            Flow::Completed
        };
        RunOutcome {
            status,
            flow,
            diagnostics: Vec::new(),
        }
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
        self
    }

    /// The session is an interactive shell: the flag
    /// [`Session::close`] passes to the finish machinery.
    pub fn interactive(mut self, interactive: bool) -> SessionBuilder {
        self.interactive = interactive;
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
            _single_thread: PhantomData,
        })
    }
}
