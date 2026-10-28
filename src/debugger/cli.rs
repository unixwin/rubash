//! The interactive `--debug` frontend (PLAN row 11; debugger
//! design sections 2, 6.1, 7.6, 9.3, 10 M1, 13; PLAN section 3).
//!
//! A pure view over the session core, exactly like the DAP
//! adapter: it translates terminal command lines into session
//! operations and renders the session's own snapshots (the row-6
//! views, through `super::inspect`) back. It owns no stop logic,
//! no breakpoint state, and no second rendering of anything.
//!
//! The command stream is an engine-level injectable (design
//! section 14, Q5 ruling (a)): the binary supplies the
//! controlling terminal's stream, a host embedding this crate
//! may supply its own, and the test harness supplies a scripted
//! stream whose session is recorded as a golden transcript. The
//! debuggee's stdio is never the command stream (section 9.5):
//! the debuggee keeps the process's real stdin/stdout/stderr,
//! and everything this frontend prints goes to its own output
//! sink — stderr in the binary — so a piped stdout stays
//! parseable.
//!
//! `quit` (and EOF on the command stream) ends the session
//! through the debuggee's exit path, exactly as an un-debugged
//! run ends (section 9.3); `kill` terminates the session
//! without the exit path and reports the debuggee as killed —
//! the same `die` + resume discipline the DAP adapter's
//! `terminate` uses, so the two frontends' lifecycle semantics
//! are one implementation, not two.

use std::io::{BufRead, BufReader, Write};

use super::breakpoints::{BreakpointKind, BreakpointResolution, BreakpointTarget};
use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::evaluation::{EvaluationRequest, EvaluationResult, PowerMode};
use super::event::{DebugEvent, ExceptionKind, StopInfo, StopReason};
use super::inspect::{
    render_dynamic_state, render_info_args, render_variables, render_where, VariablePartition,
};
use super::session::{DebugSession, StepArming, EXCEPTION_FILTER_IDS};
use crate::executor::Executor;

/// The launch preset for a `--debug` run: the program operand
/// and its arguments, as the binary's leading scan split them.
#[derive(Debug, Clone, Default)]
pub struct DebugCliPreset {
    /// The script to debug (required; the frontend refuses
    /// without one, per design section 2's batch rule).
    pub program: Option<String>,
    /// Arguments passed to the program as positional parameters.
    pub args: Vec<String>,
}

/// Run a `--debug` session on the controlling terminal.
///
/// The command stream is the terminal device (`/dev/tty`, resp.
/// the Windows console input device); when no terminal device
/// opens and stdin is not itself a terminal, the session refuses
/// to start and says why (design section 2) — batch driving is
/// the `source`-file form, not a piped debugger.
pub fn run_debug_cli(preset: DebugCliPreset) -> i32 {
    let Some(input) = terminal_command_stream() else {
        eprintln!(
            "rubash: --debug: no controlling terminal for the debugger \
             command stream (batch driving is source-file based)"
        );
        return 2;
    };
    run_debug_cli_with_stream(preset, input, Box::new(std::io::stderr()))
}

/// Run a `--debug` session over an injected command stream —
/// the Q5 (a) injection point. Hosts (and the transcript
/// harness) supply the stream; production supplies the terminal
/// through [`run_debug_cli`].
pub fn run_debug_cli_with_stream(
    preset: DebugCliPreset,
    input: Box<dyn BufRead>,
    output: Box<dyn Write>,
) -> i32 {
    let Some(program) = preset.program.clone() else {
        let mut output = output;
        let _ = writeln!(output, "rubash: --debug: a script operand is required");
        return 2;
    };
    // The program's canonical identity: the one key the session
    // core registers and queries under (row 11 convergence).
    let identity = super::file_identity(&program);
    if !std::path::Path::new(&identity).is_file() {
        let mut output = output;
        let _ = writeln!(output, "rubash: --debug: cannot open program '{program}'");
        return 2;
    }

    let mut session = DebugSession::new();
    // Pre-register the program's syntax view, parsed from the
    // file itself, so breakpoints set at the entry stop resolve
    // immediately (the DAP adapter's launch discipline).
    if let Ok(text) = crate::script_driver::read_script_bytes(std::path::Path::new(&identity)) {
        let tokens = crate::lexer::tokenize(&text);
        let ast = crate::parser::parse(&tokens);
        session.register_syntax_view(crate::api::syntax::SyntaxView::from_ast(&identity, &ast));
    }

    let stop_loop = CliStopLoop::new(input, output, identity.clone());
    let driver = SessionDriver::new(session, Box::new(stop_loop));
    let mut executor = Executor::new();
    executor.install_debug_session(driver);

    // The run input: source the program with its arguments as
    // positionals — one process, one session (design 6.3), the
    // same input the DAP run builds.
    let mut input = String::from(". ");
    input.push_str(&super::dap::shell_single_quote(&identity));
    for arg in &preset.args {
        input.push(' ');
        input.push_str(&super::dap::shell_single_quote(arg));
    }
    input.push('\n');
    let status = crate::script_driver::run_source(&mut executor, &input, false);

    // The session's own ending wins over the driver's raw status
    // (a `quit` unwinds with the session's exit status; a `kill`
    // with 128 + signal) — the DAP epilogue's rule.
    let mut final_status = status;
    if let Some(driver) = executor.debug_driver_mut() {
        for event in driver.session_mut().drain_events() {
            match event {
                DebugEvent::SessionExited { status } => final_status = status,
                DebugEvent::DebuggeeDied { signal } => final_status = 128 + signal,
                _ => {}
            }
        }
    }
    final_status
}

/// The controlling terminal as a command stream, when one can be
/// opened; stdin when stdin itself is a terminal; else None.
fn terminal_command_stream() -> Option<Box<dyn BufRead>> {
    #[cfg(unix)]
    {
        if let Ok(file) = std::fs::File::open("/dev/tty") {
            return Some(Box::new(BufReader::new(file)));
        }
    }
    #[cfg(windows)]
    {
        if let Ok(file) = std::fs::File::open("CONIN$") {
            return Some(Box::new(BufReader::new(file)));
        }
    }
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Some(Box::new(BufReader::new(std::io::stdin())));
    }
    None
}

/// What an evaluation the loop asked for was for, so its outcome
/// renders in the right shape when the serve re-enters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingEval {
    /// `print`: render the produced value.
    Print,
    /// `eval` / `set …`: the side effect is the point; failures
    /// and rejections render, success is quiet (plus any output
    /// the evaluation itself produced, which the engine wrote to
    /// the debuggee's real streams already).
    Quiet,
}

/// The CLI stop loop: presentation state only (the selected
/// frame, the list size, the pending evaluation's purpose) —
/// every fact about the debuggee lives in the session.
struct CliStopLoop {
    input: Box<dyn BufRead>,
    out: Box<dyn Write>,
    /// The program's canonical identity (the entry stop carries
    /// no location; `break` with no file means this file).
    program: String,
    /// The selected frame index into the stop's snapshot.
    frame: usize,
    /// `list` window size (`show`/`set listsize`).
    listsize: usize,
    /// Set when the previous serve asked for an evaluation: the
    /// re-entered serve renders its outcome before reading more
    /// commands.
    pending_eval: Option<PendingEval>,
    /// Source text cache for `list`, by file identity.
    sources: std::collections::HashMap<String, Vec<String>>,
}

impl CliStopLoop {
    fn new(input: Box<dyn BufRead>, out: Box<dyn Write>, program: String) -> Self {
        Self {
            input,
            out,
            program,
            frame: 0,
            listsize: 10,
            pending_eval: None,
            sources: std::collections::HashMap::new(),
        }
    }

    fn line(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
        let _ = self.out.flush();
    }

    /// The current stop's file (engine spelling) and line, if the
    /// stop carries a location.
    fn stop_location(session: &DebugSession) -> Option<(String, u32)> {
        session
            .last_stop()
            .and_then(|stop| stop.location.as_ref())
            .map(|loc| (loc.file.clone(), loc.line))
    }

    /// Render the stop banner: reason, location, the frozen
    /// command text, and the source line when it can be read.
    fn banner(&mut self, session: &DebugSession) {
        let stop = session.last_stop().cloned().unwrap_or_else(StopInfo::entry);
        let reason = match &stop.reason {
            StopReason::Entry => "entry".to_string(),
            StopReason::Breakpoint(id) => format!("breakpoint {}", id.number()),
            StopReason::FunctionBreakpoint(id) => {
                format!("function breakpoint {}", id.number())
            }
            StopReason::Step => "step".to_string(),
            StopReason::Watchpoint(id) => format!("watchpoint {}", id.number()),
            StopReason::Pause => "pause".to_string(),
            StopReason::Exception(kind) => {
                let kind = match kind {
                    ExceptionKind::ErrTrap => "ERR trap",
                    ExceptionKind::Errexit => "errexit",
                    ExceptionKind::ParseError => "parse error",
                };
                format!("exception ({kind})")
            }
        };
        match &stop.location {
            Some(loc) => {
                let mut text = format!("Stopped ({reason}) at {}:{}", loc.file, loc.line);
                if let Some(command) = &stop.command_text {
                    text.push_str(": ");
                    text.push_str(command);
                }
                self.line(&text);
                if let Some(source) = self.source_line(&loc.file, loc.line) {
                    self.line(&format!("{} => {}", loc.line, source.trim_end()));
                }
            }
            None => {
                self.line(&format!("Stopped ({reason}) — {}", self.program));
            }
        }
    }

    fn source_line(&mut self, file: &str, line: u32) -> Option<String> {
        let lines = self.source_lines(file)?;
        lines.get(line.saturating_sub(1) as usize).cloned()
    }

    fn source_lines(&mut self, file: &str) -> Option<&Vec<String>> {
        if !self.sources.contains_key(file) {
            let text = std::fs::read_to_string(file).ok()?;
            let lines: Vec<String> = text.lines().map(str::to_string).collect();
            self.sources.insert(file.to_string(), lines);
        }
        self.sources.get(file)
    }

    /// Drain and render the session's queued events (watch
    /// banners, resolution changes, exception sites) in the
    /// shapes the DAP adapter's console uses — one vocabulary
    /// for both frontends.
    fn drain_events(&mut self, session: &mut DebugSession) {
        for event in session.drain_events() {
            match event {
                DebugEvent::WatchChanged(change) => {
                    let render = |value: &Option<String>| match value {
                        Some(text) => format!("'{text}'"),
                        None => "unset".to_string(),
                    };
                    self.line(&format!(
                        "watch: {} {} → {}",
                        change.expression,
                        render(&change.old_value),
                        render(&change.new_value)
                    ));
                }
                DebugEvent::BreakpointResolved { id, resolution } => {
                    let text = match &resolution {
                        BreakpointResolution::Verified => "verified".to_string(),
                        BreakpointResolution::Pending => "pending".to_string(),
                        BreakpointResolution::Unverified { reason } => {
                            format!("unverified ({reason})")
                        }
                    };
                    self.line(&format!("breakpoint {}: {text}", id.number()));
                }
                DebugEvent::ExceptionSite(site) => {
                    let kind = match site.kind {
                        ExceptionKind::ErrTrap => "ERR trap",
                        ExceptionKind::Errexit => "errexit",
                        ExceptionKind::ParseError => "parse error",
                    };
                    let location = site
                        .location
                        .map(|loc| format!(" at {}:{}", loc.file, loc.line))
                        .unwrap_or_default();
                    self.line(&format!(
                        "exception site: {kind} (status {}){location}",
                        site.status
                    ));
                }
                _ => {}
            }
        }
    }

    /// Render the outcome of the evaluation the previous serve
    /// asked for.
    fn render_evaluation(&mut self, session: &DebugSession, purpose: PendingEval) {
        let Some(outcome) = session.last_evaluation() else {
            self.line("evaluation produced no outcome");
            return;
        };
        match &outcome.result {
            EvaluationResult::Value(value) => {
                if purpose == PendingEval::Print {
                    self.line(value);
                }
            }
            EvaluationResult::Status(status) => {
                if purpose == PendingEval::Print {
                    self.line(&format!("(status {status})"));
                }
            }
            EvaluationResult::Rejected(diag) | EvaluationResult::Failed(diag) => {
                let message = diag.message.clone();
                self.line(&message);
            }
        }
    }

    fn evaluate(&mut self, mode: PowerMode, text: String, purpose: PendingEval) -> StopLoopOutcome {
        self.pending_eval = Some(purpose);
        StopLoopOutcome::Evaluate(EvaluationRequest {
            mode,
            text,
            frame: Some(self.frame),
        })
    }

    /// End the session through the debuggee's exit path (design
    /// 9.3): the session records the exit with the debuggee's
    /// current status, and the resume the consult then attempts
    /// unwinds the run with exactly that status.
    fn quit(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        let status = session
            .inspection()
            .map(|snapshot| snapshot.dynamic.last_status)
            .unwrap_or(0);
        let _ = session.exit(status);
        StopLoopOutcome::Resume(StepArming::Continue)
    }

    /// Terminate without the exit path (design 9.3's `kill`):
    /// the same die + resume shape as the DAP `terminate`.
    fn kill(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        let _ = session.die(15);
        self.line("killed");
        StopLoopOutcome::Resume(StepArming::Continue)
    }
}

impl StopLoop for CliStopLoop {
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        // Re-entry after an evaluation: the same stop, still
        // parked; render the outcome and keep reading commands.
        if let Some(purpose) = self.pending_eval.take() {
            self.render_evaluation(session, purpose);
            return self.command_loop(session);
        }
        self.frame = 0;
        self.drain_events(session);
        self.banner(session);
        self.command_loop(session)
    }
}

/// Split a command line into (head, rest) on the first run of
/// whitespace.
fn split_head(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.find(char::is_whitespace) {
        Some(index) => (&text[..index], text[index..].trim_start()),
        None => (text, ""),
    }
}

const HELP: &str = "\
commands (design section 10, M1):
  break [line | file:line | function] [if condition]   set a breakpoint
  tbreak …                                             one-shot breakpoint
  enable/disable/delete [id …]                         manage breakpoints
  clear [line | file:line]                             delete breakpoints at a location
  step (s) / step+ / step-                             step into / next in file / leave function
  next (n)                                             step over
  finish                                               run to the frame's return
  return [status]                                      force the frame to return
  continue (c)                                         resume
  quit (q) / kill                                      end the session (exit path / without)
  where (bt) / frame N / up / down                     the call stack
  list [line]                                          show source around a line
  print (p) <expr>                                     evaluate (implicit power)
  eval <command>                                       evaluate (explicit power)
  set args <words…> / set var <name> = <expr>          write back into the frame
  info breakpoints|locals|variables|args|functions|files|exceptions
  exceptions <filter> on|off                           exception filters (WP-15)
  show listsize / set listsize N
  tty                                                  report the command stream
  help                                                 this text";

impl CliStopLoop {
    fn command_loop(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        loop {
            let _ = write!(self.out, "(rubash-dbg) ");
            let _ = self.out.flush();
            let mut raw = String::new();
            match self.input.read_line(&mut raw) {
                Ok(0) | Err(_) => {
                    // EOF on the command stream is `quit`
                    // (design 9.3).
                    let _ = writeln!(self.out);
                    return self.quit(session);
                }
                _ => {}
            }
            let text = raw.trim();
            if text.is_empty() {
                continue;
            }
            if let Some(outcome) = self.command(session, text) {
                return outcome;
            }
        }
    }

    /// Run one command line. `None` keeps the loop reading;
    /// `Some` is the outcome that ends this serve.
    fn command(&mut self, session: &mut DebugSession, text: &str) -> Option<StopLoopOutcome> {
        let (head, rest) = split_head(text);
        match head {
            "help" | "h" => {
                self.line(HELP);
            }
            "break" | "b" => self.set_breakpoint(session, rest, false),
            "tbreak" => self.set_breakpoint(session, rest, true),
            "enable" => self.set_enabled(session, rest, true),
            "disable" => self.set_enabled(session, rest, false),
            "delete" => self.delete(session, rest),
            "clear" => self.clear(session, rest),
            "step" | "s" => return Some(StopLoopOutcome::Resume(StepArming::StepInto)),
            "step+" => {
                // Scope the arming to this stop's file identity.
                let scope = Self::stop_location(session)
                    .map(|(file, _)| super::file_identity(&file))
                    .unwrap_or_else(|| self.program.clone());
                session.set_step_file_scope(Some(scope));
                return Some(StopLoopOutcome::Resume(StepArming::StepInFile));
            }
            "step-" => {
                let depth = session
                    .last_stop()
                    .map(|stop| stop.frame_depth)
                    .unwrap_or(0);
                return Some(StopLoopOutcome::Resume(StepArming::StepOutOfFrame {
                    frame_depth: depth,
                }));
            }
            "next" | "n" => {
                let depth = session
                    .last_stop()
                    .map(|stop| stop.frame_depth)
                    .unwrap_or(0);
                return Some(StopLoopOutcome::Resume(StepArming::StepOver {
                    max_depth: depth,
                }));
            }
            "finish" => {
                let depth = session
                    .last_stop()
                    .map(|stop| stop.frame_depth)
                    .unwrap_or(0);
                return Some(StopLoopOutcome::Resume(StepArming::Finish {
                    frame_depth: depth,
                }));
            }
            "return" => {
                let stop = session.last_stop().cloned().unwrap_or_else(StopInfo::entry);
                if stop.frame_depth == 0 && stop.reason == StopReason::Entry {
                    self.line("return: not inside a function or sourced file");
                    return None;
                }
                let status = if rest.is_empty() {
                    None
                } else {
                    match rest.parse::<i32>() {
                        Ok(value) => Some(value),
                        Err(_) => {
                            self.line(&format!("return: '{rest}' is not a status"));
                            return None;
                        }
                    }
                };
                return Some(StopLoopOutcome::ForceReturn(status));
            }
            "continue" | "c" => return Some(StopLoopOutcome::Resume(StepArming::Continue)),
            "quit" | "q" | "exit" => return Some(self.quit(session)),
            "kill" => return Some(self.kill(session)),
            "where" | "bt" | "backtrace" => match session.inspection() {
                Some(snapshot) => {
                    let rendered = render_where(snapshot);
                    self.line(&rendered);
                }
                None => self.line("where: no inspection snapshot at this stop"),
            },
            "frame" => self.select_frame(session, rest),
            "up" => self.move_frame(session, 1),
            "down" => self.move_frame(session, -1),
            "list" | "l" => self.list(session, rest),
            "print" | "p" => return self.print(session, rest),
            "eval" => {
                if rest.is_empty() {
                    self.line("eval: a command text is required");
                    return None;
                }
                return Some(self.evaluate(
                    PowerMode::Explicit,
                    rest.to_string(),
                    PendingEval::Quiet,
                ));
            }
            "set" => return self.set(session, rest),
            "info" => self.info(session, rest),
            "exceptions" => self.exceptions(session, rest),
            "show" => {
                if rest == "listsize" {
                    self.line(&format!("listsize: {}", self.listsize));
                } else {
                    self.line(&format!("show: unknown setting '{rest}'"));
                }
            }
            "tty" => {
                self.line("tty: the debugger command stream (never the debuggee's stdin)");
            }
            other => {
                self.line(&format!("unknown command '{other}' — try 'help'"));
            }
        }
        None
    }

    fn set_breakpoint(&mut self, session: &mut DebugSession, rest: &str, one_shot: bool) {
        // Optional trailing `if <condition>` (the session's
        // condition machinery, design 5.3).
        let (target, condition) = match rest.find(" if ") {
            Some(index) => (
                rest[..index].trim(),
                Some(rest[index + 4..].trim().to_string()),
            ),
            None => (rest.trim(), None),
        };
        let (id, label) = if target.is_empty() {
            match Self::stop_location(session) {
                Some((file, line)) => {
                    let id = session.set_line_breakpoint(file.clone(), line);
                    (id, format!("{file}:{line}"))
                }
                None => {
                    let id = session.set_line_breakpoint(self.program.clone(), 1);
                    (id, format!("{}:1", self.program))
                }
            }
        } else if let Some((file, line)) = parse_file_line(target) {
            let id = session.set_line_breakpoint(file.to_string(), line);
            (id, format!("{file}:{line}"))
        } else if let Ok(line) = target.parse::<u32>() {
            let file = Self::stop_location(session)
                .map(|(file, _)| file)
                .unwrap_or_else(|| self.program.clone());
            let id = session.set_line_breakpoint(file.clone(), line);
            (id, format!("{file}:{line}"))
        } else {
            let id = session.set_function_breakpoint(target.to_string());
            (id, target.to_string())
        };
        if one_shot {
            if let Some(entry) = session.breakpoints_mut().get_mut(id) {
                entry.one_shot = true;
            }
        }
        if let Some(condition) = condition {
            session.breakpoints_mut().set_condition(id, Some(condition));
        }
        let state = match session.breakpoints().get(id).map(|e| &e.resolution) {
            Some(BreakpointResolution::Verified) => "verified".to_string(),
            Some(BreakpointResolution::Pending) => "pending".to_string(),
            Some(BreakpointResolution::Unverified { reason }) => {
                format!("unverified ({reason})")
            }
            None => "unknown".to_string(),
        };
        let shot = if one_shot { " (one-shot)" } else { "" };
        self.line(&format!(
            "breakpoint {} at {label}: {state}{shot}",
            id.number()
        ));
    }

    fn set_enabled(&mut self, session: &mut DebugSession, rest: &str, enabled: bool) {
        let ids = parse_ids(session, rest);
        for id in ids {
            session.breakpoints_mut().set_enabled(id, enabled);
        }
    }

    fn delete(&mut self, session: &mut DebugSession, rest: &str) {
        if rest.is_empty() {
            session.breakpoints_mut().clear();
            self.line("all breakpoints deleted");
            return;
        }
        for id in parse_ids(session, rest) {
            session.breakpoints_mut().remove(id);
        }
    }

    fn clear(&mut self, session: &mut DebugSession, rest: &str) {
        let (file, line) = if rest.is_empty() {
            match Self::stop_location(session) {
                Some(pair) => pair,
                None => {
                    self.line("clear: no current location");
                    return;
                }
            }
        } else if let Some((file, line)) = parse_file_line(rest) {
            (file.to_string(), line)
        } else if let Ok(line) = rest.parse::<u32>() {
            let file = Self::stop_location(session)
                .map(|(file, _)| file)
                .unwrap_or_else(|| self.program.clone());
            (file, line)
        } else {
            self.line(&format!("clear: cannot parse '{rest}'"));
            return;
        };
        let removed = session.breakpoints_mut().clear_at(&file, line);
        self.line(&format!("cleared {removed} breakpoint(s) at {file}:{line}"));
    }

    fn select_frame(&mut self, session: &DebugSession, rest: &str) {
        let Some(snapshot) = session.inspection() else {
            self.line("frame: no inspection snapshot at this stop");
            return;
        };
        let Ok(index) = rest.parse::<usize>() else {
            self.line(&format!("frame: '{rest}' is not a frame number"));
            return;
        };
        let Some(frame) = snapshot.frames.get(index) else {
            self.line(&format!(
                "frame: only {} frame(s) at this stop",
                snapshot.frames.len()
            ));
            return;
        };
        self.frame = index;
        self.line(&format!(
            "#{index} {} at {}:{}",
            frame.function, frame.source, frame.line
        ));
    }

    fn move_frame(&mut self, session: &DebugSession, delta: i32) {
        let Some(snapshot) = session.inspection() else {
            self.line("frame: no inspection snapshot at this stop");
            return;
        };
        let target = self.frame as i32 + delta;
        if target < 0 || target as usize >= snapshot.frames.len() {
            self.line("frame: no such frame");
            return;
        }
        self.frame = target as usize;
        let frame = &snapshot.frames[self.frame];
        self.line(&format!(
            "#{} {} at {}:{}",
            self.frame, frame.function, frame.source, frame.line
        ));
    }

    fn list(&mut self, session: &DebugSession, rest: &str) {
        let (file, center) = if let Ok(line) = rest.parse::<u32>() {
            let file = Self::stop_location(session)
                .map(|(file, _)| file)
                .unwrap_or_else(|| self.program.clone());
            (file, line)
        } else {
            match Self::stop_location(session) {
                Some(pair) => pair,
                None => (self.program.clone(), 1),
            }
        };
        let Some(lines) = self.source_lines(&file).cloned() else {
            self.line(&format!("list: cannot read {file}"));
            return;
        };
        let half = (self.listsize / 2).max(1);
        let start = center.saturating_sub(half as u32).max(1);
        let end = (center + half as u32).min(lines.len() as u32);
        for number in start..=end {
            let marker = if number == center { "=>" } else { "  " };
            let text = &lines[(number - 1) as usize];
            self.line(&format!("{number:>5} {marker} {text}"));
        }
    }

    fn print(&mut self, session: &DebugSession, rest: &str) -> Option<StopLoopOutcome> {
        let text = rest.trim();
        if text.is_empty() {
            self.line("print: an expression is required");
            return None;
        }
        // A bare variable name renders from the stop's snapshot
        // through the views (arrays whole, unset named) — the
        // same rendering the watch machinery uses.
        if is_bare_name(text) {
            if let Some(snapshot) = session.inspection() {
                if let Some(variable) = snapshot.variable(text) {
                    let rendered = super::inspect::render_watch_value(variable)
                        .unwrap_or_else(|| "unset".to_string());
                    self.line(&format!("{text} = {rendered}"));
                    return None;
                }
            }
        }
        Some(self.evaluate(PowerMode::Implicit, text.to_string(), PendingEval::Print))
    }

    fn set(&mut self, session: &mut DebugSession, rest: &str) -> Option<StopLoopOutcome> {
        let (head, tail) = split_head(rest);
        match head {
            "args" => {
                let words: Vec<&str> = tail.split_whitespace().collect();
                let mut text = String::from("set --");
                for word in words {
                    text.push(' ');
                    text.push_str(&super::dap::shell_single_quote(word));
                }
                Some(self.evaluate(PowerMode::Explicit, text, PendingEval::Quiet))
            }
            "var" => {
                // `set var <name> = <expr>` desugars to the one
                // write-back path: a real assignment evaluated
                // explicitly (design 7.6).
                let tail = tail.trim();
                let Some(eq) = tail.find('=') else {
                    self.line("set var: expected 'set var <name> = <expression>'");
                    return None;
                };
                let name = tail[..eq].trim();
                let expr = tail[eq + 1..].trim();
                if !is_bare_name(name) || expr.is_empty() {
                    self.line("set var: expected 'set var <name> = <expression>'");
                    return None;
                }
                Some(self.evaluate(
                    PowerMode::Explicit,
                    format!("{name}={expr}"),
                    PendingEval::Quiet,
                ))
            }
            "listsize" => {
                match tail.parse::<usize>() {
                    Ok(value) if value > 0 => self.listsize = value,
                    _ => self.line(&format!("set listsize: '{tail}' is not a positive count")),
                }
                None
            }
            "exceptions" => {
                self.exceptions(session, tail);
                None
            }
            _ => {
                self.line(&format!(
                    "set: unknown setting '{head}' (args | var | listsize | exceptions)"
                ));
                None
            }
        }
    }

    /// The WP-15 CLI face of the exception filters: list the
    /// canonical ids with their selection state, or toggle one.
    /// The store is the session's — the same one the DAP
    /// adapter's `setExceptionBreakpoints` writes.
    fn exceptions(&mut self, session: &mut DebugSession, rest: &str) {
        let rest = rest.trim();
        // The toggle form is `<filter> on|off`, and a filter id
        // may itself contain a space (`set -e`): the state is
        // the LAST word and the id is everything before it.
        // (Row 12: the first-word grammar could never spell the
        // `set -e` id, so the CLI face could not arm that filter
        // at all.) A rest that names a known id whole is the
        // query form; anything else with a second word is the
        // malformed-toggle error the original grammar printed.
        let toggle = match rest.rsplit_once(char::is_whitespace) {
            Some((head, tail)) if tail == "on" || tail == "off" => Some((head.trim(), tail)),
            _ => None,
        };
        let (filter, state) = match toggle {
            Some((filter, state)) => (filter, state),
            None => {
                let mut parts = rest.split_whitespace();
                let Some(filter) = parts.next() else {
                    let selected = session.exception_filters();
                    for id in EXCEPTION_FILTER_IDS {
                        let state = if selected.contains(&id) { "on" } else { "off" };
                        self.line(&format!("exception {id}: {state}"));
                    }
                    return;
                };
                match parts.next() {
                    None => {
                        let on = session.exception_filters().contains(&filter);
                        self.line(&format!(
                            "exception {filter}: {}",
                            if on { "on" } else { "off" }
                        ));
                        return;
                    }
                    Some(state) if !EXCEPTION_FILTER_IDS.contains(&rest) => {
                        self.line(&format!("exceptions: expected on|off, got '{state}'"));
                        return;
                    }
                    Some(_) => {
                        let on = session.exception_filters().contains(&rest);
                        self.line(&format!(
                            "exception {rest}: {}",
                            if on { "on" } else { "off" }
                        ));
                        return;
                    }
                }
            }
        };
        let mut selected: Vec<String> = session
            .exception_filters()
            .iter()
            .map(|id| id.to_string())
            .collect();
        match state {
            "on" => {
                if !selected.iter().any(|id| id == filter) {
                    selected.push(filter.to_string());
                }
            }
            "off" => selected.retain(|id| id != filter),
            other => {
                self.line(&format!("exceptions: expected on|off, got '{other}'"));
                return;
            }
        }
        match session.set_exception_filters(selected) {
            Ok(()) => self.line(&format!("exception {filter}: {state}")),
            Err(unknown) => self.line(&format!(
                "exceptions: unknown filter(s): {}",
                unknown.join(", ")
            )),
        }
    }

    fn info(&mut self, session: &DebugSession, rest: &str) {
        let (head, _) = split_head(rest);
        match head {
            "breakpoints" | "b" => {
                if session.breakpoints().is_empty() {
                    self.line("no breakpoints");
                    return;
                }
                for entry in session.breakpoints().iter() {
                    let target = match &entry.target {
                        BreakpointTarget::Line { file, line } => {
                            format!("{file}:{line}")
                        }
                        BreakpointTarget::Function { name } => format!("function {name}"),
                        BreakpointTarget::Expression { text } => format!("watch {text}"),
                    };
                    let kind = match entry.kind {
                        BreakpointKind::Watch => "watch",
                        _ => "breakpoint",
                    };
                    let resolution = match &entry.resolution {
                        BreakpointResolution::Verified => "verified".to_string(),
                        BreakpointResolution::Pending => "pending".to_string(),
                        BreakpointResolution::Unverified { reason } => {
                            format!("unverified ({reason})")
                        }
                    };
                    let condition = entry
                        .condition
                        .as_deref()
                        .map(|text| format!(" if {text}"))
                        .unwrap_or_default();
                    self.line(&format!(
                        "{} {kind} {target} {} hits={} {resolution}{condition}",
                        entry.id.number(),
                        if entry.enabled { "enabled" } else { "disabled" },
                        entry.hit_count,
                    ));
                }
            }
            "locals" | "variables" => {
                let partition = if head == "locals" {
                    VariablePartition::Locals
                } else {
                    VariablePartition::All
                };
                match session.inspection() {
                    Some(snapshot) => {
                        let rendered = render_variables(snapshot, partition);
                        if rendered.is_empty() {
                            self.line("(none)");
                        } else {
                            self.line(&rendered);
                        }
                    }
                    None => self.line("info: no inspection snapshot at this stop"),
                }
            }
            "args" => match session.inspection() {
                Some(snapshot) => match snapshot.frames.get(self.frame) {
                    Some(frame) => {
                        let rendered = render_info_args(frame);
                        if rendered.is_empty() {
                            self.line("(no arguments)");
                        } else {
                            self.line(&rendered);
                        }
                    }
                    None => self.line("info args: no such frame"),
                },
                None => self.line("info: no inspection snapshot at this stop"),
            },
            "functions" => {
                let mut names: Vec<String> = Vec::new();
                for view in session.syntax_views.values() {
                    for site in &view.functions {
                        names.push(format!(
                            "{} at {}:{}",
                            site.name, view.file, site.definition_line
                        ));
                    }
                }
                for name in &session.defined_functions {
                    names.push(format!("{name} (defined at runtime)"));
                }
                names.sort();
                names.dedup();
                if names.is_empty() {
                    self.line("(no functions known yet)");
                } else {
                    for name in names {
                        self.line(&name);
                    }
                }
            }
            "files" | "source" => {
                let mut files: Vec<&String> = session.syntax_views.keys().collect();
                files.sort();
                if files.is_empty() {
                    self.line("(no files parsed yet)");
                } else {
                    for file in files {
                        self.line(file);
                    }
                }
            }
            "exceptions" => {
                let selected = session.exception_filters();
                for id in EXCEPTION_FILTER_IDS {
                    let state = if selected.contains(&id) { "on" } else { "off" };
                    self.line(&format!("exception {id}: {state}"));
                }
            }
            "state" => match session.inspection() {
                Some(snapshot) => {
                    let rendered = render_dynamic_state(&snapshot.dynamic);
                    self.line(&rendered);
                }
                None => self.line("info: no inspection snapshot at this stop"),
            },
            _ => {
                self.line(&format!(
                    "info: unknown topic '{head}' \
                     (breakpoints|locals|variables|args|functions|files|exceptions|state)"
                ));
            }
        }
    }
}

/// Parse `file:line` (the line is the numeric tail after the
/// last colon, so Windows drive spellings survive).
fn parse_file_line(target: &str) -> Option<(&str, u32)> {
    let (file, tail) = target.rsplit_once(':')?;
    let line: u32 = tail.parse().ok()?;
    if file.is_empty() {
        return None;
    }
    Some((file, line))
}

/// Ids named by a command tail; empty means every entry.
fn parse_ids(session: &DebugSession, rest: &str) -> Vec<super::breakpoints::BreakpointId> {
    if rest.is_empty() {
        return session.breakpoints().iter().map(|entry| entry.id).collect();
    }
    rest.split_whitespace()
        .filter_map(|word| word.parse::<u32>().ok())
        .filter_map(|number| {
            session
                .breakpoints()
                .iter()
                .find(|entry| entry.id.number() == number)
                .map(|entry| entry.id)
        })
        .collect()
}

fn is_bare_name(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
