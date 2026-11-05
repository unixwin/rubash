//! PLAN row 6 exit gate: a consumer test at niubash's scale,
//! driven end-to-end through the Tier-A-shaped surface.
//!
//! Act 1 is the host act (§5.3's census made flesh): a host drives
//! a session line by line and touches state ONLY through
//! [`StateViews`] between runs — variables, arrays, options, last
//! status, positional parameters, cwd, aliases, functions — and the
//! shell it drives observes every write, because the views write
//! through the engine's own machinery.
//!
//! Act 2 is the debugger act: a session stops a real debuggee at
//! two breakpoints, and the inspection snapshot the consult
//! captured is checked item by item against what the debuggee
//! itself prints at the same stop (`declare -p`, `caller`) — the
//! alignment table of the row-6 gate, enforced as a test.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use super::views::{InspectionSnapshot, VariableValue};
use crate::debugger::driver::{SessionDriver, StopLoop, StopLoopOutcome};
use crate::debugger::inspect::{render_caller, render_where, VariablePartition};
use crate::debugger::session::{DebugSession, StepArming};
use crate::executor::Executor;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-150-views-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run(executor: &mut Executor, line: &str) -> i32 {
    crate::script_driver::run_source(executor, line, false)
}

fn read(dir: &PathBuf, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("output file written by the script")
}

/// The native spelling of a path the shell printed (its `pwd`
/// output): on Windows the shell namespace spells a drive path
/// `/c/...`; everywhere else the printed spelling is already the
/// native one. Only the drive-letter class is translated — the
/// temp dirs these tests use always live on one.
fn shell_spelling_to_native(text: &str) -> PathBuf {
    if cfg!(windows) {
        if let Some(rest) = text.strip_prefix('/') {
            let mut chars = rest.chars();
            if let (Some(drive), Some('/')) = (chars.next(), chars.next()) {
                if drive.is_ascii_alphabetic() {
                    return PathBuf::from(format!("{drive}:{}", chars.as_str()));
                }
            }
        }
    }
    PathBuf::from(text)
}

#[test]
fn act1_host_drives_session_through_state_views_only() {
    let dir = temp_dir("act1");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut executor = Executor::new();

    // Host-side setup, through the views only.
    {
        let mut views = executor.state_views();
        views.variables().set("OUT", &dir_text);
        views.variables().set("GREETING", "hello world");
        views.variables().set_exported("GREETING", true);
        assert!(
            views
                .environment()
                .contains(&("GREETING".to_string(), "hello world".to_string())),
            "the exported variable is in the child environment view"
        );
        views.set_positional(vec!["one".to_string(), "two".to_string()]);
    }
    assert_eq!(
        run(
            &mut executor,
            "printf '%s' \"$GREETING\" > \"$OUT/greeting.txt\""
        ),
        0
    );
    assert_eq!(read(&dir, "greeting.txt"), "hello world");
    assert_eq!(
        run(&mut executor, "printf '%s' \"$# $1 $2\" > \"$OUT/pos.txt\""),
        0
    );
    assert_eq!(read(&dir, "pos.txt"), "2 one two");
    // Set $? last, immediately before the read: every run in between
    // would (correctly) replace it with its own status.
    executor.state_views().set_last_status(7);
    assert_eq!(
        run(&mut executor, "printf '%s' \"$?\" > \"$OUT/status.txt\""),
        0
    );
    assert_eq!(
        read(&dir, "status.txt"),
        "7",
        "the host-set $? is the shell's $?"
    );

    // Options: both families answer, unknown names refuse honestly.
    {
        let mut views = executor.state_views();
        let mut options = views.options();
        assert!(options.set("extglob", true));
        assert_eq!(options.get("extglob"), Some(true));
        assert_eq!(options.get("no_such_option_xyz"), None);
        assert!(!options.set("no_such_option_xyz", true));
        assert!(!options.list().is_empty());
    }
    assert_eq!(
        run(
            &mut executor,
            "shopt -q extglob; printf '%s' \"$?\" > \"$OUT/opt.txt\""
        ),
        0
    );
    assert_eq!(
        read(&dir, "opt.txt"),
        "0",
        "the view-set option is the shell's option"
    );

    // Arrays through the typed-cell view; declarations byte-match
    // the shell's own `declare -p`.
    {
        let mut views = executor.state_views();
        let mut variables = views.variables();
        assert!(variables.set_array("ARR", &[(0, "a".to_string()), (2, "cee".to_string())],));
        assert!(
            !variables.set_array("NEG", &[(-1, "x".to_string())]),
            "a negative index refuses"
        );
        assert!(variables.set_assoc(
            "MAP",
            &[
                ("k2".to_string(), "v2".to_string()),
                ("k1".to_string(), "v1".to_string()),
            ],
        ));
        let arr = variables.get("ARR").expect("ARR captured");
        assert_eq!(
            arr.value,
            VariableValue::Indexed(vec![(0, "a".to_string()), (2, "cee".to_string())]),
            "a sparse array snapshots sparse"
        );
        assert_eq!(
            arr.declaration.as_deref(),
            Some("declare -a ARR=([0]=\"a\" [2]=\"cee\")")
        );
        let map = variables.get("MAP").expect("MAP captured");
        assert_eq!(
            map.value,
            VariableValue::Associative(vec![
                ("k1".to_string(), "v1".to_string()),
                ("k2".to_string(), "v2".to_string()),
            ]),
            "associative entries snapshot sorted by key"
        );
    }
    assert_eq!(
        run(
            &mut executor,
            "printf '%s' \"${ARR[2]} ${#ARR[@]} ${MAP[k1]}\" > \"$OUT/arr.txt\""
        ),
        0
    );
    assert_eq!(read(&dir, "arr.txt"), "cee 2 v1");
    assert_eq!(
        run(&mut executor, "declare -p MAP > \"$OUT/mapdecl.txt\""),
        0
    );
    {
        let mut views = executor.state_views();
        let map = views.variables().get("MAP").expect("MAP still captured");
        assert_eq!(
            map.declaration.as_deref(),
            Some(read(&dir, "mapdecl.txt").trim_end()),
            "snapshot declaration and declare -p are the same bytes"
        );
    }

    // cwd through the cd machinery: PWD and the session field move
    // together; a failed move changes nothing.
    let subdir = dir.join("sub");
    std::fs::create_dir_all(&subdir).expect("subdir");
    let subdir_canon = std::fs::canonicalize(&subdir).expect("canonical subdir");
    // The session field carries the getcwd form of the canonical path:
    // canonicalize's extended-length `\\?\` prefix is a Rust/Win32
    // artifact GNU's getcwd never reports (unixwin/niubash#659), so
    // the field comparisons use the de-verbatimized form; the raw
    // canonical value remains the filesystem-identity yardstick for
    // the printed-pwd comparison below.
    let subdir_field_form = crate::executor::path::strip_verbatim_prefix(&subdir_canon);
    {
        let mut views = executor.state_views();
        assert!(views.set_cwd(&subdir));
        assert_eq!(views.cwd(), subdir_field_form.as_path());
        assert!(!views.set_cwd(PathBuf::from("/nonexistent-150-views").as_path()));
        assert_eq!(
            views.cwd(),
            subdir_field_form.as_path(),
            "a failed cd leaves the cwd"
        );
    }
    assert_eq!(run(&mut executor, "pwd > \"$OUT/pwd.txt\""), 0);
    // `pwd` prints the shell spelling of the directory (the shell
    // namespace on Windows, the logical path through symlinks on
    // macOS); the view-set cwd is the native path in getcwd form.
    // The two spellings differ as strings (on Windows by the `/c/`
    // namespace and 8.3 short names; on macOS by `/private`), so
    // compare filesystem identity instead:
    // canonicalize the printed spelling and require the same
    // directory the view set.
    let pwd_text = read(&dir, "pwd.txt").trim_end().to_string();
    let pwd_canon = std::fs::canonicalize(shell_spelling_to_native(&pwd_text))
        .expect("the shell's pwd names a real directory");
    assert_eq!(
        pwd_canon, subdir_canon,
        "the shell's pwd is the view-set cwd"
    );

    // Aliases and functions through the views.
    {
        let mut views = executor.state_views();
        views.define_alias("ll", "ls -l");
        assert!(views
            .aliases()
            .contains(&("ll".to_string(), "ls -l".to_string())));
        assert!(views.undefine_alias("ll"));
        assert!(!views.undefine_alias("ll"));
    }
    assert_eq!(run(&mut executor, "myfn() { return 3; }"), 0);
    {
        let mut views = executor.state_views();
        assert!(views.has_function("myfn"));
        assert!(views.functions().contains(&"myfn".to_string()));
        assert!(views.undefine_function("myfn"));
        assert!(!views.has_function("myfn"));
    }

    // Unset through the unset builtin's own leaf sequence.
    {
        let mut views = executor.state_views();
        assert!(views.variables().unset("GREETING"));
        assert!(
            !views.variables().unset("GREETING"),
            "a second unset reports not-bound"
        );
        assert!(
            !views.variables().unset("BASH_SOURCE"),
            "the att_nounset stack arrays refuse unbinding"
        );
    }
    assert_eq!(
        run(
            &mut executor,
            "printf '%s' \"${GREETING-unset}\" > \"$OUT/unset.txt\""
        ),
        0
    );
    assert_eq!(read(&dir, "unset.txt"), "unset");

    // Dynamic state reads: PIPESTATUS after a pipeline, $? after a
    // failure — the same getters the expander uses.
    assert_eq!(run(&mut executor, "true | false"), 1);
    {
        let mut views = executor.state_views();
        let pipe = views
            .variables()
            .get("PIPESTATUS")
            .expect("PIPESTATUS captured");
        assert!(pipe.dynamic);
        assert_eq!(
            pipe.value,
            VariableValue::Indexed(vec![(0, "0".to_string()), (1, "1".to_string())])
        );
        let inspection = views.inspection();
        assert_eq!(inspection.dynamic.last_status, 1);
        assert_eq!(inspection.dynamic.pipestatus, vec![(0, 0), (1, 1)]);
    }
}

/// Holds the gate test lock and guarantees the gate is disarmed
/// when the test ends (the gate is process-global).
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

type StopHandler = Box<dyn FnMut(&mut DebugSession) -> StopLoopOutcome>;

struct InspectLoop {
    stops: Rc<RefCell<Vec<(Option<u32>, InspectionSnapshot)>>>,
    command_texts: Rc<RefCell<Vec<(Option<u32>, Option<String>)>>>,
    handler: StopHandler,
}

impl StopLoop for InspectLoop {
    fn serve(&mut self, session: &mut DebugSession) -> StopLoopOutcome {
        if let Some(inspection) = session.inspection().cloned() {
            let line = session
                .last_stop()
                .and_then(|stop| stop.location.as_ref().map(|loc| loc.line));
            self.stops.borrow_mut().push((line, inspection));
            let command_text = session
                .last_stop()
                .and_then(|stop| stop.command_text.clone());
            self.command_texts.borrow_mut().push((line, command_text));
        }
        (self.handler)(session)
    }
}

/// Debug dump of every recorded stop (line, frame count, dynamic
/// state) for diagnosing stop attribution in this test.
fn dump_stops(stops: &[(Option<u32>, InspectionSnapshot)]) -> String {
    stops
        .iter()
        .map(|(line, snap)| {
            format!(
                "line={line:?} frames={:?} bash_command={:?} pipestatus={:?}",
                snap.frames
                    .iter()
                    .map(|frame| frame.function.as_str())
                    .collect::<Vec<_>>(),
                snap.dynamic.bash_command,
                snap.dynamic.pipestatus
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse a `declare -p` output file into (name, line) pairs: the
/// name is the token after the flags word.
fn declaration_lines(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("declare ")?;
            let (_flags, tail) = rest.split_once(' ')?;
            let name: String = tail
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            Some((name, line.to_string()))
        })
        .collect()
}

#[test]
fn act2_inspection_snapshot_matches_debuggee_own_output() {
    let _span = GateTestSpan::new();
    let dir = temp_dir("act2");
    let dir_text = dir.to_string_lossy().into_owned();
    // Line numbers are load-bearing (breakpoints and caller lines
    // name them); the vector index + 1 is the line number.
    let lines = [
        "shopt -s extdebug",
        "SCALAR=\"hello world\"",
        "declare -i NUM=42",
        "readonly RO=locked",
        "export EXP=shipped",
        "ARR=([0]=\"a\" [2]=\"cee\")",
        "declare -A MAP=([k1]=\"v1\" [k2]=\"v2\")",
        "declare UNSETV",
        "REF=SCALAR",
        "declare -n NR=SCALAR",
        "false | true | false",
        "declare -p PIPESTATUS > \"$OUT/pipe.txt\"",
        "inner() {",
        "  declare -p SCALAR NUM RO EXP ARR MAP UNSETV NR L LUNSET FUNCNAME PIPESTATUS > \"$OUT/decl.txt\"",
        "  caller 0 > \"$OUT/caller0.txt\"",
        "  caller 1 > \"$OUT/caller1.txt\"",
        "}",
        "outer() {",
        "  local L=\"local val\"",
        "  local LUNSET",
        "  inner p1 p2",
        "}",
        "outer",
    ];
    let script = lines.join("\n") + "\n";
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, &script).expect("write script");
    let script_path_text = script_path.to_string_lossy().into_owned();

    let stops: Rc<RefCell<Vec<(Option<u32>, InspectionSnapshot)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let command_texts: Rc<RefCell<Vec<(Option<u32>, Option<String>)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let file_for_bp = script_path_text.clone();
    let driver = SessionDriver::new(
        DebugSession::new(),
        Box::new(InspectLoop {
            stops: stops.clone(),
            command_texts: command_texts.clone(),
            handler: Box::new(move |session: &mut DebugSession| {
                if session.last_stop().map(|stop| stop.reason)
                    == Some(crate::debugger::event::StopReason::Entry)
                {
                    session
                        .breakpoints_mut()
                        .insert_line(file_for_bp.clone(), 12);
                    session
                        .breakpoints_mut()
                        .insert_line(file_for_bp.clone(), 14);
                }
                StopLoopOutcome::Resume(StepArming::Continue)
            }),
        }),
    );
    let mut executor = Executor::new();
    executor.set_env("OUT", &dir_text);
    executor.install_debug_session(driver);
    // The path is interpolated into shell source, so quote it: an
    // unquoted native Windows spelling cannot survive the lexer
    // (a backslash in a bare word is an escape and eats the path
    // separators, so the source fails before anything runs).
    let status = run(&mut executor, &format!(". \"{script_path_text}\""));
    assert_eq!(status, 0);

    let stops = stops.borrow();
    let at = |line: u32| {
        stops
            .iter()
            .find(|(stop_line, _)| *stop_line == Some(line))
            .map(|(_, inspection)| inspection)
            .unwrap_or_else(|| panic!("a stop was recorded at line {line}\n{}", dump_stops(&stops)))
    };

    // The stop right after the pipeline: PIPESTATUS in the snapshot
    // is the pipeline's, and its declaration is byte-identical to
    // the `declare -p PIPESTATUS` the debuggee prints at that stop.
    let pipe_stop = at(12);
    assert_eq!(
        pipe_stop.dynamic.pipestatus,
        vec![(0, 1), (1, 0), (2, 1)],
        "PIPESTATUS at the stop is the pipeline's statuses"
    );
    let pipe_decl = pipe_stop
        .variable("PIPESTATUS")
        .and_then(|var| var.declaration.clone())
        .expect("PIPESTATUS declaration captured");
    assert_eq!(
        pipe_decl,
        read(&dir, "pipe.txt").trim_end(),
        "snapshot PIPESTATUS declaration == declare -p PIPESTATUS at the same stop"
    );
    // $BASH_COMMAND at a funnel stop is the about-to-run command
    // (design 7.3; the section 4.2 ordering refreshes the
    // engine's cell before the consult — WP-7 closed the
    // one-command lag this fixture originally recorded, under
    // which the read was the pipeline's last stage, `false`).
    // The snapshot's dynamic read equals the stop's own frozen
    // command text — the engine's dequoted rendering of line
    // 12's command, the command this stop precedes.
    let stop_text = command_texts
        .borrow()
        .iter()
        .find(|(stop_line, _)| *stop_line == Some(12))
        .and_then(|(_, text)| text.clone())
        .expect("the line-12 stop recorded its command text");
    assert_eq!(stop_text, "declare -p PIPESTATUS > $OUT/pipe.txt");
    assert_eq!(
        pipe_stop.dynamic.bash_command.as_deref(),
        Some(stop_text.as_str()),
        "BASH_COMMAND at the stop is the about-to-run command\n{}",
        dump_stops(&stops)
    );

    // The stop inside `inner`: the full battery aligns with the
    // debuggee's own declare -p, printed at the same stop.
    let inner_stop = at(14);
    let declared = declaration_lines(&read(&dir, "decl.txt"));
    assert_eq!(
        declared.len(),
        12,
        "the battery printed twelve declarations"
    );
    for (name, line) in &declared {
        let snapshot = inner_stop
            .variable(name)
            .unwrap_or_else(|| panic!("{name} captured at the inner stop"));
        assert_eq!(
            snapshot.declaration.as_deref(),
            Some(line.as_str()),
            "snapshot declaration for {name} == the debuggee's declare -p line"
        );
    }
    let var = |name: &str| {
        inner_stop
            .variable(name)
            .expect("battery variable captured")
    };
    assert_eq!(var("SCALAR").value.as_scalar(), Some("hello world"));
    assert!(
        var("NUM").attrs.integer,
        "declare -i is in the snapshot attrs"
    );
    assert!(var("RO").attrs.readonly);
    assert!(var("EXP").attrs.exported);
    assert!(var("NR").attrs.nameref);
    assert_eq!(var("NR").value.as_scalar(), Some("SCALAR"));
    assert_eq!(var("UNSETV").value, VariableValue::Unset);
    assert_eq!(var("L").value.as_scalar(), Some("local val"));
    assert_eq!(
        var("L").scope_depth,
        Some(1),
        "outer's local is one scope out from inner"
    );
    assert_eq!(var("LUNSET").value, VariableValue::Unset);
    assert_eq!(var("LUNSET").scope_depth, Some(1));
    assert_eq!(
        var("ARR").value,
        VariableValue::Indexed(vec![(0, "a".to_string()), (2, "cee".to_string())])
    );
    assert_eq!(
        var("MAP").value,
        VariableValue::Associative(vec![
            ("k1".to_string(), "v1".to_string()),
            ("k2".to_string(), "v2".to_string()),
        ])
    );

    // Frames: the zip the snapshot renders is the stack `caller`
    // reads — the WP-6 rendering of the snapshot equals the
    // debuggee's own caller output at the same stop.
    // The frame stack below `outer` is the harness's own shape
    // (the driver input sources the probe file, so a `source`
    // frame — and the driver's bottom frame — sit under it, the
    // same shape GNU shows for nested sourcing). The battery pins
    // the two function frames and lets the caller comparisons
    // below validate the rest of the zip against the debuggee.
    assert!(
        inner_stop.frames.len() >= 3,
        "inner, outer, and the sourcing frame(s) below"
    );
    assert_eq!(inner_stop.frames[0].function, "inner");
    assert_eq!(inner_stop.frames[1].function, "outer");
    assert_eq!(
        inner_stop.frames[0].args,
        vec!["p1".to_string(), "p2".to_string()],
        "frame args in call order (extdebug materialized)"
    );
    assert_eq!(
        render_caller(&inner_stop.frames, 0).as_deref(),
        Some(read(&dir, "caller0.txt").trim_end()),
        "render_caller(0) == caller 0 at the same stop"
    );
    assert_eq!(
        render_caller(&inner_stop.frames, 1).as_deref(),
        Some(read(&dir, "caller1.txt").trim_end()),
        "render_caller(1) == caller 1 at the same stop"
    );
    let where_text = render_where(inner_stop);
    assert!(
        where_text.contains("#0 inner"),
        "where renders the innermost frame first"
    );
    assert!(where_text.contains("#1 outer"));
    let locals = crate::debugger::inspect::render_variables(inner_stop, VariablePartition::Locals);
    assert!(
        locals.contains("L=\"local val\""),
        "the locals partition renders the local"
    );
    assert!(
        !locals.contains("SCALAR"),
        "the locals partition excludes globals"
    );

    // Nameref resolution through the view reads the final value.
    let mut views = executor.state_views();
    assert_eq!(
        views.variables().resolved_value("NR").as_deref(),
        Some("hello world"),
        "NR (nameref to SCALAR) resolves to the scalar's value"
    );
}

// ---- rubash#2084 / rubash#2130: Session <-> environment boundary ----
//
// #2130: the invocation shape is declared construction data
// (InvocationShape on the builder); engine decision points read the
// typed executor state, and the __RUBASH_* carriers survive only as
// projections for observable faces. #2084: a root Session's era of
// process-global writes (signal policy, umask, env mirror) is
// restored when the last root executor drops.

use super::capabilities::StartupClass;
use super::session::{InvocationShape, RespawnSource, RunOptions, Session};

#[test]
fn issue2130_invocation_shape_reaches_executor_state() {
    let session = Session::builder()
        .invocation_shape(InvocationShape::login_interactive())
        .build()
        .expect("session builds");
    let shape = session.invocation_shape();
    assert!(shape.interactive && shape.login);
    assert_eq!(shape.respawn, RespawnSource::Fresh);
    assert!(session.executor().invocation_interactive());
    assert!(session.executor().invocation_login());

    // The legacy per-flag builder methods declare the same shape.
    let session = Session::builder()
        .login_shell(true)
        .interactive(true)
        .build()
        .expect("session builds");
    let shape = session.invocation_shape();
    assert!(shape.interactive && shape.login);
    assert!(session.executor().invocation_interactive());
    assert!(session.executor().invocation_login());
}

#[test]
fn issue2130_forged_carrier_assignment_does_not_flip_decisions() {
    let mut session = Session::builder().build().expect("session builds");
    assert!(!session.executor().invocation_interactive());
    assert!(!session.executor().invocation_login());
    // A script assignment to a carrier name is a variable write, not
    // a shape declaration: the typed decision state must not move.
    session
        .run(
            "__RUBASH_INTERACTIVE=1\n__RUBASH_LOGIN_SHELL=1",
            &RunOptions::new(),
        )
        .expect("run");
    assert!(!session.executor().invocation_interactive());
    assert!(!session.executor().invocation_login());
    // The host funnel, by contrast, is a legitimate shape write: it
    // moves the typed state (and keeps the carrier projection in
    // step), and removing the carrier clears both.
    session.executor_mut().set_env("__RUBASH_INTERACTIVE", "1");
    assert!(session.executor().invocation_interactive());
    session.executor_mut().remove_env("__RUBASH_INTERACTIVE");
    assert!(!session.executor().invocation_interactive());
}

#[test]
fn issue2130_engine_child_shape_skips_startup_files() {
    let dir = temp_dir("issue2130-respawn");
    let env_file = dir.join("env.sh");
    let marker = dir.join("sourced.txt");
    std::fs::write(&env_file, format!("printf x > '{}'", marker.display())).expect("env file");
    let mut fresh = Session::builder()
        .env("BASH_ENV", env_file.to_string_lossy().into_owned())
        .build()
        .expect("session builds");
    fresh
        .source_startup_files(StartupClass::EnvFile)
        .expect("startup chain runs");
    assert!(
        marker.exists(),
        "control: a fresh session sources $BASH_ENV"
    );
    std::fs::remove_file(&marker).expect("marker cleanup");

    let mut shape = InvocationShape::new();
    shape.respawn = RespawnSource::EngineChild;
    let mut child = Session::builder()
        .env("BASH_ENV", env_file.to_string_lossy().into_owned())
        .invocation_shape(shape)
        .build()
        .expect("session builds");
    child
        .source_startup_files(StartupClass::EnvFile)
        .expect("startup chain is a no-op, not an error");
    assert!(
        !marker.exists(),
        "an engine-child session runs no startup chain (execute_cmd.c:1576)"
    );
}

/// The #2084 end-to-end pin. Process-global state (signal
/// dispositions, umask, the env mirror) is only deterministic when
/// no sibling test's session era overlaps, so the scenario runs in a
/// re-executed child test process; the parent asserts its exit.
#[test]
fn issue2084_session_end_restores_host_process_state() {
    if std::env::var_os("ISSUE2084_CHILD_SCENARIO").is_none() {
        let exe = std::env::current_exe().expect("test exe");
        let status = std::process::Command::new(exe)
            .args([
                "--exact",
                "api::consumer_tests::issue2084_session_end_restores_host_process_state",
                "--nocapture",
            ])
            .env("ISSUE2084_CHILD_SCENARIO", "1")
            .status()
            .expect("child test process spawns");
        assert!(status.success(), "child scenario failed: {status}");
        return;
    }
    issue2084_child_scenario();
}

#[cfg(unix)]
fn issue2084_disposition(sig: i32) -> usize {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: querying this process's own disposition.
    unsafe {
        libc::sigaction(sig, std::ptr::null(), &mut action);
    }
    action.sa_sigaction
}

fn issue2084_child_scenario() {
    // Environment. Ordinary exports never touch the host env at all
    // (#916: children receive them through the explicit spawn env).
    // The special-name set (PATH/PWD/OLDPWD/HOME, every __RUBASH_*
    // bookkeeping name, the locale family) DOES mirror to the
    // process env while the session lives — those writes are
    // journaled (G-03) and unwound when the session's executor
    // drops. PATH is the #2084 evidence shape (a session rewrote
    // the host's PATH/PWD/OLDPWD).
    std::env::set_var("__RUBASH_ISSUE2084_HOSTVAR", "before");
    let host_path = std::env::var("PATH").unwrap_or_default();
    {
        let mut session = Session::builder().build().expect("session builds");
        session
            .run(
                "export ISSUE2084_SESSION_ONLY=1\nexport PATH=/issue2084-session-path",
                &RunOptions::new(),
            )
            .expect("run");
        assert!(
            std::env::var_os("ISSUE2084_SESSION_ONLY").is_none(),
            "an ordinary export never reaches the host env"
        );
        assert_eq!(
            std::env::var("PATH").unwrap(),
            "/issue2084-session-path",
            "a special-name (PATH) write mirrors while the session lives"
        );
        // The host funnel writes through the same journal for
        // mirrored names, and stays out of the process env for
        // ordinary ones.
        session
            .executor_mut()
            .set_env("__RUBASH_ISSUE2084_HOSTVAR", "session");
        assert_eq!(
            std::env::var("__RUBASH_ISSUE2084_HOSTVAR").unwrap(),
            "session"
        );
        session.executor_mut().set_env("ISSUE2084_PLAIN", "x");
        assert!(
            std::env::var_os("ISSUE2084_PLAIN").is_none(),
            "the funnel does not mirror ordinary names either"
        );
    }
    assert_eq!(
        std::env::var("PATH").unwrap(),
        host_path,
        "the mirrored PATH returns to its pre-session value at drop"
    );
    assert_eq!(
        std::env::var("__RUBASH_ISSUE2084_HOSTVAR").unwrap(),
        "before",
        "the host's variable returns to its pre-session value at drop"
    );
    assert!(
        std::env::var_os("__RUBASH_CURRENT_LINE").is_none(),
        "engine bookkeeping never persists in the host env"
    );
    std::env::remove_var("__RUBASH_ISSUE2084_HOSTVAR");

    #[cfg(unix)]
    {
        // Signal dispositions: an interactive session era applies
        // the shell policy (SIGQUIT ignored, sig.c:333); at the last
        // root drop the host's exact pre-session handlers return.
        let quit_before = issue2084_disposition(libc::SIGQUIT);
        let pipe_before = issue2084_disposition(libc::SIGPIPE);
        let int_before = issue2084_disposition(libc::SIGINT);
        {
            let mut session = Session::builder()
                .invocation_shape(InvocationShape::interactive())
                .build()
                .expect("session builds");
            session.run("true", &RunOptions::new()).expect("run");
            assert_eq!(
                issue2084_disposition(libc::SIGQUIT),
                libc::SIG_IGN,
                "during the session the SIGQUIT policy holds"
            );
        }
        assert_eq!(
            issue2084_disposition(libc::SIGQUIT),
            quit_before,
            "SIGQUIT disposition restored at session end"
        );
        assert_eq!(
            issue2084_disposition(libc::SIGPIPE),
            pipe_before,
            "SIGPIPE disposition restored at session end"
        );
        assert_eq!(
            issue2084_disposition(libc::SIGINT),
            int_before,
            "SIGINT disposition restored at session end"
        );

        // umask: a session-set mask returns to the birth value when
        // the session ends and nothing else overwrote it.
        let birth = crate::builtins::umask::snapshot_process_umask().expect("umask readable");
        {
            let mut session = Session::builder().build().expect("session builds");
            session.run("umask 077", &RunOptions::new()).expect("run");
            assert_eq!(
                crate::builtins::umask::snapshot_process_umask(),
                Some(0o077),
                "during the session its umask holds"
            );
        }
        assert_eq!(
            crate::builtins::umask::snapshot_process_umask(),
            Some(birth),
            "process umask restored at session end"
        );
    }
}

// ---------------------------------------------------------------------------
// niubash#822 (Bug L): the winsize bind writes BOTH storages
// ---------------------------------------------------------------------------

/// The shell's own `$name` expansion, captured through a real run — the
/// value script text observes, not a host-side view. This is the probe
/// shape the Linux session used (A4/A8): `$COLUMNS` as the shell prints
/// it.
fn issue822_expansion(session: &mut Session, dir: &PathBuf, name: &str) -> String {
    session
        .run(
            &format!("printf '%s' \"${name}\" > \"$OUT/val.txt\""),
            &RunOptions::new(),
        )
        .expect("expansion run");
    read(dir, "val.txt")
}

#[test]
fn issue822_winsize_bind_updates_imported_columns_record() {
    // GNU pin (WSL /usr/local/bin/bash 5.3.x, pty 30x100, parent exports
    // COLUMNS=200): A1 `$COLUMNS` = 100, A2 child env COLUMNS=100 — the
    // interactive bind OVERRIDES the imported value in every view, and an
    // exported COLUMNS stays exported with the new value (A3
    // `declare -x COLUMNS="100"`).
    let dir = temp_dir("issue822-import");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = Session::builder()
        .env("OUT", &dir_text)
        .build()
        .expect("session builds");
    {
        let executor = session.executor_mut();
        // Probe A4's storage shape: the parent process exported
        // COLUMNS=200/LINES=55, and Executor::new imported them as
        // EXPORTED scalar records (init.rs:949 ->
        // VariableStore::from_environment, variables.rs:496-515 binds
        // every valid identifier with exported=true) next to the env
        // mirror. set_env alone cannot create that record
        // (public_accessors.rs:887 updates existing records only), so
        // rebuild the imported pair explicitly.
        executor.set_env("COLUMNS", "200");
        executor.set_env("LINES", "55");
        let mut columns = crate::shell::Variable::scalar("200");
        columns.exported = true;
        executor
            .shell_state
            .variables
            .set("COLUMNS", columns)
            .expect("COLUMNS record binds");
        let mut lines = crate::shell::Variable::scalar("55");
        lines.exported = true;
        executor
            .shell_state
            .variables
            .set("LINES", lines)
            .expect("LINES record binds");
    }
    assert_eq!(issue822_expansion(&mut session, &dir, "COLUMNS"), "200");

    // Session::notify_terminal_size: the host's resize report
    // (columns=100, rows=30) — the landing the niubash host uses.
    session.notify_terminal_size(100, 30);

    assert_eq!(
        issue822_expansion(&mut session, &dir, "COLUMNS"),
        "100",
        "$COLUMNS expansion must track the bind (GNU A1), not stay at the imported 200"
    );
    assert_eq!(
        issue822_expansion(&mut session, &dir, "LINES"),
        "30",
        "$LINES expansion must track the bind too"
    );
    assert_eq!(
        session.executor_mut().get_env("COLUMNS"),
        Some("100"),
        "the env mirror (get_env / child-env composition) tracks as before"
    );
    // GNU A3/pin C: an exported COLUMNS stays exported across the rebind
    // (variables.c:3207 replaces only the value cell; the attribute word
    // is untouched).
    assert!(
        session
            .variables()
            .attributes("COLUMNS")
            .expect("COLUMNS still bound")
            .exported,
        "an exported COLUMNS stays exported across the winsize rebind"
    );
    // Resizing the other way keeps both views in lockstep (GNU A4/A5).
    session.notify_terminal_size(120, 40);
    assert_eq!(issue822_expansion(&mut session, &dir, "COLUMNS"), "120");
    assert_eq!(issue822_expansion(&mut session, &dir, "LINES"), "40");
}

#[test]
fn issue822_winsize_bind_updates_user_assigned_record() {
    // GNU pin (WSL, pty 30x100): `COLUMNS=55` holds (B1), a resize to
    // 40x120 rebinds (B2: 120), a resize back to 24x80 rebinds both
    // dimensions (B3: cols=80 lines=24).
    let dir = temp_dir("issue822-assign");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = Session::builder()
        .env("OUT", &dir_text)
        .build()
        .expect("session builds");
    // The assignment machinery itself creates the record (probe A8's
    // shape) — no store surgery in this test.
    session
        .run("COLUMNS=55", &RunOptions::new())
        .expect("assignment run");
    assert_eq!(issue822_expansion(&mut session, &dir, "COLUMNS"), "55");

    session.notify_terminal_size(120, 40);
    assert_eq!(
        issue822_expansion(&mut session, &dir, "COLUMNS"),
        "120",
        "a user-assigned COLUMNS must track the resize (GNU B2)"
    );
    session.notify_terminal_size(80, 24);
    assert_eq!(issue822_expansion(&mut session, &dir, "COLUMNS"), "80");
    assert_eq!(issue822_expansion(&mut session, &dir, "LINES"), "24");
    assert_eq!(session.executor_mut().get_env("COLUMNS"), Some("80"));
}

#[test]
fn issue822_winsize_bind_creates_no_record() {
    // Regression guard for the no-record case (fresh shells looked fine
    // before the fix): with no record, expansion falls through to the
    // mirror and tracks; the bind itself must not create a record — the
    // dual-write mirrors set_env, which updates existing scalar records
    // only — and must not export the name (GNU's fresh bind is a
    // non-exported shell variable).
    let ambient_columns = std::env::var_os("COLUMNS");
    let ambient_lines = std::env::var_os("LINES");
    std::env::remove_var("COLUMNS");
    std::env::remove_var("LINES");
    let dir = temp_dir("issue822-fresh");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = Session::builder()
        .env("OUT", &dir_text)
        .build()
        .expect("session builds");
    assert!(
        session
            .executor_mut()
            .shell_state
            .variables
            .get("COLUMNS")
            .is_none(),
        "a scrubbed process env must not seed a COLUMNS record"
    );
    session.notify_terminal_size(100, 30);
    assert_eq!(
        issue822_expansion(&mut session, &dir, "COLUMNS"),
        "100",
        "the mirror fallthrough keeps tracking"
    );
    assert!(
        session
            .executor_mut()
            .shell_state
            .variables
            .get("COLUMNS")
            .is_none(),
        "the bind writes existing records only, like set_env"
    );
    assert!(
        !session
            .environment()
            .iter()
            .any(|(name, _)| name == "COLUMNS"),
        "the bind must not export COLUMNS to children"
    );
    // Restore the runner's ambient environment (tests.rs issue2178
    // pattern).
    if let Some(value) = ambient_columns {
        std::env::set_var("COLUMNS", value);
    }
    if let Some(value) = ambient_lines {
        std::env::set_var("LINES", value);
    }
}

// ---- unixwin/niubash#863 GAP-1: run classification on the contract ----
//
// The grouped driver's stop bool conflates a real exit jump with
// the errexit break, and Flow folds the errexit break into
// Completed — so a REPL host could not reproduce the driver's
// stop rule from Session::run's outcome and kept the direct
// driver call. RunOutcome::classification now carries the
// driver's own decomposition. These pins drive the same outcome
// classes niubash's issue863_reader_batch_* pins lock from the
// host side; together they define the 1:1 mapping the reader
// batch migrates against: end the session iff
// classification.unwind.is_some(), with outcome.status.
//
// GNU anchors: EXITPROG/ERREXIT are caught at parse_and_execute's
// setjmp and re-raised to the reader's top level
// (evalstring.c:396-403, :618-619); a parse error alone is a
// DISCARD (evalstring.c:585-601); the `!` inversion exemption is
// execute_cmd.c:652-656 (CMD_IGNORE_RETURN).

use super::session::{Flow, RunUnwind};

fn issue863_gap1_interactive_session() -> Session {
    Session::builder()
        .invocation_shape(InvocationShape::interactive())
        .build()
        .expect("session builds")
}

fn issue863_gap1_interactive_options() -> RunOptions {
    let mut options = RunOptions::new();
    options.interactive = true;
    options
}

#[test]
fn issue863_gap1_errexit_break_is_classified_apart_from_flow() {
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session
        .run("false; echo unreachable", &options)
        .expect("run");
    assert_eq!(outcome.status, 1);
    // Q6 arm B contract: the errexit break no longer folds
    // into Completed — Flow carries it as its own arm.
    assert_eq!(outcome.flow, Flow::ErrexitBreak(1));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(class.errexit_break);
    assert!(!class.exit_jump);
    assert!(!class.parse_error);
}

#[test]
fn issue863_gap1_exit_jump_under_errexit_keeps_kind_and_status() {
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session.run("true; exit 3", &options).expect("run");
    // The jump's own status (rubash#2535), not a trap-restored $?.
    assert_eq!(outcome.status, 3);
    // Q6 arm B contract: the jump outranks the errexit verdict
    // in Flow too (arm A's fold resolved this shape to
    // Completed, hiding a real exit from Flow-reading hosts)…
    assert_eq!(outcome.flow, Flow::ExitRequested(3));
    // …while the classification names the stop the driver
    // adjudicated: the jump counted, so the stop is the jump —
    // with the errexit verdict also standing, as computed.
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ExitJump));
    assert!(class.exit_jump);
    assert!(class.errexit_break);
    assert!(!class.parse_error);
}

#[test]
fn issue863_gap1_parse_error_discards_without_errexit() {
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    let outcome = session.run("echo ok; echo )bad", &options).expect("run");
    assert_eq!(outcome.status, 2);
    assert_eq!(outcome.flow, Flow::Completed);
    let class = outcome.classification;
    assert!(class.parse_error);
    assert!(!class.errexit_break);
    // The DISCARD class: no unwind ends the session. (The raw
    // exit_jump is deliberately unasserted — the parse-abort
    // arm may stand it; the adjudicated form is `unwind`.)
    assert_eq!(class.unwind, None);
}

#[test]
fn issue863_gap1_parse_error_under_errexit_is_an_errexit_break() {
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session.run("echo ok; echo )bad", &options).expect("run");
    assert_eq!(outcome.status, 2);
    // Q6 arm B contract: Flow carries the break, as on the
    // plain errexit shape.
    assert_eq!(outcome.flow, Flow::ErrexitBreak(2));
    let class = outcome.classification;
    assert!(class.parse_error);
    assert!(class.errexit_break);
    // GNU: the errexit break propagates even on a failed parse
    // status — the session ends, at the parse status.
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
}

#[test]
fn issue863_gap1_inverted_last_command_is_errexit_exempt() {
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session.run("echo ok; ! true", &options).expect("run");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::Completed);
    let class = outcome.classification;
    // execute_cmd.c:652-656: the inverted command's failing
    // status is exempt — no errexit verdict, no unwind, and
    // the inversion itself is on the record for hosts whose
    // stop rule reads it (the stdin feeder's shape).
    assert!(class.last_inverted);
    assert!(!class.errexit_break);
    assert_eq!(class.unwind, None);
}

#[test]
fn issue863_gap1_plain_exit_jump_and_ordinary_runs() {
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    let outcome = session.run("true; exit 3", &options).expect("run");
    assert_eq!(outcome.status, 3);
    assert_eq!(outcome.flow, Flow::ExitRequested(3));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ExitJump));
    assert!(class.exit_jump);
    assert!(!class.errexit_break);
    assert!(!class.parse_error);

    // run_line composes the same run: same classification.
    let mut session = issue863_gap1_interactive_session();
    let outcome = session.run_line("exit 4").expect("run_line");
    assert_eq!(outcome.status, 4);
    assert_eq!(outcome.flow, Flow::ExitRequested(4));
    assert_eq!(outcome.classification.unwind, Some(RunUnwind::ExitJump));

    // Ordinary runs classify clear, pass or fail.
    let mut session = issue863_gap1_interactive_session();
    let outcome = session.run("false", &options).expect("run");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::Completed);
    let class = outcome.classification;
    assert!(!class.parse_error);
    assert!(!class.signal_discard);
    assert!(!class.exit_jump);
    assert!(!class.errexit_break);
    assert!(!class.last_inverted);
    assert_eq!(class.unwind, None);
    let outcome = session.run("true", &options).expect("run");
    assert_eq!(outcome.status, 0);
    assert_eq!(outcome.classification.unwind, None);
}

#[test]
fn issue863_gap1_line_route_reports_its_own_classification() {
    // Non-interactive, no history/alias use: the line-offset
    // route. Its outcome mapping computes the same errexit
    // verdict as the grouped driver (the shared
    // `errexit_break_verdict`) and reports the route's own
    // adjudication — the same reads Flow is folded from.
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("exit 5", &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 5);
    assert_eq!(outcome.flow, Flow::ExitRequested(5));
    let class = outcome.classification;
    assert!(class.exit_jump);
    assert!(!class.parse_error);
    assert!(!class.errexit_break);
    assert_eq!(class.unwind, Some(RunUnwind::ExitJump));

    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("echo )bad", &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 2);
    assert_eq!(outcome.flow, Flow::Completed);
    let class = outcome.classification;
    assert!(class.parse_error);
    // A parse-flagged run's jump is the reader discard: the
    // adjudicated unwind is none.
    assert_eq!(class.unwind, None);
}

#[test]
fn issue863_gap1_prompt_command_jump_drains_as_classification() {
    let mut session = issue863_gap1_interactive_session();
    // Control: no prompt command, nothing pending — the drain
    // is clear and adjudicates nothing.
    assert_eq!(session.run_prompt_command(), None);
    let class = session.take_classification();
    assert!(!class.exit_jump);
    assert!(!class.parse_error);
    assert!(!class.signal_discard);
    assert_eq!(class.unwind, None);

    // A PROMPT_COMMAND exit is re-armed by its runner
    // (rubash#433; evalstring.c:618-619's re-raise) — the
    // exact pending niu's shell.rs polls after
    // run_prompt_command, now drained as one value.
    let options = issue863_gap1_interactive_options();
    session
        .run("PROMPT_COMMAND='exit 7'", &options)
        .expect("run");
    assert_eq!(session.run_prompt_command(), Some(7));
    let class = session.take_classification();
    assert!(class.exit_jump);
    assert_eq!(class.unwind, None);
}

#[test]
fn issue863_gap1_arith_zero_under_errexit_is_an_errexit_break() {
    // Q6 corpus L2-28: an arithmetic command evaluating to 0
    // fails with status 1, and under `set -e` that failure is
    // an errexit break — GNU executes no further command and
    // the shell exits 1 (execute_cmd.c:624's exit_immediately
    // decision on the arithmetic command's result). The Q6
    // prototype mislabeled this cell ExitJump on both routes:
    // the arithmetic raise reached the jump bookkeeping
    // outside the marked raise sites. This pin is the
    // enumeration-completeness witness for the arithmetic
    // raise path.
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session.run("(( 0 )); echo alive", &options).expect("run");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(1));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(class.errexit_break);
    assert!(!class.exit_jump);
    assert!(!class.parse_error);

    // The line-offset route adjudicates the same cell the
    // same way (one verdict, every route).
    let mut session = Session::builder().build().expect("session builds");
    session.run("set -e", &RunOptions::new()).expect("run");
    let outcome = session
        .run("(( 0 )); echo alive", &RunOptions::new())
        .expect("run");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(1));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(class.errexit_break);
    assert!(!class.exit_jump);
}

#[test]
fn issue863_gap1_conditional_failure_under_errexit_is_an_errexit_break() {
    // Q6 corpus L2-29: the `[[ ]]` sibling of the arithmetic
    // cell — a conditional command failing at statement
    // position under `set -e` is an errexit break (GNU
    // execute_cmd.c:624's decision on the cm_cond result).
    // Pinned alongside L2-28 as the enumeration witness for
    // the conditional raise class.
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session
        .run("[[ 1 == 2 ]]; echo alive", &options)
        .expect("run");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(1));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(class.errexit_break);
    assert!(!class.exit_jump);
}

#[test]
fn issue863_gap1_eval_parse_error_under_errexit_is_an_errexit_break() {
    // GNU error.c:324-326: a parser_error printed while
    // errexit is live exit_shell(2)s on the spot — so an
    // `eval` whose string fails to parse, under `set -e`,
    // ends the session as an errexit-class exit at status 2,
    // not as an exit jump (and without live errexit the same
    // eval merely returns 2 and the session lives). The
    // eval raise sites carry the errexit mark like every
    // other errexit gate.
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session
        .run("eval 'echo )bad'; echo alive", &options)
        .expect("run");
    assert_eq!(outcome.status, 2);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(2));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(class.errexit_break);
    assert!(!class.exit_jump);
}

#[test]
fn issue863_gap1_startup_chain_decomposes_the_file_unwind() {
    // The startup chain reads the same decomposition as the
    // run routes: a startup file that arms errexit itself and
    // fails its last command ends the chain in an errexit
    // break; a file that exits ends it in an exit jump. (The
    // raw pending read mislabeled the first shape ExitJump —
    // Q6 arm B approximation (iv).)
    let dir = temp_dir("issue863-gap1-startup");
    let marker = dir.join("reached.txt");

    let env_file = dir.join("env-errexit.sh");
    std::fs::write(
        &env_file,
        format!("set -e\nfalse\nprintf x > '{}'", marker.display()),
    )
    .expect("env file");
    let mut session = Session::builder()
        .env("BASH_ENV", env_file.to_string_lossy().into_owned())
        .build()
        .expect("session builds");
    let outcome = session
        .source_startup_files(StartupClass::EnvFile)
        .expect("startup chain runs");
    assert_eq!(outcome.status, 1);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(1));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
    assert!(class.errexit_break);
    assert!(!class.exit_jump);
    assert!(
        !marker.exists(),
        "the errexit break ends the file (and the chain)"
    );

    let env_file = dir.join("env-exit.sh");
    std::fs::write(&env_file, "exit 7").expect("env file");
    let mut session = Session::builder()
        .env("BASH_ENV", env_file.to_string_lossy().into_owned())
        .build()
        .expect("session builds");
    let outcome = session
        .source_startup_files(StartupClass::EnvFile)
        .expect("startup chain runs");
    assert_eq!(outcome.status, 7);
    assert_eq!(outcome.flow, Flow::ExitRequested(7));
    let class = outcome.classification;
    assert_eq!(class.unwind, Some(RunUnwind::ExitJump));
    assert!(class.exit_jump);
    assert!(!class.errexit_break);
}

#[test]
fn issue863_gap1_line_route_parse_error_under_errexit_is_an_errexit_break() {
    // Q6 corpus L1-08, line-offset route: arm A's line route
    // was totally silent on this cell (classification
    // all-zero, Flow::Completed) while GNU exits 2 — the
    // hole that made HB fire on the line route. The shared
    // verdict closes it: a failed parse status under armed
    // errexit is an errexit break on this route too.
    let mut session = Session::builder().build().expect("session builds");
    session.run("set -e", &RunOptions::new()).expect("run");
    let outcome = session
        .run("echo ok; echo )bad", &RunOptions::new())
        .expect("run");
    assert_eq!(outcome.status, 2);
    assert_eq!(outcome.flow, Flow::ErrexitBreak(2));
    let class = outcome.classification;
    assert!(class.parse_error);
    assert!(class.errexit_break);
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
}

// ---------------------------------------------------------------------------
// niubash#863 GAP-2: run-route diagnostics as values + the raw
// exit-jump carrier.
//
// GAP-1 left two host routes on the direct driver call: the
// stdin feeder's unclosed-quote arm needs the run's parse
// diagnostics as values (RunOutcome::diagnostics was empty for
// run()), and the feeder + PROMPT_COMMAND tail reads poll the
// executor's raw exit-jump pending (either unwind class arms
// it; classification.exit_jump is the corrected read). The
// executor now records every parse diagnostic it renders
// during an armed run, the session drains the records into
// Diagnostic values (and the diagnostic sink), and
// classification.exit_jump_raw carries the pending as armed.
//
// GNU anchors: read-time EOF diagnostics (parse.y:5419-5437),
// yyerror's EOF path (parse.y:6890-6901), parser_error under
// errexit exit_shell(2)s (error.c:324-327), and a parse error
// alone is a DISCARD (evalstring.c:585-601).

use super::capabilities::DiagnosticSink;
use super::diagnostic::{Diagnostic, DiagnosticKind};

#[test]
fn issue863_gap2_unclosed_quote_reports_a_diagnostic_value() {
    let mut session = Session::builder().build().expect("session builds");
    let mut options = RunOptions::new();
    options.source_name = Some("gap2-pin".to_string());
    let outcome = session.run("echo 'abc", &options).expect("run");
    assert_eq!(outcome.status, 2);
    assert!(outcome.classification.parse_error);
    assert_eq!(outcome.diagnostics.len(), 1);
    let diagnostic = &outcome.diagnostics[0];
    assert_eq!(diagnostic.kind, DiagnosticKind::Parse);
    assert_eq!(
        diagnostic.message,
        "unexpected EOF while looking for matching `''"
    );
    assert_eq!(diagnostic.source_name.as_deref(), Some("gap2-pin"));
    assert_eq!(diagnostic.line, Some(1));
    assert_eq!(diagnostic.status, None);
}

#[test]
fn issue863_gap2_unclosed_comsub_reports_matching_paren() {
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("echo $(abc", &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 2);
    assert!(outcome.classification.parse_error);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(
        outcome.diagnostics[0].message,
        "unexpected EOF while looking for matching `)'"
    );
    // The comsub verdict class reports the EOF line, not the
    // opener's: GNU 5.3.20 on `echo $(abc` prints
    // "line 2: unexpected EOF while looking for matching `)'"
    // (captain-verified probe; the quote class below reports
    // its opener line instead).
    assert_eq!(outcome.diagnostics[0].line, Some(2));
}

#[test]
fn issue863_gap2_midstream_syntax_error_reports_near_value() {
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("echo ok\n)", &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 2);
    assert!(outcome.classification.parse_error);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(
        outcome.diagnostics[0].message,
        "syntax error near unexpected token `)'"
    );
    assert_eq!(outcome.diagnostics[0].line, Some(2));
}

#[test]
fn issue863_gap2_heredoc_delimiter_unclosed_quote_reports() {
    // The delimiter-word shape of issue160's text pin
    // (`cat << "q`): the quote opens inside the heredoc
    // delimiter word on the command line and never closes —
    // the WORD read fails before any heredoc is gathered
    // (parse.y:5419-5437). A quote in a heredoc BODY is
    // literal text and reports nothing (GNU-verified).
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("cat << \"q", &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 2);
    assert!(outcome.classification.parse_error);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(
        outcome.diagnostics[0].message,
        "unexpected EOF while looking for matching `\"'"
    );
    assert_eq!(outcome.diagnostics[0].line, Some(1));
}

#[test]
fn issue863_gap2_clean_runs_report_no_diagnostics() {
    // Line-offset route.
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("echo hi", &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0);
    assert!(outcome.diagnostics.is_empty());
    // Grouped route.
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    let outcome = session.run("echo hi", &options).expect("run");
    assert_eq!(outcome.status, 0);
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn issue863_gap2_grouped_route_reports_diagnostics() {
    // Interactive options route an ordinary source through the
    // grouped driver; its marker-node reports record the same
    // values the line route's do.
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    let outcome = session.run("echo ok; echo )bad", &options).expect("run");
    assert_eq!(outcome.status, 2);
    assert!(outcome.classification.parse_error);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(
        outcome.diagnostics[0].message,
        "syntax error near unexpected token `)'"
    );
}

#[test]
fn issue863_gap2_parse_error_under_errexit_still_reports() {
    let mut session = Session::builder().build().expect("session builds");
    session.run("set -e", &RunOptions::new()).expect("run");
    let outcome = session
        .run("echo ok; echo )bad", &RunOptions::new())
        .expect("run");
    assert_eq!(outcome.status, 2);
    let class = outcome.classification;
    assert!(class.parse_error);
    assert!(class.errexit_break);
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].kind, DiagnosticKind::Parse);
}

#[test]
fn issue863_gap2_exit_jump_raw_discriminates_the_unwind_classes() {
    // A real exit arms the pending and stands as a jump on
    // every read.
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("exit 3", &RunOptions::new()).expect("run");
    assert!(outcome.classification.exit_jump_raw);
    assert!(outcome.classification.exit_jump);

    // The discriminating cell: an errexit break arms the raw
    // pending (evalstring.c:396-403 re-raises both classes)
    // while the corrected read says no jump stood.
    let mut session = Session::builder().build().expect("session builds");
    session.run("set -e", &RunOptions::new()).expect("run");
    let outcome = session.run("false", &RunOptions::new()).expect("run");
    let class = outcome.classification;
    assert!(class.exit_jump_raw);
    assert!(!class.exit_jump);
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));

    // The grouped route reports the same pair.
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    let outcome = session
        .run("false; echo unreachable", &options)
        .expect("run");
    let class = outcome.classification;
    assert!(class.exit_jump_raw);
    assert!(!class.exit_jump);
    assert_eq!(class.unwind, Some(RunUnwind::ErrexitBreak));

    // An ordinary run arms nothing.
    let mut session = Session::builder().build().expect("session builds");
    let outcome = session.run("true", &RunOptions::new()).expect("run");
    assert!(!outcome.classification.exit_jump_raw);
    assert!(!outcome.classification.exit_jump);
}

#[test]
fn issue863_gap2_prompt_command_errexit_break_drains_raw() {
    // The PROMPT_COMMAND tail read niu performs after
    // run_prompt_command: a failing prompt command under
    // `set -e` arms the raw pending, and the PC runner's
    // re-arm (rubash#433 — the jump is re-armed for the
    // reader to honor) makes the corrected read stand too:
    // on this route the raw and corrected reads agree, and
    // both now arrive as one drained value instead of a
    // hand-polled pending.
    let mut session = issue863_gap1_interactive_session();
    let options = issue863_gap1_interactive_options();
    session.run("set -e", &options).expect("run");
    session.run("PROMPT_COMMAND=false", &options).expect("run");
    assert_eq!(session.run_prompt_command(), Some(1));
    let class = session.take_classification();
    assert!(class.exit_jump_raw);
    assert!(class.exit_jump);
    assert_eq!(class.unwind, None);
}

#[derive(Clone, Default)]
struct Issue863Gap2Sink(Rc<RefCell<Vec<Diagnostic>>>);

impl DiagnosticSink for Issue863Gap2Sink {
    fn diagnostic(&mut self, diagnostic: &Diagnostic) {
        self.0.borrow_mut().push(diagnostic.clone());
    }
}

#[test]
fn issue863_gap2_sink_receives_the_run_diagnostics() {
    let sink = Issue863Gap2Sink::default();
    let mut session = Session::builder()
        .diagnostic_sink(Box::new(sink.clone()))
        .build()
        .expect("session builds");
    let mut options = RunOptions::new();
    options.source_name = Some("sink-pin".to_string());
    let outcome = session.run("echo 'abc", &options).expect("run");
    assert_eq!(outcome.diagnostics.len(), 1);
    let delivered = sink.0.borrow();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0], outcome.diagnostics[0]);
    assert_eq!(delivered[0].source_name.as_deref(), Some("sink-pin"));
}

#[test]
fn issue863_gap2_driver_route_produces_the_same_values() {
    // The equivalence basis for the host migration: the values
    // Session::run returns for an unclosed source are the
    // records the driver route itself produces on an armed
    // executor — same status, same message, same line.
    let source = "echo 'abc";
    let mut executor = Executor::new();
    executor.arm_run_diagnostics();
    let status = crate::script_driver::run_source_with_line_offset(
        &mut executor,
        source,
        false,
        0,
        None,
        Some("gap2-pin"),
    );
    let records = executor.take_run_diagnostics();
    assert_eq!(records.len(), 1);

    let mut session = Session::builder().build().expect("session builds");
    let mut options = RunOptions::new();
    options.source_name = Some("gap2-pin".to_string());
    let outcome = session.run(source, &options).expect("run");
    assert_eq!(outcome.status, status);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].message, records[0].message);
    assert_eq!(outcome.diagnostics[0].line, records[0].line);
}

// ---------------------------------------------------------------
// niubash#863 GAP-3: the tier-A bind view. The registry is the
// bind builtin's data plane; the view is one owned snapshot of
// it — generation, editing mode, the variable table, and every
// entry with the canonical bytes the ENGINE's keyseq_bytes
// produces (rl_translate_keyseq, lib/readline/bind.c:516), the
// written form, the kind, the resolved keymap, and the dump
// line the builtin's own format helpers render
// (rl_function_dumper bind.c:2800, rl_macro_dumper bind.c:2899,
// print_unix_command_map bashline.c:4729). Pins drive the
// registry through the session's own `bind` commands and read
// it back through Session::bind_view only.
// ---------------------------------------------------------------

use super::bind::{BindKindView, BindView};

fn issue863_gap3_session() -> Session {
    // An empty INPUTRC keeps the host's own inputrc out of the
    // registry: the first `bind` runs initialize_readline.
    #[cfg(unix)]
    const EMPTY_INPUTRC: &str = "/dev/null";
    #[cfg(windows)]
    const EMPTY_INPUTRC: &str = "NUL";
    Session::builder()
        .env("INPUTRC", EMPTY_INPUTRC)
        .build()
        .expect("session builds")
}

fn issue863_gap3_run(session: &mut Session, source: &str) {
    let outcome = session.run(source, &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0, "run of {source:?}");
}

fn issue863_gap3_variable<'a>(view: &'a BindView, name: &str) -> Option<&'a str> {
    view.variables
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.as_str())
}

fn issue863_gap3_entry<'a>(view: &'a BindView, keyseq: &str) -> &'a super::bind::BindEntryView {
    view.entries
        .iter()
        .find(|entry| entry.keyseq == keyseq)
        .unwrap_or_else(|| panic!("entry {keyseq} registered"))
}

#[test]
fn issue863_gap3_canonical_bytes_cover_the_spelling_family() {
    let mut session = issue863_gap3_session();
    issue863_gap3_run(&mut session, r#"bind '"\C-a": beginning-of-line'"#);
    issue863_gap3_run(&mut session, r#"bind '"\C-?": backward-delete-char'"#);
    issue863_gap3_run(&mut session, r#"bind '"\e[A": previous-history'"#);
    issue863_gap3_run(&mut session, r#"bind '"\M-a": do-lowercase-version'"#);
    issue863_gap3_run(&mut session, r#"bind '"\x41": self-insert'"#);
    issue863_gap3_run(&mut session, r#"bind '"\010": backward-char'"#);
    issue863_gap3_run(&mut session, r#"bind '"\C-x\C-f": character-search'"#);
    let view = session.bind_view();
    let bytes = |keyseq: &str| issue863_gap3_entry(&view, keyseq).bytes.clone();
    // \C- control form and the \C-? DEL spelling.
    assert_eq!(bytes(r"\C-a"), vec![0x01]);
    assert_eq!(bytes(r"\C-?"), vec![0x7f]);
    // CSI written form: ESC + the literal tail.
    assert_eq!(bytes(r"\e[A"), vec![0x1b, b'[', b'A']);
    // \M- meta form: ESC + the base byte.
    assert_eq!(bytes(r"\M-a"), vec![0x1b, b'a']);
    // \x hex and \0 octal escapes.
    assert_eq!(bytes(r"\x41"), vec![0x41]);
    assert_eq!(bytes(r"\010"), vec![0x08]);
    // A multi-unit chord decodes unit by unit.
    assert_eq!(bytes(r"\C-x\C-f"), vec![0x18, 0x06]);
    // The written form and the kind ride along verbatim.
    let entry = issue863_gap3_entry(&view, r"\C-a");
    assert_eq!(entry.keyseq, r"\C-a");
    assert_eq!(
        entry.kind,
        BindKindView::Function {
            name: "beginning-of-line".to_string()
        }
    );
}

#[test]
fn issue863_gap3_generation_bumps_on_mutation_not_on_read() {
    let mut session = issue863_gap3_session();
    let g0 = session.bind_view().generation;
    // A pure read moves nothing: the view reports the generation
    // it leaves behind.
    assert_eq!(session.bind_view().generation, g0);
    issue863_gap3_run(&mut session, r#"bind '"\C-a": beginning-of-line'"#);
    assert_eq!(session.bind_view().generation, g0 + 1);
    // A `bind 'set …'` variable update is a mutation too.
    issue863_gap3_run(&mut session, "bind 'set bell-style none'");
    assert_eq!(session.bind_view().generation, g0 + 2);
    // Unregister bumps (entry removal + the recorded tombstone)
    // and the entry is gone from the next view.
    issue863_gap3_run(&mut session, r"bind -r '\C-a'");
    assert!(session.bind_view().generation > g0 + 2);
    assert!(session
        .bind_view()
        .entries
        .iter()
        .all(|entry| entry.keyseq != r"\C-a"));
}

#[test]
fn issue863_gap3_variables_snapshot_reflects_bind_set() {
    let mut session = issue863_gap3_session();
    let before = session.bind_view();
    assert_eq!(
        issue863_gap3_variable(&before, "keyseq-timeout"),
        Some("500")
    );
    assert_eq!(
        issue863_gap3_variable(&before, "completion-ignore-case"),
        Some("off")
    );
    let slot = before
        .variables
        .iter()
        .position(|(name, _)| name == "keyseq-timeout");
    let count = before.variables.len();
    issue863_gap3_run(&mut session, "bind 'set keyseq-timeout 250'");
    issue863_gap3_run(&mut session, "bind 'set completion-ignore-case on'");
    let view = session.bind_view();
    assert_eq!(issue863_gap3_variable(&view, "keyseq-timeout"), Some("250"));
    assert_eq!(
        issue863_gap3_variable(&view, "completion-ignore-case"),
        Some("on")
    );
    // Updates land in place: registry order and size are the
    // table's own, unchanged by the sets.
    assert_eq!(
        view.variables
            .iter()
            .position(|(name, _)| name == "keyseq-timeout"),
        slot
    );
    assert_eq!(view.variables.len(), count);
}

#[test]
fn issue863_gap3_dump_lines_match_the_builtin_dumps() {
    let dir = temp_dir("gap3-dump");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = issue863_gap3_session();
    session.variables().set("OUT", &dir_text);
    issue863_gap3_run(&mut session, r#"bind '"\C-t": "ins-macro"'"#);
    issue863_gap3_run(&mut session, r#"bind -x '"\C-g": echo bound-c-g'"#);
    issue863_gap3_run(&mut session, r#"bind '"\C-a": beginning-of-line'"#);
    let view = session.bind_view();
    assert_eq!(
        issue863_gap3_entry(&view, r"\C-t").dump,
        r#""\C-t": "ins-macro""#
    );
    assert_eq!(
        issue863_gap3_entry(&view, r"\C-g").dump,
        r#""\C-g" "echo bound-c-g""#
    );
    assert_eq!(
        issue863_gap3_entry(&view, r"\C-a").dump,
        r#""\C-a": beginning-of-line"#
    );
    assert_eq!(
        issue863_gap3_entry(&view, r"\C-g").kind,
        BindKindView::Execute {
            command: "echo bound-c-g".to_string()
        }
    );
    assert_eq!(
        issue863_gap3_entry(&view, r"\C-t").kind,
        BindKindView::Macro {
            text: "ins-macro".to_string()
        }
    );
    // The builtin's own dumps print the same lines for the same
    // registry state: -s is exactly the macro set, -X exactly the
    // execute set, and -p carries the function line among its
    // effective-map listing.
    issue863_gap3_run(&mut session, "bind -s > \"$OUT/macros.txt\"");
    assert_eq!(
        read(&dir, "macros.txt").lines().collect::<Vec<_>>(),
        vec![r#""\C-t": "ins-macro""#]
    );
    issue863_gap3_run(&mut session, "bind -X > \"$OUT/exec.txt\"");
    assert_eq!(
        read(&dir, "exec.txt").lines().collect::<Vec<_>>(),
        vec![r#""\C-g" "echo bound-c-g""#]
    );
    issue863_gap3_run(&mut session, "bind -p > \"$OUT/funcs.txt\"");
    assert!(read(&dir, "funcs.txt")
        .lines()
        .any(|line| line == r#""\C-a": beginning-of-line"#));
}

#[test]
fn issue863_gap3_none_keymap_lands_under_the_current_keymap() {
    let mut session = issue863_gap3_session();
    assert_eq!(session.bind_view().editing_mode, None);
    issue863_gap3_run(&mut session, r#"bind '"\C-a": beginning-of-line'"#);
    issue863_gap3_run(
        &mut session,
        r#"bind -m vi-insert '"\C-a": beginning-of-line'"#,
    );
    issue863_gap3_run(&mut session, "bind 'set editing-mode vi'");
    // Registered after the mode flip, with no -m: the registry's
    // current keymap is vi-insert now, and the entry lands there.
    issue863_gap3_run(&mut session, r#"bind '"\C-b": backward-char'"#);
    let view = session.bind_view();
    assert_eq!(view.editing_mode.as_deref(), Some("vi"));
    let keymaps: Vec<&str> = view
        .entries
        .iter()
        .filter(|entry| entry.keyseq == r"\C-a")
        .map(|entry| entry.keymap.as_str())
        .collect();
    assert_eq!(keymaps, vec!["emacs-standard", "vi-insert"]);
    assert_eq!(issue863_gap3_entry(&view, r"\C-b").keymap, "vi-insert");
}

#[test]
fn issue863_gap3_removals_ride_the_view() {
    let mut session = issue863_gap3_session();
    issue863_gap3_run(&mut session, r#"bind '"\C-a": beginning-of-line'"#);
    let view = session.bind_view();
    assert!(view.removals.is_empty());
    assert!(view.entries.iter().any(|entry| entry.keyseq == r"\C-a"));

    // Removing a registered entry: it leaves `entries` and its
    // tombstone rides the view with canonical bytes + the
    // resolved keymap, so a mirroring host needs no tier-C read.
    // (`bind -r` takes the bare seq — bind.def:168/292 hands the
    // raw optarg to unbind_keyseq, no inputrc quoting.)
    issue863_gap3_run(&mut session, r"bind -r '\C-a'");
    let view = session.bind_view();
    assert!(view.entries.iter().all(|entry| entry.keyseq != r"\C-a"));
    assert_eq!(view.removals.len(), 1);
    assert_eq!(view.removals[0].keyseq, r"\C-a");
    assert_eq!(view.removals[0].bytes, vec![0x01]);
    assert_eq!(view.removals[0].keymap, "emacs-standard");

    // Removing a sequence that only ever lived in the DEFAULT
    // table records a tombstone too (GNU deletes the slot from
    // the map itself, defaults included — rubash#2337).
    issue863_gap3_run(&mut session, r"bind -r '\C-t'");
    let view = session.bind_view();
    assert_eq!(view.removals.len(), 2);
    assert_eq!(view.removals[1].keyseq, r"\C-t");
    assert_eq!(view.removals[1].bytes, vec![0x14]);

    // Registering the sequence again clears its tombstone.
    issue863_gap3_run(&mut session, r#"bind '"\C-a": beginning-of-line'"#);
    let view = session.bind_view();
    assert!(view
        .removals
        .iter()
        .all(|removal| removal.keyseq != r"\C-a"));
    assert!(view.entries.iter().any(|entry| entry.keyseq == r"\C-a"));
}

// ---------------------------------------------------------------
// niubash#863 GAP-4 (+ GAP-4b): the tier-A completion-state
// view. A completer reads live state — session cwd, the whole
// variable table, function names, open fds — and until GAP-4
// the contract carried only pieces of it, so hosts mirrored
// the rest through the tier-C executor seam into a snapshot
// struct re-synchronized after every line. The view is one
// capture through `&self` from the executor's own snapshot
// functions; these pins drive state through the session's own
// run lines and read it back through Session::completion_view
// / Session::has_function only, checking every field against
// the executor truth the seam used to expose. The host's /tmp
// answer is deliberately NOT in the view (GAP-5's object).
// ---------------------------------------------------------------

use super::completion_view::CompletionView;

fn issue863_gap4_session() -> Session {
    Session::builder().build().expect("session builds")
}

fn issue863_gap4_run(session: &mut Session, source: &str) {
    let outcome = session.run(source, &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0, "run of {source:?}");
}

/// The executor truth, in the view's presentation: the
/// snapshot map's items, sorted by name.
fn issue863_gap4_executor_env(session: &Session) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> =
        session.executor().env_vars_snapshot().into_iter().collect();
    env.sort();
    env
}

fn issue863_gap4_env_get<'a>(view: &'a CompletionView, name: &str) -> Option<&'a str> {
    view.env
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.as_str())
}

/// Every field of the view, against the executor truth the
/// tier-C seam used to hand the host piece by piece.
fn issue863_gap4_assert_view_matches_truth(session: &Session, view: &CompletionView) {
    assert_eq!(view.cwd, session.executor().session_cwd());
    assert_eq!(view.env, issue863_gap4_executor_env(session));
    assert_eq!(view.functions, session.executor().functions_snapshot());
    assert_eq!(view.open_fds, session.executor().open_fds());
}

#[test]
fn issue863_gap4_shared_reference_reads_both_faces() {
    let session = issue863_gap4_session();
    // Both faces are `&self`: a shared reference is all a
    // completer (or a widget probe) ever holds. That this
    // compiles is half the pin; the reads below are the other.
    let shared: &Session = &session;
    let view = shared.completion_view();
    assert!(!shared.has_function("gap4_no_such_function"));
    assert_eq!(view.cwd, shared.cwd());
    // The env presentation is sorted by name (names unique).
    assert!(view.env.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert_eq!(view.functions, shared.functions());
    issue863_gap4_assert_view_matches_truth(shared, &view);
}

#[test]
fn issue863_gap4_has_function_flips_with_define_and_unset() {
    let mut session = issue863_gap4_session();
    assert!(!session.has_function("gap4_probe_fn"));
    issue863_gap4_run(&mut session, "gap4_probe_fn() { return 7; }");
    assert!(session.has_function("gap4_probe_fn"));
    // Same answer from every face: the executor's own probe
    // and the &mut-shaped state view delegate to it, and the
    // tier-A face must not drift from either.
    assert!(session.executor().has_function("gap4_probe_fn"));
    assert!(session.views().has_function("gap4_probe_fn"));
    assert!(session
        .functions()
        .iter()
        .any(|name| name == "gap4_probe_fn"));
    issue863_gap4_run(&mut session, "unset -f gap4_probe_fn");
    assert!(!session.has_function("gap4_probe_fn"));
    assert!(!session.executor().has_function("gap4_probe_fn"));
}

#[test]
fn issue863_gap4_view_tracks_executor_truth_step_by_step() {
    let mut session = issue863_gap4_session();
    issue863_gap4_assert_view_matches_truth(&session, &session.completion_view());

    // A plain variable lands in the table.
    issue863_gap4_run(&mut session, "GAP4_ALPHA=one");
    let view = session.completion_view();
    assert_eq!(issue863_gap4_env_get(&view, "GAP4_ALPHA"), Some("one"));
    issue863_gap4_assert_view_matches_truth(&session, &view);

    // An attribute change moves the ATTR pseudo-entry the
    // snapshot materializes (exported names list), which the
    // narrower Session::environment subset could never show.
    issue863_gap4_run(&mut session, "export GAP4_BETA=two");
    let view = session.completion_view();
    assert_eq!(issue863_gap4_env_get(&view, "GAP4_BETA"), Some("two"));
    let exported = issue863_gap4_env_get(&view, "__RUBASH_EXPORTED_VARS")
        .expect("exported pseudo-entry present once a variable is exported");
    assert!(
        exported.contains("GAP4_BETA"),
        "exported list {exported:?} names GAP4_BETA"
    );
    issue863_gap4_assert_view_matches_truth(&session, &view);

    // Readonly is another attribute class, another pseudo-entry.
    issue863_gap4_run(&mut session, "readonly GAP4_GAMMA=three");
    let view = session.completion_view();
    let readonly = issue863_gap4_env_get(&view, "__RUBASH_READONLY_VARS")
        .expect("readonly pseudo-entry present once a variable is readonly");
    assert!(
        readonly.contains("GAP4_GAMMA"),
        "readonly list {readonly:?} names GAP4_GAMMA"
    );
    issue863_gap4_assert_view_matches_truth(&session, &view);

    // A defined function joins the names.
    issue863_gap4_run(&mut session, "gap4_tracked_fn() { return 3; }");
    let view = session.completion_view();
    assert!(view.functions.iter().any(|name| name == "gap4_tracked_fn"));
    issue863_gap4_assert_view_matches_truth(&session, &view);

    // `exec {fd}>file` opens an fd in the session's table; the
    // number the shell assigned rides the variable, and the
    // view's fd list is the table itself.
    let dir = temp_dir("gap4-fd");
    let target = dir.join("gap4-fd.txt");
    let spelled = target.to_string_lossy().replace('\\', "/");
    issue863_gap4_run(&mut session, &format!("exec {{gap4fd}}>{spelled}"));
    let view = session.completion_view();
    let assigned: u32 = issue863_gap4_env_get(&view, "gap4fd")
        .expect("fd variable assigned")
        .parse()
        .expect("fd variable holds a number");
    assert!(
        view.open_fds.contains(&assigned),
        "open_fds {:?} contains the exec-opened {assigned}",
        view.open_fds
    );
    assert!(target.exists());
    issue863_gap4_assert_view_matches_truth(&session, &view);

    // A cd moves the session cwd the view reports.
    issue863_gap4_run(&mut session, "cd /");
    let view = session.completion_view();
    assert_eq!(view.cwd, session.cwd());
    issue863_gap4_assert_view_matches_truth(&session, &view);
}

#[test]
fn issue863_gap4_env_fingerprint_stability_and_sensitivity() {
    let mut session = issue863_gap4_session();
    // Same state, captured twice: same fingerprint (and the
    // whole view is byte-identical — nothing re-renders
    // differently between reads).
    let first = session.completion_view();
    let second = session.completion_view();
    assert_eq!(first, second);
    assert_eq!(first.env_fingerprint, second.env_fingerprint);

    // A variable appearing changes it.
    issue863_gap4_run(&mut session, "GAP4_FP=1");
    let after_set = session.completion_view();
    assert_ne!(after_set.env_fingerprint, first.env_fingerprint);

    // A value change changes it.
    issue863_gap4_run(&mut session, "GAP4_FP=2");
    let after_change = session.completion_view();
    assert_ne!(after_change.env_fingerprint, after_set.env_fingerprint);
    assert_ne!(after_change.env, after_set.env);

    // An attribute-only change — same name, same value, the
    // exported rendering moves — changes it too: the
    // fingerprint covers the rendered snapshot, attributes
    // included, not just values.
    issue863_gap4_run(&mut session, "export GAP4_FP");
    let after_export = session.completion_view();
    assert_eq!(issue863_gap4_env_get(&after_export, "GAP4_FP"), Some("2"));
    assert_ne!(after_export.env_fingerprint, after_change.env_fingerprint);
}

// ---------------------------------------------------------------
// niubash#863 GAP-5: the host adopts the path resolver through
// Session faces only — the `&self` instance form, the two
// associated env forms, and the associated display form. The
// pins are parity pins: for every shape, the Session face's
// result equals the tier-C face's result, because both
// delegate to the one resolver in `executor::path`. The ruled
// path contract itself (bare `/` is the payload root,
// Option B, the drive spellings) is pinned by the resolver's
// own suites; here only the consumption shape is.
// ---------------------------------------------------------------

use std::collections::HashMap;

fn issue863_gap5_session() -> Session {
    Session::builder().build().expect("session builds")
}

fn issue863_gap5_run(session: &mut Session, source: &str) {
    let outcome = session.run(source, &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0, "run of {source:?}");
}

/// The shape matrix the instance and from-env pins share:
/// the payload root, virtual-system spellings, both drive
/// spellings, relative shapes, the device/proc classes, and
/// the empty string.
const ISSUE863_GAP5_SHAPES: &[&str] = &[
    "/",
    "/tmp",
    "/etc/profile",
    "/d/.",
    "/mnt/d/",
    "D:/",
    "plainname",
    "foo/bar",
    "/dev/null",
    "/proc/self/fd",
    "",
];

fn issue863_gap5_populated_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("HOME".to_string(), "/home/gap5".to_string());
    env.insert("TMPDIR".to_string(), "/mnt/d/tmp".to_string());
    env.insert("PATH".to_string(), "/usr/bin:/mnt/d/tools".to_string());
    env.insert("USERPROFILE".to_string(), "D:\\Users\\gap5".to_string());
    env
}

#[test]
fn issue863_gap5_instance_face_matches_executor_matrix() {
    let session = issue863_gap5_session();
    // The instance face is `&self`: a shared reference is all
    // a host consumer ever holds. That this compiles is half
    // the pin; the matrix below is the other.
    let shared: &Session = &session;
    for shape in ISSUE863_GAP5_SHAPES {
        assert_eq!(
            shared.resolve_shell_path(shape),
            shared.executor().resolve_shell_path(shape),
            "instance parity for {shape:?}"
        );
    }
}

#[test]
fn issue863_gap5_instance_face_reads_live_session_state() {
    let mut session = issue863_gap5_session();
    // The face reads the session's live environment (the
    // executor form feeds `&self.shell_state.env_vars`), so
    // parity must survive the state moving under it: a cd, a
    // plain assignment, an export, a cd back.
    let steps = ["cd /tmp", "GAP5_MARK=one", "export GAP5_MARK", "cd /"];
    for step in steps {
        issue863_gap5_run(&mut session, step);
        let shared: &Session = &session;
        for shape in ISSUE863_GAP5_SHAPES {
            assert_eq!(
                shared.resolve_shell_path(shape),
                shared.executor().resolve_shell_path(shape),
                "instance parity for {shape:?} after {step:?}"
            );
        }
    }
}

#[test]
fn issue863_gap5_from_env_matches_executor_assoc() {
    // The associated form resolves against a caller-supplied
    // map — an empty one and a populated one — and must agree
    // with the executor's associated form on every shape,
    // because both call the one `executor::path` function.
    let envs = [HashMap::new(), issue863_gap5_populated_env()];
    for env in &envs {
        for shape in ISSUE863_GAP5_SHAPES {
            assert_eq!(
                Session::resolve_shell_path_from_env(shape, env),
                Executor::resolve_shell_path_from_env(shape, env),
                "from-env parity for {shape:?} with env {env:?}"
            );
        }
    }
}

#[test]
fn issue863_gap5_process_entries_match_executor_assoc() {
    // PATH-shaped inputs: one shell PATH entry resolved to
    // its native directories, Session face against the
    // executor's associated form, under both env shapes.
    let inputs = ["/usr/bin", "/mnt/d/tools", "bin", "/d/.", "D:/tools", ""];
    let envs = [HashMap::new(), issue863_gap5_populated_env()];
    for env in &envs {
        for input in inputs {
            assert_eq!(
                Session::resolve_shell_path_process_entries_from_env(input, env),
                Executor::resolve_shell_path_process_entries_from_env(input, env),
                "process-entries parity for {input:?} with env {env:?}"
            );
        }
    }
}

#[test]
fn issue863_gap5_display_path_matches_module_function() {
    // The display form is three-way pinned: the Session
    // associated face, the module function it delegates to,
    // and the executor's script-identity wrapper over the
    // same function must all spell a path identically.
    let session = issue863_gap5_session();
    let inputs = [
        "D:\\Users\\cmx",
        "D:/Users/cmx",
        "/c/Users/cmx",
        "\\\\?\\C:\\Users\\cmx",
        "//?/C:/Users/cmx",
        "/tmp",
        "relative/name",
        "",
    ];
    for input in inputs {
        let expected = crate::executor::shell_pwd_display_path(input);
        assert_eq!(
            Session::shell_pwd_display_path(input),
            expected,
            "display parity (module function) for {input:?}"
        );
        assert_eq!(
            session.executor().script_identity_display_path(input),
            expected,
            "display parity (executor wrapper) for {input:?}"
        );
    }
}
