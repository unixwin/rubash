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
// niubash#863 GAP-12: post-build invocation faces
// ---------------------------------------------------------------------------
//
// The apply face GAP-7 deferred, paired with its read face: a parsed
// ShellInvocation is applied to a live session (the Session retains
// it), the live shape is read back from the executor's typed state —
// never from the exported carriers — and the restore-grade removal
// face is the Session home of Executor::remove_env. The journal is
// pinned only as the removal face's implementation semantics: a
// removal is journaled, and the session's Drop restores the birth
// value; no journal read/drain face exists by design.

use crate::invocation::ShellInvocation;

/// Parse helper: the host's argv-shaped invocation.
fn gap12_parse(args: &[&str]) -> ShellInvocation {
    ShellInvocation::parse(&args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>())
        .expect("invocation parses")
}

#[test]
fn issue863_gap12_apply_interactive_login_arms_typed_state() {
    // Q-A + Q-B: the apply face arms the typed interactive flag the
    // apply body alone never writes (the hop-2 asymmetry), and the
    // live read reports it — declared and live agree here only
    // because the session was built plain.
    let mut session = Session::builder().build().expect("session builds");
    assert!(!session.applied_invocation_shape().interactive);
    assert!(!session.applied_invocation_shape().login);
    session
        .apply_invocation_before_startup(&gap12_parse(&["-i", "-l"]))
        .expect("apply succeeds");
    let live = session.applied_invocation_shape();
    assert!(live.interactive && live.login);
    assert_eq!(live.respawn, RespawnSource::Fresh);
    assert!(session.executor().invocation_interactive());
    assert!(session.executor().invocation_login());
    // The declared shape is untouched by the apply: declared-vs-live
    // is an explicit pinned distinction, not a silent merge.
    let declared = session.invocation_shape();
    assert!(!declared.interactive && !declared.login);
    // The carrier projections track the typed state in step.
    assert_eq!(
        session.variable_value("__RUBASH_INTERACTIVE"),
        Some("1".to_string())
    );
    assert_eq!(
        session.variable_value("__RUBASH_LOGIN_SHELL"),
        Some("1".to_string())
    );
}

#[test]
fn issue863_gap12_restricted_is_two_phase() {
    // Q-A: restrictions must not be live while startup files source
    // (GNU shell.c:679-744), so apply alone arms nothing and the
    // retained invocation's second phase activates them.
    let mut session = Session::builder().build().expect("session builds");
    session
        .apply_invocation_before_startup(&gap12_parse(&["-r"]))
        .expect("apply succeeds");
    assert_eq!(
        session.views().options().get("restricted"),
        Some(false),
        "restrictions must not be live before finish_invocation_startup"
    );
    session.finish_invocation_startup();
    assert_eq!(
        session.views().options().get("restricted"),
        Some(true),
        "finish activates the retained restriction"
    );
    // Idempotent: a second finish re-applies the same restriction.
    session.finish_invocation_startup();
    assert_eq!(session.views().options().get("restricted"), Some(true));
}

#[test]
fn issue863_gap12_finish_without_apply_is_noop() {
    // Finish with nothing retained is a no-op, not an error: a host
    // that never applied an invocation finishes a plain session.
    let mut session = Session::builder().build().expect("session builds");
    session.finish_invocation_startup();
    assert_eq!(session.views().options().get("restricted"), Some(false));
    assert!(!session.applied_invocation_shape().interactive);
    assert!(!session.applied_invocation_shape().login);
}

#[test]
fn issue863_gap12_failed_apply_retains_nothing() {
    // A failed apply retains nothing: the session is left as the
    // partial apply left it, and finish is a no-op until a
    // successful apply. The failure is an invalid shell-option name
    // in the parsed invocation (the apply body's own error).
    let mut invocation = gap12_parse(&["-i"]);
    invocation
        .shell_flags
        .push(("nosuchoption".to_string(), true));
    let mut session = Session::builder().build().expect("session builds");
    let err = session
        .apply_invocation_before_startup(&invocation)
        .expect_err("apply must fail on an invalid shell option");
    assert!(
        err.contains("nosuchoption"),
        "error names the bad option: {err}"
    );
    assert!(
        !session.applied_invocation_shape().interactive,
        "a failed apply leaves the live shape untouched"
    );
    session.finish_invocation_startup();
    assert_eq!(
        session.views().options().get("restricted"),
        Some(false),
        "finish after a failed apply is still a no-op"
    );
}

#[test]
fn issue863_gap12_remove_env_vs_unset_differential() {
    // Q-C: the two removal faces differ on every arm — the side-by-side
    // differential the Session face exists to carry. `remove_env` is
    // the restore-grade removal (env-table entry gone, typed cell
    // left as-is, no readonly guard, carrier clears the typed flag);
    // `VariablesView::unset` is the `unset` builtin's leaf (typed cell
    // dropped, readonly refused, carrier leaves a shape lie).

    // Arm 1 — a plain variable: unset drops the typed cell,
    // remove_env leaves it. The setup matters: a typed cell exists
    // only for a variable the shell itself bound — a script
    // assignment creates it, and the view's set then syncs it in
    // place (`set_env`'s sync is update-in-place only; a fresh name
    // set through the view has no typed cell at all).
    let mut session = Session::builder().build().expect("session builds");
    session
        .run("GAP12_DIFF=placeholder", &RunOptions::new())
        .expect("run");
    session.variables().set("GAP12_DIFF", "diff-value");
    assert_eq!(
        session.variable_value("GAP12_DIFF"),
        Some("diff-value".to_string())
    );
    assert_eq!(
        session.variables().resolved_value("GAP12_DIFF"),
        Some("diff-value".to_string())
    );
    assert!(
        session.variables().attributes("GAP12_DIFF").is_some(),
        "the setup holds a typed cell to preserve"
    );
    session.remove_env("GAP12_DIFF");
    assert_eq!(
        session.variable_value("GAP12_DIFF"),
        None,
        "remove_env clears the env-table entry"
    );
    assert_eq!(
        session.variables().resolved_value("GAP12_DIFF"),
        Some("diff-value".to_string()),
        "remove_env leaves the typed cell as-is"
    );
    assert!(
        session.variables().attributes("GAP12_DIFF").is_some(),
        "the typed cell itself survives the removal"
    );

    let mut session = Session::builder().build().expect("session builds");
    session
        .run("GAP12_DIFF=placeholder", &RunOptions::new())
        .expect("run");
    session.variables().set("GAP12_DIFF", "diff-value");
    assert!(
        session.variables().unset("GAP12_DIFF"),
        "unset accepts a plain variable"
    );
    assert_eq!(session.variable_value("GAP12_DIFF"), None);
    assert_eq!(
        session.variables().resolved_value("GAP12_DIFF"),
        None,
        "the unset leaf drops the typed cell"
    );
    assert!(
        session.variables().attributes("GAP12_DIFF").is_none(),
        "the typed cell is gone with the unset"
    );

    // Arm 2 — a readonly variable: unset refuses, remove_env removes.
    let mut session = Session::builder().build().expect("session builds");
    session
        .run("GAP12_RO=placeholder", &RunOptions::new())
        .expect("run");
    session.variables().set("GAP12_RO", "ro-value");
    session
        .run("readonly GAP12_RO", &RunOptions::new())
        .expect("run");
    assert!(
        !session.variables().unset("GAP12_RO"),
        "the unset leaf refuses a readonly name"
    );
    assert_eq!(
        session.variable_value("GAP12_RO"),
        Some("ro-value".to_string())
    );
    session.remove_env("GAP12_RO");
    assert_eq!(
        session.variable_value("GAP12_RO"),
        None,
        "remove_env has no readonly guard — it is restore-grade, not the builtin"
    );
    assert_eq!(
        session.variables().resolved_value("GAP12_RO"),
        Some("ro-value".to_string()),
        "the typed cell survives the removal"
    );

    // Arm 3 — a shape carrier: unset leaves a shape lie (carrier
    // gone, typed flag still set), remove_env clears both.
    let mut session = Session::builder().build().expect("session builds");
    session
        .apply_invocation_before_startup(&gap12_parse(&["-i"]))
        .expect("apply succeeds");
    assert!(session.applied_invocation_shape().interactive);
    assert!(
        session.variables().unset("__RUBASH_INTERACTIVE"),
        "the unset leaf accepts the carrier name"
    );
    assert_eq!(session.variable_value("__RUBASH_INTERACTIVE"), None);
    assert!(
        session.applied_invocation_shape().interactive,
        "unset leaves the typed flag set — the shape lie remove_env exists to avoid"
    );

    let mut session = Session::builder().build().expect("session builds");
    session
        .apply_invocation_before_startup(&gap12_parse(&["-i"]))
        .expect("apply succeeds");
    session.remove_env("__RUBASH_INTERACTIVE");
    assert_eq!(session.variable_value("__RUBASH_INTERACTIVE"), None);
    assert!(
        !session.applied_invocation_shape().interactive,
        "remove_env clears the typed flag with the carrier"
    );
}

#[test]
fn issue863_gap12_mark_login_shell_companion() {
    // Q-E: the login funnel as a Session face — the carrier and the
    // `login_shell` shopt are written together, so they can never
    // split. The companion of the apply face for routes whose parse
    // never reaches a ShellInvocation.
    let mut session = Session::builder().build().expect("session builds");
    assert_eq!(session.views().options().get("login_shell"), Some(false));
    assert!(!session.applied_invocation_shape().login);
    session.mark_login_shell();
    assert!(session.applied_invocation_shape().login);
    assert_eq!(
        session.variable_value("__RUBASH_LOGIN_SHELL"),
        Some("1".to_string())
    );
    assert_eq!(
        session.views().options().get("login_shell"),
        Some(true),
        "the funnel writes the shopt with the carrier"
    );
    // The removal face clears the typed flag with the carrier; the
    // shopt the marking wrote is not part of the removal (pure lift
    // of Executor::remove_env, verbatim).
    session.remove_env("__RUBASH_LOGIN_SHELL");
    assert_eq!(session.variable_value("__RUBASH_LOGIN_SHELL"), None);
    assert!(!session.applied_invocation_shape().login);
}

/// The Q-D journal pin. A removal is journaled, and the session's
/// Drop restores the birth value — pinned as the removal face's
/// implementation semantics, in a re-executed child test process
/// like the #2084 pin (process-global state is only deterministic
/// when no sibling test's session era overlaps).
#[test]
fn issue863_gap12_journal_drop_restores_birth() {
    if std::env::var_os("ISSUE863_GAP12_JOURNAL_CHILD").is_none() {
        let exe = std::env::current_exe().expect("test exe");
        let status = std::process::Command::new(exe)
            .args([
                "--exact",
                "api::consumer_tests::issue863_gap12_journal_drop_restores_birth",
                "--nocapture",
            ])
            .env("ISSUE863_GAP12_JOURNAL_CHILD", "1")
            .env("__RUBASH_GAP12_JOURNAL", "journal-birth")
            .status()
            .expect("child test process spawns");
        assert!(status.success(), "child scenario failed: {status}");
        return;
    }
    issue863_gap12_journal_child_scenario();
}

fn issue863_gap12_journal_child_scenario() {
    assert_eq!(
        std::env::var("__RUBASH_GAP12_JOURNAL").unwrap(),
        "journal-birth"
    );
    {
        let mut session = Session::builder().build().expect("session builds");
        assert_eq!(
            session.variable_value("__RUBASH_GAP12_JOURNAL"),
            Some("journal-birth".to_string())
        );
        session.remove_env("__RUBASH_GAP12_JOURNAL");
        assert_eq!(session.variable_value("__RUBASH_GAP12_JOURNAL"), None);
        assert!(
            std::env::var_os("__RUBASH_GAP12_JOURNAL").is_none(),
            "the removal mirrors out of the process env while the session lives"
        );
    }
    assert_eq!(
        std::env::var("__RUBASH_GAP12_JOURNAL").unwrap(),
        "journal-birth",
        "the session's Drop restores the birth value its journal recorded"
    );
    std::env::remove_var("__RUBASH_GAP12_JOURNAL");
}

/// The U-2 read-safety pin. A poisoned environment must never
/// classify the session: the live read consults typed state, never
/// the carriers, and the import strips the family (table and
/// process-env mirror) so a respawn builder seeded from the process
/// env cannot re-import it either.
#[test]
fn issue863_gap12_poisoned_env_never_classifies() {
    if std::env::var_os("ISSUE863_GAP12_U2_CHILD").is_none() {
        let exe = std::env::current_exe().expect("test exe");
        let status = std::process::Command::new(exe)
            .args([
                "--exact",
                "api::consumer_tests::issue863_gap12_poisoned_env_never_classifies",
                "--nocapture",
            ])
            .env("ISSUE863_GAP12_U2_CHILD", "1")
            .env("__RUBASH_INTERACTIVE", "1")
            .env("__RUBASH_LOGIN_SHELL", "1")
            .status()
            .expect("child test process spawns");
        assert!(status.success(), "child scenario failed: {status}");
        return;
    }
    issue863_gap12_u2_child_scenario();
}

fn issue863_gap12_u2_child_scenario() {
    assert_eq!(std::env::var("__RUBASH_INTERACTIVE").unwrap(), "1");
    assert_eq!(std::env::var("__RUBASH_LOGIN_SHELL").unwrap(), "1");
    let mut session = Session::builder().build().expect("session builds");
    assert_eq!(
        session.variable_value("__RUBASH_INTERACTIVE"),
        None,
        "the import strips the poisoned interactive carrier"
    );
    assert_eq!(
        session.variable_value("__RUBASH_LOGIN_SHELL"),
        None,
        "the import strips the poisoned login carrier"
    );
    let live = session.applied_invocation_shape();
    assert!(
        !live.interactive && !live.login,
        "typed state is born unshaped despite the poisoned env"
    );
    assert!(
        std::env::var_os("__RUBASH_INTERACTIVE").is_none(),
        "the import scrubs the carrier's process-env mirror too"
    );
    assert!(std::env::var_os("__RUBASH_LOGIN_SHELL").is_none());
    // The session still runs: the strip is silent, not an error.
    session.run("true", &RunOptions::new()).expect("run");
}

/// The U-3 interleaving pin. Two live sessions interleave writes to
/// one mirrored name; each session's journal is its own lineage — a
/// dropping session restores only keys whose current value still
/// equals its own journal's final state, so it can never roll back
/// a live sibling's write.
#[test]
fn issue863_gap12_interleaved_sessions_keep_lineage() {
    if std::env::var_os("ISSUE863_GAP12_U3_CHILD").is_none() {
        let exe = std::env::current_exe().expect("test exe");
        let status = std::process::Command::new(exe)
            .args([
                "--exact",
                "api::consumer_tests::issue863_gap12_interleaved_sessions_keep_lineage",
                "--nocapture",
            ])
            .env("ISSUE863_GAP12_U3_CHILD", "1")
            .env("__RUBASH_GAP12_U3", "u3-birth")
            .status()
            .expect("child test process spawns");
        assert!(status.success(), "child scenario failed: {status}");
        return;
    }
    issue863_gap12_u3_child_scenario();
}

fn issue863_gap12_u3_child_scenario() {
    assert_eq!(std::env::var("__RUBASH_GAP12_U3").unwrap(), "u3-birth");
    let mut first = Session::builder().build().expect("first session builds");
    let mut second = Session::builder().build().expect("second session builds");
    first.remove_env("__RUBASH_GAP12_U3");
    assert!(
        std::env::var_os("__RUBASH_GAP12_U3").is_none(),
        "the first session's removal mirrors out while it lives"
    );
    second
        .run("export __RUBASH_GAP12_U3=u3-second", &RunOptions::new())
        .expect("run");
    assert_eq!(
        std::env::var("__RUBASH_GAP12_U3").unwrap(),
        "u3-second",
        "the live sibling's write mirrors in"
    );
    drop(first);
    assert_eq!(
        std::env::var("__RUBASH_GAP12_U3").unwrap(),
        "u3-second",
        "the dropped session does not roll back its live sibling"
    );
    drop(second);
    assert_eq!(
        std::env::var("__RUBASH_GAP12_U3").unwrap(),
        "u3-birth",
        "the last drop restores the birth value"
    );
    std::env::remove_var("__RUBASH_GAP12_U3");
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

// ---------------------------------------------------------------
// niubash#863 GAP-6: the `&self` variable read faces — the
// full-table snapshot (`environment_full`) and the single-value
// probe (`variable_value`). Until GAP-6 the contract's variable
// reads were `&mut`-shaped (the variables view) or exported-only
// (`environment()`), so `&self` host methods read the table
// through the tier-C seam: `executor().env_vars_snapshot()` for
// the full table (the process-PATH synchronization's conversion
// context) and `executor().get_env(name)` for the probes in
// prompt / title / mode code (ledger §2.2's retained-set note
// counts both). The pins are parity pins against those seam
// reads, plus a recorded divergence map: `variable_value` is
// the table's STORED string, while the variables view's
// `resolved_value` is the `$name` expansion read — the two
// agree on plain shapes and differ exactly where expansion
// semantics live (dynamic getters, nameref resolution, array
// element-0). The face is defined to match the seam read it
// replaces (`get_env`); the divergence pins below record both
// readings per shape so the difference is a documented fact,
// not a silent choice.
// ---------------------------------------------------------------

fn issue863_gap6_session() -> Session {
    Session::builder().build().expect("session builds")
}

fn issue863_gap6_run(session: &mut Session, source: &str) {
    let outcome = session.run(source, &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0, "run of {source:?}");
}

/// One session carrying every shape the parity matrix reads:
/// plain and empty scalars, an exported scalar, an indexed
/// array (plus empty-element-0 and empty forms), an associative
/// array, an integer-attribute scalar, a readonly scalar, and
/// a nameref with its target.
fn issue863_gap6_populated_session() -> Session {
    let mut session = issue863_gap6_session();
    for line in [
        "GAP6_PLAIN=alpha",
        "GAP6_EMPTY=",
        "export GAP6_EXPORTED=beta",
        "GAP6_ARR=(aa bb cc)",
        "GAP6_ARR0=('' bb)",
        "GAP6_ARR_EMPTY=()",
        "declare -A GAP6_ASSOC=([k1]=v1 [k2]=v2)",
        "declare -i GAP6_INT=41+1",
        "declare -r GAP6_RO=locked",
        "GAP6_TARGET=hello_target",
        "declare -n GAP6_REF=GAP6_TARGET",
    ] {
        issue863_gap6_run(&mut session, line);
    }
    session
}

/// The executor truth, in the faces' presentation: the
/// snapshot map's items, sorted by name.
fn issue863_gap6_executor_env(session: &Session) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> =
        session.executor().env_vars_snapshot().into_iter().collect();
    env.sort();
    env
}

fn issue863_gap6_env_get<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
    env.iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.as_str())
}

#[test]
fn issue863_gap6_environment_full_matches_snapshot_and_view() {
    let session = issue863_gap6_populated_session();
    // The face is `&self`: a shared reference is all a host
    // consumer ever holds.
    let shared: &Session = &session;
    let full = shared.environment_full();
    // Byte-identical to the tier-C snapshot it replaces, in
    // the sorted presentation.
    assert_eq!(full, issue863_gap6_executor_env(shared));
    // And to the GAP-4 completion view's env component: one
    // source (the shared `full_env_sorted` rendering over the
    // executor's snapshot), so the readings cannot disagree.
    assert_eq!(full, shared.completion_view().env);
    // The populated shapes are all in the table, exported or
    // not — that width is the face's reason to exist.
    for name in [
        "GAP6_PLAIN",
        "GAP6_EMPTY",
        "GAP6_EXPORTED",
        "GAP6_ARR",
        "GAP6_ASSOC",
        "GAP6_INT",
        "GAP6_RO",
    ] {
        assert!(
            issue863_gap6_env_get(&full, name).is_some(),
            "full table carries {name}"
        );
    }
    assert_eq!(issue863_gap6_env_get(&full, "GAP6_PLAIN"), Some("alpha"));
    assert_eq!(issue863_gap6_env_get(&full, "GAP6_INT"), Some("42"));
    // The attribute pseudo-entries ride the snapshot: the
    // exported rendering names the exported variable, the
    // readonly rendering names the readonly one.
    let exported = issue863_gap6_env_get(&full, "__RUBASH_EXPORTED_VARS")
        .expect("exported pseudo-entry present");
    assert!(
        exported.split('\u{1f}').any(|name| name == "GAP6_EXPORTED"),
        "exported rendering names GAP6_EXPORTED: {exported:?}"
    );
    let readonly = issue863_gap6_env_get(&full, "__RUBASH_READONLY_VARS")
        .expect("readonly pseudo-entry present");
    assert!(
        readonly.split('\u{1f}').any(|name| name == "GAP6_RO"),
        "readonly rendering names GAP6_RO: {readonly:?}"
    );
}

#[test]
fn issue863_gap6_environment_is_the_exported_subset_of_full() {
    let session = issue863_gap6_populated_session();
    let full = session.environment_full();
    let exported = session.environment();
    // Every exported pair appears in the full table with the
    // same value — `environment()` is a strict subset
    // statement about `environment_full()`.
    for (name, value) in &exported {
        assert_eq!(
            issue863_gap6_env_get(&full, name),
            Some(value.as_str()),
            "exported {name} appears in the full table with its value"
        );
    }
    // The subset is strict in exactly the designed place: an
    // unexported variable is in the full table and absent
    // from the exported subset, while an exported one is in
    // both.
    assert_eq!(issue863_gap6_env_get(&full, "GAP6_PLAIN"), Some("alpha"));
    assert!(
        !exported.iter().any(|(name, _)| name == "GAP6_PLAIN"),
        "unexported GAP6_PLAIN is absent from environment()"
    );
    assert_eq!(
        issue863_gap6_env_get(&exported, "GAP6_EXPORTED"),
        Some("beta")
    );
}

#[test]
fn issue863_gap6_variable_value_matches_get_env_matrix() {
    let session = issue863_gap6_populated_session();
    // Both reads are `&self`; parity is asserted through one
    // shared reference, the only handle a host probe holds.
    let shared: &Session = &session;
    for name in [
        // The populated shapes.
        "GAP6_PLAIN",
        "GAP6_EMPTY",
        "GAP6_EXPORTED",
        "GAP6_ARR",
        "GAP6_ARR0",
        "GAP6_ARR_EMPTY",
        "GAP6_ASSOC",
        "GAP6_INT",
        "GAP6_RO",
        "GAP6_TARGET",
        "GAP6_REF",
        // Unset, and the names `&self` host methods probe.
        "GAP6_NEVER_SET",
        "PS1",
        "PWD",
        "SHELLOPTS",
        // Dynamic names: parity with the seam read is None on
        // both sides (see the divergence pin for the other
        // reading).
        "RANDOM",
        "SECONDS",
        "BASHPID",
    ] {
        assert_eq!(
            shared.variable_value(name),
            shared.executor().get_env(name).map(str::to_string),
            "variable_value parity with get_env for {name}"
        );
    }
    // Spot values, so the matrix is not only self-referential.
    assert_eq!(
        shared.variable_value("GAP6_PLAIN"),
        Some("alpha".to_string())
    );
    assert_eq!(shared.variable_value("GAP6_EMPTY"), Some(String::new()));
    assert_eq!(shared.variable_value("GAP6_INT"), Some("42".to_string()));
    assert_eq!(shared.variable_value("GAP6_RO"), Some("locked".to_string()));
    assert_eq!(shared.variable_value("GAP6_NEVER_SET"), None);
}

#[test]
fn issue863_gap6_variable_value_agrees_with_resolved_on_plain_shapes() {
    let mut session = issue863_gap6_populated_session();
    // On shapes with no expansion semantics in play, the
    // stored read and the `$name` read are the same read —
    // including the associative rendering and the empty
    // array's stored rendering, which the expansion read
    // falls through to.
    for name in [
        "GAP6_PLAIN",
        "GAP6_EMPTY",
        "GAP6_EXPORTED",
        "GAP6_ASSOC",
        "GAP6_ARR_EMPTY",
        "GAP6_INT",
        "GAP6_RO",
        "GAP6_NEVER_SET",
        "PWD",
    ] {
        assert_eq!(
            session.variable_value(name),
            session.variables().resolved_value(name),
            "stored and resolved readings agree for {name}"
        );
    }
}

#[test]
fn issue863_gap6_readings_diverge_exactly_at_expansion_semantics() {
    let mut session = issue863_gap6_populated_session();
    // Nameref: the table stores the target NAME; the
    // expansion read follows the reference.
    assert_eq!(
        session.variable_value("GAP6_REF"),
        Some("GAP6_TARGET".to_string())
    );
    assert_eq!(
        session.variables().resolved_value("GAP6_REF"),
        Some("hello_target".to_string())
    );
    // Indexed arrays: the table stores the compound rendering
    // (the carrier-prefixed snapshot string); the expansion
    // read yields element 0 — including the empty-element-0
    // form.
    assert_eq!(
        session.variable_value("GAP6_ARR"),
        session.executor().get_env("GAP6_ARR").map(str::to_string)
    );
    assert_eq!(
        session.variables().resolved_value("GAP6_ARR"),
        Some("aa".to_string())
    );
    assert_ne!(
        session.variable_value("GAP6_ARR"),
        session.variables().resolved_value("GAP6_ARR")
    );
    assert_eq!(
        session.variables().resolved_value("GAP6_ARR0"),
        Some(String::new())
    );
    // Dynamic parameters: nothing is stored under the name,
    // so the stored read is None; the expansion read computes
    // the value (a digit string for each of these).
    for name in ["RANDOM", "SECONDS", "BASHPID"] {
        assert_eq!(session.variable_value(name), None, "stored read of {name}");
        let resolved = session
            .variables()
            .resolved_value(name)
            .unwrap_or_else(|| panic!("resolved read of {name} is computed"));
        assert!(
            !resolved.is_empty() && resolved.bytes().all(|b| b.is_ascii_digit()),
            "resolved read of {name} is a digit string: {resolved:?}"
        );
    }
}

#[test]
fn issue863_gap6_faces_read_live_state() {
    let mut session = issue863_gap6_session();
    // Neither face is a cache: a set, a change and an unset
    // between reads are all visible in the second read, in
    // both faces (and in the completion view's env, the third
    // reading of the same source).
    assert_eq!(session.variable_value("GAP6_LIVE"), None);
    assert!(issue863_gap6_env_get(&session.environment_full(), "GAP6_LIVE").is_none());

    issue863_gap6_run(&mut session, "GAP6_LIVE=first");
    assert_eq!(
        session.variable_value("GAP6_LIVE"),
        Some("first".to_string())
    );
    assert_eq!(
        issue863_gap6_env_get(&session.environment_full(), "GAP6_LIVE"),
        Some("first")
    );
    assert_eq!(
        issue863_gap6_env_get(&session.completion_view().env, "GAP6_LIVE"),
        Some("first")
    );

    issue863_gap6_run(&mut session, "GAP6_LIVE=second");
    assert_eq!(
        session.variable_value("GAP6_LIVE"),
        Some("second".to_string())
    );
    assert_eq!(
        issue863_gap6_env_get(&session.environment_full(), "GAP6_LIVE"),
        Some("second")
    );

    issue863_gap6_run(&mut session, "unset GAP6_LIVE");
    assert_eq!(session.variable_value("GAP6_LIVE"), None);
    assert!(issue863_gap6_env_get(&session.environment_full(), "GAP6_LIVE").is_none());
}

// ---------------------------------------------------------------------------
// GAP-7 (niubash#863): the completion-bridge execution core.
// The contract shape decision is (b) — an explicit,
// reentrancy-aware guard object — over (a) a scoped `&self`
// context: an attempt EXECUTES (it runs compspec functions,
// captures COMPREPLY, mutates the governing-compspec copy
// through compopt, and lets function bodies write session
// state), and no `&self` context can own that without
// interior mutability across the whole executor; the read
// half already has `&self` faces. The first three pins are
// the prototype differential promoted verbatim: they measured
// current behavior before the guard landed and all three were
// already green — nested engine reads were live, the dark
// faces were host-side only (the scoped bridge's "no bridge"
// under nesting). The guard pins prove the new shape keeps
// them live through the guard.
// ---------------------------------------------------------------------------

/// Nested `compgen` inside a -F function reads live state: a
/// function defined during the outer attempt is visible to
/// the inner compgen.
#[test]
fn issue863_gap7_nested_compgen_sees_live_state() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(
        &mut session,
        "_f() { _gap7_inner_fn() { :; }; COMPREPLY=( $(compgen -A function -- _gap7) ); }; complete -F _f mycmd",
    );
    let outcome = session.complete_detailed("mycmd d", 7);
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["_gap7_inner_fn"],
        "inner compgen inside a -F function must see state created during the outer attempt"
    );
}

/// The bare-`compopt` window (RL_STATE_COMPLETING port)
/// survives an inner compgen run inside the -F function: the
/// window is an attempt property, not a single-resolution
/// property.
#[test]
fn issue863_gap7_compopt_window_survives_inner_compgen() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(
        &mut session,
        "_f() { compgen -W 'a b' -- \"$2\" > /dev/null; compopt -o nospace; COMPREPLY=(zz); }; complete -F _f mycmd",
    );
    let outcome = session.complete_detailed("mycmd zz", 8);
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["zz"]
    );
    assert!(
        outcome.policy.suppress_append,
        "bare compopt after an inner compgen must still reach the policy"
    );
}

/// Nested dynamic resolution integrity: an inner `compgen -F`
/// run inside the outer -F function must not clobber the
/// outer attempt's governing-compspec copy, and the inner
/// function's side effects persist.
#[test]
fn issue863_gap7_nested_dynamic_resolution_keeps_outer_copy() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(
        &mut session,
        "_g() { GAP7_INNER_SEEN=1; COMPREPLY=(inner_done); }; _f() { compgen -F _g -- x > /dev/null; compopt -o nospace; COMPREPLY=(outer_done); }; complete -F _f mycmd",
    );
    let outcome = session.complete_detailed("mycmd ou", 8);
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["outer_done"],
        "the outer function's COMPREPLY wins; the inner compgen output was captured aside"
    );
    assert!(
        outcome.policy.suppress_append,
        "outer compopt after nested dynamic resolution must reach the outer policy"
    );
    assert_eq!(
        session.variable_value("GAP7_INNER_SEEN"),
        Some("1".to_string()),
        "the inner completion function's side effects persist"
    );
}

/// Availability probe under nesting, through the guard: the
/// -F function registers a spec and runs `complete -p`
/// DURING the in-flight attempt; the guard's `&self` reads
/// afterwards observe the registration live — never dark.
#[test]
fn issue863_gap7_guard_attempt_reads_live_state() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(
        &mut session,
        "_f() { complete -F _g othercmd; RESULT=$(complete -p othercmd); COMPREPLY=(x); }; complete -F _f mycmd",
    );
    let mut attempt = session.begin_completion_attempt();
    assert!(!attempt.session().has_compspec("othercmd"));
    let outcome = attempt.complete_detailed("mycmd x", 7);
    assert_eq!(
        outcome
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["x"]
    );
    assert!(attempt.session().has_compspec("othercmd"));
    let result = attempt
        .session()
        .variable_value("RESULT")
        .unwrap_or_else(|| panic!("the in-flight complete -p capture persisted"));
    assert!(
        result.contains("othercmd") && result.contains("_g"),
        "the in-flight availability probe saw the just-registered spec: {result:?}"
    );
}

/// rubash#2856 (B5-11): the position-aware compspec gate
/// `Session::has_compspec_at` agrees with the generation path on
/// the shapes the command-name form cannot name — the `-E` empty
/// line (no command), the `-I` initial word (the command being
/// completed), and the whitespace-line corners (W1/W2/W3). The
/// position-blind `has_compspec` stays (Q-E): an `-I`-only
/// registration still opens it.
#[test]
fn issue2856_b511_session_has_compspec_at() {
    let mut session = issue863_gap6_session();
    // Nothing registered: the gate stays shut everywhere.
    assert!(!session.has_compspec_at("", 0));
    assert!(!session.has_compspec_at("   ", 3));
    assert!(!session.has_compspec_at("d", 1));
    issue863_gap6_run(
        &mut session,
        "complete -E -W 'eone etwo'; complete -I -W 'ione itwo'; complete -D -W 'dalpha dbeta'",
    );
    // Empty line: -E governs (b01/b05); the -I fallback (W3) is
    // covered by the Executor pins.
    assert!(session.has_compspec_at("", 0));
    // Whitespace line at end-of-line: -E never governs (W1/b09) —
    // no -I-free shape governs here because -I IS registered, so
    // the -I arm answers instead.
    assert!(session.has_compspec_at("   ", 3));
    // Column 0 ahead of the name: -E governs (W2/b11/b15).
    assert!(session.has_compspec_at("   ls", 0));
    assert!(session.has_compspec_at("   ", 0));
    // Command word: -I governs, -D never does (b02/b06).
    assert!(session.has_compspec_at("i", 1));
    // Argument word: -D governs (b07).
    assert!(session.has_compspec_at("zzcmd d", 6));
    // Separator-followed position: -I governs (arm 8).
    assert!(session.has_compspec_at("echo hi; ", 9));
    // The position-blind face is kept (Q-E) and still counts -I.
    assert!(session.has_compspec("git"));
    assert!(session.has_compspec("zzcmd"));
}

#[test]
fn issue2856_b511_session_gate_d_only_never_governs_command_word() {
    // Q-D through the Session face: with only -D registered, no
    // command-word position opens the gate.
    let mut session = issue863_gap6_session();
    issue863_gap6_run(&mut session, "complete -D -W 'dalpha dbeta'");
    assert!(!session.has_compspec_at("d", 1), "b04");
    assert!(!session.has_compspec_at("d", 0), "b16");
    // The empty line still opens it (-D fallback, b03).
    assert!(session.has_compspec_at("", 0));
    // But a whitespace line at end-of-line does not (W1/b09:
    // no -I registered, so nothing governs).
    assert!(!session.has_compspec_at("   ", 3));
}

/// Best-effort cleanup for the #2859 pins: kill every background job
/// this pin started and reap it, so no `sleep 30` outlives the pin.
/// The result is ignored — a missing jobspec errors harmlessly.
fn issue2859_jobs_face_cleanup(session: &mut Session) {
    let _ = session.run(
        "kill %1 %2 %3 2>/dev/null; wait 2>/dev/null",
        &RunOptions::new(),
    );
}

/// rubash#2859 Q-B rule (a), background Running: one `sleep 30 &`
/// counts 1; after `kill` + `wait` it counts 0 (P-J1 j1a/j1e).
#[test]
fn issue2859_jobs_face_background_running_counts() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(&mut session, "sleep 30 &");
    assert_eq!(session.background_job_count(), 1);
    issue2859_jobs_face_cleanup(&mut session);
    assert_eq!(
        session.background_job_count(),
        0,
        "killed + reaped job leaves the count"
    );
}

/// Two background Running jobs count 2 (P-J1 j1a).
#[test]
fn issue2859_jobs_face_two_background_jobs_count() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(&mut session, "sleep 30 &");
    issue863_gap6_run(&mut session, "sleep 31 &");
    assert_eq!(session.background_job_count(), 2);
    issue2859_jobs_face_cleanup(&mut session);
    assert_eq!(session.background_job_count(), 0);
}

/// A reaped job does not count (P-J1 j1c), and repeated reads do not
/// flicker (P-J3): `sleep 0.1 &` dies while the foreground `sleep 0.5`
/// runs; the `:` boundary refresh reaps it, so the face reads 0 and
/// stays 0 — a completed job never counts whether or not the cleanup
/// below has deleted its entry yet.
#[test]
fn issue2859_jobs_face_reaped_job_does_not_count() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(&mut session, "sleep 0.1 &");
    issue863_gap6_run(&mut session, "sleep 0.5");
    // The boundary refresh runs at the start of the next top-level
    // command, so one no-op line forces the reap before the read.
    issue863_gap6_run(&mut session, ":");
    assert_eq!(session.background_job_count(), 0);
    assert_eq!(session.background_job_count(), 0);
    assert_eq!(session.background_job_count(), 0);
}

/// A disowned job does not count (P-J1 j1d): `disown` deletes the
/// table entry (GNU jobs.c delete_job).
#[test]
fn issue2859_jobs_face_disowned_job_does_not_count() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(&mut session, "sleep 30 &");
    assert_eq!(session.background_job_count(), 1);
    issue863_gap6_run(&mut session, "disown");
    assert_eq!(session.background_job_count(), 0);
}

/// A ^Z-stopped job counts (P-J1 j1b): the stop path flips the job
/// to `background = true`, so rule (a) includes it while Stopped
/// and while Running again after CONT.
#[test]
fn issue2859_jobs_face_stopped_job_counts() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(&mut session, "sleep 30 &");
    assert_eq!(session.background_job_count(), 1);
    issue863_gap6_run(&mut session, "kill -STOP %1");
    // The intervening builtin is the observation event that applies
    // the deferred Stopped mark (rubash#1494).
    issue863_gap6_run(&mut session, ":");
    assert_eq!(
        session.background_job_count(),
        1,
        "stopped background job still counts"
    );
    issue863_gap6_run(&mut session, "kill -CONT %1");
    issue863_gap6_run(&mut session, ":");
    assert_eq!(session.background_job_count(), 1);
    issue2859_jobs_face_cleanup(&mut session);
    assert_eq!(session.background_job_count(), 0);
}

/// A process-substitution child never counts: it lives in
/// `background_children` only (GNU keeps procsub children off `jobs`,
/// jobs.c procsub_add), so rule (a) — table entries only — excludes
/// it. This pins the rejection of rule (b).
#[test]
fn issue2859_jobs_face_procsub_child_does_not_count() {
    let mut session = issue863_gap6_session();
    assert_eq!(session.background_job_count(), 0);
    issue863_gap6_run(&mut session, ": <(true)");
    assert_eq!(
        session.background_job_count(),
        0,
        "procsub child is not a table entry"
    );
}

/// Sequential attempts through one guard share live state:
/// the inner attempt resolves a compspec the outer
/// attempt's function registered; after the guard drops, the
/// completion window is closed (bare `compopt` errors).
#[test]
fn issue863_gap7_guard_sequential_attempts_share_live_state() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(
        &mut session,
        "_f() { complete -W 'q1 q2' othercmd; COMPREPLY=(x); }; complete -F _f mycmd",
    );
    let mut attempt = session.begin_completion_attempt();
    let outer = attempt.complete_detailed("mycmd x", 7);
    assert_eq!(
        outer
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["x"]
    );
    let inner = attempt.complete_detailed("othercmd q", 10);
    assert_eq!(
        inner
            .candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["q1", "q2"],
        "the inner attempt through the same guard resolves the spec the outer attempt registered"
    );
    drop(attempt);
    let outcome = session.run("compopt", &RunOptions::new()).expect("run");
    assert_eq!(
        outcome.status, 1,
        "bare compopt outside any attempt errors once the guard has dropped"
    );
}

/// Widget/compdef callback through the guard: the function
/// runs with its temporary bindings (the NIU_COMP_WORDS /
/// NIU_COMP_CWORD injection shape), its `compgen` reads live
/// state, and the temporary bindings are restored after.
#[test]
fn issue863_gap7_guard_widget_callback_runs_compgen_live() {
    let mut session = issue863_gap6_session();
    issue863_gap6_run(
        &mut session,
        "wfun() { OUT=$(compgen -W 'w1 w2' -- w); WSEEN=$NIU_COMP_WORDS; }",
    );
    let mut attempt = session.begin_completion_attempt();
    let status = attempt
        .call_function_with_env(
            "wfun",
            ["a"],
            [("NIU_COMP_WORDS", "mycmd w"), ("NIU_COMP_CWORD", "1")],
        )
        .expect("widget callback runs");
    assert_eq!(status, 0);
    let out = attempt
        .session()
        .variable_value("OUT")
        .unwrap_or_else(|| panic!("the callback's compgen capture persisted"));
    assert!(
        out.contains("w1") && out.contains("w2"),
        "compgen inside the callback read live state: {out:?}"
    );
    assert_eq!(
        attempt.session().variable_value("WSEEN"),
        Some("mycmd w".to_string())
    );
    assert_eq!(
        attempt.session().variable_value("NIU_COMP_WORDS"),
        None,
        "the temporary binding is restored after the callback"
    );
}

// ----------------------------------------------------------------
// GAP-8 (niubash#863): the finish guard — `Session::begin_finish`
// decomposes the one finish pass around the host's interleaved
// duties. GNU anchors: exit_shell's single funnel (shell.c:985)
// with the trap at shell.c:1000-1002 BEFORE the history save at
// shell.c:1007-1009 (the captain's D1 adjudication);
// run_exit_trap's override pair (trap.c:1075-1081);
// maybe_save_shell_history reading HISTFILE/HISTFILESIZE at the
// write (bashhist.c:488-530).

use super::finish::{FinishDuties, SessionHistoryWrite};

fn issue863_gap8_session() -> Session {
    Session::builder().build().expect("session builds")
}

fn issue863_gap8_run(session: &mut Session, source: &str) {
    let outcome = session.run(source, &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0, "run of {source:?}");
}

#[test]
fn issue863_gap8_finish_trap_step_observes_pending_status() {
    let dir = temp_dir("gap8-pending");
    let mut session = issue863_gap8_session();
    issue863_gap8_run(
        &mut session,
        &format!(
            "trap 'echo \"saw=$?\" > \"{}/trap_saw\"' EXIT",
            dir.display()
        ),
    );
    // The session's own last status is 0 (the trap registration
    // succeeded). The guard's pending status is a caller-supplied
    // VALUE — the trap must observe 41, not the executor's 0.
    // This is the contract the host's set_last_status injection
    // into a consuming close used to fake.
    let mut finish = session.begin_finish(41);
    assert_eq!(finish.run_exit_trap(), 41);
    drop(finish);
    assert_eq!(read(&dir, "trap_saw"), "saw=41\n");
}

#[test]
fn issue863_gap8_finish_trap_exit_overrides_pending_status() {
    // The trap.c:1075-1076 arm: an explicit `exit` inside the
    // action makes the result.
    let mut session = issue863_gap8_session();
    issue863_gap8_run(&mut session, "trap 'exit 7' EXIT");
    let mut finish = session.begin_finish(3);
    let outcome = finish.finish_all();
    assert_eq!(outcome.status, 7);
    assert!(outcome.trap_ran);
    assert!(outcome.duties_run);
    drop(finish);
    drop(session);

    // The trap.c:1081 arm: plain completion keeps the pending
    // status — the action's own last-command status (0 here)
    // does not override.
    let mut session = issue863_gap8_session();
    issue863_gap8_run(&mut session, "trap 'true' EXIT");
    let mut finish = session.begin_finish(3);
    let outcome = finish.finish_all();
    assert_eq!(outcome.status, 3);
    assert!(outcome.trap_ran);
}

#[test]
fn issue863_gap8_finish_trap_runs_once_across_entries() {
    let dir = temp_dir("gap8-once");
    let mut session = issue863_gap8_session();
    issue863_gap8_run(
        &mut session,
        &format!("trap 'echo fired >> \"{}/fires\"' EXIT", dir.display()),
    );
    {
        let mut finish = session.begin_finish(5);
        assert_eq!(finish.run_exit_trap(), 5);
        let outcome = finish.finish_all();
        assert!(outcome.trap_ran);
        assert_eq!(outcome.status, 5);
    }
    // A second guard over the same session re-enters the finish;
    // the action must not fire again (rubash#2212: the trap runs
    // once per exit), and the outcome says so as a value.
    {
        let mut finish = session.begin_finish(9);
        let outcome = finish.finish_all();
        assert!(!outcome.trap_ran, "the trap action must not fire twice");
        assert_eq!(outcome.status, 9);
    }
    // The consuming close is a third entry over the same finish;
    // still no second firing.
    let _ = session.close();
    assert_eq!(read(&dir, "fires"), "fired\n");
}

#[test]
fn issue863_gap8_finish_final_cwd_is_post_trap() {
    let dir = temp_dir("gap8-cwd");
    std::fs::create_dir_all(dir.join("sub")).expect("subdir");
    let mut session = issue863_gap8_session();
    issue863_gap8_run(&mut session, &format!("cd \"{}\"", dir.display()));
    issue863_gap8_run(&mut session, "trap 'cd sub' EXIT");
    let mut finish = session.begin_finish(0);
    let before = finish.final_cwd();
    assert_ne!(
        before.file_name().and_then(|name| name.to_str()),
        Some("sub"),
        "pre-trap cwd is the session's own directory: {before:?}"
    );
    assert_eq!(finish.run_exit_trap(), 0);
    // The rubash#668 value as a face: the directory the trap
    // left behind, read while the guard holds the session —
    // no field-parking across a consuming close.
    let after = finish.final_cwd();
    assert_eq!(after, before.join("sub"));
    let outcome = finish.finish_all();
    assert_eq!(outcome.final_cwd, after);
}

#[test]
fn issue863_gap8_finish_after_exec_replacement_runs_no_trap() {
    let dir = temp_dir("gap8-exec");
    let mut session = issue863_gap8_session();
    issue863_gap8_run(
        &mut session,
        &format!("trap 'echo fired > \"{}/fired\"' EXIT", dir.display()),
    );
    // A successful exec replaces the image — no exit path
    // exists for a finish to complete (builtins/exec.def ->
    // execute_cmd.c:6126 shell_execve; the trap flag is cleared
    // before it, trap.c:1440; rubash#1012). The finish over the
    // replaced session must report that no trap ran.
    let _ = session.run("exec true", &RunOptions::new());
    let mut finish = session.begin_finish(0);
    let outcome = finish.finish_all();
    assert!(!outcome.trap_ran);
    assert!(
        !dir.join("fired").exists(),
        "no exit trap fires after an exec replacement"
    );
}

#[test]
fn issue863_gap8_finish_order_history_write_observes_post_trap_state() {
    // D1, arm (a): the trap's HISTFILE edit governs the save —
    // the differential probe D shape (GNU: the session lines
    // land in the post-trap HISTFILE; the pre-trap file is
    // never written). shell.c:1000-1002 (trap) precedes
    // shell.c:1007-1009 (save); bashhist.c:488 reads HISTFILE
    // at the write.
    let dir = temp_dir("gap8-order-a");
    let mut session = issue863_gap8_session();
    issue863_gap8_run(&mut session, "set -o history");
    issue863_gap8_run(
        &mut session,
        &format!("HISTFILE=\"{}/orig\"", dir.display()),
    );
    issue863_gap8_run(&mut session, "HISTFILESIZE=10000");
    issue863_gap8_run(&mut session, "history -s \"echo gap8-order-marker\"");
    issue863_gap8_run(
        &mut session,
        &format!("trap 'HISTFILE=\"{}/fromtrap\"' EXIT", dir.display()),
    );
    {
        let mut finish = session.begin_finish(0);
        let outcome = finish.finish_all();
        assert!(outcome.trap_ran);
        assert!(outcome.history_written);
    }
    assert!(
        read(&dir, "fromtrap").contains("gap8-order-marker"),
        "the save lands in the post-trap HISTFILE"
    );
    assert!(
        !dir.join("orig").exists(),
        "the save must not land in the pre-trap HISTFILE"
    );
    drop(session);

    // D1, arm (b): a history line the trap itself adds is in
    // the save (the differential probe E shape; GNU's file
    // carries the trap-added line because the save runs after
    // the trap).
    let dir = temp_dir("gap8-order-b");
    let mut session = issue863_gap8_session();
    issue863_gap8_run(&mut session, "set -o history");
    issue863_gap8_run(
        &mut session,
        &format!("HISTFILE=\"{}/hist\"", dir.display()),
    );
    issue863_gap8_run(&mut session, "HISTFILESIZE=10000");
    issue863_gap8_run(&mut session, "history -s \"echo gap8-before-trap\"");
    issue863_gap8_run(
        &mut session,
        "trap 'history -s \"echo gap8-trap-added\"' EXIT",
    );
    {
        let mut finish = session.begin_finish(0);
        let outcome = finish.finish_all();
        assert!(outcome.history_written);
    }
    let saved = read(&dir, "hist");
    assert!(
        saved.contains("gap8-before-trap"),
        "the session line is saved: {saved:?}"
    );
    assert!(
        saved.contains("gap8-trap-added"),
        "the trap-added line is saved — the write observes post-trap state: {saved:?}"
    );
}

#[test]
fn issue863_gap8_finish_duties_without_history_step() {
    // The D3 shape: the interactive route's finish — trap step
    // and stream-end duties with the engine history step gated
    // OFF (the host's history is its own provider's store;
    // writing both double-writes, niu#642). The duties still
    // run: the persistent output procsub closes (its deferred
    // body receives the accumulated input, rubash#1336).
    let dir = temp_dir("gap8-duties");
    let mut session = issue863_gap8_session();
    issue863_gap8_run(&mut session, "set -o history");
    issue863_gap8_run(
        &mut session,
        &format!("HISTFILE=\"{}/hist\"", dir.display()),
    );
    issue863_gap8_run(&mut session, "HISTFILESIZE=10000");
    issue863_gap8_run(&mut session, "history -s \"echo gap8-duties-marker\"");
    issue863_gap8_run(
        &mut session,
        &format!("exec > >(cat > \"{}/procsub_out\")", dir.display()),
    );
    issue863_gap8_run(&mut session, "echo gap8-procsub-payload");
    let outcome = {
        let mut finish = session.begin_finish(0);
        finish.run_stream_end_duties(FinishDuties {
            history: SessionHistoryWrite::Skip,
        });
        finish.finish_all()
    };
    assert!(outcome.duties_run);
    assert!(
        !outcome.history_written,
        "the gated-off history step writes nothing"
    );
    assert!(
        !dir.join("hist").exists(),
        "gated-off duties must not write the history file"
    );
    assert_eq!(read(&dir, "procsub_out"), "gap8-procsub-payload\n");
}

// ----------------------------------------------------------------
// GAP-9 (niubash#863): the token view — `Session::token_view`
// projects the lexer onto caller-fed text (absolute byte spans,
// kinds, dequoted values, folded command positions and roles,
// synthesized comment spans), and `Session::needs_more_input`
// enters the ledger as the completeness face the host validator
// adopts. The fold's sources are the lexer's own stamps and the
// engine's builtin table; the value rule is the host-shaped
// dequote the editor consumers are built on.

use super::syntax::{TokenKindView, TokenRole, TokenView};

fn issue863_gap9_session() -> Session {
    Session::builder().build().expect("session builds")
}

fn issue863_gap9_posix_session() -> Session {
    let mut session = issue863_gap9_session();
    let outcome = session
        .run("set -o posix", &RunOptions::new())
        .expect("set posix");
    assert_eq!(outcome.status, 0);
    session
}

/// One span as a comparable tuple:
/// (start, end, kind, text, command_position, role).
type Gap9Shape = (usize, usize, TokenKindView, String, bool, TokenRole);

fn issue863_gap9_shape(view: &TokenView) -> Vec<Gap9Shape> {
    view.tokens
        .iter()
        .map(|span| {
            (
                span.start,
                span.end,
                span.kind,
                span.text.clone(),
                span.command_position,
                span.role,
            )
        })
        .collect()
}

fn gap9_shape(
    start: usize,
    end: usize,
    kind: TokenKindView,
    text: &str,
    command_position: bool,
    role: TokenRole,
) -> Gap9Shape {
    (start, end, kind, text.to_string(), command_position, role)
}

#[test]
fn issue863_gap9_token_view_spans_and_kinds() {
    use TokenKindView as K;
    use TokenRole as R;
    let session = issue863_gap9_session();

    // One full line: prefix assignment, builtin, quoted words, a
    // pipeline, a demoted keyword as argument, both redirect
    // shapes, and a trailing comment.
    let view = session
        .token_view("FOO=bar printf '%s' \"$HOME\" | wc -c && echo done > out.txt 2>&1 # tally");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 7, K::Assignment, "FOO=bar", true, R::Assignment),
            gap9_shape(8, 14, K::Word, "printf", true, R::Builtin),
            gap9_shape(15, 19, K::Word, "'%s'", false, R::Plain),
            gap9_shape(20, 27, K::Word, "\"$HOME\"", false, R::Plain),
            gap9_shape(28, 29, K::Pipe, "|", false, R::Plain),
            gap9_shape(30, 32, K::Word, "wc", true, R::Plain),
            gap9_shape(33, 35, K::Word, "-c", false, R::Plain),
            gap9_shape(36, 38, K::And, "&&", false, R::Plain),
            gap9_shape(39, 43, K::Word, "echo", true, R::Builtin),
            gap9_shape(44, 48, K::Keyword, "done", false, R::Plain),
            gap9_shape(49, 50, K::RedirectOut, ">", false, R::Plain),
            gap9_shape(51, 58, K::Word, "out.txt", false, R::RedirectTarget),
            gap9_shape(59, 62, K::RedirectErr, "2>&", false, R::Plain),
            gap9_shape(62, 63, K::Word, "1", false, R::RedirectTarget),
            gap9_shape(64, 71, K::Comment, "# tally", false, R::Plain),
        ]
    );
    // Dequoted values ride along: quotes vanish, the variable
    // word keeps its spelling.
    let values: Vec<&str> = view.tokens.iter().map(|span| span.value.as_str()).collect();
    assert_eq!(values[2], "%s");
    assert_eq!(values[3], "$HOME");
    assert_eq!(values[14], "# tally");

    // Comment lines and a trailing comment: comments are spans,
    // line breaks are Semicolon spans over the newline byte.
    let view = session.token_view("# head\necho a # trail\n# last");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 6, K::Comment, "# head", false, R::Plain),
            gap9_shape(6, 7, K::Semicolon, "\n", false, R::Plain),
            gap9_shape(7, 11, K::Word, "echo", true, R::Builtin),
            gap9_shape(12, 13, K::Word, "a", false, R::Plain),
            gap9_shape(14, 21, K::Comment, "# trail", false, R::Plain),
            gap9_shape(21, 22, K::Semicolon, "\n", false, R::Plain),
            gap9_shape(22, 28, K::Comment, "# last", false, R::Plain),
        ]
    );

    // A quoted here-doc: the delimiter word is the redirect
    // target, the body span covers the body lines as fed, and the
    // delimiter line itself carries no span.
    let view = session.token_view("cat <<'EOF'\nbody line\nEOF\necho after");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 3, K::Word, "cat", true, R::Plain),
            gap9_shape(4, 6, K::HereDoc, "<<", false, R::Plain),
            gap9_shape(6, 11, K::Word, "'EOF'", false, R::RedirectTarget),
            gap9_shape(12, 22, K::HereDocBody, "body line\n", false, R::Plain),
            gap9_shape(25, 26, K::Semicolon, "\n", false, R::Plain),
            gap9_shape(26, 30, K::Word, "echo", true, R::Builtin),
            gap9_shape(31, 36, K::Word, "after", false, R::Plain),
        ]
    );
    assert_eq!(view.tokens[2].value, "EOF");

    // A `<<-` body: the span covers the input lines WITH their
    // leading tabs (the lexer's raw body has them stripped; the
    // input slice is the truth).
    let view = session.token_view("cat <<-EOF\n\tbody\n\tEOF\necho done");
    let body = &view.tokens[3];
    assert_eq!(body.kind, K::HereDocBody);
    assert_eq!((body.start, body.end), (11, 17));
    assert_eq!(body.text, "\tbody\n");

    // A line continuation: the second physical line's word is
    // addressed within its logical line.
    let view = session.token_view("echo one \\\n two");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 4, K::Word, "echo", true, R::Builtin),
            gap9_shape(5, 8, K::Word, "one", false, R::Plain),
            gap9_shape(12, 15, K::Word, "two", false, R::Plain),
        ]
    );

    // The derivation methods: word under the cursor, the words of
    // the cursor's segment, the segment's separator.
    let view = session.token_view("echo one | wc -c");
    assert_eq!(view.word_at(2).map(|span| span.text.as_str()), Some("echo"));
    assert_eq!(view.word_at(6).map(|span| span.text.as_str()), Some("one"));
    assert_eq!(view.word_at(9), None, "the pipe is not a word");
    assert_eq!(view.word_at(5), None, "a word start is not inside the word");
    let before: Vec<&str> = view
        .segment_words_before(14)
        .iter()
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(before, vec!["wc"], "only the cursor's segment counts");
    let before: Vec<&str> = view
        .segment_words_before(16)
        .iter()
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(before, vec!["wc", "-c"], "a straddling word counts");
    assert_eq!(
        view.last_separator_before(15)
            .map(|span| span.text.as_str()),
        Some("|")
    );
    assert_eq!(view.last_separator_before(4), None);
}

#[test]
fn issue863_gap9_token_view_command_position() {
    let session = issue863_gap9_session();
    // (line, [(span text, command_position)]) over the whole view.
    let cases: Vec<(&str, Vec<(&str, bool)>)> = vec![
        (
            "echo a | wc -c",
            vec![
                ("echo", true),
                ("a", false),
                ("|", false),
                ("wc", true),
                ("-c", false),
            ],
        ),
        (
            "A=1 B=2 cmd x",
            vec![("A=1", true), ("B=2", true), ("cmd", true), ("x", false)],
        ),
        (
            // A keyword after a separator stands where a command
            // word may stand (the condition/body list could
            // continue there), so `then` / `fi` / `do` / `done`
            // in that position report command position too.
            "if true; then echo yes; fi",
            vec![
                ("if", true),
                ("true", true),
                (";", false),
                ("then", true),
                ("echo", true),
                ("yes", false),
                (";", false),
                ("fi", true),
            ],
        ),
        (
            "while read -r line; do echo \"$line\"; done < names",
            vec![
                ("while", true),
                ("read", true),
                ("-r", false),
                ("line", false),
                (";", false),
                ("do", true),
                ("echo", true),
                ("\"$line\"", false),
                (";", false),
                ("done", true),
                ("<", false),
                ("names", false),
            ],
        ),
        (
            "cmd && other || third",
            vec![
                ("cmd", true),
                ("&&", false),
                ("other", true),
                ("||", false),
                ("third", true),
            ],
        ),
        (
            "! time -p ls",
            vec![("!", true), ("time", true), ("-p", false), ("ls", true)],
        ),
        (
            // A brace group folds into ONE lexer token; the
            // projection descends into it (round 2), so its
            // interior words carry their own positions.
            "{ echo grouped; }",
            vec![
                ("{", true),
                ("echo", true),
                ("grouped", false),
                (";", false),
                ("}", false),
            ],
        ),
        (
            "(cd sub && ls)",
            vec![
                ("(", true),
                ("cd", true),
                ("sub", false),
                ("&&", false),
                ("ls", true),
                (")", false),
            ],
        ),
        (
            "foo() { echo body; }",
            vec![
                ("foo", true),
                ("(", false),
                (")", false),
                ("{", false),
                ("echo", true),
                ("body", false),
                (";", false),
                ("}", false),
            ],
        ),
        ("time ls", vec![("time", true), ("ls", true)]),
        (
            // Nothing follows the delimiter line, so the lexer
            // emits no line-break separator after the body.
            "cat <<EOF\nbody\nEOF",
            vec![
                ("cat", true),
                ("<<", false),
                ("EOF", false),
                ("body\n", false),
            ],
        ),
    ];
    for (line, expected) in cases {
        let view = session.token_view(line);
        let got: Vec<(&str, bool)> = view
            .tokens
            .iter()
            .map(|span| (span.text.as_str(), span.command_position))
            .collect();
        assert_eq!(got, expected, "command positions for {line:?}");
    }
}

#[test]
fn issue863_gap9_token_view_word_roles() {
    use TokenRole as R;
    let session = issue863_gap9_session();
    let roles = |line: &str| -> Vec<(String, R)> {
        session
            .token_view(line)
            .tokens
            .iter()
            .map(|span| (span.text.clone(), span.role))
            .collect()
    };
    let expect = |line: &str, expected: Vec<(&str, R)>| {
        let expected: Vec<(String, R)> = expected
            .into_iter()
            .map(|(t, r)| (t.to_string(), r))
            .collect();
        assert_eq!(roles(line), expected, "roles for {line:?}");
    };

    // `printf` is an engine builtin — the role table is the
    // engine's own, not any host-side list.
    expect(
        "printf '%s' x",
        vec![("printf", R::Builtin), ("'%s'", R::Plain), ("x", R::Plain)],
    );
    expect(
        "echo x > f",
        vec![
            ("echo", R::Builtin),
            ("x", R::Plain),
            (">", R::Plain),
            ("f", R::RedirectTarget),
        ],
    );
    expect("A=1 cmd", vec![("A=1", R::Assignment), ("cmd", R::Plain)]);
    // An assignment-shaped word in argument position is plain:
    // kind is not role.
    expect("echo A=1", vec![("echo", R::Builtin), ("A=1", R::Plain)]);
    expect(
        "if true; then echo ok; fi",
        vec![
            ("if", R::Reserved),
            ("true", R::Builtin),
            (";", R::Plain),
            ("then", R::Reserved),
            ("echo", R::Builtin),
            ("ok", R::Plain),
            (";", R::Plain),
            ("fi", R::Reserved),
        ],
    );
    expect(
        "for i in a b; do echo $i; done",
        vec![
            ("for", R::Reserved),
            ("i", R::Plain),
            ("in", R::Plain),
            ("a", R::Plain),
            ("b", R::Plain),
            (";", R::Plain),
            ("do", R::Reserved),
            ("echo", R::Builtin),
            ("$i", R::Plain),
            (";", R::Plain),
            ("done", R::Reserved),
        ],
    );
    expect(
        "case $x in a) echo hit;; esac",
        vec![
            ("case", R::Reserved),
            ("$x", R::Plain),
            ("in", R::Reserved),
            ("a", R::Plain),
            (")", R::Plain),
            ("echo", R::Builtin),
            ("hit", R::Plain),
            (";;", R::Plain),
            ("esac", R::Reserved),
        ],
    );
    expect(
        "time -p ls",
        vec![("time", R::Reserved), ("-p", R::Plain), ("ls", R::Plain)],
    );
    expect(
        // A pending here-doc's body token rides the stream with
        // an empty span at the end of the text.
        "cat <<EOF",
        vec![
            ("cat", R::Plain),
            ("<<", R::Plain),
            ("EOF", R::RedirectTarget),
            ("", R::Plain),
        ],
    );
    expect(
        "function greet { echo hi; }",
        vec![
            ("function", R::Reserved),
            ("greet", R::Plain),
            ("{", R::Reserved),
            ("echo", R::Builtin),
            ("hi", R::Plain),
            (";", R::Plain),
            ("}", R::Reserved),
        ],
    );
    expect(
        "select opt in x y; do break; done",
        vec![
            ("select", R::Reserved),
            ("opt", R::Plain),
            ("in", R::Plain),
            ("x", R::Plain),
            ("y", R::Plain),
            (";", R::Plain),
            ("do", R::Reserved),
            ("break", R::Builtin),
            (";", R::Plain),
            ("done", R::Reserved),
        ],
    );
    // Reserved spellings in argument position are demoted words.
    expect(
        "echo if done time",
        vec![
            ("echo", R::Builtin),
            ("if", R::Plain),
            ("done", R::Plain),
            ("time", R::Plain),
        ],
    );
}

#[test]
fn issue863_gap9_token_view_value_dequote_parity() {
    use TokenKindView as K;
    let session = issue863_gap9_session();
    // (line, [(span text, dequoted value)] over the word-shaped
    // spans). The expected values are the host editor's current
    // `unescape_shell_word` outputs for the same words — the
    // parity this view exists to keep when the host retires its
    // private copy of the machine.
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "echo 'it'\\''s'",
            vec![("echo", "echo"), ("'it'\\''s'", "it's")],
        ),
        (
            "echo \"a\\\"b\"",
            vec![("echo", "echo"), ("\"a\\\"b\"", "a\"b")],
        ),
        // Inside double quotes the host rule quotes the next
        // character literally, whatever it is.
        (
            "echo \"a\\nb\"",
            vec![("echo", "echo"), ("\"a\\nb\"", "anb")],
        ),
        // Inside single quotes a backslash is literal.
        ("echo 'a\\nb'", vec![("echo", "echo"), ("'a\\nb'", "a\\nb")]),
        ("echo a\\ b", vec![("echo", "echo"), ("a\\ b", "a b")]),
        // $'...' ANSI-C escapes are NOT interpreted by the host
        // machine: it reads a `$` plus a single-quoted body — the
        // `$` survives, the quotes vanish, the backslash stays a
        // literal character (never a TAB).
        (
            "echo $'a\\tb'",
            vec![("echo", "echo"), ("$'a\\tb'", "$a\\tb")],
        ),
        (
            "echo \"$(foo bar)\"",
            vec![("echo", "echo"), ("\"$(foo bar)\"", "$(foo bar)")],
        ),
        ("echo $(foo)", vec![("echo", "echo"), ("$(foo)", "$(foo)")]),
        (
            "echo \"pre 'mid' post\"",
            vec![("echo", "echo"), ("\"pre 'mid' post\"", "pre 'mid' post")],
        ),
        (
            "echo 'don'\\''t' \"say \\\"hi\\\"\"",
            vec![
                ("echo", "echo"),
                ("'don'\\''t'", "don't"),
                ("\"say \\\"hi\\\"\"", "say \"hi\""),
            ],
        ),
        // A trailing lone backslash survives.
        ("echo abc\\", vec![("echo", "echo"), ("abc\\", "abc\\")]),
        // An unterminated quote yields its content.
        (
            "echo \"unclosed",
            vec![("echo", "echo"), ("\"unclosed", "unclosed")],
        ),
        // Dequote applies to assignment words too.
        ("x='a b' cmd", vec![("x='a b'", "x=a b"), ("cmd", "cmd")]),
    ];
    for (line, expected) in cases {
        let view = session.token_view(line);
        let got: Vec<(&str, &str)> = view
            .tokens
            .iter()
            .filter(|span| span.kind.is_word_shaped())
            .map(|span| (span.text.as_str(), span.value.as_str()))
            .collect();
        assert_eq!(got, expected, "dequote parity for {line:?}");
        // Carrier bytes never cross: the lexer's in-band quoting
        // carriers (PUA block and the C0 carriers) appear in no
        // span value, for any shape.
        for span in &view.tokens {
            assert!(
                !span
                    .value
                    .chars()
                    .any(|ch| ('\u{e000}'..='\u{f8ff}').contains(&ch)
                        || matches!(ch, '\u{17}' | '\u{18}' | '\u{1f}')),
                "carrier byte in value of {span:?} for {line:?}"
            );
        }
    }
    // The $'...' value holds the `$` and a literal backslash +
    // t, never a TAB, and the bare substitution is its own kind.
    let view = session.token_view("echo $'a\\tb'");
    assert_eq!(view.tokens[1].value, "$a\\tb");
    assert!(!view.tokens[1].value.contains('\t'));
    let view = session.token_view("echo $(foo)");
    assert_eq!(view.tokens[1].kind, K::CommandSubst);
}

#[test]
fn issue863_gap9_token_view_case_pattern_close() {
    use TokenKindView as K;
    let session = issue863_gap9_session();
    let view = session.token_view("case $x in a) echo one;; b|c) echo two;; esac");
    let closes: Vec<&str> = view
        .tokens
        .iter()
        .filter(|span| span.case_pattern_close)
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(
        closes,
        vec![")", ")"],
        "exactly the two pattern closes are stamped"
    );
    assert!(
        view.tokens
            .iter()
            .filter(|span| span.case_pattern_close)
            .all(|span| span.kind == K::Keyword),
        "pattern closes are keyword spans"
    );
    // The pattern alternation pipe is a Pipe span inside the
    // clause, and the second body still folds as a command.
    let pipe = view.tokens.iter().find(|span| span.text == "|");
    assert_eq!(pipe.map(|span| span.kind), Some(K::Pipe));
    let two = view.tokens.iter().find(|span| span.text == "two");
    assert_eq!(two.map(|span| span.command_position), Some(false));
    let echo_two = view.tokens.iter().filter(|span| span.text == "echo").nth(1);
    assert_eq!(echo_two.map(|span| span.command_position), Some(true));

    // The optional `(` opener is not a close; a subshell close
    // and a function-definition paren pair carry no stamp either.
    let view = session.token_view("case $x in (a) echo one;; esac");
    assert_eq!(
        view.tokens
            .iter()
            .filter(|span| span.case_pattern_close)
            .count(),
        1
    );
    let view = session.token_view("(echo a); foo() { x; }");
    assert_eq!(
        view.tokens
            .iter()
            .filter(|span| span.case_pattern_close)
            .count(),
        0
    );
}

#[test]
fn issue863_gap9_needs_more_input_verdict_parity() {
    let session = issue863_gap9_session();
    // The GAP-9 Q-B differential, fully adjudicated: the host
    // completeness corpus (the inputs behind niubash's
    // `repl_input_complete_*` tests and the issue482 suite shapes)
    // run through the engine face, with the host's scanner-walk
    // verdict beside it — 66 of 66 rows AGREE. The first 54 rows
    // agreed from the start. The remaining 12 (pinned separately
    // below) diverged at the first differential and were
    // classified by the captain as DIVERGE, engine reader governs
    // — the face was deficient on both classes and was fixed in
    // round 2: heredoc-pending (9 rows: the face's token walk now
    // reads the lexer's unterminated-body stamp — parse.y:3119
    // `gather_here_documents` makes an unread body part of the
    // still-open read) and the function-body stray close (3 rows:
    // the composition gained a terminal channel, so a token-walk
    // terminal verdict overrides the text half's unclosed-body
    // continuation — parse.y:6724 `yyerror` ends the read at the
    // token). Where a row's verdict is terminal for a stray close,
    // the terminal verdict itself was verified against the
    // engine's reader behavior (the issue482 suite forms), not
    // just the host walk.
    // (input, host_complete) — the face must answer
    // needs_more_input == !host_complete.
    let corpus: Vec<(&str, bool)> = vec![
        ("# 11. if comment", true),
        ("# case/esac/function() are comments", true),
        ("(echo a\n)", true),
        (")", true),
        ("a=(1 2\n)", true),
        ("case $x in", false),
        ("case $x in\n  a) echo A ;;\n  *) echo other ;;\nesac", true),
        ("case x in\na) echo hit\n)", true),
        ("cat <<'EOF'\nbody with \"quotes && (parens)\nEOF", true),
        ("cat <<-EOF\n\tindented; if x\n\tEOF", true),
        ("cat <<< \"unterminated", false),
        ("cat <<E1 <<E2\nA; if a\nB; fi b\nE1\nE2", true),
        ("cat <<EOF\nx; if y\nEOF", true),
        ("cat > out.txt <<'EOF'\nbody A\nEOF", true),
        ("cat > out.txt <<EOF\nbody A\nEOF", true),
        ("echo $((1 + 2)", false),
        ("echo $(foo bar", false),
        ("echo 'unterminated", false),
        ("echo \"<<EOF\"", true),
        ("echo \"terminated\"", true),
        ("echo \"unterminated", false),
        ("echo done", true),
        ("echo foo # if (comment)", true),
        ("echo foo#bar", true),
        ("echo hi ;;", true),
        ("echo one \\", false),
        ("echo one \\\n  two", true),
        ("echo one |", false),
        ("echo one |\n  grep one", true),
        ("f() {\ncat <<EOF\nx; if y\nEOF\n}", true),
        ("fi", true),
        ("for ((i=0; i<height; i++)); do\n  echo $i\ndone", true),
        ("for ((i=0; i<height; i++); do\n  echo $i\ndone", false),
        ("for item in a b; do", false),
        ("for item in a b; do\n  echo $item\ndone", true),
        ("hello() {", false),
        ("hello() {\n  echo hi\n}", true),
        ("hello()", false),
        ("if [ $HTTP_CODE -eq 200 ]; then echo OK; fi", true),
        ("if [ $HTTP_CODE -eq 200 ]; then\n  echo OK", false),
        ("if [ $HTTP_CODE -eq 200 ]; then\n  echo OK\nfi", true),
        ("if true; then", false),
        ("if true; then\n  # fi in comment", false),
        ("if true; then\n  # fi in comment\n  echo ok\nfi", true),
        ("if true; then\n)", true),
        ("while true; do", false),
        ("while true; do\n  break\ndone", true),
        ("while true; do\nfi", true),
        ("x=$(echo hi\n)", true),
        ("{ echo c\n)", true),
        ("{ echo c\necho a)", true),
        ("{ echo hi; }", true),
        ("{\n)", true),
        ("}", true),
    ];
    assert_eq!(corpus.len(), 54);
    for (input, host_complete) in corpus {
        assert_eq!(
            session.needs_more_input(input),
            !host_complete,
            "face verdict for {input:?}"
        );
    }
    // The round-2 adjudicated rows: (input, host_complete).
    let adjudicated: Vec<(&str, bool)> = vec![
        // Heredoc-pending: the host's heredoc arm reads these as
        // incomplete; the face answers the same from the token
        // stream's unterminated-body stamp. The `EOF trailing`
        // row pins that a delimiter word with trailing text is
        // not a delimiter line; the `<<A <<'B'` row pins that one
        // closed body does not clear the other's pending read.
        ("cat > out.txt <<'EOF'", false),
        ("cat > out.txt <<\"EOF\"", false),
        ("cat > out.txt <<EOF", false),
        ("cat > out.txt <<-EOF", false),
        ("cat <<A <<'B'\nA-body\nA", false),
        ("cat <<EOF\nnever closed", false),
        ("cat <<EOF\nbody\nEOF trailing\nmore", false),
        ("cat <<EOF\n", false),
        ("echo x; cat <<EOF", false),
        // Function-body stray close: terminal for the host walk
        // and the engine reader; the face's terminal channel
        // answers the same through the text half's unclosed-body
        // continuation verdict.
        ("f() {\n)", true),
        ("hello() {\n)", true),
        ("f() {\necho a\n)", true),
    ];
    assert_eq!(adjudicated.len(), 12);
    for (input, host_complete) in &adjudicated {
        assert_eq!(
            session.needs_more_input(input),
            !host_complete,
            "face verdict for {input:?}"
        );
    }
    // Both adjudicated classes are posix-invariant: neither the
    // heredoc stamp nor the terminal channel depends on the mode.
    let posix_session = issue863_gap9_posix_session();
    for (input, host_complete) in &adjudicated {
        assert_eq!(
            posix_session.needs_more_input(input),
            !host_complete,
            "posix face verdict for {input:?}"
        );
    }
    // The posix arm: inside "${...}", a `'` opens a quote in the
    // default mode (the face asks for more input) and is literal
    // in posix mode (complete) — the face derives in the
    // session's posix mode.
    assert!(session.needs_more_input("echo \"${x'}\""));
    let posix = issue863_gap9_posix_session();
    assert!(!posix.needs_more_input("echo \"${x'}\""));
}

#[test]
fn issue863_gap9_token_view_brace_group_descent() {
    use TokenKindView as K;
    use TokenRole as R;
    let session = issue863_gap9_session();

    // A complete brace group reaches the lexer as ONE folded
    // token; the projection descends into it (GAP-9 round 2,
    // captain-ruled): the `{` span carries the folded token's own
    // position and role, the interior is a command list folded
    // from a fresh command position, and the `}` closes.
    let view = session.token_view("{ echo body; }");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 1, K::Keyword, "{", true, R::Reserved),
            gap9_shape(2, 6, K::Word, "echo", true, R::Builtin),
            gap9_shape(7, 11, K::Word, "body", false, R::Plain),
            gap9_shape(11, 12, K::Semicolon, ";", false, R::Plain),
            gap9_shape(13, 14, K::Keyword, "}", false, R::Reserved),
        ]
    );

    // A function definition's body folds the same way; the name
    // and parens keep the outer fold's verdicts, and the body's
    // `{` is not itself a command word.
    let view = session.token_view("f() { echo body; }");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 1, K::Word, "f", true, R::Plain),
            gap9_shape(1, 2, K::Keyword, "(", false, R::Reserved),
            gap9_shape(2, 3, K::Keyword, ")", false, R::Reserved),
            gap9_shape(4, 5, K::Keyword, "{", false, R::Reserved),
            gap9_shape(6, 10, K::Word, "echo", true, R::Builtin),
            gap9_shape(11, 15, K::Word, "body", false, R::Plain),
            gap9_shape(15, 16, K::Semicolon, ";", false, R::Plain),
            gap9_shape(17, 18, K::Keyword, "}", false, R::Reserved),
        ]
    );

    // Nested groups descend recursively.
    let view = session.token_view("{ { echo a; }; }");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 1, K::Keyword, "{", true, R::Reserved),
            gap9_shape(2, 3, K::Keyword, "{", true, R::Reserved),
            gap9_shape(4, 8, K::Word, "echo", true, R::Builtin),
            gap9_shape(9, 10, K::Word, "a", false, R::Plain),
            gap9_shape(10, 11, K::Semicolon, ";", false, R::Plain),
            gap9_shape(12, 13, K::Keyword, "}", false, R::Reserved),
            gap9_shape(13, 14, K::Semicolon, ";", false, R::Plain),
            gap9_shape(15, 16, K::Keyword, "}", false, R::Reserved),
        ]
    );

    // A `case` in the interior folds through its phases exactly
    // as at top level, pattern-close stamp included.
    let view = session.token_view("{ case $x in a) b;; esac; }");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 1, K::Keyword, "{", true, R::Reserved),
            gap9_shape(2, 6, K::Keyword, "case", true, R::Reserved),
            gap9_shape(7, 9, K::Variable, "$x", false, R::Plain),
            gap9_shape(10, 12, K::Keyword, "in", false, R::Reserved),
            gap9_shape(13, 14, K::Word, "a", false, R::Plain),
            gap9_shape(14, 15, K::Keyword, ")", false, R::Plain),
            gap9_shape(16, 17, K::Word, "b", true, R::Plain),
            gap9_shape(17, 19, K::Word, ";;", false, R::Plain),
            gap9_shape(20, 24, K::Keyword, "esac", false, R::Reserved),
            gap9_shape(24, 25, K::Semicolon, ";", false, R::Plain),
            gap9_shape(26, 27, K::Keyword, "}", false, R::Reserved),
        ]
    );
    let close = view.tokens.iter().find(|span| span.text == ")");
    assert_eq!(close.map(|span| span.case_pattern_close), Some(true));

    // Command substitutions are NOT descended (captain-ruled:
    // word-shaped units; the view mirrors the token stream).
    let view = session.token_view("{ echo $(echo inner); }");
    assert_eq!(
        issue863_gap9_shape(&view),
        vec![
            gap9_shape(0, 1, K::Keyword, "{", true, R::Reserved),
            gap9_shape(2, 6, K::Word, "echo", true, R::Builtin),
            gap9_shape(7, 20, K::CommandSubst, "$(echo inner)", false, R::Plain),
            gap9_shape(20, 21, K::Semicolon, ";", false, R::Plain),
            gap9_shape(22, 23, K::Keyword, "}", false, R::Reserved),
        ]
    );

    // Honesty battery: over every group shape, spans are ordered
    // and non-overlapping, every span's text is the input slice
    // at its own offsets, and the gaps between spans are
    // whitespace only (comments are spans of their own).
    for input in [
        "{ echo body; }",
        "f() { echo body; }",
        "{ { echo a; }; }",
        "{ case $x in a) b;; esac; }",
        "{ # c\necho a\n}",
        "{ local x=1; }",
        "{ echo a; } > out",
        "if { echo a; }; then echo ok; fi",
        "{ echo a\necho b\n}",
        "{ }",
        "echo before; { echo body; }; echo after",
    ] {
        let view = session.token_view(input);
        let mut covered = 0usize;
        for span in &view.tokens {
            assert!(
                span.start >= covered,
                "spans ordered and non-overlapping in {input:?}: {span:?} after {covered}"
            );
            assert_eq!(
                &input[span.start..span.end],
                span.text,
                "span text is the input slice in {input:?}"
            );
            assert!(
                input[covered..span.start].trim().is_empty(),
                "gap before {span:?} in {input:?} is whitespace only"
            );
            covered = span.end;
        }
        assert!(
            input[covered..].trim().is_empty(),
            "tail after the last span in {input:?} is whitespace only"
        );
    }
    // The comment inside a folded group is a span of its own.
    let view = session.token_view("{ # c\necho a\n}");
    let comment = view.tokens.iter().find(|span| span.kind == K::Comment);
    assert_eq!(comment.map(|span| span.text.as_str()), Some("# c"));
}

#[test]
fn issue863_gap9_token_view_posix_mode() {
    use TokenKindView as K;
    use TokenRole as R;
    // `time` is a reserved word in the default mode and a plain
    // word in posix mode — the one tokenization delta between
    // the modes. The view derives in the session's mode: under
    // posix, `time` itself takes the command position and `ls`
    // becomes its argument.
    let plain = issue863_gap9_session();
    let view = plain.token_view("time -p ls");
    assert_eq!(view.tokens.len(), 3);
    assert_eq!(view.tokens[0].kind, K::Keyword);
    assert_eq!(view.tokens[0].role, R::Reserved);
    assert!(view.tokens[0].command_position);
    assert_eq!(view.tokens[2].text, "ls");
    assert!(
        view.tokens[2].command_position,
        "after the `time` keyword and its option, `ls` is the command"
    );

    let posix = issue863_gap9_posix_session();
    let view = posix.token_view("time -p ls");
    assert_eq!(view.tokens.len(), 3);
    assert_eq!(view.tokens[0].kind, K::Word);
    assert_eq!(view.tokens[0].role, R::Plain);
    assert!(view.tokens[0].command_position);
    assert_eq!(view.tokens[2].text, "ls");
    assert!(
        !view.tokens[2].command_position,
        "in posix mode `ls` is `time`'s argument"
    );
}

// ---------------------------------------------------------------------------
// niubash#863 GAP-10: the history-provider install is a Session
// contract face. The face is a pure lift of the executor setter:
// replace-always, unguarded; install-once stays the host's guard.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Gap10Provider {
    entries: Vec<String>,
}

impl crate::history::HistoryProvider for Gap10Provider {
    fn entries(&mut self) -> std::io::Result<Vec<String>> {
        Ok(self.entries.clone())
    }
    fn clear(&mut self) -> std::io::Result<()> {
        self.entries.clear();
        Ok(())
    }
    fn append(&mut self, command: String) -> std::io::Result<()> {
        self.entries.push(command);
        Ok(())
    }
    fn replace(&mut self, entries: Vec<String>) -> std::io::Result<()> {
        self.entries = entries;
        Ok(())
    }
    fn write_history(&mut self, path: &str) -> std::io::Result<()> {
        std::fs::write(path, self.entries.join("\n"))
    }
    fn read_history(&mut self, path: &str) -> std::io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        self.entries = text.lines().map(str::to_string).collect();
        Ok(())
    }
    fn append_history(&mut self, path: &str) -> std::io::Result<()> {
        let mut text = std::fs::read_to_string(path).unwrap_or_default();
        for entry in &self.entries {
            text.push_str(entry);
            text.push('\n');
        }
        std::fs::write(path, text)
    }
    fn read_new_history(&mut self, path: &str) -> std::io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        for line in text.lines() {
            if !self.entries.iter().any(|entry| entry == line) {
                self.entries.push(line.to_string());
            }
        }
        Ok(())
    }
}

fn issue863_gap10_provider(entries: &[&str]) -> crate::history::SharedHistoryProvider {
    Rc::new(RefCell::new(Gap10Provider {
        entries: entries.iter().map(|entry| entry.to_string()).collect(),
    }))
}

fn issue863_gap10_run(session: &mut Session, source: &str) {
    let outcome = session.run(source, &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0, "run of {source:?}");
}

#[test]
fn issue863_gap10_install_via_session_face_reaches_history_faces() {
    use super::session::PreparedLine;
    let dir = temp_dir("gap10-install");
    let mut session = issue863_gap8_session();
    // Install AFTER build, through the Session face alone (no
    // builder injection, no executor seam).
    session.set_history_provider(issue863_gap10_provider(&["echo alpha", "echo beta"]));

    // History expansion resolves against the provider's list.
    issue863_gap10_run(&mut session, "set -H");
    assert_eq!(
        session.prepare_line("!!"),
        PreparedLine::Execute("echo beta".to_string())
    );

    // The history builtin reads the provider's list (provider
    // precedence over the engine's own session list).
    let out = dir.join("hist.txt");
    issue863_gap10_run(
        &mut session,
        &format!("history > \"{}\"", out.to_string_lossy()),
    );
    let text = read(&dir, "hist.txt");
    assert!(
        text.contains("echo alpha"),
        "provider entry listed: {text:?}"
    );
    assert!(
        text.contains("echo beta"),
        "provider entry listed: {text:?}"
    );
}

#[test]
fn issue863_gap10_second_install_replaces_first() {
    let dir = temp_dir("gap10-replace");
    let mut session = issue863_gap8_session();
    session.set_history_provider(issue863_gap10_provider(&["echo first-provider"]));
    // Replace-always: the face carries no install-once guard;
    // the second install wins outright.
    session.set_history_provider(issue863_gap10_provider(&["echo second-provider"]));
    let out = dir.join("hist.txt");
    issue863_gap10_run(
        &mut session,
        &format!("history > \"{}\"", out.to_string_lossy()),
    );
    let text = read(&dir, "hist.txt");
    assert!(
        text.contains("echo second-provider"),
        "second install is the list: {text:?}"
    );
    assert!(
        !text.contains("echo first-provider"),
        "first install fully replaced: {text:?}"
    );
}

#[test]
fn issue863_gap10_no_provider_default_unchanged() {
    let dir = temp_dir("gap10-default");
    let mut session = issue863_gap8_session();
    // No install: the engine's own session list is the plane,
    // exactly as before the face existed.
    issue863_gap10_run(&mut session, "set -o history");
    issue863_gap10_run(&mut session, "history -s \"echo gap10-engine-list\"");
    let out = dir.join("hist.txt");
    issue863_gap10_run(
        &mut session,
        &format!("history > \"{}\"", out.to_string_lossy()),
    );
    let text = read(&dir, "hist.txt");
    assert!(
        text.contains("echo gap10-engine-list"),
        "engine list serves the builtin: {text:?}"
    );
    assert_eq!(
        session.history_entries().map(|entries| entries.concat()),
        Some("echo gap10-engine-list".to_string())
    );
}

// ---------------------------------------------------------------------------
// niubash#863 GAP-11: `Candidate.kind` — the engine marks each
// candidate at generation from evidence already in hand (the
// dirent's class, the followed stat a directory action already
// performs, the PATH walk's executability verdict); the host
// consumes the mark for painting and the append decision instead
// of re-statting engine output or guessing from a trailing `/`
// the engine never appends. GNU anchors: pcomplete.c flattens
// every generator into a STRINGLIST (GNU carries no kind — this
// is a contract design decision); lib/readline/complete.c:1087
// `path_isdir` follows links at display, which is what the
// generator-classification principle mirrors at generation.
// ---------------------------------------------------------------------------

use super::capabilities::{Candidate, CandidateKind, CompletionOutcome, CompletionProvider};

/// Fixture directory for the kind pins: a real directory, a
/// plain file, an executable file, and (unix only) a symlink to
/// the directory and a symlink to the file.
fn issue863_gap11_fixture(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::create_dir_all(dir.join("realdir")).expect("fixture realdir");
    std::fs::write(dir.join("plain.txt"), "x\n").expect("fixture file");
    std::fs::write(dir.join("run.sh"), "#!/bin/sh\n").expect("fixture exec file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("run.sh"), std::fs::Permissions::from_mode(0o755))
            .expect("fixture exec bit");
        std::os::unix::fs::symlink("realdir", dir.join("linkdir")).expect("symlink linkdir");
        std::os::unix::fs::symlink("plain.txt", dir.join("linkfile")).expect("symlink linkfile");
    }
    dir
}

fn issue863_gap11_session(dir: &PathBuf) -> Session {
    // The kind pins complete absolute fixture words. The
    // default COMP_WORDBREAKS splits at `:` (GNU bashline.c:313)
    // — on Windows that strips the drive letter from a `C:/...`
    // word, leaving a root-relative `/Users/...` that never
    // resolves to the fixture, while compgen (explicit word, no
    // split) completes the same word fine. Wordbreaks are not
    // what these pins test (issue2578 owns them), so the pins
    // bind the default set minus `:` and isolate kind marking.
    // The drive-letter completion shape itself is flagged for
    // triage in REPORT.md, not fixed here.
    Session::builder()
        .working_directory(dir)
        .env("COMP_WORDBREAKS", " \t\n\"'@><=;|&(")
        .build()
        .expect("session builds")
}

fn issue863_gap11_kinds(outcome: &CompletionOutcome) -> HashMap<String, Option<CandidateKind>> {
    outcome
        .candidates
        .iter()
        .map(|candidate| (issue863_gap11_slash(&candidate.text), candidate.kind))
        .collect()
}

/// An absolute completion word under the fixture dir: relative
/// words resolve against the PROCESS cwd in embedded sessions
/// (the session's logical cwd moves PWD, not the process — the
/// shell binary chdirs the process, a library session does
/// not), so the kind pins complete absolute words to reach the
/// fixture. `typed` is the partial name after the fixture
/// prefix (empty for the whole listing). The dir is spelled
/// with forward slashes: a backslash word is split at every
/// `\` before it reaches the filename generator on Windows,
/// while the path layer accepts `/` on all platforms.
/// The fixture dir spelled with forward slashes. Candidate
/// text is separator-sensitive on Windows (the engine's display
/// prefix keeps the word's separators, and the path layer may
/// canonicalize to `\`), and a backslash word is split at every
/// `\` before it reaches the filename generator — so every word
/// and every expected key is built from this spelling, and
/// `issue863_gap11_kinds` normalizes engine texts the same way
/// before lookup. Linux is unaffected (no backslashes occur).
fn issue863_gap11_slash_dir(dir: &PathBuf) -> String {
    dir.display().to_string().replace('\\', "/")
}

/// Normalize one candidate text for lookup: `\` -> `/`.
fn issue863_gap11_slash(text: &str) -> String {
    text.replace('\\', "/")
}

fn issue863_gap11_word(dir: &PathBuf, typed: &str) -> String {
    format!("{}/{typed}", issue863_gap11_slash_dir(dir))
}

fn issue863_gap11_complete(
    session: &mut Session,
    cmd: &str,
    dir: &PathBuf,
    typed: &str,
) -> CompletionOutcome {
    let line = format!("{cmd} {}", issue863_gap11_word(dir, typed));
    let cursor = line.len();
    session.complete_detailed(&line, cursor)
}

fn issue863_gap11_texts(outcome: &CompletionOutcome) -> Vec<String> {
    outcome
        .candidates
        .iter()
        .map(|candidate| candidate.text.clone())
        .collect()
}

/// `-A file` marks each entry by its own class: a real
/// directory is Directory, a regular file is File, a symlink
/// (to a file, a directory, or nothing) is Symlink — the
/// producing action did not follow links, so the entry's own
/// class is the honest mark. The plain `Session::complete`
/// face carries the same kinds as the detailed one.
#[test]
fn issue863_gap11_file_action_marks_per_entry() {
    let dir = issue863_gap11_fixture("gap11-file");
    let mut session = issue863_gap11_session(&dir);
    issue863_gap6_run(&mut session, "complete -A file mycmd");
    let kinds = issue863_gap11_kinds(&issue863_gap11_complete(&mut session, "mycmd", &dir, ""));
    let slash = issue863_gap11_slash_dir(&dir);
    let plain = format!("{slash}/plain.txt");
    let run = format!("{slash}/run.sh");
    let real = format!("{slash}/realdir");
    assert_eq!(kinds.get(&plain), Some(&Some(CandidateKind::File)));
    assert_eq!(kinds.get(&run), Some(&Some(CandidateKind::File)));
    assert_eq!(kinds.get(&real), Some(&Some(CandidateKind::Directory)));
    #[cfg(unix)]
    {
        let linkdir = format!("{slash}/linkdir");
        let linkfile = format!("{slash}/linkfile");
        assert_eq!(kinds.get(&linkdir), Some(&Some(CandidateKind::Symlink)));
        assert_eq!(kinds.get(&linkfile), Some(&Some(CandidateKind::Symlink)));
    }
    // The plain face rides the same candidates.
    let line = format!("mycmd {}", issue863_gap11_word(&dir, ""));
    let plain_face = session.complete(&line, line.len());
    let plain_txt = plain_face
        .iter()
        .find(|candidate| issue863_gap11_slash(&candidate.text) == plain)
        .expect("plain.txt on the plain face");
    assert_eq!(plain_txt.kind, Some(CandidateKind::File));
}

/// `-A directory` marks every entry Directory — including a
/// symlink to a directory, because the directory action
/// follows links (the generator-classification principle; the
/// #2822 generation face). This is the engine half of the
/// directory-append acceptance face: with the mark present the
/// host can suppress the appended space for an engine-produced
/// directory instead of guessing from a trailing `/` the engine
/// never adds (the host-side append pin lands with the host
/// lane).
#[test]
fn issue863_gap11_directory_action_marks_symlink_to_dir_as_directory() {
    let dir = issue863_gap11_fixture("gap11-dir");
    let mut session = issue863_gap11_session(&dir);
    issue863_gap6_run(&mut session, "complete -A directory mycmd");
    let outcome = issue863_gap11_complete(&mut session, "mycmd", &dir, "");
    let kinds = issue863_gap11_kinds(&outcome);
    let real = format!("{}/realdir", issue863_gap11_slash_dir(&dir));
    assert_eq!(kinds.get(&real), Some(&Some(CandidateKind::Directory)));
    #[cfg(unix)]
    {
        let linkdir = format!("{}/linkdir", issue863_gap11_slash_dir(&dir));
        assert_eq!(kinds.get(&linkdir), Some(&Some(CandidateKind::Directory)));
    }
    assert!(
        outcome
            .candidates
            .iter()
            .all(|c| c.kind == Some(CandidateKind::Directory)),
        "a directory action's every entry is a directory: {:?}",
        outcome.candidates
    );
}

/// `-A command` marks PATH-walk entries Executable — each one
/// already passed the walk's executability verdict — while
/// names from the other command sources (aliases, keywords,
/// functions, builtins) stay unmarked: the taxonomy is
/// filesystem-only and no origin class is fabricated for them.
#[test]
fn issue863_gap11_command_action_marks_path_executables() {
    let dir = issue863_gap11_fixture("gap11-command");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("fixture bin dir");
    std::fs::write(bin.join("gap11tool"), "#!/bin/sh\n").expect("fixture tool");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            bin.join("gap11tool"),
            std::fs::Permissions::from_mode(0o755),
        )
        .expect("fixture tool exec bit");
    }
    let separator = if cfg!(windows) { ";" } else { ":" };
    let path = format!(
        "{}{}{}",
        bin.display(),
        separator,
        std::env::var("PATH").unwrap_or_default()
    );
    let mut session = Session::builder()
        .working_directory(&dir)
        .env("PATH", path)
        .build()
        .expect("session builds");
    issue863_gap6_run(&mut session, "complete -A command mycmd");
    let kinds = issue863_gap11_kinds(&session.complete_detailed("mycmd gap11t", 12));
    assert_eq!(
        kinds.get("gap11tool"),
        Some(&Some(CandidateKind::Executable)),
        "a PATH entry that qualified as executable is Executable"
    );
    let builtin_kinds = issue863_gap11_kinds(&session.complete_detailed("mycmd ec", 10));
    assert_eq!(
        builtin_kinds.get("echo"),
        Some(&None),
        "a builtin command name carries no filesystem class"
    );
}

/// Dynamic candidates are user text the engine did not
/// classify: `-F` COMPREPLY entries ride unmarked, and the
/// filenames-intent policy bit — not a fabricated per-candidate
/// kind — is the only filename signal for them. With `-o
/// filenames` the bit is set; without it, it is not.
#[test]
fn issue863_gap11_dynamic_candidates_stay_unmarked() {
    let dir = issue863_gap11_fixture("gap11-dynamic");
    let mut session = issue863_gap11_session(&dir);
    issue863_gap6_run(
        &mut session,
        "_f() { COMPREPLY=(dyn-one dyn-two); }; complete -F _f mycmd",
    );
    let outcome = session.complete_detailed("mycmd dyn", 9);
    assert_eq!(
        issue863_gap11_texts(&outcome),
        vec!["dyn-one".to_string(), "dyn-two".to_string()]
    );
    assert!(
        outcome.candidates.iter().all(|c| c.kind.is_none()),
        "dynamic candidates are never classified: {:?}",
        outcome.candidates
    );
    assert!(
        !outcome.policy.filenames_intent,
        "a bare -F spec is not filename-flavored"
    );
    issue863_gap6_run(&mut session, "complete -o filenames -F _f mycmd");
    let outcome = session.complete_detailed("mycmd dyn", 9);
    assert!(
        outcome.candidates.iter().all(|c| c.kind.is_none()),
        "even under -o filenames, dynamic candidates stay unmarked: {:?}",
        outcome.candidates
    );
    assert!(
        outcome.policy.filenames_intent,
        "-o filenames sets the filenames-intent bit"
    );
}

/// The filenames-intent bit is a spec-intent fact about the
/// attempt's generation: set when the spec's actions include
/// file/directory production, unset for a non-filesystem
/// action, set for the no-spec filename fallback at argument
/// position, unset for the no-spec command fallback.
#[test]
fn issue863_gap11_filenames_intent_arms() {
    let dir = issue863_gap11_fixture("gap11-intent");
    let mut session = issue863_gap11_session(&dir);
    issue863_gap6_run(&mut session, "complete -A variable mycmd");
    let outcome = session.complete_detailed("mycmd ", 6);
    assert!(
        !outcome.policy.filenames_intent,
        "a variable-only spec is not filename-flavored"
    );
    assert!(
        outcome.candidates.iter().all(|c| c.kind.is_none()),
        "non-filesystem candidates stay unmarked: {:?}",
        outcome.candidates
    );
    issue863_gap6_run(&mut session, "complete -A file mycmd");
    let outcome = session.complete_detailed("mycmd ", 6);
    assert!(
        outcome.policy.filenames_intent,
        "a file-action spec is filename-flavored"
    );
    // No compspec governs `echo`: argument position falls back
    // to filename generation, command position to command
    // generation.
    let outcome = session.complete_detailed("echo pl", 7);
    assert!(
        outcome.policy.filenames_intent,
        "the no-spec argument-position fallback is filename generation"
    );
    let outcome = session.complete_detailed("ec", 2);
    assert!(
        !outcome.policy.filenames_intent,
        "the no-spec command-position fallback is not filename generation"
    );
}

/// Provider merge: dedup-by-text keeps the ENGINE's candidate
/// and its mark over a provider's same-text one, and a provider
/// candidate keeps whatever kind the provider set — the engine
/// never re-marks provider output.
struct Gap11Provider {
    candidates: Vec<Candidate>,
}

impl CompletionProvider for Gap11Provider {
    fn candidates(&mut self, _line: &str, _cursor: usize) -> Vec<Candidate> {
        self.candidates.clone()
    }
}

#[test]
fn issue863_gap11_provider_merge_keeps_engine_mark_and_provider_kind() {
    let dir = issue863_gap11_fixture("gap11-provider");
    // The provider must return the engine candidate's EXACT
    // text for dedup-by-text to engage: on Windows the engine
    // text may spell separators differently from the test's
    // forward-slash spelling, so learn it from a probe
    // completion first (same fixture dir, same spec).
    let mut probe = issue863_gap11_session(&dir);
    issue863_gap6_run(&mut probe, "complete -A file mycmd");
    let engine_plain =
        issue863_gap11_texts(&issue863_gap11_complete(&mut probe, "mycmd", &dir, ""))
            .into_iter()
            .find(|text| issue863_gap11_slash(&text).ends_with("/plain.txt"))
            .expect("engine produced plain.txt");
    let provider = Gap11Provider {
        candidates: vec![
            // Same text as an engine file candidate, with a
            // deliberately wrong kind: the engine mark must win.
            Candidate::kinded(engine_plain, CandidateKind::Directory),
            Candidate::kinded("provider-only", CandidateKind::Symlink),
        ],
    };
    let mut session = Session::builder()
        .working_directory(&dir)
        .env("COMP_WORDBREAKS", " \t\n\"'@><=;|&(")
        .completion_provider(Box::new(provider))
        .build()
        .expect("session builds");
    issue863_gap6_run(&mut session, "complete -A file mycmd");
    let kinds = issue863_gap11_kinds(&issue863_gap11_complete(&mut session, "mycmd", &dir, ""));
    let plain = format!("{}/plain.txt", issue863_gap11_slash_dir(&dir));
    assert_eq!(
        kinds.get(&plain),
        Some(&Some(CandidateKind::File)),
        "dedup keeps the engine candidate and its mark"
    );
    assert_eq!(
        kinds.get("provider-only"),
        Some(&Some(CandidateKind::Symlink)),
        "a provider candidate keeps the kind the provider set"
    );
}

/// compgen's printed output stays byte-identical to the
/// detailed attempt's texts: kind never leaks into text. The
/// two paths print in different orders (compgen keeps
/// generation order, the attempt sorts), so the golden compares
/// sorted multisets.
#[test]
fn issue863_gap11_compgen_text_stays_byte_identical() {
    let dir = issue863_gap11_fixture("gap11-compgen");
    let mut session = issue863_gap11_session(&dir);
    issue863_gap6_run(&mut session, "complete -A file mycmd");
    let mut detailed =
        issue863_gap11_texts(&issue863_gap11_complete(&mut session, "mycmd", &dir, "pl"));
    detailed.sort();
    issue863_gap6_run(
        &mut session,
        &format!(
            "OUT=$(compgen -A file -- \"{}\")",
            issue863_gap11_word(&dir, "pl")
        ),
    );
    let out = session
        .variable_value("OUT")
        .expect("compgen capture persisted");
    let mut printed: Vec<String> = out.lines().map(str::to_string).collect();
    printed.sort();
    assert_eq!(
        printed, detailed,
        "compgen prints exactly the detailed attempt's texts"
    );
}

/// The U-5 fallback arms mark through the same generators:
/// `-o dirnames` replaces an empty result with directory
/// candidates, `-o plusdirs` appends them beside the spec's
/// own, and `-o default` materializes filename candidates —
/// each marked, and each attempt filename-flavored.
#[test]
fn issue863_gap11_fallback_arms_mark_directories() {
    let dir = issue863_gap11_fixture("gap11-fallback");
    let mut session = issue863_gap11_session(&dir);
    let slash = issue863_gap11_slash_dir(&dir);
    let real = format!("{slash}/realdir");
    let plain = format!("{slash}/plain.txt");
    // plusdirs: the wordlist candidate does not extend the
    // absolute word, so it is prefix-filtered out, and the
    // directory candidates append beside the empty result. The
    // fallback sets `filename_sourced`, so the trailing-slash
    // block marks the directory texts with `/` (the insert
    // layer's mark — the kind stays Directory).
    issue863_gap6_run(&mut session, "complete -W 'zzz' -o plusdirs mycmd");
    let outcome = issue863_gap11_complete(&mut session, "mycmd", &dir, "");
    let kinds = issue863_gap11_kinds(&outcome);
    assert_eq!(
        kinds.get(&format!("{real}/")),
        Some(&Some(CandidateKind::Directory)),
        "plusdirs appends marked directory candidates"
    );
    assert!(outcome.policy.filenames_intent);
    // dirnames: the wordlist candidate is prefix-filtered out,
    // so the directory set replaces the empty result.
    issue863_gap6_run(&mut session, "complete -W 'zzz' -o dirnames mycmd2");
    let outcome = issue863_gap11_complete(&mut session, "mycmd2", &dir, "r");
    let kinds = issue863_gap11_kinds(&outcome);
    assert_eq!(
        kinds.get(&format!("{real}/")),
        Some(&Some(CandidateKind::Directory)),
        "dirnames replaces an empty result with marked directories"
    );
    assert!(outcome.policy.filenames_intent);
    // default: the wordlist candidate is prefix-filtered out,
    // so the filename set materializes.
    issue863_gap6_run(&mut session, "complete -W 'zzz' -o default mycmd3");
    let outcome = issue863_gap11_complete(&mut session, "mycmd3", &dir, "p");
    let kinds = issue863_gap11_kinds(&outcome);
    assert_eq!(
        kinds.get(&plain),
        Some(&Some(CandidateKind::File)),
        "default materializes marked filename candidates"
    );
    assert_eq!(
        kinds.get(&real),
        None,
        "default filename candidates are prefix-filtered like the rest"
    );
    assert!(outcome.policy.filenames_intent);
}

// ---------------------------------------------------------------
// niubash#863 GAP-13: the bind/readline assembly faces. The
// registry is the `bind` builtin's data plane and the host
// mirrors it through Session::bind_view (GAP-3), but the two
// assembly verbs — registering a binding and triggering the
// once-per-process inputrc read — existed only as tier-C
// executor seam calls, and the host built the engine's internal
// BindEntry by hand to register anything. These pins drive the
// registry through the Session faces only and read it back
// through Session::bind_view / the `bind` builtin's own dumps.
// ---------------------------------------------------------------

fn issue863_gap13_session() -> Session {
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

fn issue863_gap13_session_with_inputrc(path: &std::path::Path) -> Session {
    Session::builder()
        .env("INPUTRC", path.to_string_lossy())
        .build()
        .expect("session builds")
}

#[test]
fn issue863_gap13_register_binding_reads_back_through_bind_view() {
    let mut session = issue863_gap13_session();
    let g0 = session.bind_view().generation;

    // None keymap resolves to the registry's current keymap at
    // registration time; the three kinds round-trip verbatim.
    session.register_binding(
        r"\C-a",
        BindKindView::Function {
            name: "beginning-of-line".to_string(),
        },
        None,
    );
    let view = session.bind_view();
    assert_eq!(view.generation, g0 + 1);
    let entry = issue863_gap3_entry(&view, r"\C-a");
    assert_eq!(entry.keymap, "emacs-standard");
    assert_eq!(
        entry.kind,
        BindKindView::Function {
            name: "beginning-of-line".to_string()
        }
    );
    assert_eq!(entry.dump, r#""\C-a": beginning-of-line"#);

    session.register_binding(
        r"\C-t",
        BindKindView::Macro {
            text: "ins-macro".to_string(),
        },
        Some("vi-insert"),
    );
    let view = session.bind_view();
    assert_eq!(view.generation, g0 + 2);
    let entry = issue863_gap3_entry(&view, r"\C-t");
    assert_eq!(entry.keymap, "vi-insert");
    assert_eq!(
        entry.kind,
        BindKindView::Macro {
            text: "ins-macro".to_string()
        }
    );
    assert_eq!(entry.dump, r#""\C-t": "ins-macro""#);

    session.register_binding(
        r"\C-g",
        BindKindView::Execute {
            command: "echo bound-c-g".to_string(),
        },
        None,
    );
    let view = session.bind_view();
    assert_eq!(view.generation, g0 + 3);
    let entry = issue863_gap3_entry(&view, r"\C-g");
    assert_eq!(entry.keymap, "emacs-standard");
    assert_eq!(
        entry.kind,
        BindKindView::Execute {
            command: "echo bound-c-g".to_string()
        }
    );
    assert_eq!(entry.dump, r#""\C-g" "echo bound-c-g""#);

    // The view normalizes the keymap alias: "vi" folds to
    // "vi-movement" (normalize_keymap_name, bind.def:37-40).
    session.register_binding(
        r"\C-b",
        BindKindView::Function {
            name: "backward-char".to_string(),
        },
        Some("vi"),
    );
    let view = session.bind_view();
    assert_eq!(issue863_gap3_entry(&view, r"\C-b").keymap, "vi-movement");
}

#[test]
fn issue863_gap13_reregister_replaces_in_place_and_bumps_generation_once() {
    let mut session = issue863_gap13_session();
    session.register_binding(
        r"\C-a",
        BindKindView::Function {
            name: "beginning-of-line".to_string(),
        },
        None,
    );
    let before = session.bind_view();
    assert_eq!(
        before
            .entries
            .iter()
            .filter(|entry| entry.keyseq == r"\C-a")
            .count(),
        1
    );

    // Same keyseq, same keymap, different function: replace in
    // place (no duplicate) and bump the generation by exactly one.
    session.register_binding(
        r"\C-a",
        BindKindView::Function {
            name: "backward-char".to_string(),
        },
        None,
    );
    let view = session.bind_view();
    assert_eq!(view.generation, before.generation + 1);
    let matches: Vec<_> = view
        .entries
        .iter()
        .filter(|entry| entry.keyseq == r"\C-a")
        .collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].kind,
        BindKindView::Function {
            name: "backward-char".to_string()
        }
    );
    assert_eq!(matches[0].dump, r#""\C-a": backward-char"#);
}

#[test]
fn issue863_gap13_register_clears_bind_r_tombstone() {
    let mut session = issue863_gap13_session();
    session.register_binding(
        r"\C-a",
        BindKindView::Function {
            name: "beginning-of-line".to_string(),
        },
        None,
    );
    issue863_gap3_run(&mut session, r"bind -r '\C-a'");
    let view = session.bind_view();
    assert!(view.entries.iter().all(|entry| entry.keyseq != r"\C-a"));
    assert!(view
        .removals
        .iter()
        .any(|removal| removal.keyseq == r"\C-a"));

    // Registering the sequence again through the face clears its
    // tombstone — the same sink the `bind` builtin writes.
    session.register_binding(
        r"\C-a",
        BindKindView::Function {
            name: "beginning-of-line".to_string(),
        },
        None,
    );
    let view = session.bind_view();
    assert!(view
        .removals
        .iter()
        .all(|removal| removal.keyseq != r"\C-a"));
    assert!(view.entries.iter().any(|entry| entry.keyseq == r"\C-a"));
}

#[test]
fn issue863_gap13_face_writes_visible_to_bind_builtin_dump() {
    let dir = temp_dir("gap13-dump");
    let dir_text = dir.to_string_lossy().into_owned();
    let mut session = issue863_gap13_session();
    session.variables().set("OUT", &dir_text);
    session.register_binding(
        r"\C-a",
        BindKindView::Function {
            name: "beginning-of-line".to_string(),
        },
        None,
    );
    session.register_binding(
        r"\C-t",
        BindKindView::Macro {
            text: "ins-macro".to_string(),
        },
        None,
    );
    session.register_binding(
        r"\C-g",
        BindKindView::Execute {
            command: "echo bound-c-g".to_string(),
        },
        None,
    );
    // One pool, two doors: the face's writes show up in the
    // `bind` builtin's own dumps.
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
fn issue863_gap13_initialize_readline_loads_inputrc_and_second_call_is_noop() {
    let dir = temp_dir("gap13-initrc");
    let rc_path = dir.join(".inputrc");
    std::fs::write(
        &rc_path,
        "\"\\C-t\": \"ins-macro\"\nset editing-mode vi\nset keyseq-timeout 250\n",
    )
    .expect("inputrc written");
    let mut session = issue863_gap13_session_with_inputrc(&rc_path);
    session.initialize_readline();
    let view = session.bind_view();
    assert_eq!(view.editing_mode.as_deref(), Some("vi"));
    assert_eq!(issue863_gap3_variable(&view, "keyseq-timeout"), Some("250"));
    let entry = issue863_gap3_entry(&view, r"\C-t");
    assert_eq!(
        entry.kind,
        BindKindView::Macro {
            text: "ins-macro".to_string()
        }
    );

    // The file changes between calls — the second call must not
    // re-read it (re-reading is the separate `bind -f` route).
    std::fs::write(
        &rc_path,
        "\"\\C-y\": \"second-macro\"\nset editing-mode emacs\nset keyseq-timeout 300\n",
    )
    .expect("inputrc rewritten");
    session.initialize_readline();
    let view = session.bind_view();
    assert_eq!(view.editing_mode.as_deref(), Some("vi"));
    assert_eq!(issue863_gap3_variable(&view, "keyseq-timeout"), Some("250"));
    assert!(view.entries.iter().all(|entry| entry.keyseq != r"\C-y"));
}

#[test]
fn issue863_gap13_initialize_flag_poisoned_when_home_empty_first() {
    // U-1 through the Session face: the first call with HOME
    // empty sets the engine flag and reads nothing; a later call
    // with HOME set must not re-read the file. GNU poisons the
    // same way (bashline.c:451-456 early return on the flag, :646
    // flag set unconditionally after rl_initialize). The session
    // is built plain — NOT with INPUTRC=/dev/null, which would
    // take precedence over the HOME path and mask the probe.
    let dir = temp_dir("gap13-u1");
    std::fs::write(dir.join(".inputrc"), "set editing-mode vi\n").expect("inputrc written");
    let dir_text = dir.to_string_lossy().into_owned();

    // Control: HOME set from the start — the file IS read.
    let mut control = Session::builder().build().expect("session builds");
    control.variables().unset("INPUTRC");
    control.variables().set("HOME", &dir_text);
    control.initialize_readline();
    assert_eq!(
        control.bind_view().editing_mode.as_deref(),
        Some("vi"),
        "control: inputrc read when HOME is set before the first call"
    );

    let mut session = Session::builder().build().expect("session builds");
    session.variables().unset("INPUTRC");
    session.variables().set("HOME", "");
    session.initialize_readline();
    assert_eq!(
        session.bind_view().editing_mode.as_deref(),
        None,
        "first call with HOME empty reads nothing"
    );
    session.variables().set("HOME", &dir_text);
    session.initialize_readline();
    assert_eq!(
        session.bind_view().editing_mode.as_deref(),
        None,
        "U-1: second call after HOME is set must NOT re-read the inputrc"
    );
}

#[test]
fn issue863_gap13_is_known_keymap() {
    let session = issue863_gap13_session();
    for name in [
        "emacs",
        "emacs-standard",
        "emacs-meta",
        "emacs-ctlx",
        "vi",
        "vi-move",
        "vi-command",
        "vi-insert",
    ] {
        assert!(
            session.is_known_keymap(name),
            "{name} is one of the eight GNU keymap names (bind.def:37-40)"
        );
    }
    assert!(!session.is_known_keymap("bogus"));
    assert!(!session.is_known_keymap(""));
    assert!(!session.is_known_keymap("VI"));
}

#[test]
fn issue863_gap13_applied_invocation_shopt_flag_oracle_arms() {
    // rubash#2848: the retained invocation's explicit shopt flag
    // — the read `enter_interactive` needs for `expand_aliases`
    // (last entry wins, default true). GNU oracle arms, as the
    // face's Option values (the host maps None to the interactive
    // default, ON): silent + `-i` -> None; `+O expand_aliases
    // -i` -> Some(false), OFF wins; `-O expand_aliases -i` ->
    // Some(true), ON.
    let mut silent = Session::builder().build().expect("session builds");
    silent
        .apply_invocation_before_startup(&gap12_parse(&["-i"]))
        .expect("apply succeeds");
    assert_eq!(
        silent.applied_invocation_shopt_flag("expand_aliases"),
        None,
        "silent invocation says nothing about expand_aliases"
    );

    let mut off = Session::builder().build().expect("session builds");
    off.apply_invocation_before_startup(&gap12_parse(&["+O", "expand_aliases", "-i"]))
        .expect("apply succeeds");
    assert_eq!(
        off.applied_invocation_shopt_flag("expand_aliases"),
        Some(false),
        "+O expand_aliases -i: the explicit OFF wins over the interactive default"
    );

    let mut on = Session::builder().build().expect("session builds");
    on.apply_invocation_before_startup(&gap12_parse(&["-O", "expand_aliases", "-i"]))
        .expect("apply succeeds");
    assert_eq!(
        on.applied_invocation_shopt_flag("expand_aliases"),
        Some(true),
        "-O expand_aliases -i: explicitly ON"
    );

    // No invocation applied at all: None, like silence.
    let plain = Session::builder().build().expect("session builds");
    assert_eq!(plain.applied_invocation_shopt_flag("expand_aliases"), None);
    // An unrelated flag does not leak into the answer.
    assert_eq!(
        silent.applied_invocation_shopt_flag("extglob"),
        None,
        "silent invocation says nothing about extglob either"
    );
    let mut other = Session::builder().build().expect("session builds");
    other
        .apply_invocation_before_startup(&gap12_parse(&["-O", "extglob"]))
        .expect("apply succeeds");
    assert_eq!(other.applied_invocation_shopt_flag("extglob"), Some(true));
    assert_eq!(other.applied_invocation_shopt_flag("expand_aliases"), None);
}

#[test]
fn issue863_gap13_applied_invocation_shopt_flag_last_entry_wins() {
    // The argv walk records every -O/+O pair in order; the face
    // answers from the last entry for the name (shell.c:931-958).
    let mut off = Session::builder().build().expect("session builds");
    off.apply_invocation_before_startup(&gap12_parse(&[
        "-O",
        "expand_aliases",
        "+O",
        "expand_aliases",
    ]))
    .expect("apply succeeds");
    assert_eq!(
        off.applied_invocation_shopt_flag("expand_aliases"),
        Some(false)
    );
    let mut on = Session::builder().build().expect("session builds");
    on.apply_invocation_before_startup(&gap12_parse(&[
        "+O",
        "expand_aliases",
        "-O",
        "expand_aliases",
    ]))
    .expect("apply succeeds");
    assert_eq!(
        on.applied_invocation_shopt_flag("expand_aliases"),
        Some(true)
    );
}
