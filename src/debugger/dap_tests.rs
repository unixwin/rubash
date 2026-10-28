//! Protocol fixtures for the DAP frontend (PLAN row 10):
//! scripted DAP conversations over an in-memory duplex —
//! deterministic, no editor, no sleeps. The client is
//! lock-step: it sends one request and reads until the owed
//! response or expected event arrives, so a conversation can
//! only proceed in the order the fixture writes it.
//!
//! The §8 golden DAP scenarios of the finalized design are
//! pinned here item by item; each test names its item.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use super::dap::{run_dap_server, DapLaunchPreset};
use super::json::Json;

// ---------------------------------------------------------------------
// In-memory duplex + scripted client
// ---------------------------------------------------------------------

struct PipeReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
    /// When set, a stalled conversation panics instead of
    /// hanging the test binary.
    timeout: Option<Duration>,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            let chunk = match self.timeout {
                Some(limit) => match self.rx.recv_timeout(limit) {
                    Ok(chunk) => chunk,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!("no bytes within {limit:?}"),
                        ));
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                },
                None => match self.rx.recv() {
                    Ok(chunk) => chunk,
                    Err(_) => return Ok(0),
                },
            };
            self.buf = chunk;
            self.pos = 0;
        }
        let n = (self.buf.len() - self.pos).min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct PipeWriter {
    tx: mpsc::Sender<Vec<u8>>,
}

impl Write for PipeWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.tx
            .send(data.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe closed"))?;
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn duplex() -> ((PipeReader, PipeWriter), (PipeReader, PipeWriter)) {
    let (tx_a, rx_a) = mpsc::channel::<Vec<u8>>();
    let (tx_b, rx_b) = mpsc::channel::<Vec<u8>>();
    (
        (
            PipeReader {
                rx: rx_b,
                buf: Vec::new(),
                pos: 0,
                timeout: Some(Duration::from_secs(120)),
            },
            PipeWriter { tx: tx_a },
        ),
        (
            PipeReader {
                rx: rx_a,
                buf: Vec::new(),
                pos: 0,
                timeout: None,
            },
            PipeWriter { tx: tx_b },
        ),
    )
}

/// One scripted DAP client: sends requests, and reads the
/// conversation in fixture order.
struct Client {
    reader: PipeReader,
    writer: PipeWriter,
    seq: i64,
    bytes: Vec<u8>,
    backlog: VecDeque<Json>,
    /// Every message received, in order (the transcript the
    /// assertions reason about).
    log: Vec<Json>,
}

impl Client {
    fn send_request(&mut self, command: &str, arguments: Json) -> i64 {
        self.seq += 1;
        let message = Json::obj(vec![
            ("seq", Json::Int(self.seq)),
            ("type", Json::from("request")),
            ("command", Json::from(command)),
            ("arguments", arguments),
        ]);
        let body = message.stringify();
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        self.writer
            .write_all(frame.as_bytes())
            .expect("client write");
        self.seq
    }

    /// Read exactly one more framed message off the wire.
    fn pump_one(&mut self) -> Json {
        loop {
            if let Some(message) = self.try_parse_frame() {
                self.log.push(message.clone());
                return message;
            }
            let mut chunk = [0u8; 16384];
            let n = self.reader.read(&mut chunk).unwrap_or_else(|error| {
                panic!(
                    "DAP conversation stalled ({error}); transcript so far: {:?}",
                    self.log
                )
            });
            assert!(
                n > 0,
                "DAP stream closed while a message was owed; transcript: {:?}",
                self.log
            );
            self.bytes.extend_from_slice(&chunk[..n]);
        }
    }

    fn try_parse_frame(&mut self) -> Option<Json> {
        let header_end = find_bytes(&self.bytes, b"\r\n\r\n")?;
        let header = String::from_utf8_lossy(&self.bytes[..header_end]).into_owned();
        let mut length: Option<usize> = None;
        for line in header.lines() {
            if let Some((name, value)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("Content-Length") {
                    length = value.trim().parse::<usize>().ok();
                }
            }
        }
        let length = length.expect("frame without Content-Length");
        if self.bytes.len() < header_end + 4 + length {
            return None;
        }
        let body = self.bytes[header_end + 4..header_end + 4 + length].to_vec();
        self.bytes.drain(..header_end + 4 + length);
        Json::parse(&String::from_utf8(body).expect("frame body is UTF-8")).ok()
    }

    /// Read until the response to `seq` arrives; anything else
    /// is backlogged for the fixture's next expectation.
    fn take_response(&mut self, seq: i64) -> Json {
        loop {
            if let Some(at) = self.backlog.iter().position(|m| is_response_for(m, seq)) {
                return self.backlog.remove(at).expect("position checked");
            }
            let message = self.pump_one();
            if is_response_for(&message, seq) {
                return message;
            }
            self.backlog.push_back(message);
        }
    }

    /// Read until the named event arrives; returns its body.
    fn take_event(&mut self, name: &str) -> Json {
        loop {
            if let Some(at) = self.backlog.iter().position(|m| is_event(m, name)) {
                let message = self.backlog.remove(at).expect("position checked");
                return message.get("body").cloned().unwrap_or(Json::Null);
            }
            let message = self.pump_one();
            if is_event(&message, name) {
                return message.get("body").cloned().unwrap_or(Json::Null);
            }
            self.backlog.push_back(message);
        }
    }

    /// Send a request and return its response.
    fn command(&mut self, command: &str, arguments: Json) -> Json {
        let seq = self.send_request(command, arguments);
        self.take_response(seq)
    }

    /// Send a request and return its response body, asserting
    /// success.
    fn ok(&mut self, command: &str, arguments: Json) -> Json {
        let response = self.command(command, arguments);
        assert_eq!(
            response.get("success").and_then(Json::as_bool),
            Some(true),
            "{command} response: {response}"
        );
        response.get("body").cloned().unwrap_or(Json::Null)
    }

    /// Read until `terminated`, collecting every `output`
    /// event as `(category, text)` and the `exited` code.
    /// Backlogged messages are consumed in arrival order —
    /// output events legitimately precede the stops a fixture
    /// already stepped past.
    fn collect_outputs_until_terminated(&mut self) -> (Vec<(String, String)>, i64) {
        let mut outputs = Vec::new();
        let mut exit_code: Option<i64> = None;
        loop {
            let message = match self.backlog.pop_front() {
                Some(message) => message,
                None => self.pump_one(),
            };
            if is_event(&message, "output") {
                let body = message.get("body").expect("output body");
                outputs.push((
                    body.get("category")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_string(),
                    body.get("output")
                        .and_then(Json::as_str)
                        .unwrap_or("")
                        .to_string(),
                ));
            } else if is_event(&message, "exited") {
                exit_code = message
                    .get("body")
                    .and_then(|body| body.get("exitCode"))
                    .and_then(Json::as_i64);
            } else if is_event(&message, "terminated") {
                break;
            }
        }
        (outputs, exit_code.expect("exited precedes terminated"))
    }
}

fn is_response_for(message: &Json, seq: i64) -> bool {
    message.get("type").and_then(Json::as_str) == Some("response")
        && message.get("request_seq").and_then(Json::as_i64) == Some(seq)
}

fn is_event(message: &Json, name: &str) -> bool {
    message.get("type").and_then(Json::as_str) == Some("event")
        && message.get("event").and_then(Json::as_str) == Some(name)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------
// Session scaffolding
// ---------------------------------------------------------------------

/// Serializes the gate-arming fixtures in this binary (the
/// gate is process-global; the same span the wiring fixtures
/// hold).
struct GateTestSpan {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl GateTestSpan {
    fn new() -> Self {
        let lock = crate::debugger::gate::GATE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self { _lock: lock }
    }
}

impl Drop for GateTestSpan {
    fn drop(&mut self) {
        crate::debugger::SESSION_ACTIVE.disarm();
    }
}

/// Write a fixture program; returns its path as a string.
fn program_file(name: &str, script: &str) -> String {
    let dir = std::env::temp_dir().join(format!("rubash-150-dap-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, script).expect("write program");
    path.to_string_lossy().into_owned()
}

/// Start a server on one end of a duplex; the client end and
/// the server's join handle (its exit code) come back.
fn start_server(preset: DapLaunchPreset) -> (Client, std::thread::JoinHandle<i32>) {
    let ((client_reader, client_writer), (server_reader, server_writer)) = duplex();
    let handle = std::thread::spawn(move || run_dap_server(server_reader, server_writer, preset));
    (
        Client {
            reader: client_reader,
            writer: client_writer,
            seq: 0,
            bytes: Vec::new(),
            backlog: VecDeque::new(),
            log: Vec::new(),
        },
        handle,
    )
}

/// The initialize + launch + initialized handshake every
/// scenario starts with. `extra` merges into the launch
/// arguments (e.g. `stopOnEntry`).
fn handshake(client: &mut Client, program: &str, extra: Vec<(&str, Json)>) {
    let mut launch_args = vec![("program", Json::from(program))];
    launch_args.extend(extra);
    client.ok("initialize", Json::obj(vec![]));
    client.ok("launch", Json::obj(launch_args));
    client.take_event("initialized");
}

/// The canonical spelling of a fixture path — the identity
/// the server (and therefore the engine) reports for it.
/// Temp-path assertions compare canonical-to-canonical:
/// on macOS the temp dir is a symlink (`/var` → `/private/var`)
/// and the raw spelling never matches the reported one.
fn canonical_string(path: &str) -> String {
    let resolved = std::fs::canonicalize(path).expect("fixture path canonicalizes");
    let text = resolved.to_string_lossy().into_owned();
    text.strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or(text)
}

/// Field shortcuts for assertions.
fn s<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key)
        .and_then(Json::as_str)
        .unwrap_or_else(|| panic!("{key} is not a string in {json}"))
}
fn i(json: &Json, key: &str) -> i64 {
    json.get(key)
        .and_then(Json::as_i64)
        .unwrap_or_else(|| panic!("{key} is not an integer in {json}"))
}
fn b(json: &Json, key: &str) -> bool {
    json.get(key)
        .and_then(Json::as_bool)
        .unwrap_or_else(|| panic!("{key} is not a boolean in {json}"))
}

// ---------------------------------------------------------------------
// §8 golden DAP scenarios
// ---------------------------------------------------------------------

/// §8 (1): the initialize handshake answers with the
/// capabilities the adapter actually has — each claimed flag
/// is backed by a fixture in this file, and the never-claims
/// (data breakpoints et al.) are absent.
#[test]
fn dap_initialize_capabilities_are_exactly_the_landed_features() {
    let _span = GateTestSpan::new();
    let (mut client, server) = start_server(DapLaunchPreset::default());
    let response = client.command("initialize", Json::obj(vec![]));
    assert_eq!(response.get("success").and_then(Json::as_bool), Some(true));
    let caps = response.get("body").expect("capabilities body");
    for claimed in [
        "supportsConfigurationDoneRequest",
        "supportsFunctionBreakpoints",
        "supportsConditionalBreakpoints",
        "supportsSetVariable",
        "supportsEvaluateForHovers",
        "supportsTerminateRequest",
        "supportTerminateDebuggee",
        // WP-15 (row 11): the filters and exceptionInfo landed
        // at the operation-set layer in this change, so the
        // claim lands in the same change (section 13's rule).
        "supportsExceptionInfoRequest",
    ] {
        assert_eq!(
            caps.get(claimed).and_then(Json::as_bool),
            Some(true),
            "capability {claimed}"
        );
    }
    for unclaimed in [
        "supportsDataBreakpoints",
        "supportsStepBack",
        "supportsRestartRequest",
        "supportsLoadedSourcesRequest",
    ] {
        assert!(caps.get(unclaimed).is_none(), "must not claim {unclaimed}");
    }
    // The offered filters are exactly the design's three ids.
    let filters = caps
        .get("exceptionBreakpointFilters")
        .and_then(Json::as_array)
        .expect("exceptionBreakpointFilters claimed with WP-15");
    let filter_ids: Vec<&str> = filters
        .iter()
        .filter_map(|filter| filter.get("filter").and_then(Json::as_str))
        .collect();
    assert_eq!(filter_ids, vec!["err-trap", "set -e", "parse-error"]);
    // A disconnect before any launch ends the conversation.
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// WP-15 (row 11): the exception filters are stored by the
/// session at the operation-set layer — known ids accepted,
/// unknown ids rejected as a batch — and `exceptionInfo` at a
/// stop that carries no exception answers with the honest
/// error. (At an exception stop it names the site — the
/// row-12 pins at the end of this file.)
#[test]
fn dap_exception_filters_and_exception_info() {
    let _span = GateTestSpan::new();
    let program = program_file("exceptions", "X=1\necho done\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok(
        "setExceptionBreakpoints",
        Json::obj(vec![(
            "filters",
            Json::Array(vec![Json::from("err-trap"), Json::from("parse-error")]),
        )]),
    );
    let rejected = client.command(
        "setExceptionBreakpoints",
        Json::obj(vec![("filters", Json::Array(vec![Json::from("bogus")]))]),
    );
    assert_eq!(
        rejected.get("success").and_then(Json::as_bool),
        Some(false),
        "an unknown filter id is an error, never silently dropped: {rejected:?}"
    );
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "entry");
    let info = client.command("exceptionInfo", Json::obj(vec![("threadId", Json::Int(1))]));
    assert_eq!(
        info.get("success").and_then(Json::as_bool),
        Some(false),
        "no exception at the entry stop: {info:?}"
    );
    client.ok("continue", Json::obj(vec![]));
    let (_, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// §8 (2): launch → configurationDone → terminated, with the
/// default entry stop in between.
#[test]
fn dap_launch_lifecycle_with_entry_stop() {
    let _span = GateTestSpan::new();
    let program = program_file("lifecycle", "X=1\necho \"ran $X\"\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "entry");
    assert_eq!(i(&stopped, "threadId"), 1);
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(outputs, vec![("stdout".to_string(), "ran 1\n".to_string())]);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// §8 (3): setBreakpoints — a live line verifies, a dead line
/// (a comment inside a fully-parsed file) reports
/// `verified: false` with the design's reason.
#[test]
fn dap_breakpoints_hit_and_dead_line_verdicts() {
    let _span = GateTestSpan::new();
    let program = program_file("verdicts", "A=1\n# a comment\nB=2\necho \"B=$B\"\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    let body = client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![
                    Json::obj(vec![("line", Json::Int(2))]),
                    Json::obj(vec![("line", Json::Int(3))]),
                ]),
            ),
        ]),
    );
    let entries = body
        .get("breakpoints")
        .and_then(Json::as_array)
        .expect("entries");
    assert_eq!(entries.len(), 2);
    assert_eq!(b(&entries[0], "verified"), false);
    assert_eq!(s(&entries[0], "message"), "no command on this line");
    assert_eq!(b(&entries[1], "verified"), true);
    let hit_id = i(&entries[1], "id");
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "breakpoint");
    let hits = stopped
        .get("hitBreakpointIds")
        .and_then(Json::as_array)
        .expect("hit ids");
    assert_eq!(hits, &[Json::Int(hit_id)]);
    // The stack at the hit reports line 3 of the program.
    let trace = client.ok("stackTrace", Json::obj(vec![]));
    let frames = trace
        .get("stackFrames")
        .and_then(Json::as_array)
        .expect("frames");
    assert!(!frames.is_empty());
    assert_eq!(i(&frames[0], "line"), 3);
    assert_eq!(
        frames[0]
            .get("source")
            .and_then(|src| src.get("path"))
            .and_then(Json::as_str),
        Some(canonical_string(&program).as_str())
    );
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(outputs, vec![("stdout".to_string(), "B=2\n".to_string())]);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// §8 (3), second half: a breakpoint in a file the program
/// sources starts Pending (the file is unparsed at
/// configuration time), flips to Verified through a
/// `breakpoint` changed event when the file is parsed during
/// the run (DE-11), and then hits.
#[test]
fn dap_breakpoint_in_sourced_file_pending_then_verified() {
    let _span = GateTestSpan::new();
    let lib = program_file("sourced_lib", "LIBVAR=from-lib\necho \"lib ran\"\n");
    // Identity contract (row-7 design 5.1): a file's debugger
    // identity is the spelling the engine itself reports — for a
    // sourced file, the spelling the script's `.` used. The
    // adapter canonicalizes the client's path, so the two meet
    // exactly when the script sources the file under that same
    // (canonical) spelling, as here. A script sourcing through a
    // divergent spelling (a symlinked directory, an 8.3 short
    // name) registers the divergent identity and the canonical
    // entry stays pending — canonical keying inside the session
    // core, shared by both frontends, is the mechanism-level home
    // for that convergence (row 11/12 residual, REPORT-noted).
    let program = program_file(
        "sourced_main",
        &format!(". \"{}\"\necho after\n", canonical_string(&lib)),
    );
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    let body = client.ok(
        "setBreakpoints",
        Json::obj(vec![
            ("source", Json::obj(vec![("path", Json::from(lib.clone()))])),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![("line", Json::Int(2))])]),
            ),
        ]),
    );
    let entries = body
        .get("breakpoints")
        .and_then(Json::as_array)
        .expect("entries");
    assert_eq!(b(&entries[0], "verified"), false);
    assert!(
        s(&entries[0], "message").starts_with("pending"),
        "pending message: {}",
        s(&entries[0], "message")
    );
    client.ok("configurationDone", Json::obj(vec![]));
    // The resolution flip is reported before the stop.
    let changed = client.take_event("breakpoint");
    assert_eq!(s(&changed, "reason"), "changed");
    let entry = changed.get("breakpoint").expect("breakpoint object");
    assert_eq!(b(entry, "verified"), true);
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "breakpoint");
    let trace = client.ok("stackTrace", Json::obj(vec![]));
    let frames = trace
        .get("stackFrames")
        .and_then(Json::as_array)
        .expect("frames");
    assert_eq!(i(&frames[0], "line"), 2);
    assert_eq!(
        frames[0]
            .get("source")
            .and_then(|src| src.get("path"))
            .and_then(Json::as_str),
        Some(canonical_string(&lib).as_str())
    );
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    // Both lines were written during the final free run, so
    // they arrive in one drain chunk (the adapter drains per
    // stop and at the epilogue; chunking is drain granularity).
    assert_eq!(
        outputs,
        vec![("stdout".to_string(), "lib ran\nafter\n".to_string())]
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// §8 (4) + (5): next / stepIn / stepOut across a function,
/// and the stackTrace cross-checked against the debuggee's own
/// FUNCNAME / BASH_LINENO views through evaluate.
#[test]
fn dap_stepping_across_a_function_and_stack_crosscheck() {
    let _span = GateTestSpan::new();
    let program = program_file(
        "stepping",
        "greet() {\n  echo \"in greet\"\n}\nX=1\ngreet\nY=2\n",
    );
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "entry");

    // The stepping sequence follows the session core's seam
    // model (pinned by rows 7/9): the function definition is
    // itself a step stop (a compound command; its snapshot
    // carries no frame yet), then lines 4 and 5 stop in main.
    client.ok("next", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "step");
    client.ok("next", Json::obj(vec![]));
    client.take_event("stopped");
    assert_eq!(top_frame_line(&mut client), 4);
    client.ok("next", Json::obj(vec![]));
    client.take_event("stopped");
    assert_eq!(top_frame_line(&mut client), 5);

    // stepIn crosses to the function-entry seam.
    client.ok("stepIn", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "step");

    // §8 (5): the DAP stack and the engine's own dynamic views
    // agree at this stop — frame 0 is greet (the entry seam
    // reports the definition line), frame 1 is the caller at
    // the call line 5, $FUNCNAME is greet, and
    // ${BASH_LINENO[0]} is the call line frame 1 reports.
    let trace = client.ok("stackTrace", Json::obj(vec![]));
    let frames = trace
        .get("stackFrames")
        .and_then(Json::as_array)
        .expect("frames");
    assert!(frames.len() >= 2, "frames: {frames:?}");
    assert_eq!(s(&frames[0], "name"), "greet");
    assert_eq!(i(&frames[1], "line"), 5);
    let body = client.ok(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("$FUNCNAME")),
            ("context", Json::from("repl")),
        ]),
    );
    assert_eq!(s(&body, "result"), "greet");
    let body = client.ok(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("${BASH_LINENO[0]}")),
            ("context", Json::from("repl")),
        ]),
    );
    assert_eq!(s(&body, "result"), "5");

    // One next inside the body: frame 0 is greet at line 2.
    client.ok("next", Json::obj(vec![]));
    client.take_event("stopped");
    let trace = client.ok("stackTrace", Json::obj(vec![]));
    let frames = trace
        .get("stackFrames")
        .and_then(Json::as_array)
        .expect("frames");
    assert_eq!(s(&frames[0], "name"), "greet");
    assert_eq!(i(&frames[0], "line"), 2);

    // stepOut leaves the function; the following next stops
    // back in main at line 6.
    client.ok("stepOut", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "step");
    client.ok("next", Json::obj(vec![]));
    client.take_event("stopped");
    assert_eq!(top_frame_line(&mut client), 6);

    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(
        outputs,
        vec![("stdout".to_string(), "in greet\n".to_string())]
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// The top frame's line at the current stop, via stackTrace.
fn top_frame_line(client: &mut Client) -> i64 {
    let trace = client.ok("stackTrace", Json::obj(vec![]));
    let frames = trace
        .get("stackFrames")
        .and_then(Json::as_array)
        .expect("frames");
    assert!(!frames.is_empty(), "a stop always has a frame");
    i(&frames[0], "line")
}

/// §8 (6): scopes and variables, including an indexed array's
/// children and an associative array's children, addressed
/// through variablesReference.
#[test]
fn dap_scopes_and_variables_including_array_children() {
    let _span = GateTestSpan::new();
    let program = program_file(
        "variables",
        "NAME=world\nARR=(alpha beta gamma)\ndeclare -A MAP=([k1]=v1 [k2]=v2)\necho end\n",
    );
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![("line", Json::Int(4))])]),
            ),
        ]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    client.take_event("stopped");
    let trace = client.ok("stackTrace", Json::obj(vec![]));
    let frames = trace
        .get("stackFrames")
        .and_then(Json::as_array)
        .expect("frames");
    let frame_id = i(&frames[0], "id");
    let scopes = client.ok("scopes", Json::obj(vec![("frameId", Json::Int(frame_id))]));
    let scope_list = scopes
        .get("scopes")
        .and_then(Json::as_array)
        .expect("scopes");
    let globals_ref = scope_list
        .iter()
        .find(|scope| s(scope, "name") == "Globals")
        .map(|scope| i(scope, "variablesReference"))
        .expect("globals scope");
    let globals = client.ok(
        "variables",
        Json::obj(vec![("variablesReference", Json::Int(globals_ref))]),
    );
    let vars = globals
        .get("variables")
        .and_then(Json::as_array)
        .expect("variables");
    let by_name = |name: &str| {
        vars.iter()
            .find(|var| s(var, "name") == name)
            .unwrap_or_else(|| panic!("variable {name} in {vars:?}"))
    };
    assert_eq!(s(by_name("NAME"), "value"), "world");
    let arr_ref = i(by_name("ARR"), "variablesReference");
    assert!(arr_ref != 0, "arrays carry a reference");
    let children = client.ok(
        "variables",
        Json::obj(vec![("variablesReference", Json::Int(arr_ref))]),
    );
    let elements = children
        .get("variables")
        .and_then(Json::as_array)
        .expect("elements");
    let pairs: Vec<(&str, &str)> = elements
        .iter()
        .map(|el| (s(el, "name"), s(el, "value")))
        .collect();
    assert_eq!(
        pairs,
        vec![("[0]", "alpha"), ("[1]", "beta"), ("[2]", "gamma")]
    );
    let map_ref = i(by_name("MAP"), "variablesReference");
    assert!(map_ref != 0);
    let children = client.ok(
        "variables",
        Json::obj(vec![("variablesReference", Json::Int(map_ref))]),
    );
    let elements = children
        .get("variables")
        .and_then(Json::as_array)
        .expect("elements");
    let mut pairs: Vec<(&str, &str)> = elements
        .iter()
        .map(|el| (s(el, "name"), s(el, "value")))
        .collect();
    pairs.sort();
    assert_eq!(pairs, vec![("k1", "v1"), ("k2", "v2")]);
    client.ok("continue", Json::obj(vec![]));
    let (_, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// §8 (7): evaluate — a safe form answers; the rejected
/// implicit forms (`${x:=y}`, `$(...)` in the hover context)
/// come back as DAP errors and provably never executed.
#[test]
fn dap_evaluate_safe_and_rejected_forms() {
    let _span = GateTestSpan::new();
    let dir = std::env::temp_dir().join(format!("rubash-150-dap-eval-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let marker = dir.join("pwned");
    let _ = std::fs::remove_file(&marker);
    let program = program_file("evaluate", "X=41\nY=2\necho $((X + Y))\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![("line", Json::Int(3))])]),
            ),
        ]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    client.take_event("stopped");

    // Safe forms, explicit (repl) and implicit (hover).
    let body = client.ok(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("$X")),
            ("context", Json::from("repl")),
        ]),
    );
    assert_eq!(s(&body, "result"), "41");
    let body = client.ok(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("$((X + 1))")),
            ("context", Json::from("hover")),
        ]),
    );
    assert_eq!(s(&body, "result"), "42");

    // Rejected implicit forms error over DAP...
    let response = client.command(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("${X:=99}")),
            ("context", Json::from("hover")),
        ]),
    );
    assert_eq!(response.get("success").and_then(Json::as_bool), Some(false));
    assert!(
        s(&response, "message").contains("implicit evaluation skipped"),
        "message: {}",
        s(&response, "message")
    );
    let response = client.command(
        "evaluate",
        Json::obj(vec![
            (
                "expression",
                Json::from(format!("$(touch \"{}\")", marker.to_string_lossy())),
            ),
            ("context", Json::from("hover")),
        ]),
    );
    assert_eq!(response.get("success").and_then(Json::as_bool), Some(false));
    // ...and never executed: X is untouched, the marker absent.
    let body = client.ok(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("$X")),
            ("context", Json::from("repl")),
        ]),
    );
    assert_eq!(s(&body, "result"), "41");
    assert!(!marker.exists(), "a rejected form must not execute");

    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(outputs, vec![("stdout".to_string(), "43\n".to_string())]);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// §8 (8): output categories — the debuggee's stdout and
/// stderr arrive as distinctly categorized output events,
/// drained before the stop they precede.
#[test]
fn dap_output_categories_stdout_stderr_before_the_stop() {
    let _span = GateTestSpan::new();
    let program = program_file("output", "echo out1\necho err1 >&2\necho out2\necho last\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![("line", Json::Int(4))])]),
            ),
        ]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    // The transcript order is the pin: both streams' events
    // precede the stopped event at line 4.
    let mut categories = Vec::new();
    loop {
        let message = client.pump_one();
        if is_event(&message, "output") {
            let body = message.get("body").expect("output body");
            categories.push((
                body.get("category")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                body.get("output")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
            ));
        } else if is_event(&message, "stopped") {
            break;
        } else {
            panic!("unexpected message before the stop: {message}");
        }
    }
    assert_eq!(
        categories,
        vec![
            ("stdout".to_string(), "out1\nout2\n".to_string()),
            ("stderr".to_string(), "err1\n".to_string()),
        ]
    );
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(outputs, vec![("stdout".to_string(), "last\n".to_string())]);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

// ---------------------------------------------------------------------
// Watch, write-back, terminate, pause (brief items 3-4)
// ---------------------------------------------------------------------

/// Watch over DAP (row-10 deliverable 3's protocol face): an
/// `evaluate` in the watch context registers the expression in
/// the session's watch table; the §5.4 stop-and-diff then
/// tracks it — a step over the changing line stops with reason
/// `data breakpoint` and the banner arrives as a console
/// output event before the stopped event.
#[test]
fn dap_watch_registration_step_diff_and_banner() {
    let _span = GateTestSpan::new();
    let program = program_file("watch", "V=one\nW=1\nV=two\necho \"V=$V\"\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![("line", Json::Int(2))])]),
            ),
        ]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    client.take_event("stopped");

    // Register the watch at the line-2 stop; it evaluates to
    // the current value, and the baseline is this stop.
    let body = client.ok(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("V")),
            ("context", Json::from("watch")),
        ]),
    );
    assert_eq!(s(&body, "result"), "one");

    // A step that does not touch V: a plain step stop, no
    // banner (the transcript holds no console output).
    client.ok("next", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "step");
    assert!(
        !client.log.iter().any(|m| is_event(m, "output")
            && m.get("body")
                .and_then(|body| body.get("category"))
                .and_then(Json::as_str)
                == Some("console")),
        "no banner for an unchanged watch"
    );

    // The step over `V=two` fires the diff: console banner
    // first, then the stopped event with the watch reason.
    client.ok("next", Json::obj(vec![]));
    let banner = client.take_event("output");
    assert_eq!(s(&banner, "category"), "console");
    assert_eq!(s(&banner, "output"), "watch: V 'one' → 'two'\n");
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "data breakpoint");

    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(outputs, vec![("stdout".to_string(), "V=two\n".to_string())]);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// Write-back over DAP (§7.6 through the session core): a
/// setVariable at a stop changes the debuggee's variable
/// through the real assignment path — the script itself prints
/// the new value after continue — and a readonly write is
/// refused with the path's own error.
#[test]
fn dap_set_variable_writeback_and_readonly_refusal() {
    let _span = GateTestSpan::new();
    let program = program_file(
        "writeback",
        "MSG=hello\ndeclare -r LOCKED=yes\necho \"MSG=$MSG\"\n",
    );
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![("line", Json::Int(3))])]),
            ),
        ]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    client.take_event("stopped");
    let trace = client.ok("stackTrace", Json::obj(vec![]));
    let frames = trace
        .get("stackFrames")
        .and_then(Json::as_array)
        .expect("frames");
    let scopes = client.ok(
        "scopes",
        Json::obj(vec![("frameId", Json::Int(i(&frames[0], "id")))]),
    );
    let scope_list = scopes
        .get("scopes")
        .and_then(Json::as_array)
        .expect("scopes");
    let globals_ref = scope_list
        .iter()
        .find(|scope| s(scope, "name") == "Globals")
        .map(|scope| i(scope, "variablesReference"))
        .expect("globals scope");

    let body = client.ok(
        "setVariable",
        Json::obj(vec![
            ("variablesReference", Json::Int(globals_ref)),
            ("name", Json::from("MSG")),
            ("value", Json::from("goodbye")),
        ]),
    );
    assert_eq!(s(&body, "value"), "goodbye");

    // The readonly refusal is the real path's own answer.
    let response = client.command(
        "setVariable",
        Json::obj(vec![
            ("variablesReference", Json::Int(globals_ref)),
            ("name", Json::from("LOCKED")),
            ("value", Json::from("nope")),
        ]),
    );
    assert_eq!(response.get("success").and_then(Json::as_bool), Some(false));

    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    // The refused write-back also left the real path's own
    // diagnostic on the debuggee's stderr (drained when the
    // evaluation was answered, ahead of the final output):
    // the write travels the real assignment path, diagnostics
    // included — that is what "no debugger-private write
    // path" means on the wire.
    assert_eq!(
        outputs,
        vec![
            (
                "stderr".to_string(),
                format!(
                    "{}: line 1: LOCKED: readonly variable\n",
                    canonical_string(&program)
                )
            ),
            ("stdout".to_string(), "MSG=goodbye\n".to_string()),
        ]
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// The brief's closing beat: terminate at a stop ends the
/// debuggee with the signal status (128 + SIGTERM), reported
/// through exited + terminated, and the server exits with it.
#[test]
fn dap_terminate_at_a_stop_reports_signal_status() {
    let _span = GateTestSpan::new();
    let program = program_file("terminate", "A=1\nB=2\necho unreachable\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![("line", Json::Int(2))])]),
            ),
        ]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    client.take_event("stopped");
    client.ok("terminate", Json::obj(vec![]));
    let (_, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 143);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 143);
}

/// Pause during a free run: the reader thread applies the
/// request to the session's asynchronous stop flag and the
/// boundary poll parks the debuggee (DE-07). The script is a
/// pure-builtin loop long enough for the reader thread to be
/// scheduled and land the pause (thousands of iterations),
/// short enough for a debug test suite (seconds, not
/// minutes) — no external binary, no sleep.
#[test]
fn dap_pause_during_a_free_run() {
    let _span = GateTestSpan::new();
    let program = program_file(
        "pause",
        "I=0\nwhile [ $I -lt 6000 ]; do I=$((I + 1)); done\necho \"done $I\"\n",
    );
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok("configurationDone", Json::obj(vec![]));
    client.take_event("stopped"); // entry
    client.ok("continue", Json::obj(vec![]));
    let pause_seq = client.send_request("pause", Json::obj(vec![]));
    // The pause response is owed at the stop that services it;
    // the stopped event follows it.
    let response = client.take_response(pause_seq);
    assert_eq!(response.get("success").and_then(Json::as_bool), Some(true));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "pause");
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(
        outputs,
        vec![("stdout".to_string(), "done 6000\n".to_string())]
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

// ---------------------------------------------------------------------
// Watch wiring pins (the §5.4 pre-pass in debugger_serve_stop)
// ---------------------------------------------------------------------

use std::cell::RefCell;
use std::rc::Rc;

use super::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use super::event::{DebugEvent, StopReason, WatchChange};
use super::session::{DebugSession, StepArming};
use crate::executor::Executor;

/// What the scripted loop does at the entry stop.
enum EntryAction {
    None,
    Watch(String),
    WatchExpression(String),
    WatchThenDisable(String),
}

/// A stop loop that records stop reasons and DE-05 payloads,
/// applies one entry action, and resumes under a fixed arming.
struct WatchProbeLoop {
    action: EntryAction,
    arming: StepArming,
    stops: Rc<RefCell<Vec<StopReason>>>,
    changes: Rc<RefCell<Vec<WatchChange>>>,
    served_entry: bool,
}

impl StopLoop for WatchProbeLoop {
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        for event in session.drain_events() {
            if let DebugEvent::WatchChanged(change) = event {
                self.changes.borrow_mut().push(change);
            }
        }
        if let Some(stop) = session.last_stop() {
            self.stops.borrow_mut().push(stop.reason.clone());
        }
        if !self.served_entry {
            self.served_entry = true;
            match &self.action {
                EntryAction::None => {}
                EntryAction::Watch(name) => {
                    session.breakpoints_mut().insert_watch(name.clone());
                }
                EntryAction::WatchExpression(text) => {
                    session
                        .breakpoints_mut()
                        .insert_watch_expression(text.clone());
                }
                EntryAction::WatchThenDisable(name) => {
                    let id = session.breakpoints_mut().insert_watch(name.clone());
                    session.breakpoints_mut().set_enabled(id, false);
                }
            }
        }
        StopLoopOutcome::Resume(self.arming)
    }
}

struct WatchRun {
    status: i32,
    stops: Vec<StopReason>,
    changes: Vec<WatchChange>,
    executor: Executor,
}

fn run_watch_probe(name: &str, script: &str, action: EntryAction, arming: StepArming) -> WatchRun {
    let dir = std::env::temp_dir().join(format!(
        "rubash-150-dap-watch-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("{name}.sh"));
    std::fs::write(&path, script).expect("write script");
    let path = path.to_string_lossy().into_owned();
    // The path is interpolated into shell source: quoted, per
    // the row-9/consumer lesson (a bare Windows path loses its
    // backslashes to the lexer).
    let input = format!(". \"{path}\"\n");
    let stops = Rc::new(RefCell::new(Vec::new()));
    let changes = Rc::new(RefCell::new(Vec::new()));
    let driver = SessionDriver::new(
        DebugSession::new(),
        Box::new(WatchProbeLoop {
            action,
            arming,
            stops: stops.clone(),
            changes: changes.clone(),
            served_entry: false,
        }),
    );
    let mut executor = Executor::new();
    executor.install_debug_session(driver);
    let status = crate::script_driver::run_source(&mut executor, &input, false);
    WatchRun {
        status,
        stops: stops.take(),
        changes: changes.take(),
        executor,
    }
}

/// Announcement model: a change that coincides with a
/// breakpoint stop is announced (DE-05) at that stop; the stop
/// keeps its own reason.
#[test]
fn watch_prepass_announces_change_at_a_breakpoint_stop() {
    let _span = GateTestSpan::new();
    let run = run_watch_probe_with_breakpoint();
    assert_eq!(run.status, 0);
    assert_eq!(run.stops.len(), 2);
    assert!(matches!(run.stops[1], StopReason::Breakpoint(_)));
    assert_eq!(run.changes.len(), 1);
    assert_eq!(run.changes[0].old_value, None);
    assert_eq!(run.changes[0].new_value.as_deref(), Some("1"));
    assert_eq!(run.changes[0].expression, "V");
}

fn run_watch_probe_with_breakpoint() -> WatchRun {
    let dir = std::env::temp_dir().join(format!(
        "rubash-150-dap-watch-announce2-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("announce2.sh");
    std::fs::write(&path, "V=1\nV=2\nV=3\n").expect("write script");
    let path = path.to_string_lossy().into_owned();
    let input = format!(". \"{path}\"\n");
    let stops = Rc::new(RefCell::new(Vec::new()));
    let changes = Rc::new(RefCell::new(Vec::new()));
    let mut session = DebugSession::new();
    session.set_line_breakpoint(path.clone(), 2);
    // The watch is registered before the run, so the entry
    // stop's pre-pass establishes its baseline (V unset); a
    // watch inserted during a serve only baselines at the
    // NEXT stop — that stop's pre-pass already ran.
    session.breakpoints_mut().insert_watch("V".to_string());
    let driver = SessionDriver::new(
        session,
        Box::new(WatchProbeLoop {
            action: EntryAction::None,
            arming: StepArming::Continue,
            stops: stops.clone(),
            changes: changes.clone(),
            served_entry: false,
        }),
    );
    let mut executor = Executor::new();
    executor.install_debug_session(driver);
    let status = crate::script_driver::run_source(&mut executor, &input, false);
    WatchRun {
        status,
        stops: stops.take(),
        changes: changes.take(),
        executor,
    }
}

/// Baseline + quiet: the first step stop after registration
/// diffs against the entry-stop baseline (unset → the value);
/// later stops where the value does not move stay silent.
#[test]
fn watch_prepass_baseline_then_quiet_when_unchanged() {
    let _span = GateTestSpan::new();
    let run = run_watch_probe(
        "baseline",
        "V=1\nW=2\nX=3\n",
        EntryAction::Watch("V".to_string()),
        StepArming::StepInto,
    );
    assert_eq!(run.status, 0);
    assert!(run.stops.len() >= 3, "stops: {:?}", run.stops);
    assert_eq!(run.changes.len(), 1);
    assert_eq!(run.changes[0].old_value, None);
    assert_eq!(run.changes[0].new_value.as_deref(), Some("1"));
}

/// An element write to a watched array is detected between
/// stops (the tracked-name freshness path and the rendered
/// comparison agree on the outcome).
#[test]
fn watch_prepass_detects_array_element_write() {
    let _span = GateTestSpan::new();
    let dir =
        std::env::temp_dir().join(format!("rubash-150-dap-watch-array-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("array.sh");
    std::fs::write(&path, "A=(x y)\nA[1]=z\necho done\n").expect("write script");
    let path = path.to_string_lossy().into_owned();
    let input = format!(". \"{path}\"\n");
    let stops = Rc::new(RefCell::new(Vec::new()));
    let changes = Rc::new(RefCell::new(Vec::new()));
    let mut session = DebugSession::new();
    session.set_line_breakpoint(path.clone(), 2);
    session.set_line_breakpoint(path.clone(), 3);
    // Registered before the run (see the announce pin): the
    // entry stop baselines A as unset.
    session.breakpoints_mut().insert_watch("A".to_string());
    let driver = SessionDriver::new(
        session,
        Box::new(WatchProbeLoop {
            action: EntryAction::None,
            arming: StepArming::Continue,
            stops: stops.clone(),
            changes: changes.clone(),
            served_entry: false,
        }),
    );
    let mut executor = Executor::new();
    executor.install_debug_session(driver);
    let status = crate::script_driver::run_source(&mut executor, &input, false);
    assert_eq!(status, 0);
    assert_eq!(stops.borrow().len(), 3);
    let changes = changes.borrow();
    assert_eq!(changes.len(), 2, "changes: {changes:?}");
    // The first change is the array's creation; the second is
    // the element write alone, and the rendering shows it.
    assert_eq!(changes[0].old_value, None);
    assert_eq!(changes[1].old_value.as_deref(), Some("([0]=x [1]=y)"));
    assert_eq!(changes[1].new_value.as_deref(), Some("([0]=x [1]=z)"));
}

/// A disabled watch is skipped by the pre-pass entirely.
#[test]
fn watch_prepass_skips_disabled_entries() {
    let _span = GateTestSpan::new();
    let run = run_watch_probe(
        "disabled",
        "V=1\nV=2\n",
        EntryAction::WatchThenDisable("V".to_string()),
        StepArming::StepInto,
    );
    assert_eq!(run.status, 0);
    assert!(run.changes.is_empty(), "changes: {:?}", run.changes);
}

/// A rejected watch expression is skipped and marked by its
/// absence (§7.5): it never diffs, never executes, and the run
/// is unaffected.
#[test]
fn watch_prepass_rejected_expression_never_diffs() {
    let _span = GateTestSpan::new();
    let run = run_watch_probe(
        "rejected",
        "V=1\nV=2\n",
        EntryAction::WatchExpression("$(echo boo)".to_string()),
        StepArming::StepInto,
    );
    assert_eq!(run.status, 0);
    assert!(run.changes.is_empty(), "changes: {:?}", run.changes);
    assert_eq!(run.executor.get_env("V").as_deref(), Some("2"));
}

/// The design's documented limit (§5.4): a watch alone never
/// keeps the consult awake — with no other entries the run is
/// free after the entry stop, produces no further stops, and
/// reports no changes even though the variable moved.
#[test]
fn watch_only_table_stays_inert_between_stops() {
    let _span = GateTestSpan::new();
    let run = run_watch_probe(
        "inert",
        "V=1\nV=2\nDONE=yes\n",
        EntryAction::Watch("V".to_string()),
        StepArming::Continue,
    );
    assert_eq!(run.status, 0);
    assert_eq!(run.stops, vec![StopReason::Entry]);
    assert!(run.changes.is_empty());
    assert_eq!(run.executor.get_env("DONE").as_deref(), Some("yes"));
}

// ---------------------------------------------------------------------
// KPI (row-9 residual): the step-echo loop through the real
// adapter. Ignored: run explicitly, release mode. Numbers are
// archived in the row-10 REPORT with their caliber; nothing
// here asserts a timing bound.
// ---------------------------------------------------------------------

#[test]
#[ignore = "KPI harness (row 10): run explicitly with --release --nocapture"]
fn dap_kpi_4500_line_step_echo() {
    let _span = GateTestSpan::new();
    let mut script = String::from("ACC=0\n");
    for _ in 0..4498 {
        script.push_str("ACC=$((ACC+1))\n");
    }
    script.push_str("DONE=$ACC\n");
    let program = program_file("kpi4500", &script);

    for round in 1..=2 {
        let (mut client, server) = start_server(DapLaunchPreset::default());
        handshake(&mut client, &program, vec![]);
        let started = std::time::Instant::now();
        client.ok("configurationDone", Json::obj(vec![]));
        // The conversation is a strict alternation — a stopped
        // event, answered by `next` — until the run ends and
        // exited/terminated close it. (On this flat script the
        // adapter's `next` arms StepOver, which stops at every
        // line exactly as row 9's StepInto echo did.)
        let mut stops = 0usize;
        let mut exit_code: Option<i64> = None;
        loop {
            let message = client.pump_one();
            if is_event(&message, "stopped") {
                stops += 1;
                client.send_request("next", Json::obj(vec![]));
            } else if is_event(&message, "exited") {
                exit_code = message
                    .get("body")
                    .and_then(|body| body.get("exitCode"))
                    .and_then(Json::as_i64);
            } else if is_event(&message, "terminated") {
                break;
            }
        }
        let elapsed = started.elapsed();
        assert_eq!(exit_code, Some(0));
        // Entry stop + the source command's own seam + one stop
        // per file line: 4502. Row 9's harness ran the script as
        // the top-level input (no source seam), hence its 4501;
        // the per-stop figures remain directly comparable.
        assert_eq!(stops, 4502, "entry + source seam + one stop per line");
        println!(
            "KPI dap step-echo round {round}: {elapsed:?} for 4502 stops ({:.3} ms/stop)",
            elapsed.as_secs_f64() * 1000.0 / 4502.0
        );
        client.ok("disconnect", Json::obj(vec![]));
        assert_eq!(server.join().expect("server thread"), 0);
    }
}

// ---------------------------------------------------------------------
// Capability backing fixtures (every claimed flag has one)
// ---------------------------------------------------------------------

/// supportsConditionalBreakpoints: a condition on a line
/// breakpoint is the engine's own full-power condition
/// evaluation (§5.3) — the stop lands only on the pass where
/// the condition holds.
#[test]
fn dap_conditional_breakpoint_stops_only_when_the_condition_holds() {
    let _span = GateTestSpan::new();
    let program = program_file(
        "conditional",
        "I=0\nwhile [ $I -lt 5 ]; do\n  I=$((I + 1))\ndone\necho \"I=$I\"\n",
    );
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    let body = client.ok(
        "setBreakpoints",
        Json::obj(vec![
            (
                "source",
                Json::obj(vec![("path", Json::from(program.clone()))]),
            ),
            (
                "breakpoints",
                Json::Array(vec![Json::obj(vec![
                    ("line", Json::Int(3)),
                    ("condition", Json::from("[ $I -eq 2 ]")),
                ])]),
            ),
        ]),
    );
    let entries = body
        .get("breakpoints")
        .and_then(Json::as_array)
        .expect("entries");
    assert_eq!(b(&entries[0], "verified"), true);
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "breakpoint");
    // The stop is the pass where I is exactly 2.
    let body = client.ok(
        "evaluate",
        Json::obj(vec![
            ("expression", Json::from("$I")),
            ("context", Json::from("repl")),
        ]),
    );
    assert_eq!(s(&body, "result"), "2");
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(outputs, vec![("stdout".to_string(), "I=5\n".to_string())]);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// supportsFunctionBreakpoints: a function breakpoint on a
/// program-defined function verifies at configuration time
/// (the launch pre-registered view proves it) and stops at
/// the call with the function reason; an unknown name stays
/// pending and is not an error.
#[test]
fn dap_function_breakpoints_verify_and_hit() {
    let _span = GateTestSpan::new();
    let program = program_file(
        "funcbp",
        "greet() {\n  echo \"in greet\"\n}\ngreet\necho after\n",
    );
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(
        &mut client,
        &program,
        vec![("stopOnEntry", Json::Bool(false))],
    );
    let body = client.ok(
        "setFunctionBreakpoints",
        Json::obj(vec![(
            "breakpoints",
            Json::Array(vec![
                Json::obj(vec![("name", Json::from("greet"))]),
                Json::obj(vec![("name", Json::from("nosuch"))]),
            ]),
        )]),
    );
    let entries = body
        .get("breakpoints")
        .and_then(Json::as_array)
        .expect("entries");
    assert_eq!(b(&entries[0], "verified"), true);
    assert_eq!(b(&entries[1], "verified"), false);
    assert!(s(&entries[1], "message").starts_with("pending"));
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "function breakpoint");
    // An unsupported request at a stop is a clean error, and
    // the session is undisturbed by it.
    let response = client.command("restart", Json::obj(vec![]));
    assert_eq!(response.get("success").and_then(Json::as_bool), Some(false));
    assert!(s(&response, "message").contains("unsupported request"));
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(
        outputs,
        vec![("stdout".to_string(), "in greet\nafter\n".to_string())]
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

/// attach without a program is refused with the design's
/// reason (§13: background jobs are separate OS processes).
#[test]
fn dap_attach_without_a_program_is_refused_with_a_reason() {
    let _span = GateTestSpan::new();
    let (mut client, server) = start_server(DapLaunchPreset::default());
    client.ok("initialize", Json::obj(vec![]));
    let response = client.command("attach", Json::obj(vec![]));
    assert_eq!(response.get("success").and_then(Json::as_bool), Some(false));
    assert!(
        s(&response, "message").contains("launch-equivalent"),
        "message: {}",
        s(&response, "message")
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 1);
}

/// A noDebug launch runs the program plainly through the
/// adapter: no stops, output events still flow, and the
/// termination pair closes the conversation.
#[test]
fn dap_no_debug_launch_runs_plainly() {
    let _span = GateTestSpan::new();
    let program = program_file("nodebug", "echo plain\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![("noDebug", Json::Bool(true))]);
    client.ok("configurationDone", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    assert_eq!(outputs, vec![("stdout".to_string(), "plain\n".to_string())]);
    assert!(
        !client.log.iter().any(|m| is_event(m, "stopped")),
        "no stops in a noDebug run"
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

// ---------------------------------------------------------------------
// PLAN row 12: the DE-06 exception-site producers, DAP face.
// A selected filter parks the run with stopped reason
// "exception"; `exceptionInfo` names the recorded site; the
// pending engine action still runs after `continue`.
// ---------------------------------------------------------------------

#[test]
fn dap_err_trap_exception_stop_reports_and_parks() {
    let _span = GateTestSpan::new();
    let program = program_file("errsite", "trap 'echo TRAPPED' ERR\nfalse\necho after\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok(
        "setExceptionBreakpoints",
        Json::obj(vec![("filters", Json::Array(vec![Json::from("err-trap")]))]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "entry");
    client.ok("continue", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "exception");
    // exceptionInfo names the site the session parked at.
    let info = client.ok("exceptionInfo", Json::obj(vec![("threadId", Json::Int(1))]));
    assert_eq!(
        info.get("exceptionId").and_then(Json::as_str),
        Some("err-trap")
    );
    let message = info
        .get("details")
        .and_then(|details| details.get("message"))
        .and_then(Json::as_str)
        .unwrap_or("");
    assert!(
        message.contains("ERR trap") && message.contains("status 1"),
        "exceptionInfo message: {message}"
    );
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 0);
    // The DE-06 console note arrived first; the trap action ran
    // after the resume; the script then completed. The capture
    // drain may merge adjacent stdout into one chunk, so the
    // assertions are chunk-robust: order by first chunk that
    // carries the text, and compare the joined stdout whole.
    let console_at = outputs
        .iter()
        .position(|(category, text)| {
            category == "console" && text.contains("exception site: ERR trap (status 1)")
        })
        .expect("the DE-06 console note is emitted");
    let trapped_at = outputs
        .iter()
        .position(|(category, text)| category == "stdout" && text.contains("TRAPPED"))
        .expect("the trap action ran after the resume");
    assert!(
        console_at < trapped_at,
        "the note precedes the trap action: {outputs:?}"
    );
    let joined_stdout: String = outputs
        .iter()
        .filter(|(category, _)| category == "stdout")
        .map(|(_, text)| text.as_str())
        .collect();
    assert_eq!(
        joined_stdout, "TRAPPED\nafter\n",
        "the trap action ran, then the script completed"
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 0);
}

#[test]
fn dap_errexit_exception_stop_and_wrong_filter_passes_through() {
    let _span = GateTestSpan::new();
    let script = "set -e\nX=7\nfalse\necho unreachable\n";
    // The matching filter parks the run at the errexit site.
    let program = program_file("errexitsite", script);
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok(
        "setExceptionBreakpoints",
        Json::obj(vec![("filters", Json::Array(vec![Json::from("set -e")]))]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "entry");
    client.ok("continue", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "exception");
    let info = client.ok("exceptionInfo", Json::obj(vec![("threadId", Json::Int(1))]));
    assert_eq!(
        info.get("exceptionId").and_then(Json::as_str),
        Some("set -e")
    );
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 1, "the errexit exit still happens");
    assert!(
        outputs
            .iter()
            .all(|(_, text)| !text.contains("unreachable")),
        "the line after the failing command never runs: {outputs:?}"
    );
    // One death, one report: the frames the exit unwinds
    // through (the `.` that sourced this program, here) must
    // not re-report the site the origin frame already parked.
    let notes = outputs
        .iter()
        .filter(|(category, text)| {
            category == "console" && text.contains("exception site: errexit (status 1)")
        })
        .count();
    assert_eq!(notes, 1, "one exception note per death: {outputs:?}");
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 1);

    // A filter naming a different kind does not park this run:
    // it terminates straight through with the same status.
    let program = program_file("errexitother", script);
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok(
        "setExceptionBreakpoints",
        Json::obj(vec![(
            "filters",
            Json::Array(vec![Json::from("parse-error")]),
        )]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "entry");
    client.ok("continue", Json::obj(vec![]));
    let (_, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 1);
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 1);
}

#[test]
fn dap_parse_error_exception_stop() {
    let _span = GateTestSpan::new();
    let program = program_file("parsesite", "echo before\necho )\necho after\n");
    let (mut client, server) = start_server(DapLaunchPreset::default());
    handshake(&mut client, &program, vec![]);
    client.ok(
        "setExceptionBreakpoints",
        Json::obj(vec![(
            "filters",
            Json::Array(vec![Json::from("parse-error")]),
        )]),
    );
    client.ok("configurationDone", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "entry");
    client.ok("continue", Json::obj(vec![]));
    let stopped = client.take_event("stopped");
    assert_eq!(s(&stopped, "reason"), "exception");
    let info = client.ok("exceptionInfo", Json::obj(vec![("threadId", Json::Int(1))]));
    assert_eq!(
        info.get("exceptionId").and_then(Json::as_str),
        Some("parse-error")
    );
    client.ok("continue", Json::obj(vec![]));
    let (outputs, exit_code) = client.collect_outputs_until_terminated();
    assert_eq!(exit_code, 2, "the parse error still ends the run");
    assert!(
        outputs
            .iter()
            .any(|(category, text)| category == "stdout" && text.contains("before")),
        "the valid prefix ran: {outputs:?}"
    );
    assert!(
        outputs
            .iter()
            .any(|(category, text)| category == "stderr" && text.contains("syntax error")),
        "the diagnostic was emitted before the park: {outputs:?}"
    );
    assert!(
        outputs.iter().all(|(_, text)| !text.contains("after")),
        "the unit after the error never runs: {outputs:?}"
    );
    client.ok("disconnect", Json::obj(vec![]));
    assert_eq!(server.join().expect("server thread"), 2);
}
