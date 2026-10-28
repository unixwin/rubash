//! The DAP frontend (debugger design section 13; PLAN row 10,
//! WP-12/13/14).
//!
//! `--debug` and `--dap` are two frontends of ONE debug session
//! (the form the user settled on 2026-10-25): this module is a
//! protocol adapter over the landed session core. It owns no
//! debugging semantics — no second state machine, no private
//! breakpoint or watch state. Stops, stepping, evaluation, and
//! write-back all go through the session core's own machinery
//! (the stepping predicates, the section 7.5 evaluation
//! checker, the row-6 state views); the adapter translates
//! between framed DAP messages and session operations, and
//! renders the session's snapshots into DAP shapes. Where a
//! rendering needs something the views do not offer, the gap
//! belongs to the session layer (WP-13 reuse discipline), not
//! to a workaround here.
//!
//! Layout, following the design's frontend shape:
//!
//! - transport: Content-Length framed JSON on two streams,
//!   with a reader thread that only frames and parses — plus
//!   one sanctioned side channel: a `pause` request that
//!   arrives while the debuggee runs free is applied to the
//!   session's asynchronous stop flag (the design's one
//!   cross-thread surface) instead of queueing behind a run
//!   that may never stop.
//! - protocol: the request set of section 13, answered against
//!   the session; capabilities are claimed only where the
//!   underlying feature has landed.
//! - rendering: session events (DE-01..DE-12) and inspection
//!   snapshots translated to DAP events and objects.
//!
//! The debuggee's stdio never touches the protocol streams:
//! the run binds the executor's fd 0 to an empty file and its
//! fd 1/2 to capture files (see
//! `Executor::debugger_bind_stdio`), and the adapter drains
//! the captures into `output` events.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use super::breakpoints::{BreakpointKind, BreakpointResolution, BreakpointTarget};
use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::evaluation::{EvaluationRequest, EvaluationResult, PowerMode};
use super::event::{DebugEvent, ExceptionKind, OutputStream, StopReason};
use super::json::Json;
use super::session::{DebugSession, StepArming};
use crate::api::views::{VariableSnapshot, VariableValue};
use crate::executor::Executor;

/// The one thread a DAP session ever reports (design section
/// 13: single thread plus synthesized frames, consensus D3).
const THREAD_ID: i64 = 1;

/// Signal the adapter terminates the debuggee with on
/// `terminate`, on a `disconnect` that asks for termination,
/// and when the client vanishes mid-stop: SIGTERM.
const TERMINATE_SIGNAL: i32 = 15;

// ---------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------

/// One inbound protocol message, as framed and parsed by the
/// reader thread.
enum Inbound {
    /// A client request: `(seq, command, arguments)`.
    Request(i64, String, Json),
    /// The stream broke: malformed framing or JSON. No response
    /// is possible (a seq cannot be trusted), so the server
    /// treats this exactly like EOF.
    Broken,
}

/// State shared with the reader thread. The reader thread never
/// touches the session itself; the flag handle is the whole of
/// its access, per the session's cross-thread contract.
struct ReaderBridge {
    /// The running session's asynchronous stop flag; `None`
    /// whenever no debuggee is running.
    session_flag: Mutex<Option<Arc<AtomicBool>>>,
    /// True while the debuggee runs free (not parked at a stop
    /// the adapter is serving).
    running: AtomicBool,
    /// The seq of a `pause` request the reader thread applied
    /// to the stop flag directly; the adapter owes it a
    /// response at the stop that services it.
    pause_reply_seq: Mutex<Option<i64>>,
}

impl ReaderBridge {
    fn new() -> Arc<ReaderBridge> {
        Arc::new(ReaderBridge {
            session_flag: Mutex::new(None),
            running: AtomicBool::new(false),
            pause_reply_seq: Mutex::new(None),
        })
    }
}

/// Spawn the reader thread: frame messages off `reader`,
/// forward them into the channel the server consumes.
fn spawn_reader<R: Read + Send + 'static>(
    reader: R,
    bridge: Arc<ReaderBridge>,
) -> mpsc::Receiver<Inbound> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(reader);
        loop {
            match read_framed(&mut reader) {
                Ok(Some(text)) => match parse_request(&text) {
                    Ok((seq, command, arguments)) => {
                        if command == "pause" && bridge.running.load(Ordering::Acquire) {
                            // A pause during a free run cannot
                            // queue behind the run: apply it to
                            // the session's stop flag (the
                            // boundary poll services it, DE-07)
                            // and remember the response owed.
                            if let Ok(slot) = bridge.session_flag.lock() {
                                if let Some(flag) = slot.as_ref() {
                                    flag.store(true, Ordering::Release);
                                    if let Ok(mut pending) = bridge.pause_reply_seq.lock() {
                                        *pending = Some(seq);
                                    }
                                    continue;
                                }
                            }
                        }
                        if sender
                            .send(Inbound::Request(seq, command, arguments))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(Inbound::Broken);
                        return;
                    }
                },
                Ok(None) => return, // clean EOF: dropping the sender says so
                Err(_) => {
                    let _ = sender.send(Inbound::Broken);
                    return;
                }
            }
        }
    });
    receiver
}

/// Read one Content-Length framed body. `Ok(None)` is a clean
/// EOF before any header byte.
fn read_framed(reader: &mut impl std::io::BufRead) -> std::io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return if content_length.is_none() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF inside DAP headers",
                ))
            };
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Content-Length") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let Some(length) = content_length else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "DAP frame without Content-Length",
        ));
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "DAP body not UTF-8"))
}

/// Parse one framed body into `(seq, command, arguments)`.
fn parse_request(text: &str) -> Result<(i64, String, Json), String> {
    let value = Json::parse(text)?;
    if value.get("type").and_then(Json::as_str) != Some("request") {
        return Err("message is not a request".to_string());
    }
    let seq = value
        .get("seq")
        .and_then(Json::as_i64)
        .ok_or("request without an integer seq")?;
    let command = value
        .get("command")
        .and_then(Json::as_str)
        .ok_or("request without a command")?
        .to_string();
    let arguments = value.get("arguments").cloned().unwrap_or(Json::Null);
    Ok((seq, command, arguments))
}

/// The writing half of the transport: the server thread is the
/// single writer, so protocol sequence numbers are assigned
/// here, in send order.
struct Transport<W: Write> {
    out: W,
    seq: i64,
}

impl<W: Write> Transport<W> {
    fn new(out: W) -> Transport<W> {
        Transport { out, seq: 0 }
    }

    fn send(&mut self, mut message: Json) {
        self.seq += 1;
        if let Json::Object(pairs) = &mut message {
            pairs.insert(0, ("seq".to_string(), Json::Int(self.seq)));
        }
        let body = message.stringify();
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        // A failed write means the client is gone; the server's
        // next read reports EOF and unwinds the session there.
        let _ = self.out.write_all(frame.as_bytes());
        let _ = self.out.flush();
    }

    fn response(&mut self, request_seq: i64, command: &str, success: bool, body: Option<Json>) {
        let mut pairs = vec![
            ("type", Json::from("response")),
            ("request_seq", Json::Int(request_seq)),
            ("command", Json::from(command)),
            ("success", Json::Bool(success)),
        ];
        if let Some(body) = body {
            pairs.push(("body", body));
        }
        self.send(Json::obj(pairs));
    }

    fn error_response(&mut self, request_seq: i64, command: &str, message: &str) {
        let mut pairs = vec![
            ("type", Json::from("response")),
            ("request_seq", Json::Int(request_seq)),
            ("command", Json::from(command)),
            ("success", Json::Bool(false)),
            ("message", Json::from(message)),
        ];
        pairs.push((
            "body",
            Json::obj(vec![(
                "error",
                Json::obj(vec![
                    ("id", Json::Int(1)),
                    ("format", Json::from(message)),
                    ("showUser", Json::Bool(true)),
                ]),
            )]),
        ));
        self.send(Json::obj(pairs));
    }

    fn event(&mut self, name: &str, body: Option<Json>) {
        let mut pairs = vec![("type", Json::from("event")), ("event", Json::from(name))];
        if let Some(body) = body {
            pairs.push(("body", body));
        }
        self.send(Json::obj(pairs));
    }
}

// ---------------------------------------------------------------------
// Debuggee output capture
// ---------------------------------------------------------------------

/// Read-back state for the two capture files the debuggee's
/// fd 1/2 are bound to. Offsets make each drain incremental:
/// every byte is reported exactly once, in arrival order per
/// stream.
struct OutputCapture {
    stdout: Option<std::fs::File>,
    stderr: Option<std::fs::File>,
    stdout_offset: u64,
    stderr_offset: u64,
    dir: std::path::PathBuf,
}

impl OutputCapture {
    /// The capture directory's three files: `stdin` (created
    /// empty: the debuggee's fd 0 reads EOF), `stdout`,
    /// `stderr`.
    fn paths(&self) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        (
            self.dir.join("stdin"),
            self.dir.join("stdout"),
            self.dir.join("stderr"),
        )
    }

    /// Create the capture directory and the empty stdin file,
    /// and open read handles for the two output captures.
    fn create() -> Option<OutputCapture> {
        let dir = std::env::temp_dir().join(format!(
            "rubash-dap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).ok()?;
        let mut capture = OutputCapture {
            stdout: None,
            stderr: None,
            stdout_offset: 0,
            stderr_offset: 0,
            dir,
        };
        let (stdin_path, stdout_path, stderr_path) = capture.paths();
        std::fs::File::create(&stdin_path).ok()?;
        // The write handles are the executor's, opened by the
        // bind; these read handles are the adapter's own.
        capture.stdout = std::fs::File::open(&stdout_path).ok();
        if capture.stdout.is_none() {
            std::fs::File::create(&stdout_path).ok()?;
            capture.stdout = std::fs::File::open(&stdout_path).ok();
        }
        capture.stderr = std::fs::File::open(&stderr_path).ok();
        if capture.stderr.is_none() {
            std::fs::File::create(&stderr_path).ok()?;
            capture.stderr = std::fs::File::open(&stderr_path).ok();
        }
        Some(capture)
    }

    /// Read whatever the debuggee wrote since the last drain,
    /// as `(stream, text)` pairs in stdout-then-stderr order.
    fn drain(&mut self) -> Vec<(OutputStream, String)> {
        let mut chunks = Vec::new();
        for (handle, offset, stream) in [
            (
                &mut self.stdout,
                &mut self.stdout_offset,
                OutputStream::Stdout,
            ),
            (
                &mut self.stderr,
                &mut self.stderr_offset,
                OutputStream::Stderr,
            ),
        ] {
            let Some(file) = handle.as_mut() else {
                continue;
            };
            use std::io::{Read, Seek, SeekFrom};
            if file.seek(SeekFrom::Start(*offset)).is_err() {
                continue;
            }
            let mut bytes = Vec::new();
            if file.read_to_end(&mut bytes).is_ok() && !bytes.is_empty() {
                *offset += bytes.len() as u64;
                chunks.push((stream, String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
        chunks
    }
}

impl Drop for OutputCapture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Everything the server thread shares between its own phases
/// and the stop loop (which runs nested inside the run, on the
/// same thread).
struct Shared<W: Write> {
    transport: Transport<W>,
    inbox: mpsc::Receiver<Inbound>,
    bridge: Arc<ReaderBridge>,
    capture: Option<OutputCapture>,
    thread_name: String,
    stop_on_entry: bool,
}

impl<W: Write> Shared<W> {
    /// Drain the debuggee captures into `output` events.
    fn drain_output(&mut self) {
        let chunks = match self.capture.as_mut() {
            Some(capture) => capture.drain(),
            None => Vec::new(),
        };
        for (stream, text) in chunks {
            self.transport.event(
                "output",
                Some(Json::obj(vec![
                    ("category", Json::from(output_category(stream))),
                    ("output", Json::from(text)),
                ])),
            );
        }
    }

    /// Send one console-category `output` event (the adapter's
    /// own announcements: the section 5.4 watch banner, an
    /// exception-site note).
    fn console(&mut self, text: String) {
        self.transport.event(
            "output",
            Some(Json::obj(vec![
                ("category", Json::from("console")),
                ("output", Json::from(text)),
            ])),
        );
    }

    /// Receive the next client request; `None` when the client
    /// is gone (EOF or a broken stream — the two are the same
    /// to the session: nobody is listening anymore).
    fn next_request(&mut self) -> Option<(i64, String, Json)> {
        match self.inbox.recv() {
            Ok(Inbound::Request(seq, command, arguments)) => Some((seq, command, arguments)),
            _ => None,
        }
    }
}

/// The DAP category for one of the session's output streams.
fn output_category(stream: OutputStream) -> &'static str {
    match stream {
        OutputStream::Stdout => "stdout",
        OutputStream::Stderr => "stderr",
        OutputStream::Console => "console",
    }
}

// ---------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------

/// What the command line hands the adapter when the editor
/// launches it in the flag form (`rubash --debug --dap
/// script.sh args...`): the program and its arguments, already
/// chosen. Either may be absent — the pure server form takes
/// both from the client's `launch` request instead.
#[derive(Debug, Clone, Default)]
pub struct DapLaunchPreset {
    /// The program path, when the flag form named one.
    pub program: Option<String>,
    /// The program's arguments (`$1` and onward).
    pub args: Vec<String>,
}

/// Run the DAP frontend on the process's stdio. This is the
/// entry `main` dispatches to for `--dap`; the return value is
/// the process exit code — the debuggee's final status when a
/// debuggee ran, 0 for a session that never launched one.
pub fn run_dap_stdio(preset: DapLaunchPreset) -> i32 {
    run_dap_server(std::io::stdin(), std::io::stdout(), preset)
}

/// Run one DAP server session over the given streams: the
/// whole conversation, from `initialize` to the debuggee's
/// termination and the client's `disconnect` (or EOF).
pub fn run_dap_server<R: Read + Send + 'static, W: Write + 'static>(
    reader: R,
    writer: W,
    preset: DapLaunchPreset,
) -> i32 {
    let bridge = ReaderBridge::new();
    let inbox = spawn_reader(reader, bridge.clone());
    let shared = Rc::new(RefCell::new(Shared {
        transport: Transport::new(writer),
        inbox,
        bridge,
        capture: None,
        thread_name: "main".to_string(),
        stop_on_entry: true,
    }));
    let mut server = Server {
        shared: shared.clone(),
        preset,
        program_identity: None,
        program_args: Vec::new(),
        no_debug: false,
        launch_env: Vec::new(),
        launch_cwd: None,
        session: None,
        launched: false,
    };
    server.converse()
}

/// The server-side conversation state. Phases: initialize →
/// launch → configuration → run → epilogue, per section 13.
struct Server<W: Write> {
    shared: Rc<RefCell<Shared<W>>>,
    preset: DapLaunchPreset,
    /// The program's canonical identity: the spelling the
    /// engine's source stack will report for it, used for the
    /// syntax view, the run input, and breakpoint matching.
    program_identity: Option<String>,
    program_args: Vec<String>,
    no_debug: bool,
    launch_env: Vec<(String, String)>,
    launch_cwd: Option<String>,
    /// The session under configuration; created at launch and
    /// moved into the driver when the run starts.
    session: Option<DebugSession>,
    launched: bool,
}

impl<W: Write + 'static> Server<W> {
    fn converse(&mut self) -> i32 {
        // Phase 1: initialize. Anything else first is a protocol
        // error; EOF first is a client that never started.
        let Some((seq, command, _)) = self.next() else {
            return 0;
        };
        if command != "initialize" {
            self.error(seq, &command, "the first request must be initialize");
            return self.drain_until_disconnect(0);
        }
        self.shared
            .borrow_mut()
            .transport
            .response(seq, "initialize", true, Some(capabilities()));

        // Phase 2: launch (or the launch-equivalent attach).
        let Some((seq, command, arguments)) = self.next() else {
            return 0;
        };
        match command.as_str() {
            "launch" => {
                if !self.accept_launch(seq, &arguments) {
                    return self.drain_until_disconnect(1);
                }
            }
            "attach" => {
                if arguments.get("program").is_some() {
                    if !self.accept_launch(seq, &arguments) {
                        return self.drain_until_disconnect(1);
                    }
                } else {
                    self.error(
                        seq,
                        "attach",
                        "attach is honored only in its launch-equivalent form \
                         (a `program` argument): background jobs are separate \
                         OS processes and cannot be attached (design section 13)",
                    );
                    return self.drain_until_disconnect(1);
                }
            }
            "disconnect" => {
                self.respond(seq, "disconnect", true, None);
                return 0;
            }
            _ => {
                self.error(seq, &command, "expected launch after initialize");
                return self.drain_until_disconnect(0);
            }
        }
        self.shared
            .borrow_mut()
            .transport
            .event("initialized", Some(Json::obj(vec![])));

        // Phase 3: configuration, until configurationDone.
        loop {
            let Some((seq, command, arguments)) = self.next() else {
                return 0;
            };
            match command.as_str() {
                "setBreakpoints" => {
                    let body = self.apply_source_breakpoints(&arguments);
                    self.respond(seq, "setBreakpoints", true, Some(body));
                }
                "setFunctionBreakpoints" => {
                    let body = self.apply_function_breakpoints(&arguments);
                    self.respond(seq, "setFunctionBreakpoints", true, Some(body));
                }
                "setExceptionBreakpoints" => match self.session.as_mut() {
                    Some(session) => match apply_exception_filters(session, &arguments) {
                        Ok(()) => {
                            self.respond(
                                seq,
                                "setExceptionBreakpoints",
                                true,
                                Some(Json::obj(vec![("breakpoints", Json::Array(vec![]))])),
                            );
                        }
                        Err(unknown) => {
                            self.error(
                                seq,
                                "setExceptionBreakpoints",
                                &format!("unknown exception filter(s): {}", unknown.join(", ")),
                            );
                        }
                    },
                    None => {
                        self.respond(
                            seq,
                            "setExceptionBreakpoints",
                            true,
                            Some(Json::obj(vec![("breakpoints", Json::Array(vec![]))])),
                        );
                    }
                },
                "threads" => self.answer_threads(seq),
                "configurationDone" => {
                    self.respond(seq, "configurationDone", true, None);
                    break;
                }
                "disconnect" => {
                    self.respond(seq, "disconnect", true, None);
                    return 0;
                }
                _ => {
                    self.error(
                        seq,
                        &command,
                        &format!("request '{command}' is not available before configurationDone"),
                    );
                }
            }
        }

        // Phase 4 + 5: run the debuggee, then the epilogue.
        self.run()
    }

    /// Validate and record a launch configuration, create the
    /// session, and pre-register the program's syntax view so
    /// configuration-phase breakpoints adjudicate immediately.
    /// False when the launch was refused (the error response is
    /// already sent).
    fn accept_launch(&mut self, seq: i64, arguments: &Json) -> bool {
        let program = arguments
            .get("program")
            .and_then(Json::as_str)
            .map(str::to_string)
            .or_else(|| self.preset.program.clone());
        let Some(program) = program else {
            self.error(seq, "launch", "launch requires a `program` argument");
            return false;
        };
        // Arguments: the request's win over the preset's.
        self.program_args = match arguments.get("args") {
            Some(Json::Array(items)) => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect(),
            Some(Json::Str(text)) => vec![text.clone()],
            _ => self.preset.args.clone(),
        };
        self.no_debug = arguments
            .get("noDebug")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        self.shared.borrow_mut().stop_on_entry = arguments
            .get("stopOnEntry")
            .and_then(Json::as_bool)
            .unwrap_or(true);
        if let Some(env) = arguments.get("env") {
            if let Json::Object(pairs) = env {
                self.launch_env = pairs
                    .iter()
                    .filter_map(|(name, value)| {
                        value.as_str().map(|v| (name.clone(), v.to_string()))
                    })
                    .collect();
            }
        }
        self.launch_cwd = arguments
            .get("cwd")
            .and_then(Json::as_str)
            .map(str::to_string);

        // Resolve the program against the launch cwd, then
        // canonicalize: the canonical spelling is the identity
        // the engine will report, on every platform.
        let candidate = {
            let path = std::path::Path::new(&program);
            if path.is_absolute() {
                path.to_path_buf()
            } else if let Some(cwd) = &self.launch_cwd {
                std::path::Path::new(cwd).join(path)
            } else {
                path.to_path_buf()
            }
        };
        let identity = match std::fs::canonicalize(&candidate) {
            Ok(path) => path_string(&path),
            Err(error) => {
                self.error(
                    seq,
                    "launch",
                    &format!("cannot open program '{program}': {error}"),
                );
                return false;
            }
        };
        self.program_identity = Some(identity.clone());
        self.shared.borrow_mut().thread_name = std::path::Path::new(&identity)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main".to_string());

        let mut session = DebugSession::new();
        // Pre-register the program's syntax view, parsed from
        // the file itself, so configuration-phase breakpoints
        // resolve now instead of pending until the run. The
        // executor re-registers reader units during the run;
        // registration is idempotent and this whole-file view
        // only widens what is known.
        if let Ok(text) = crate::script_driver::read_script_bytes(std::path::Path::new(&identity)) {
            let tokens = crate::lexer::tokenize(&text);
            let ast = crate::parser::parse(&tokens);
            session.register_syntax_view(crate::api::syntax::SyntaxView::from_ast(&identity, &ast));
        }
        self.session = Some(session);
        self.launched = true;
        self.respond(seq, "launch", true, None);
        true
    }

    /// `setBreakpoints` against the session under
    /// configuration: replace the source's line entries and
    /// report each entry's adjudicated state, per DAP's
    /// replace-per-source rule.
    fn apply_source_breakpoints(&mut self, arguments: &Json) -> Json {
        let Some(session) = self.session.as_mut() else {
            return Json::obj(vec![("breakpoints", Json::Array(vec![]))]);
        };
        let given = arguments
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let identity = file_identity(&given);
        let specs = arguments
            .get("breakpoints")
            .and_then(Json::as_array)
            .map(|items| items.to_vec())
            .unwrap_or_default();
        let rendered = apply_source_breakpoints(session, &identity, &specs);
        Json::obj(vec![("breakpoints", Json::Array(rendered))])
    }

    /// `setFunctionBreakpoints`: replace all function entries.
    fn apply_function_breakpoints(&mut self, arguments: &Json) -> Json {
        let Some(session) = self.session.as_mut() else {
            return Json::obj(vec![("breakpoints", Json::Array(vec![]))]);
        };
        let specs = arguments
            .get("breakpoints")
            .and_then(Json::as_array)
            .map(|items| items.to_vec())
            .unwrap_or_default();
        let rendered = apply_function_breakpoints(session, &specs);
        Json::obj(vec![("breakpoints", Json::Array(rendered))])
    }

    /// Run the debuggee and serve the epilogue.
    fn run(&mut self) -> i32 {
        let Some(identity) = self.program_identity.clone() else {
            return 1;
        };
        let mut executor = Executor::new();
        for (name, value) in &self.launch_env {
            executor.set_env(name, value);
        }
        // Capture the debuggee's stdio before anything runs.
        let capture = OutputCapture::create();
        let bound = match &capture {
            Some(capture) => {
                let (stdin, stdout, stderr) = capture.paths();
                executor.debugger_bind_stdio(&stdin, &stdout, &stderr)
            }
            None => false,
        };
        self.shared.borrow_mut().capture = capture;
        if !bound {
            let mut shared = self.shared.borrow_mut();
            shared.console("dap: could not bind the debuggee's stdio captures\n".to_string());
            shared
                .transport
                .event("exited", Some(Json::obj(vec![("exitCode", Json::Int(1))])));
            shared
                .transport
                .event("terminated", Some(Json::obj(vec![])));
            return 1;
        }
        if let Some(cwd) = &self.launch_cwd {
            executor.view_set_cwd(std::path::Path::new(cwd));
        }

        // The run input: source the program with its arguments
        // as positionals. Sourcing keeps one process and one
        // session (design section 6.3); `$0` stays the shell's,
        // per GNU's own `.` semantics.
        let mut input = String::from(". ");
        input.push_str(&shell_single_quote(&identity));
        for arg in &self.program_args {
            input.push(' ');
            input.push_str(&shell_single_quote(arg));
        }
        input.push('\n');

        let detached;
        let status;
        if self.no_debug {
            status = crate::script_driver::run_source(&mut executor, &input, false);
            detached = false;
        } else {
            let session = self.session.take().unwrap_or_default();
            let stop_loop = DapStopLoop::new(self.shared.clone());
            *self.shared.borrow_mut().bridge.session_flag.lock().unwrap() =
                Some(session.async_stop_flag());
            let driver = SessionDriver::new(session, Box::new(stop_loop));
            executor.install_debug_session(driver);
            status = crate::script_driver::run_source(&mut executor, &input, false);
            detached = executor.debug_driver().is_none();
        }
        self.shared
            .borrow_mut()
            .bridge
            .running
            .store(false, Ordering::Release);
        *self.shared.borrow_mut().bridge.session_flag.lock().unwrap() = None;

        // Epilogue: remaining debuggee output first, then the
        // session's last events, then exited/terminated.
        self.shared.borrow_mut().drain_output();
        let mut final_status = status;
        if !detached {
            if let Some(driver) = executor.debug_driver_mut() {
                let events = driver.session_mut().drain_events();
                let mut exited: Option<i32> = None;
                for event in events {
                    match event {
                        DebugEvent::SessionExited { status } => exited = Some(status),
                        DebugEvent::DebuggeeDied { signal } => exited = Some(128 + signal),
                        other => {
                            let mut shared = self.shared.borrow_mut();
                            translate_event(&mut shared, other, None);
                        }
                    }
                }
                if let Some(status) = exited {
                    final_status = status;
                }
            }
            let mut shared = self.shared.borrow_mut();
            // A pause the reader thread applied but the run
            // outlived: answer it as failed before terminating.
            let pending_pause = shared.bridge.pause_reply_seq.lock().unwrap().take();
            if let Some(seq) = pending_pause {
                shared.transport.error_response(
                    seq,
                    "pause",
                    "the debuggee terminated before the pause could be serviced",
                );
            }
            shared.transport.event(
                "exited",
                Some(Json::obj(vec![(
                    "exitCode",
                    Json::Int(i64::from(final_status)),
                )])),
            );
            shared
                .transport
                .event("terminated", Some(Json::obj(vec![])));
        }

        if detached {
            // The disconnect that detached was already answered
            // inside the stop loop; the protocol is over.
            return final_status;
        }

        // Post-termination conversation: only disconnect (or
        // EOF) ends the protocol; threads answers empty.
        loop {
            let Some((seq, command, _)) = self.next() else {
                return final_status;
            };
            match command.as_str() {
                "disconnect" => {
                    self.respond(seq, "disconnect", true, None);
                    return final_status;
                }
                "threads" => {
                    self.respond(
                        seq,
                        "threads",
                        true,
                        Some(Json::obj(vec![("threads", Json::Array(vec![]))])),
                    );
                }
                _ => {
                    self.error(seq, &command, "the debuggee has terminated");
                }
            }
        }
    }

    fn answer_threads(&mut self, seq: i64) {
        let name = self.shared.borrow().thread_name.clone();
        self.respond(
            seq,
            "threads",
            true,
            Some(Json::obj(vec![(
                "threads",
                Json::Array(vec![Json::obj(vec![
                    ("id", Json::Int(THREAD_ID)),
                    ("name", Json::from(name)),
                ])]),
            )])),
        );
    }

    /// Answer requests with errors until disconnect or EOF;
    /// used after a refused launch.
    fn drain_until_disconnect(&mut self, code: i32) -> i32 {
        loop {
            let Some((seq, command, _)) = self.next() else {
                return code;
            };
            if command == "disconnect" {
                self.respond(seq, "disconnect", true, None);
                return code;
            }
            self.error(seq, &command, "no debug session is active");
        }
    }

    fn next(&mut self) -> Option<(i64, String, Json)> {
        self.shared.borrow_mut().next_request()
    }

    fn respond(&mut self, seq: i64, command: &str, success: bool, body: Option<Json>) {
        self.shared
            .borrow_mut()
            .transport
            .response(seq, command, success, body);
    }

    fn error(&mut self, seq: i64, command: &str, message: &str) {
        self.shared
            .borrow_mut()
            .transport
            .error_response(seq, command, message);
    }
}

/// The capabilities object of the `initialize` response:
/// exactly the features whose machinery has landed, per
/// section 13's claim-only-when-it-exists rule.
fn capabilities() -> Json {
    Json::obj(vec![
        // WP-14: the configuration phase itself.
        ("supportsConfigurationDoneRequest", Json::Bool(true)),
        // Session-core function breakpoints (row 7).
        ("supportsFunctionBreakpoints", Json::Bool(true)),
        // Section 5.3 condition evaluation, in the consult
        // since rows 3-5.
        ("supportsConditionalBreakpoints", Json::Bool(true)),
        // Section 7.6 write-back through the real assignment
        // path (row 8); section 13 claims it when the mapping
        // exists — it does, in this adapter.
        ("supportsSetVariable", Json::Bool(true)),
        // The section 7.5 strict evaluator (row 8, WP-7):
        // section 13 flips this claim in the same change that
        // lands it; the hover context routes Implicit.
        ("supportsEvaluateForHovers", Json::Bool(true)),
        ("supportsTerminateRequest", Json::Bool(true)),
        ("supportTerminateDebuggee", Json::Bool(true)),
        // WP-15 (row 11): the exception filters exist at the
        // operation-set layer — the session stores exactly these
        // ids, both frontends read and write the one set — and
        // the `exceptionInfo` request is answered (with the
        // no-exception-at-this-stop error until the row-12 site
        // producers land). Claimed in the same change that lands
        // the machinery, per section 13's rule.
        ("supportsExceptionInfoRequest", Json::Bool(true)),
        (
            "exceptionBreakpointFilters",
            Json::Array(vec![
                Json::obj(vec![
                    ("filter", Json::from("err-trap")),
                    ("label", Json::from("ERR trap")),
                ]),
                Json::obj(vec![
                    ("filter", Json::from("set -e")),
                    ("label", Json::from("set -e (errexit)")),
                ]),
                Json::obj(vec![
                    ("filter", Json::from("parse-error")),
                    ("label", Json::from("Parse error")),
                ]),
            ]),
        ),
        // Never claimed at this milestone: supportsDataBreakpoints
        // (M3 at the earliest), supportsStepBack / restart (no
        // such session operation).
    ])
}

/// A path as the identity string the engine reports: the
/// native spelling, with the Windows extended-length prefix
/// (`\\?\`) the canonicalizer adds stripped back off.
fn path_string(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().into_owned();
    text.strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or(text)
}

/// The breakpoint identity for a client-supplied path: its
/// canonical form when the file exists, the given string when
/// it does not (the entry then stays Pending against a
/// spelling the engine may report later).
fn file_identity(given: &str) -> String {
    match std::fs::canonicalize(given) {
        Ok(path) => path_string(&path),
        Err(_) => given.to_string(),
    }
}

/// Single-quote one word for the run input: the shell's own
/// quoting, `'` escaped as `'\''`. Shared with the CLI frontend
/// (row 11): both frontends build the same run input, so the
/// quoting lives in exactly one place.
pub(crate) fn shell_single_quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for ch in text.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Apply a `setExceptionBreakpoints` request to the session's
/// filter store (WP-15): the request's `filters` array names
/// filter ids, and the session — the operation-set owner —
/// validates them against the canonical set. Unknown ids come
/// back as the error payload; the previous selection is then
/// untouched (the session rejects as a batch).
fn apply_exception_filters(
    session: &mut DebugSession,
    arguments: &Json,
) -> Result<(), Vec<String>> {
    let ids: Vec<String> = arguments
        .get("filters")
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    session.set_exception_filters(ids)
}

/// Replace one source's line breakpoints in the session and
/// render the resulting entries as DAP breakpoint objects.
/// Shared by the configuration phase and the stop loop (a
/// mid-session `setBreakpoints` is the same operation).
fn apply_source_breakpoints(
    session: &mut DebugSession,
    identity: &str,
    specs: &[Json],
) -> Vec<Json> {
    let stale: Vec<super::breakpoints::BreakpointId> = session
        .breakpoints()
        .iter()
        .filter(|entry| {
            matches!(entry.kind, BreakpointKind::Line)
                && matches!(&entry.target, BreakpointTarget::Line { file, .. } if file == identity)
        })
        .map(|entry| entry.id)
        .collect();
    for id in stale {
        session.breakpoints_mut().remove(id);
    }
    let mut rendered = Vec::new();
    for spec in specs {
        let Some(line) = spec.get("line").and_then(Json::as_i64) else {
            continue;
        };
        if line < 1 {
            rendered.push(Json::obj(vec![
                ("verified", Json::Bool(false)),
                ("message", Json::from("line numbers start at 1")),
            ]));
            continue;
        }
        let id = session.set_line_breakpoint(identity.to_string(), line as u32);
        if let Some(condition) = spec.get("condition").and_then(Json::as_str) {
            if !condition.is_empty() {
                session
                    .breakpoints_mut()
                    .set_condition(id, Some(condition.to_string()));
            }
        }
        // Honest limits, reported on the entry rather than
        // silently dropped or falsely claimed.
        let mut notes = Vec::new();
        if spec.get("hitCondition").is_some() {
            notes.push("hitCondition is not supported");
        }
        if spec.get("logMessage").is_some() {
            notes.push("logMessage is not supported");
        }
        let mut object = render_breakpoint(session, id);
        if !notes.is_empty() {
            if let Json::Object(pairs) = &mut object {
                let note = notes.join("; ");
                match pairs.iter_mut().find(|(key, _)| key == "message") {
                    Some((_, value)) => {
                        if let Json::Str(existing) = value {
                            *existing = format!("{existing}; {note}");
                        }
                    }
                    None => pairs.push(("message".to_string(), Json::from(note))),
                }
            }
        }
        rendered.push(object);
    }
    rendered
}

/// Replace all function breakpoints and render the new set.
fn apply_function_breakpoints(session: &mut DebugSession, specs: &[Json]) -> Vec<Json> {
    let stale: Vec<super::breakpoints::BreakpointId> = session
        .breakpoints()
        .iter()
        .filter(|entry| matches!(entry.kind, BreakpointKind::Function))
        .map(|entry| entry.id)
        .collect();
    for id in stale {
        session.breakpoints_mut().remove(id);
    }
    let mut rendered = Vec::new();
    for spec in specs {
        let Some(name) = spec.get("name").and_then(Json::as_str) else {
            continue;
        };
        let id = session.set_function_breakpoint(name.to_string());
        rendered.push(render_breakpoint(session, id));
    }
    rendered
}

/// One breakpoint entry as a DAP `Breakpoint` object: the
/// session's adjudicated resolution, reported as-is — Verified
/// becomes `verified: true`; Pending stays unverified with a
/// pending note; Unverified carries the design's reason.
fn render_breakpoint(session: &DebugSession, id: super::breakpoints::BreakpointId) -> Json {
    let Some(entry) = session.breakpoints().get(id) else {
        return Json::obj(vec![("verified", Json::Bool(false))]);
    };
    let mut pairs = vec![
        ("id", Json::Int(i64::from(id.number()))),
        (
            "verified",
            Json::Bool(matches!(entry.resolution, BreakpointResolution::Verified)),
        ),
    ];
    match &entry.resolution {
        BreakpointResolution::Verified => {}
        BreakpointResolution::Pending => {
            pairs.push((
                "message",
                Json::from("pending: the target has not been parsed or defined yet"),
            ));
        }
        BreakpointResolution::Unverified { reason } => {
            pairs.push(("message", Json::from(reason.clone())));
        }
    }
    match &entry.target {
        BreakpointTarget::Line { file, line } => {
            pairs.push((
                "source",
                Json::obj(vec![
                    ("name", Json::from(source_name(file))),
                    ("path", Json::from(file.clone())),
                ]),
            ));
            pairs.push(("line", Json::Int(i64::from(*line))));
        }
        BreakpointTarget::Function { name } => {
            pairs.push(("name", Json::from(name.clone())));
        }
        BreakpointTarget::Expression { text } => {
            pairs.push(("name", Json::from(text.clone())));
        }
    }
    Json::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The display name of a source path: its final component.
fn source_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

// ---------------------------------------------------------------------
// Event translation (DE catalogue -> DAP wire)
// ---------------------------------------------------------------------

/// Translate one session event into its DAP shape. Two
/// catalogue entries are deliberately NOT translated here:
/// `Stopped` (the serve itself is the stop presentation) and
/// the termination pair (the epilogue owns `exited` /
/// `terminated`, so their order against the final output drain
/// is fixed). `WatchChanged` at the epilogue becomes the
/// section 5.4 console banner; inside a serve it is handled by
/// the stop loop, which also owns the stop-reason rule.
fn translate_event<W: Write>(
    shared: &mut Shared<W>,
    event: DebugEvent,
    session: Option<&DebugSession>,
) {
    match event {
        DebugEvent::BreakpointResolved { id, .. } => {
            if let Some(session) = session {
                shared.transport.event(
                    "breakpoint",
                    Some(Json::obj(vec![
                        ("reason", Json::from("changed")),
                        ("breakpoint", render_breakpoint(session, id)),
                    ])),
                );
            }
        }
        DebugEvent::WatchChanged(change) => {
            shared.console(watch_banner(&change));
        }
        DebugEvent::OutputProduced { stream, text } => {
            shared.transport.event(
                "output",
                Some(Json::obj(vec![
                    ("category", Json::from(output_category(stream))),
                    ("output", Json::from(text)),
                ])),
            );
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
            shared.console(format!(
                "exception site: {kind} (status {}){location}\n",
                site.status
            ));
        }
        _ => {}
    }
}

/// The section 5.4 stop banner for one watch change: changed
/// since the last stop — never "written by line N".
fn watch_banner(change: &super::event::WatchChange) -> String {
    let render = |value: &Option<String>| match value {
        Some(text) => format!("'{text}'"),
        None => "unset".to_string(),
    };
    format!(
        "watch: {} {} → {}\n",
        change.expression,
        render(&change.old_value),
        render(&change.new_value)
    )
}

// ---------------------------------------------------------------------
// The stop loop
// ---------------------------------------------------------------------

/// What the adapter owes the client when an evaluation outcome
/// comes back: the serve that returns `Evaluate` is re-entered
/// with the outcome installed on the session, and the reply is
/// built there.
enum PendingReply {
    Evaluate {
        seq: i64,
    },
    SetVariable {
        seq: i64,
        /// The variable to re-render from the refreshed
        /// snapshot: the container name, and the element's
        /// display name when the write targeted an element.
        parent: String,
        element: Option<String>,
        /// The value as sent, the fallback rendering when the
        /// refreshed snapshot cannot show the write.
        sent_value: String,
    },
}

/// One entry in the per-stop variables registry.
enum VarRefKey {
    /// A scope of one frame: its locals, or the globals.
    Scope { frame: usize, locals: bool },
    /// The elements of one array variable, by name.
    ArrayOf(String),
}

/// The DAP stop loop: serves each parked stop as protocol
/// traffic. All state here is presentation state — registries
/// that map DAP's integer handles onto the current snapshot —
/// never debugging state.
struct DapStopLoop<W: Write> {
    shared: Rc<RefCell<Shared<W>>>,
    pending_reply: Option<PendingReply>,
    frame_ids: HashMap<i64, usize>,
    next_frame_serial: i64,
    var_refs: HashMap<i64, VarRefKey>,
    next_var_ref: i64,
    /// Watch expressions registered through `evaluate` with
    /// context `watch`, by their stored table text.
    watch_registered: HashSet<String>,
}

impl<W: Write + 'static> DapStopLoop<W> {
    fn new(shared: Rc<RefCell<Shared<W>>>) -> DapStopLoop<W> {
        DapStopLoop {
            shared,
            pending_reply: None,
            frame_ids: HashMap::new(),
            next_frame_serial: 1,
            var_refs: HashMap::new(),
            next_var_ref: 1,
            watch_registered: HashSet::new(),
        }
    }

    fn alloc_ref(&mut self, key: VarRefKey) -> i64 {
        let reference = self.next_var_ref;
        self.next_var_ref += 1;
        self.var_refs.insert(reference, key);
        reference
    }

    /// Mark the debuggee as running free: from here the reader
    /// thread owns any `pause` that arrives, via the bridge.
    /// Resume-class request handlers call this BEFORE sending
    /// their response, so a client that answers the response
    /// with an immediate `pause` can never race the flag.
    fn mark_running(&mut self) {
        self.shared
            .borrow()
            .bridge
            .running
            .store(true, Ordering::Release);
    }

    /// Mark running and produce the resume outcome.
    fn resume(&mut self, arming: StepArming) -> StopLoopOutcome {
        self.mark_running();
        StopLoopOutcome::Resume(arming)
    }

    /// Answer the request an `Evaluate` outcome belongs to,
    /// from the outcome the executor installed on the session.
    fn answer_evaluation(&mut self, session: &DebugSession, pending: PendingReply) {
        // Output the evaluation itself produced (an explicit
        // `echo`, say) precedes its response.
        self.shared.borrow_mut().drain_output();
        let result = session.last_evaluation().map(|outcome| &outcome.result);
        match pending {
            PendingReply::Evaluate { seq } => match result {
                Some(EvaluationResult::Value(value)) => {
                    let value = value.clone();
                    self.shared.borrow_mut().transport.response(
                        seq,
                        "evaluate",
                        true,
                        Some(Json::obj(vec![
                            ("result", Json::from(value)),
                            ("variablesReference", Json::Int(0)),
                        ])),
                    );
                }
                Some(EvaluationResult::Status(_)) => {
                    self.shared.borrow_mut().transport.response(
                        seq,
                        "evaluate",
                        true,
                        Some(Json::obj(vec![
                            ("result", Json::from("")),
                            ("variablesReference", Json::Int(0)),
                        ])),
                    );
                }
                Some(EvaluationResult::Rejected(diag)) | Some(EvaluationResult::Failed(diag)) => {
                    let message = diag.message.clone();
                    self.shared
                        .borrow_mut()
                        .transport
                        .error_response(seq, "evaluate", &message);
                }
                None => {
                    self.shared.borrow_mut().transport.error_response(
                        seq,
                        "evaluate",
                        "the evaluation produced no outcome",
                    );
                }
            },
            PendingReply::SetVariable {
                seq,
                parent,
                element,
                sent_value,
            } => match result {
                Some(EvaluationResult::Rejected(diag)) | Some(EvaluationResult::Failed(diag)) => {
                    let message = diag.message.clone();
                    self.shared
                        .borrow_mut()
                        .transport
                        .error_response(seq, "setVariable", &message);
                }
                Some(_) => {
                    let value =
                        refreshed_value(session, &parent, element.as_deref()).unwrap_or(sent_value);
                    self.shared.borrow_mut().transport.response(
                        seq,
                        "setVariable",
                        true,
                        Some(Json::obj(vec![("value", Json::from(value))])),
                    );
                }
                None => {
                    self.shared.borrow_mut().transport.error_response(
                        seq,
                        "setVariable",
                        "the write-back produced no outcome",
                    );
                }
            },
        }
    }

    /// Register a watch for an `evaluate` in the `watch`
    /// context, in the session's own table (the stop-and-diff
    /// machinery then tracks it across stops; standard DAP has
    /// no setWatch request, so the watch list arrives exactly
    /// this way). Idempotent per stored text.
    fn register_watch(&mut self, session: &mut DebugSession, expression: &str) {
        let trimmed = expression.trim();
        let (kind, stored) = watch_registration_form(trimmed);
        if self.watch_registered.contains(&stored) {
            return;
        }
        let exists = session
            .breakpoints()
            .watch_entries()
            .any(|entry| matches!(&entry.target, BreakpointTarget::Expression { text } if text == &stored));
        if !exists {
            match kind {
                BreakpointKind::Watch => {
                    session.breakpoints_mut().insert_watch(stored.clone());
                }
                _ => {
                    session
                        .breakpoints_mut()
                        .insert_watch_expression(stored.clone());
                }
            }
        }
        self.watch_registered.insert(stored);
    }
}

/// The table form of a watch expression: a bare name (with or
/// without the leading `$`) is a name watch; anything else is
/// a watch expression, normalized to its `$`-form when it is a
/// plain subscript so the strict re-check can evaluate it.
fn watch_registration_form(expression: &str) -> (BreakpointKind, String) {
    let bare = expression.strip_prefix('$').unwrap_or(expression);
    if is_identifier(bare) {
        return (BreakpointKind::Watch, bare.to_string());
    }
    if let Some(open) = bare.find('[') {
        if is_identifier(&bare[..open]) && bare.ends_with(']') {
            return (BreakpointKind::WatchExpression, format!("${bare}"));
        }
    }
    (BreakpointKind::WatchExpression, expression.to_string())
}

/// Whether `text` is a plain shell identifier.
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// The refreshed rendering of a variable (or one element)
/// after a write-back, read from the session's new inspection
/// snapshot — the same rendering the variables face shows.
fn refreshed_value(session: &DebugSession, parent: &str, element: Option<&str>) -> Option<String> {
    let snapshot = session.inspection()?;
    let variable = snapshot.variable(parent)?;
    match (element, &variable.value) {
        (None, VariableValue::Scalar(value)) => Some(value.clone()),
        (None, VariableValue::Unset) => Some(String::new()),
        (None, _) => Some(array_display_value(variable)),
        (Some(child), VariableValue::Indexed(elements)) => {
            let index: i64 = child.strip_prefix('[')?.strip_suffix(']')?.parse().ok()?;
            elements
                .iter()
                .find(|(at, _)| *at == index)
                .map(|(_, value)| value.clone())
        }
        (Some(child), VariableValue::Associative(elements)) => elements
            .iter()
            .find(|(key, _)| key == child)
            .map(|(_, value)| value.clone()),
        _ => None,
    }
}

/// The `value` text of an array variable: its `declare -p`
/// rendering past the `=`, when the snapshot carries one, else
/// a parenthesized element list.
fn array_display_value(variable: &VariableSnapshot) -> String {
    if let Some(declaration) = &variable.declaration {
        if let Some((_, rhs)) = declaration.split_once('=') {
            return rhs.to_string();
        }
    }
    let quote = |value: &str| {
        let mut out = String::from("\"");
        for ch in value.chars() {
            match ch {
                '"' | '\\' | '$' | '`' => {
                    out.push('\\');
                    out.push(ch);
                }
                ch => out.push(ch),
            }
        }
        out.push('"');
        out
    };
    match &variable.value {
        VariableValue::Indexed(elements) => {
            let parts: Vec<String> = elements
                .iter()
                .map(|(index, value)| format!("[{index}]={}", quote(value)))
                .collect();
            format!("({})", parts.join(" "))
        }
        VariableValue::Associative(elements) => {
            let parts: Vec<String> = elements
                .iter()
                .map(|(key, value)| format!("[{key}]={}", quote(value)))
                .collect();
            format!("({})", parts.join(" "))
        }
        _ => String::new(),
    }
}

impl<W: Write + 'static> StopLoop for DapStopLoop<W> {
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        // Re-entry after an evaluation: the same stop, still
        // parked; the registries stay valid and only the owed
        // reply is new.
        if let Some(pending) = self.pending_reply.take() {
            self.answer_evaluation(session, pending);
            return self.request_loop(session);
        }

        let stop = session
            .last_stop()
            .cloned()
            .unwrap_or_else(super::event::StopInfo::entry);
        self.frame_ids.clear();
        self.var_refs.clear();
        self.next_var_ref = 1;
        self.next_frame_serial = 1;

        // The entry stop with stopOnEntry off resumes silently:
        // its queued events are drained (nothing else can be in
        // the queue yet) and no stopped event is sent. Breakpoints
        // still apply from the first line — they live in the
        // table, not in this presentation.
        if stop.reason == StopReason::Entry && !self.shared.borrow().stop_on_entry {
            let _ = session.drain_events();
            return self.resume(StepArming::Continue);
        }

        // Presentation order: debuggee output first, then the
        // stop's own announcements (watch banners), then the
        // stopped event.
        self.shared.borrow_mut().drain_output();
        let mut watch_changes = Vec::new();
        for event in session.drain_events() {
            match event {
                DebugEvent::WatchChanged(change) => watch_changes.push(change),
                DebugEvent::Stopped(_) | DebugEvent::SessionStarted => {}
                other => {
                    let mut shared = self.shared.borrow_mut();
                    translate_event(&mut shared, other, Some(session));
                }
            }
        }
        {
            let mut shared = self.shared.borrow_mut();
            for change in &watch_changes {
                shared.console(watch_banner(change));
            }
        }

        // A pause serviced at this stop answers the request the
        // reader thread applied to the flag.
        if stop.reason == StopReason::Pause {
            let pending = self
                .shared
                .borrow()
                .bridge
                .pause_reply_seq
                .lock()
                .unwrap()
                .take();
            if let Some(seq) = pending {
                self.shared
                    .borrow_mut()
                    .transport
                    .response(seq, "pause", true, None);
            }
        }

        let (reason, hit_ids): (&str, Vec<u32>) = match &stop.reason {
            StopReason::Entry => ("entry", Vec::new()),
            StopReason::Breakpoint(id) => ("breakpoint", vec![id.number()]),
            StopReason::FunctionBreakpoint(id) => ("function breakpoint", vec![id.number()]),
            StopReason::Step => {
                if watch_changes.is_empty() {
                    ("step", Vec::new())
                } else {
                    // The stop-and-diff rule of section 5.4: a
                    // step stop whose watch fired IS the watch
                    // stop, presented with DAP's watch reason.
                    ("data breakpoint", Vec::new())
                }
            }
            StopReason::Watchpoint(id) => ("data breakpoint", vec![id.number()]),
            StopReason::Pause => ("pause", Vec::new()),
            StopReason::Exception(_) => ("exception", Vec::new()),
        };
        let mut body = vec![
            ("reason", Json::from(reason)),
            ("threadId", Json::Int(THREAD_ID)),
            ("allThreadsStopped", Json::Bool(true)),
        ];
        if !hit_ids.is_empty() {
            body.push((
                "hitBreakpointIds",
                Json::Array(hit_ids.iter().map(|id| Json::Int(i64::from(*id))).collect()),
            ));
        }
        if let Some(text) = &stop.command_text {
            body.push(("text", Json::from(text.clone())));
        }
        self.shared
            .borrow_mut()
            .transport
            .event("stopped", Some(Json::obj(body)));

        self.request_loop(session)
    }
}

impl<W: Write + 'static> DapStopLoop<W> {
    /// The parked-stop request loop: answer requests against
    /// the session until one of them resumes, terminates, or
    /// detaches the debuggee — or the client vanishes, which
    /// terminates it (nobody is listening anymore).
    fn request_loop(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        let frame_depth = session
            .last_stop()
            .map(|stop| stop.frame_depth)
            .unwrap_or(0);
        loop {
            let Some((seq, command, arguments)) = self.shared.borrow_mut().next_request() else {
                let _ = session.die(TERMINATE_SIGNAL);
                return self.resume(StepArming::Continue);
            };
            match command.as_str() {
                "threads" => {
                    let name = self.shared.borrow().thread_name.clone();
                    self.respond(
                        seq,
                        "threads",
                        Some(Json::obj(vec![(
                            "threads",
                            Json::Array(vec![Json::obj(vec![
                                ("id", Json::Int(THREAD_ID)),
                                ("name", Json::from(name)),
                            ])]),
                        )])),
                    );
                }
                "stackTrace" => {
                    let frames = self.render_frames(session);
                    let total = frames.len();
                    self.respond(
                        seq,
                        "stackTrace",
                        Some(Json::obj(vec![
                            ("stackFrames", Json::Array(frames)),
                            ("totalFrames", Json::Int(total as i64)),
                        ])),
                    );
                }
                "scopes" => match self.render_scopes(&arguments) {
                    Ok(scopes) => {
                        self.respond(
                            seq,
                            "scopes",
                            Some(Json::obj(vec![("scopes", Json::Array(scopes))])),
                        );
                    }
                    Err(message) => self.fail(seq, "scopes", &message),
                },
                "variables" => match self.render_variables(session, &arguments) {
                    Ok(variables) => {
                        self.respond(
                            seq,
                            "variables",
                            Some(Json::obj(vec![("variables", Json::Array(variables))])),
                        );
                    }
                    Err(message) => self.fail(seq, "variables", &message),
                },
                "evaluate" => {
                    let Some(expression) = arguments.get("expression").and_then(Json::as_str)
                    else {
                        self.fail(seq, "evaluate", "evaluate requires an `expression`");
                        continue;
                    };
                    let expression = expression.to_string();
                    let context = arguments
                        .get("context")
                        .and_then(Json::as_str)
                        .unwrap_or("repl");
                    let mut eval_text = expression.clone();
                    let mode = match context {
                        // Implicit faces: the strict checker
                        // (design sections 5.3/7.5).
                        "hover" | "clipboard" => PowerMode::Implicit,
                        "watch" => {
                            self.register_watch(session, &expression);
                            // The watch face evaluates the
                            // registered form: a bare name the
                            // user typed means the variable, so
                            // the evaluation text is its `$`-form
                            // (the same text the stop-and-diff
                            // re-check will use at later stops).
                            let (kind, stored) = watch_registration_form(expression.trim());
                            eval_text = match kind {
                                BreakpointKind::Watch => format!("${stored}"),
                                _ => stored,
                            };
                            PowerMode::Implicit
                        }
                        // The debug console is an explicit user
                        // artifact: full power (section 5.3).
                        _ => PowerMode::Explicit,
                    };
                    let frame = match arguments.get("frameId").and_then(Json::as_i64) {
                        Some(frame_id) => match self.frame_ids.get(&frame_id) {
                            Some(index) => Some(*index),
                            None => {
                                self.fail(seq, "evaluate", "unknown frameId");
                                continue;
                            }
                        },
                        None => None,
                    };
                    self.pending_reply = Some(PendingReply::Evaluate { seq });
                    return StopLoopOutcome::Evaluate(EvaluationRequest {
                        mode,
                        text: eval_text,
                        frame,
                    });
                }
                "setVariable" => match self.prepare_set_variable(seq, &arguments) {
                    Ok((text, pending)) => {
                        self.pending_reply = Some(pending);
                        return StopLoopOutcome::Evaluate(EvaluationRequest {
                            mode: PowerMode::Explicit,
                            text,
                            frame: None,
                        });
                    }
                    Err(message) => self.fail(seq, "setVariable", &message),
                },
                "setBreakpoints" => {
                    let given = arguments
                        .get("source")
                        .and_then(|source| source.get("path"))
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_string();
                    let identity = file_identity(&given);
                    let specs = arguments
                        .get("breakpoints")
                        .and_then(Json::as_array)
                        .map(|items| items.to_vec())
                        .unwrap_or_default();
                    let rendered = apply_source_breakpoints(session, &identity, &specs);
                    self.respond(
                        seq,
                        "setBreakpoints",
                        Some(Json::obj(vec![("breakpoints", Json::Array(rendered))])),
                    );
                }
                "setFunctionBreakpoints" => {
                    let specs = arguments
                        .get("breakpoints")
                        .and_then(Json::as_array)
                        .map(|items| items.to_vec())
                        .unwrap_or_default();
                    let rendered = apply_function_breakpoints(session, &specs);
                    self.respond(
                        seq,
                        "setFunctionBreakpoints",
                        Some(Json::obj(vec![("breakpoints", Json::Array(rendered))])),
                    );
                }
                "setExceptionBreakpoints" => match apply_exception_filters(session, &arguments) {
                    Ok(()) => {
                        self.respond(
                            seq,
                            "setExceptionBreakpoints",
                            Some(Json::obj(vec![("breakpoints", Json::Array(vec![]))])),
                        );
                    }
                    Err(unknown) => {
                        self.fail(
                            seq,
                            "setExceptionBreakpoints",
                            &format!("unknown exception filter(s): {}", unknown.join(", ")),
                        );
                    }
                },
                "exceptionInfo" => {
                    // WP-15's request shape, with the row-12
                    // producers behind it: at an exception stop
                    // the session's recorded site answers — the
                    // same site DE-06 reported and the stop parked
                    // at. At any other stop the honest answer
                    // stands: the stop carries no exception.
                    let site = match session.last_stop().map(|stop| &stop.reason) {
                        Some(StopReason::Exception(_)) => session.last_exception_site().cloned(),
                        _ => None,
                    };
                    match site {
                        Some(site) => {
                            let id = super::session::exception_filter_id(site.kind);
                            let kind_name = match site.kind {
                                ExceptionKind::ErrTrap => "ERR trap",
                                ExceptionKind::Errexit => "errexit",
                                ExceptionKind::ParseError => "parse error",
                            };
                            let description = match site.kind {
                                ExceptionKind::ErrTrap => "an ERR trap action is about to run",
                                ExceptionKind::Errexit => {
                                    "an errexit decision is about to end the shell"
                                }
                                ExceptionKind::ParseError => "a parse error is terminating the run",
                            };
                            let location = site
                                .location
                                .map(|loc| format!(" at {}:{}", loc.file, loc.line))
                                .unwrap_or_default();
                            self.respond(
                                seq,
                                "exceptionInfo",
                                Some(Json::obj(vec![
                                    ("exceptionId", Json::from(id)),
                                    ("description", Json::from(description)),
                                    ("breakMode", Json::from("always")),
                                    (
                                        "details",
                                        Json::obj(vec![
                                            (
                                                "message",
                                                Json::from(format!(
                                                    "{kind_name} (status {}){location}",
                                                    site.status
                                                )),
                                            ),
                                            ("typeName", Json::from(id)),
                                        ]),
                                    ),
                                ])),
                            );
                        }
                        None => {
                            self.fail(
                                seq,
                                "exceptionInfo",
                                "exceptionInfo: the current stop carries no exception site",
                            );
                        }
                    }
                }
                "continue" => {
                    self.mark_running();
                    self.respond(
                        seq,
                        "continue",
                        Some(Json::obj(vec![("allThreadsContinued", Json::Bool(true))])),
                    );
                    return StopLoopOutcome::Resume(StepArming::Continue);
                }
                "next" => {
                    self.mark_running();
                    self.respond(seq, "next", None);
                    return StopLoopOutcome::Resume(StepArming::StepOver {
                        max_depth: frame_depth,
                    });
                }
                "stepIn" => {
                    self.mark_running();
                    self.respond(seq, "stepIn", None);
                    return StopLoopOutcome::Resume(StepArming::StepInto);
                }
                "stepOut" => {
                    self.mark_running();
                    self.respond(seq, "stepOut", None);
                    return StopLoopOutcome::Resume(StepArming::Finish { frame_depth });
                }
                "pause" => {
                    // Already parked: the pause is serviced by
                    // this very stop.
                    self.respond(seq, "pause", None);
                }
                "terminate" => {
                    self.respond(seq, "terminate", None);
                    let _ = session.die(TERMINATE_SIGNAL);
                    return self.resume(StepArming::Continue);
                }
                "disconnect" => {
                    self.respond(seq, "disconnect", None);
                    let terminate = arguments
                        .get("terminateDebuggee")
                        .and_then(Json::as_bool)
                        .unwrap_or(true);
                    if terminate {
                        let _ = session.die(TERMINATE_SIGNAL);
                        return self.resume(StepArming::Continue);
                    }
                    self.shared
                        .borrow()
                        .bridge
                        .running
                        .store(true, Ordering::Release);
                    return StopLoopOutcome::Detach;
                }
                "initialize" | "launch" | "attach" | "configurationDone" => {
                    self.fail(
                        seq,
                        &command,
                        &format!("'{command}' is not valid while the debuggee is stopped"),
                    );
                }
                _ => {
                    self.fail(seq, &command, &format!("unsupported request: {command}"));
                }
            }
        }
    }

    /// Build a `setVariable` write-back: an assignment in the
    /// design section 7.6 shape, evaluated explicitly so it
    /// travels the engine's real assignment path — attributes,
    /// namerefs, and the freshness mirrors stay coherent by
    /// construction, and a readonly refusal is the path's own
    /// answer, returned to the client as the error.
    fn prepare_set_variable(
        &mut self,
        seq: i64,
        arguments: &Json,
    ) -> Result<(String, PendingReply), String> {
        let reference = arguments
            .get("variablesReference")
            .and_then(Json::as_i64)
            .ok_or("setVariable requires a variablesReference")?;
        let name = arguments
            .get("name")
            .and_then(Json::as_str)
            .ok_or("setVariable requires a name")?
            .to_string();
        let value = arguments
            .get("value")
            .and_then(Json::as_str)
            .ok_or("setVariable requires a string value")?
            .to_string();
        match self.var_refs.get(&reference) {
            Some(VarRefKey::Scope { .. }) => {
                if !is_identifier(&name) {
                    return Err(format!("'{name}' is not a variable name"));
                }
                Ok((
                    format!("{}={}", name, shell_single_quote(&value)),
                    PendingReply::SetVariable {
                        seq,
                        parent: name,
                        element: None,
                        sent_value: value,
                    },
                ))
            }
            Some(VarRefKey::ArrayOf(parent)) => {
                let parent = parent.clone();
                // The element's display name is its subscript:
                // `[3]` for an indexed element, the bare key
                // for an associative one.
                let subscript = if name.starts_with('[') && name.ends_with(']') {
                    name.clone()
                } else {
                    format!("[{name}]")
                };
                Ok((
                    format!("{}{}={}", parent, subscript, shell_single_quote(&value)),
                    PendingReply::SetVariable {
                        seq,
                        parent,
                        element: Some(name),
                        sent_value: value,
                    },
                ))
            }
            _ => Err("unknown variablesReference".to_string()),
        }
    }

    /// Render the snapshot's frames as DAP stack frames,
    /// allocating the frame ids `evaluate` and `scopes` key on.
    fn render_frames(&mut self, session: &DebugSession) -> Vec<Json> {
        let Some(snapshot) = session.inspection() else {
            return Vec::new();
        };
        let mut frames = Vec::new();
        for (index, frame) in snapshot.frames.iter().enumerate() {
            let id = self.next_frame_serial + index as i64;
            self.frame_ids.insert(id, index);
            let line: i64 = frame.line.parse().unwrap_or(1);
            let mut source = vec![("name", Json::from(source_name(&frame.source)))];
            if std::path::Path::new(&frame.source).is_file() {
                source.push(("path", Json::from(frame.source.clone())));
            }
            frames.push(Json::obj(vec![
                ("id", Json::Int(id)),
                ("name", Json::from(frame.function.clone())),
                (
                    "source",
                    Json::Object(
                        source
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect(),
                    ),
                ),
                ("line", Json::Int(if line < 1 { 1 } else { line })),
                ("column", Json::Int(1)),
            ]));
        }
        self.next_frame_serial += snapshot.frames.len() as i64;
        frames
    }

    /// The two scopes of one frame: its locals and the globals.
    fn render_scopes(&mut self, arguments: &Json) -> Result<Vec<Json>, String> {
        let frame_id = arguments
            .get("frameId")
            .and_then(Json::as_i64)
            .ok_or("scopes requires a frameId")?;
        let frame = *self.frame_ids.get(&frame_id).ok_or("unknown frameId")?;
        let locals = self.alloc_ref(VarRefKey::Scope {
            frame,
            locals: true,
        });
        let globals = self.alloc_ref(VarRefKey::Scope {
            frame,
            locals: false,
        });
        Ok(vec![
            Json::obj(vec![
                ("name", Json::from("Locals")),
                ("variablesReference", Json::Int(locals)),
                ("expensive", Json::Bool(false)),
            ]),
            Json::obj(vec![
                ("name", Json::from("Globals")),
                ("variablesReference", Json::Int(globals)),
                ("expensive", Json::Bool(false)),
            ]),
        ])
    }

    /// Render one registry entry's children from the current
    /// snapshot.
    fn render_variables(
        &mut self,
        session: &DebugSession,
        arguments: &Json,
    ) -> Result<Vec<Json>, String> {
        let reference = arguments
            .get("variablesReference")
            .and_then(Json::as_i64)
            .ok_or("variables requires a variablesReference")?;
        let key = self
            .var_refs
            .get(&reference)
            .ok_or("unknown variablesReference")?;
        let snapshot = session
            .inspection()
            .ok_or("no inspection snapshot at this stop")?;
        let mut rendered = Vec::new();
        match key {
            VarRefKey::Scope { frame, locals } => {
                let wanted = if *locals { Some(*frame) } else { None };
                let names: Vec<String> = snapshot
                    .variables
                    .iter()
                    .filter(|variable| variable.scope_depth == wanted)
                    .map(|variable| variable.name.clone())
                    .collect();
                for name in names {
                    if let Some(variable) = snapshot.variable(&name) {
                        rendered.push(self.render_variable(variable));
                    }
                }
            }
            VarRefKey::ArrayOf(parent) => {
                let parent = parent.clone();
                let Some(variable) = snapshot.variable(&parent) else {
                    return Ok(Vec::new());
                };
                match &variable.value {
                    VariableValue::Indexed(elements) => {
                        for (index, value) in elements {
                            rendered.push(Json::obj(vec![
                                ("name", Json::from(format!("[{index}]"))),
                                ("value", Json::from(value.clone())),
                                ("evaluateName", Json::from(format!("${parent}[{index}]"))),
                                ("variablesReference", Json::Int(0)),
                            ]));
                        }
                    }
                    VariableValue::Associative(elements) => {
                        for (key, value) in elements {
                            rendered.push(Json::obj(vec![
                                ("name", Json::from(key.clone())),
                                ("value", Json::from(value.clone())),
                                ("evaluateName", Json::from(format!("${parent}[{key}]"))),
                                ("variablesReference", Json::Int(0)),
                            ]));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(rendered)
    }

    /// One snapshot variable as a DAP `Variable`: arrays get a
    /// registry reference for their elements; everything else
    /// renders inline.
    fn render_variable(&mut self, variable: &VariableSnapshot) -> Json {
        let (value, reference) = match &variable.value {
            VariableValue::Unset => (String::new(), 0),
            VariableValue::Scalar(text) => (text.clone(), 0),
            VariableValue::Indexed(_) | VariableValue::Associative(_) => (
                array_display_value(variable),
                self.alloc_ref(VarRefKey::ArrayOf(variable.name.clone())),
            ),
        };
        Json::obj(vec![
            ("name", Json::from(variable.name.clone())),
            ("value", Json::from(value)),
            ("evaluateName", Json::from(format!("${}", variable.name))),
            ("variablesReference", Json::Int(reference)),
        ])
    }

    fn respond(&mut self, seq: i64, command: &str, body: Option<Json>) {
        self.shared
            .borrow_mut()
            .transport
            .response(seq, command, true, body);
    }

    fn fail(&mut self, seq: i64, command: &str, message: &str) {
        self.shared
            .borrow_mut()
            .transport
            .error_response(seq, command, message);
    }
}
