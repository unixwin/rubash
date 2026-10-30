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
