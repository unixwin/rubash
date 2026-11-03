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
    // The published Flow fold is unchanged: the errexit break
    // still resolves to Completed here.
    assert_eq!(outcome.flow, Flow::Completed);
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
    // Flow keeps its known fold (a nonzero exit that satisfies
    // the errexit predicate resolves to the errexit arm)…
    assert_eq!(outcome.flow, Flow::Completed);
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
    assert_eq!(outcome.flow, Flow::Completed);
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
    // route. Its drivers compute no errexit verdict — the
    // classification reports the route's own adjudication
    // (the same reads Flow is folded from).
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
