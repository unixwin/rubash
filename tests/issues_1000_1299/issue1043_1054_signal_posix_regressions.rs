//! Regression pins for rubash#1043 and rubash#1054 (compgen signal table
//! shape; POSIX-mode output flips).
//!
//! #1043: `compgen -A signal` emitted 64 lines whose last 33 were EMPTY,
//! and the pseudo/RT names were missing. GNU bash 5.3 (builtins complete /
//! the signal name table behind `trap -l`): 68 entries —
//! EXIT SIGHUP SIGINT ... SIGRTMIN ... SIGRTMAX DEBUG ERR RETURN — with no
//! blank lines. Issue repro (GNU 5.3.20):
//!   compgen -A signal | grep -c '^$'  -> 0
//!   compgen -A signal | grep -c EXIT -> 1
//!   (old niu: 33 / 0)
//!
//! #1054: POSIX mode (`set -o posix` / `--posix` / POSIXLY_CORRECT) flips
//! five documented GNU output shapes that the engine did not follow:
//!   1. `trap -p` with no signal args dumps EVERY signal's trap (68 lines),
//!      not only the ones the user set (builtins/trap.def: in posix mode
//!      the dump is the full table).
//!   2. `kill -l` prints the single-line name list
//!      `HUP INT QUIT ... RTMAX` (no SIG prefixes, no numbers) instead of
//!      the numbered multi-column table (builtins/kill.def posix branch).
//!   3. `hash` with an empty table is silent rc 0 (the "hash table empty"
//!      note is non-posix only).
//!   4. `POSIXLY_CORRECT` in the environment enables posix mode by mere
//!      presence — even with an empty value (variables.c / shell.c:
//!      posixly_correct is a presence flag).
//!   5. `kill %1` for a nonexistent job in posix mode prints the message
//!      but the SHELL exits 0; non-posix it exits 1. (Invalid-PID kill is
//!      rc 1 in both modes — the quirk is jobspec-specific.)
//!
//! Expected values byte-checked against GNU bash 5.3.20 (self-built
//! oracle) on Linux x86_64: compgen signal table = 68 lines, first line
//! EXIT; posix trap -p after one trap = 68 lines; posix kill -l = the
//! single line starting `HUP INT QUIT ILL TRAP ABRT ... RTMAX`; non-posix
//! kill -l starts with ` 1) SIGHUP`.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn rubash_env(script: &str, envs: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn issue1043_compgen_signal_full_table_no_blanks() {
    let (stdout, _stderr, code) = rubash("compgen -A signal");
    assert_eq!(code, Some(0), "compgen -A signal rc, stderr follows run");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        68,
        "GNU signal completion table has 68 entries, got {}: {lines:?}",
        lines.len()
    );
    assert!(
        lines.iter().all(|l| !l.trim().is_empty()),
        "no blank lines allowed in compgen -A signal output: {lines:?}"
    );
    for name in [
        "EXIT", "SIGHUP", "SIGINT", "SIGKILL", "SIGTERM", "SIGRTMIN", "SIGRTMAX", "DEBUG", "ERR",
        "RETURN",
    ] {
        assert!(
            lines.contains(&name),
            "compgen -A signal must list {name}: {lines:?}"
        );
    }
    assert_eq!(lines[0], "EXIT", "table starts with EXIT like GNU");
}

#[test]
fn issue1054_posix_trap_p_dumps_full_table() {
    let (stdout, _stderr, _code) = rubash("set -o posix; trap 'echo hi' USR1; trap -p");
    assert_eq!(
        stdout.lines().count(),
        68,
        "posix trap -p dumps all 68 signal slots, got: {stdout:?}"
    );
    assert!(
        stdout.contains("USR1"),
        "the user's own trap must be in the dump: {stdout:?}"
    );
    // Control: non-posix dumps only the traps actually set (1 line).
    let (plain, _e, _c) = rubash("trap 'echo hi' USR1; trap -p");
    assert_eq!(
        plain.lines().count(),
        1,
        "non-posix trap -p lists only set traps, got: {plain:?}"
    );
}

#[test]
fn issue1054_posix_kill_l_is_single_line_names() {
    let (stdout, _stderr, _code) = rubash("set -o posix; kill -l");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "posix kill -l is one line, got: {stdout:?}");
    assert!(
        lines[0].starts_with("HUP INT QUIT"),
        "posix kill -l starts with bare names: {stdout:?}"
    );
    assert!(
        lines[0].ends_with("RTMAX"),
        "posix kill -l ends at RTMAX: {stdout:?}"
    );
    assert!(
        !lines[0].contains("SIG"),
        "posix kill -l strips SIG prefixes: {stdout:?}"
    );
    // Control: non-posix keeps the numbered table.
    let (plain, _e, _c) = rubash("kill -l");
    assert!(
        plain.lines().count() > 1 && plain.contains("1) SIGHUP"),
        "non-posix kill -l keeps the numbered table, got: {plain:?}"
    );
}

#[test]
fn issue1054_posix_empty_hash_is_silent() {
    let (stdout, stderr, _code) = rubash("set -o posix; hash; echo rc=$?");
    assert_eq!(
        stdout.trim(),
        "rc=0",
        "posix empty hash prints nothing and succeeds, stdout: {stdout:?}"
    );
    assert!(
        stderr.is_empty(),
        "posix empty hash is silent on stderr too, got: {stderr:?}"
    );
    // Control: non-posix keeps the informational note (GNU prints it on
    // stdout — hash is a listing builtin, the note is its empty listing).
    let (stdout2, _e2, _c2) = rubash("hash");
    assert!(
        stdout2.contains("hash table empty"),
        "non-posix empty hash keeps the note on stdout, got: {stdout2:?}"
    );
}

#[test]
fn issue1054_posixly_correct_presence_enables_posix() {
    // Presence counts — even an empty value flips posix on (issue leg 4).
    let (stdout, _e, _c) = rubash_env("set -o | grep ^posix", &[("POSIXLY_CORRECT", "")]);
    assert!(
        stdout.contains("posix") && stdout.contains("on"),
        "POSIXLY_CORRECT= (empty) must enable posix mode, got: {stdout:?}"
    );
    let (stdout2, _e2, _c2) = rubash_env("set -o | grep ^posix", &[("POSIXLY_CORRECT", "y")]);
    assert!(
        stdout2.contains("on"),
        "POSIXLY_CORRECT=y must enable posix mode, got: {stdout2:?}"
    );
    // Control: without the variable, posix starts off.
    let (plain, _e3, _c3) = rubash("set -o | grep ^posix");
    assert!(
        plain.contains("off"),
        "posix is off by default, got: {plain:?}"
    );
}

#[test]
fn issue1054_posix_kill_bad_jobspec_shell_rc_zero() {
    let (stdout, stderr, _code) = rubash("set -o posix; kill %1 2>/dev/null; echo rc=$?");
    assert_eq!(
        stdout.trim(),
        "rc=0",
        "posix kill %1 (no such job) leaves shell rc 0, got: {stdout:?}"
    );
    let (_o2, stderr2, _c2) = rubash("set -o posix; kill %1; echo rc=$?");
    assert!(
        stderr2.contains("no such job"),
        "the diagnostic is still printed in posix mode, got: {stderr2:?}"
    );
    // Control: non-posix rc is 1 for the same form.
    let (plain, _e3, _c3) = rubash("kill %1 2>/dev/null; echo rc=$?");
    assert_eq!(
        plain.trim(),
        "rc=1",
        "non-posix kill %1 leaves shell rc 1, got: {plain:?}"
    );
    let _ = stderr;
}
