//! Issue rubash#825 — the `set -T` (functrace) DEBUG trap is inherited into
//! `$( )` command substitution, and the substitution child fires it for the
//! body's commands.
//!
//! GNU keeps the DEBUG trap armed inside the substitution child when
//! function_trace_mode is set (third_party/bash trap.c:1466-1474,
//! reset_or_restore_signal_handlers: DEBUG/RETURN are cleared only when
//! `function_trace_mode == 0`). rubash's `false`/`true`/`:` trivial-body
//! shortcuts in expand_command_substitution_inner sat ABOVE the
//! functrace+DEBUG disqualification gate, so `x=$(true)` returned a
//! hardcoded empty status short-circuit without ever entering the
//! substitution child — no DEBUG fire, `got=` where GNU prints `got=D:true`
//! (rubash#825). The fix moves those two shortcuts below the gate, so under
//! functrace with a DEBUG trap armed the body reaches the real
//! parser/executor (subst.c:7143 command_substitute -> parse_and_execute).
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0(1) via the
//! stdin oracle (2026-10-13). The probe family is the audit's
//! `debug-into-comsub-functrace-on`; the functrace-OFF faces (t6/t7/t8 here)
//! already matched GNU on master and are pinned as guards.
//!
//! Known pre-existing cosmetic divergence (NOT regressed by this fix, out of
//! scope): for a top-level assignment whose command text carries a `$( ( …
//! ) )` or `$(< …)` body, rubash's BASH_COMMAND printer re-renders the
//! source (`y=$(( true ))`, `z=$( < cf.txt)`) where GNU echoes the original
//! text (`y=$( ( true ))`, `z=$(< cf.txt)`). The fire set and the captured
//! substitution output are byte-identical; only the echoed assignment text
//! differs, and it differs identically on master.

use std::path::PathBuf;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    rubash_in(script, &std::env::temp_dir())
}

fn rubash_in(script: &str, cwd: &std::path::Path) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue825-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// The audit probe `debug-into-comsub-functrace-on`: GNU fires the top-level
/// DEBUG trap for the assignment, then again for the echo, and the
/// substitution child fires for the body's `true` — captured into the
/// result (`got=D:true`). Master printed `got=`.
#[test]
fn functrace_on_debug_trap_fires_into_comsub() {
    let (stdout, stderr, code) =
        rubash("set -T\ntrap 'echo D:$BASH_COMMAND' DEBUG\nx=$(true)\necho \"got=$x\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "D:x=$(true)\nD:echo \"got=$x\"\ngot=D:true\n");
    assert_eq!(code, Some(0));
}

/// `$(false)` under `-T`: the child fires for `false`, the trap output is
/// captured, and the substitution status is 1 — GNU probe 2026-10-13:
/// `got=[D:false] rc=1`.
#[test]
fn functrace_on_debug_trap_false_body_captures_and_status() {
    let (stdout, stderr, code) =
        rubash("set -T\ntrap 'echo D:$BASH_COMMAND' DEBUG\nx=$(false)\necho \"got=[$x] rc=$?\"");
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "D:x=$(false)\nD:echo \"got=[$x] rc=$?\"\ngot=[D:false] rc=1\n"
    );
    assert_eq!(code, Some(0));
}

/// A body wrapped in `( )` reaches the child's executor under `-T` (GNU:
/// `got2=[D:true]` — one inner fire for `true`). The echoed assignment text
/// carries rubash's pre-existing printer rendering; see the module doc.
#[test]
fn functrace_on_debug_trap_wrapped_subshell_body() {
    let (stdout, stderr, code) =
        rubash("set -T\ntrap 'echo D:$BASH_COMMAND' DEBUG\ny=$( (true) )\necho \"got2=[$y]\"");
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "D:y=$(( true ))\nD:echo \"got2=[$y]\"\ngot2=[D:true]\n"
    );
    assert_eq!(code, Some(0));
}

/// `$(< file)` keeps GNU's optimize_cat_file face: the substitution reads
/// the file and NO inner DEBUG fire happens (GNU subst.c:6665 intercepts
/// before any command runs) — `got3=[hi]` with only the top-level fires.
#[test]
fn functrace_on_debug_trap_cat_file_body_does_not_fire() {
    let dir = scratch_dir("catfile");
    let (stdout, stderr, code) = rubash_in(
        "set -T\ntrap 'echo D:$BASH_COMMAND' DEBUG\necho hi > cf.txt\nz=$(< cf.txt)\necho \"got3=[$z]\"",
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "D:echo hi > cf.txt\nD:z=$( < cf.txt)\nD:echo \"got3=[$z]\"\ngot3=[hi]\n"
    );
    assert_eq!(code, Some(0));
}

/// A multi-command body already reached the real executor on master; the
/// per-command fires are pinned (`got4=[D:true\nD:true]`).
#[test]
fn functrace_on_debug_trap_multi_command_body() {
    let (stdout, stderr, code) =
        rubash("set -T\ntrap 'echo D:$BASH_COMMAND' DEBUG\nw=$(true; true)\necho \"got4=[$w]\"");
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "D:w=$(true; true)\nD:echo \"got4=[$w]\"\ngot4=[D:true\nD:true]\n"
    );
    assert_eq!(code, Some(0));
}

/// Guard (functrace OFF): GNU resets the inherited DEBUG trap in the child
/// (trap.c:1466-1474 clears it when function_trace_mode == 0), so `got=`
/// stays empty while the top-level fires keep working.
#[test]
fn functrace_off_keeps_child_debug_reset() {
    let (stdout, stderr, code) =
        rubash("trap 'echo D:$BASH_COMMAND' DEBUG\nx=$(true)\necho \"got=$x\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "D:x=$(true)\nD:echo \"got=$x\"\ngot=\n");
    assert_eq!(code, Some(0));
}

/// Guard: an explicit `set +T` with the trap armed behaves like the OFF
/// face (`got=`).
#[test]
fn debug_armed_without_functrace_no_child_fire() {
    let (stdout, stderr, code) =
        rubash("set +T\ntrap 'echo D:$BASH_COMMAND' DEBUG\nx=$(true)\necho \"got=$x\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "D:x=$(true)\nD:echo \"got=$x\"\ngot=\n");
    assert_eq!(code, Some(0));
}

/// Guard: `-T` alone with no DEBUG trap armed — the shortcuts stay on the
/// fast path, status 0, no fires.
#[test]
fn functrace_without_debug_trap_keeps_shortcut() {
    let (stdout, stderr, code) = rubash("set -T\nx=$(true)\necho \"got=$x rc=$?\"");
    assert_eq!(stderr, "");
    assert_eq!(stdout, "got= rc=0\n");
    assert_eq!(code, Some(0));
}
