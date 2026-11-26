//! Trap-action source echo lines are the action's own text, verbatim
//! (rubash#3328; GNU bash 5.3 oracle anchors below, 5.2.21 reference
//! probes 2026-11-13 — 5.3.20 byte-verification is owed to the captain
//! review round).
//!
//! GNU runs every trap action through parse_and_execute on the action
//! string — signal traps at trap.c:496 (`"trap"`), the EXIT trap at
//! trap.c:1071 (`"exit trap"`), DEBUG/RETURN/ERR through
//! _run_trap_internal at trap.c:1109-1200 (tag `"debug trap"` /
//! `"return trap"` / `"error trap"`) — and a parse failure reports
//! through parser_error (error.c:300) with the offending input line
//! echoed verbatim by print_offending_line (parse.y:6814), i.e. the
//! action's own line as read. Rubash's trap runners re-parsed the
//! action with the bare `parse()` (src/executor/trap_exec.rs:
//! run_exit_trap_action, run_debug_trap, run_return_trap,
//! run_pending_signal_traps, maybe_run_error_trap_inner,
//! run_sigchld_trap_for_reaped_child), which carries no source text,
//! so the stored `__RUBASH_PARSE_SOURCE__` fell back to the token-join
//! reconstruction (`echo inaction ; echo (` for `echo inaction; echo (`).
//! The fix threads the action string as `ParseLoopOptions::source_text`
//! at those six sites — the same shape the eval/comsub re-parses
//! already use — with no other option changed.
//!
//! Scope: these pins assert ONLY the echoed source line (verbatim
//! present, re-serialized form absent). Two adjacent divergences are
//! deliberately NOT asserted here — the missing trap-context infix on
//! the DEBUG/RETURN/ERR/CHLD faces and the rc=2 abort their parse
//! failures currently cause (GNU: rc 0, script continues) — they belong
//! to the #2716 trap-context family and need their own lanes; nailing
//! them here would widen this fix's shape.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script file; return (normalized stdout, normalized stderr, code).
fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue3328-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).replace(path.to_str().expect("utf8 path"), "S")
    };
    (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.code(),
    )
}

/// The source echo line must be the action's own text verbatim: the
/// backticked line appears exactly as written, and the token-join
/// re-serialization (single-spaced operators) must not appear.
fn assert_verbatim_echo(stderr: &str, verbatim: &str, reserialized: &str) {
    assert!(
        stderr.contains(&format!("`{verbatim}'")),
        "source echo line must be the verbatim action text `{verbatim}': {stderr:?}"
    );
    assert!(
        !stderr.contains(&format!("`{reserialized}'")),
        "source echo line must not be the re-serialized form `{reserialized}': {stderr:?}"
    );
}

const ACTION: &str = "echo inaction; echo (";
const ACTION_VERBATIM: &str = "echo inaction; echo (";
const ACTION_RESERIALIZED: &str = "echo inaction ; echo (";

#[test]
fn issue3328_exit_trap_action_echo_is_verbatim() {
    let (_stdout, stderr, _code) =
        run_script("exit", &format!("trap '{ACTION}' EXIT\necho body\n"));
    assert_verbatim_echo(&stderr, ACTION_VERBATIM, ACTION_RESERIALIZED);
}

#[test]
fn issue3328_debug_trap_action_echo_is_verbatim() {
    let (_stdout, stderr, _code) =
        run_script("debug", &format!("trap '{ACTION}' DEBUG\necho hi\n"));
    assert_verbatim_echo(&stderr, ACTION_VERBATIM, ACTION_RESERIALIZED);
}

#[test]
fn issue3328_return_trap_action_echo_is_verbatim() {
    let (_stdout, stderr, _code) = run_script(
        "return",
        &format!("f() {{ trap '{ACTION}' RETURN; echo in-f; }}\nf\necho after\n"),
    );
    assert_verbatim_echo(&stderr, ACTION_VERBATIM, ACTION_RESERIALIZED);
}

#[test]
fn issue3328_err_trap_action_echo_is_verbatim() {
    let (_stdout, stderr, _code) = run_script(
        "err",
        &format!("trap '{ACTION}' ERR\nfalse\necho after-err\n"),
    );
    assert_verbatim_echo(&stderr, ACTION_VERBATIM, ACTION_RESERIALIZED);
}

#[test]
fn issue3328_signal_trap_action_echo_is_verbatim() {
    let (_stdout, stderr, _code) = run_script(
        "usr1",
        &format!("trap '{ACTION}' USR1\nkill -USR1 $$\necho after-usr1\n"),
    );
    assert_verbatim_echo(&stderr, ACTION_VERBATIM, ACTION_RESERIALIZED);
}

#[test]
fn issue3328_sigchld_trap_action_echo_is_verbatim() {
    let (_stdout, stderr, _code) = run_script(
        "chld",
        &format!("trap '{ACTION}' CHLD\nsleep 0.05 &\nwait\necho after-chld\n"),
    );
    assert_verbatim_echo(&stderr, ACTION_VERBATIM, ACTION_RESERIALIZED);
}

#[test]
fn issue3328_exit_trap_action_echo_preserves_spacing() {
    // A second spacing shape: double space inside a word list and no
    // space before `(` — the token join collapses both (`echo a ;
    // echo (`), the verbatim echo keeps both.
    let (_stdout, stderr, _code) = run_script("spacing", "trap 'echo  a;echo(' EXIT\necho body\n");
    assert_verbatim_echo(&stderr, "echo  a;echo(", "echo a ; echo (");
}

#[test]
fn issue3328_eval_action_echo_stays_verbatim() {
    // Control: the eval re-parse already threads its source text, so
    // its echo was verbatim before this fix — guard against regressing
    // the mechanism while threading the trap sites.
    let (_stdout, stderr, _code) =
        run_script("eval", &format!("eval '{ACTION}'\necho after-eval\n"));
    assert_verbatim_echo(&stderr, ACTION_VERBATIM, ACTION_RESERIALIZED);
}
