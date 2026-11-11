//! rubash#2824 — in script mode the recorded source-line set must match
//! GNU: a `history -s` inside an EXIT trap APPENDS its argument; it must
//! not pop the last recorded source line.
//!
//! GNU: recording happens at READ time (parse.y:2661 pre_process_line ->
//! bashhist.c:760 maybe_add_history), and the `history -s` pop
//! (builtins/history.def:435-439 push_history) is gated on
//! `hist_last_line_added` — "the line the reader most recently processed
//! was added". maybe_add_history clears that flag on entry for EVERY
//! line the reader processes (bashhist.c:764), including the trap
//! action string parsed at exit, so by the time the trap's `-s` runs
//! the flag is clear and nothing is popped. The trap DEFINITION line,
//! read and recorded like any other line, stays in the saved file.
//!
//! The defect mechanism this pins: the pop's scope is the line the `-s`
//! sits on, not "whatever source line was recorded last". Cell 2 moves
//! the last recorded line past the trap definition — GNU keeps every
//! line; a last-line pop eats `marker=after` instead. Cell 3 defines
//! the trap BEFORE history is enabled: the definition line is not
//! recorded (it was read before enabling — the read-time gate), and
//! the trap's `-s` must not eat the later `echo body-cmd` either.
//! Cell 4 is the regression lock for the LEGITIMATE pop: a `-s` on a
//! recorded line replaces that line's own entry with its argument.
//!
//! The `-p` arm is the asymmetric twin: its delete at the head of
//! expand_and_print_history (history.def:462) is conditioned on
//! `hist_last_line_pushed == 0 && hist_last_line_added` alone — NO
//! remember_on_history gate — so a `-p` inside an eval unit or a trap
//! action still pops the enclosing recorded line. The eval-`-p` /
//! eval-`-s` cells at the bottom pin that surface on the in-session
//! listing; the EXIT-trap-`-p` cell pins it on the saved file.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_case(tag: &str, body_template: &str) -> (PathBuf, Option<String>) {
    let dir = std::env::temp_dir().join(format!("rubash-2824-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let dir_fwd = dir.to_string_lossy().replace('\\', "/");
    let body = body_template.replace("HISTDIR", &dir_fwd);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    assert_eq!(
        output.status.code(),
        Some(0),
        "script must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let hist = dir.join("hist");
    let saved = std::fs::read_to_string(&hist).ok();
    (dir, saved)
}

fn cleanup(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn exit_trap_history_s_appends_without_eating_the_trap_line() {
    // The issue's regression seed, verbatim (HISTDIR = the temp dir).
    let (dir, saved) = run_case(
        "seed",
        "set -o history\nHISTFILE=\"HISTDIR/hist\"\nHISTFILESIZE=10000\nhistory -s \"echo before-trap-line\"\ntrap 'history -s \"echo trap-added-line\"' EXIT\n",
    );
    let d = dir.to_string_lossy().replace('\\', "/");
    assert_eq!(
        saved.as_deref(),
        Some(
            format!(
                "HISTFILE=\"{d}/hist\"\nHISTFILESIZE=10000\necho before-trap-line\ntrap 'history -s \"echo trap-added-line\"' EXIT\necho trap-added-line\n"
            )
            .as_str()
        ),
        "the saved file must contain the trap definition line itself"
    );
    cleanup(&dir);
}

#[test]
fn exit_trap_history_s_does_not_eat_a_later_source_line() {
    // Source lines continue after the trap definition; the last
    // recorded line is `marker=after`. GNU keeps all of them.
    let (dir, saved) = run_case(
        "after",
        "set -o history\nHISTFILE=\"HISTDIR/hist\"\nHISTFILESIZE=10000\nhistory -s \"echo before-trap-line\"\ntrap 'history -s \"echo trap-added-line\"' EXIT\necho plain-cmd\nmarker=after\n",
    );
    let d = dir.to_string_lossy().replace('\\', "/");
    assert_eq!(
        saved.as_deref(),
        Some(
            format!(
                "HISTFILE=\"{d}/hist\"\nHISTFILESIZE=10000\necho before-trap-line\ntrap 'history -s \"echo trap-added-line\"' EXIT\necho plain-cmd\nmarker=after\necho trap-added-line\n"
            )
            .as_str()
        )
    );
    cleanup(&dir);
}

#[test]
fn trap_defined_before_history_is_enabled_is_not_recorded_nor_popped() {
    // The trap line is read before `set -o history` executes, so GNU
    // does not record it; the trap's `-s` still only appends.
    let (dir, saved) = run_case(
        "before",
        "trap 'history -s \"echo trap-added-line\"' EXIT\nset -o history\nHISTFILE=\"HISTDIR/hist\"\nHISTFILESIZE=10000\necho body-cmd\n",
    );
    let d = dir.to_string_lossy().replace('\\', "/");
    assert_eq!(
        saved.as_deref(),
        Some(
            format!(
                "HISTFILE=\"{d}/hist\"\nHISTFILESIZE=10000\necho body-cmd\necho trap-added-line\n"
            )
            .as_str()
        )
    );
    cleanup(&dir);
}

#[test]
fn same_line_history_s_still_replaces_its_own_recorded_line() {
    // Regression lock for the legitimate pop (history.def push_history):
    // a `-s` on a line that was itself recorded replaces that entry.
    let (dir, saved) = run_case(
        "lock",
        "set -o history\nHISTFILE=\"HISTDIR/hist\"\nHISTFILESIZE=10000\nhistory -s \"echo one\"\nhistory -s \"echo two\"\n",
    );
    let d = dir.to_string_lossy().replace('\\', "/");
    assert_eq!(
        saved.as_deref(),
        Some(format!("HISTFILE=\"{d}/hist\"\nHISTFILESIZE=10000\necho one\necho two\n").as_str())
    );
    cleanup(&dir);
}

#[test]
fn no_history_file_when_history_is_never_enabled() {
    let (dir, saved) = run_case("control", "HISTFILE=\"HISTDIR/hist\"\necho nocmd\n");
    assert_eq!(saved, None, "no history file may be written");
    cleanup(&dir);
}

/// Run a script body and return its stdout (the listing-surface cells
/// below assert on the `history` output printed inside the session,
/// not on the saved file).
fn run_stdout(tag: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("rubash-2824-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "script must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    cleanup(&dir);
    stdout
}

#[test]
fn eval_history_p_pops_the_eval_line() {
    // GNU expand_and_print_history (history.def:462): the `-p` delete
    // carries NO remember_on_history gate, so inside an eval unit the
    // enclosing eval line is still popped; the final listing holds
    // only the `history` line itself. GNU bash 5.3.20, script file.
    let stdout = run_stdout(
        "evalp",
        "set -o history\neval 'history -p zzz > /dev/null'\nhistory\n",
    );
    assert_eq!(stdout, "    1  history\n");
}

#[test]
fn eval_history_s_appends_and_keeps_the_eval_line() {
    // GNU push_history (history.def:435-439): the `-s` delete IS gated
    // on remember_on_history, which the eval unit disables — the
    // argument is appended and the eval line itself stays listed.
    // GNU bash 5.3.20, script file.
    let stdout = run_stdout(
        "evals",
        "set -o history\neval 'history -s \"in-eval\"'\nhistory\n",
    );
    assert_eq!(
        stdout,
        "    1  eval 'history -s \"in-eval\"'\n    2  in-eval\n    3  history\n"
    );
}

#[test]
fn exit_trap_history_p_pops_the_trap_line() {
    // GNU expand_and_print_history (history.def:462): the `-p`
    // delete has NO remember_on_history gate, so inside the EXIT
    // trap it still removes the last recorded line — the trap
    // definition itself — while the entry the earlier `-s` pushed
    // stays. Asserted on the saved file (the listing face inside a
    // trap proved load-sensitive in the lane's probe batches; the
    // saved-file surface is the one the cells above already use).
    // GNU bash 5.3.20, script file.
    let (dir, saved) = run_case(
        "trapp",
        "set -o history\nHISTFILE=\"HISTDIR/hist\"\nHISTFILESIZE=10000\nhistory -s \"echo before\"\ntrap 'history -p zzz > /dev/null' EXIT\n",
    );
    let d = dir.to_string_lossy().replace('\\', "/");
    assert_eq!(
        saved.as_deref(),
        Some(format!("HISTFILE=\"{d}/hist\"\nHISTFILESIZE=10000\necho before\n").as_str()),
        "the trap's -p must pop the trap definition line from the saved file"
    );
    cleanup(&dir);
}
