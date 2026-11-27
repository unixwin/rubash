//! rubash#1902 — `virtual_device_test` answered the fd-alias operands
//! (`/dev/stdin`, `/dev/stdout`, `/dev/stderr`, `/dev/fd/N`) from a
//! CONSTANT table instead of the fd fact: `test -w /dev/stdin` was false
//! even with fd 0 bound to a writable file, and `test -c /dev/stdin` was
//! true even with fd 0 bound to a regular file.
//!
//! GNU model: test.c `unary_test` (test.c:544) hands the word to
//! sh_stat/sh_eaccess; sh_eaccess routes path_is_devfd operands
//! (eaccess.c:63-77) through sh_stataccess (eaccess.c:116), which stats
//! the CURRENTLY BOUND descriptor (sh_stat → fstat(0/1/2), eaccess.c:80)
//! and checks the mode bits of what it found. The answer follows the
//! binding, never the spelling. The executor mirrors its fd table into
//! __RUBASH_FD_TARGET_<fd> / __RUBASH_FD_ACCESS_<fd> before test/[[ runs
//! (sync_fd_alias_facts); the evaluator resolves the alias through it.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files,
//! as a non-root user (uid 1000): a 644 file is owner-writable, a
//! character device is -c, a fifo is -p. Faces where the Windows
//! emulated store has no Unix analogue (chmod'd read-only attributes,
//! x-bits) are deliberately NOT pinned here — the direct-path answer and
//! the fd-alias answer agree with each other, which is the contract this
//! issue owns.

use std::path::PathBuf;
use std::process::Command;

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .current_dir(fixture_dir())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn fixture_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("rubash-issue1902-pins");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn fixture_path() -> PathBuf {
    fixture_dir().join("issue1902-fixture.txt")
}

/// Runs the script with `$F` bound to the fixture path so the binds stay
/// inside the sandboxed fixture dir regardless of the shell's cwd.
fn run_with_fixture(script: &str) -> (String, String, Option<i32>) {
    let path = fixture_path();
    std::fs::write(&path, b"hello world\n").expect("write fixture");
    // Forward slashes: a backslash path inside a -c script is an escape
    // sequence to the shell parser, not a path separator.
    let shell_path = path.to_string_lossy().replace('\\', "/");
    let run = run_c(&script.replace("$F", &shell_path));
    std::fs::remove_file(&path).ok();
    run
}

fn verdicts(stdout: &str) -> Vec<(String, bool)> {
    stdout
        .lines()
        .map(|line| {
            let (name, verdict) = line.rsplit_once('=').expect("verdict line NAME=T/F");
            (name.to_string(), verdict == "T")
        })
        .collect()
}

fn assert_verdicts(stdout: &str, expected: &[(&str, bool)]) {
    let got = verdicts(stdout);
    assert_eq!(got.len(), expected.len(), "verdict count, got: {stdout}");
    for ((name, value), (want_name, want)) in got.iter().zip(expected) {
        assert_eq!(name, want_name, "verdict order, got: {stdout}");
        assert_eq!(value, want, "verdict {name}, got: {stdout}");
    }
}

#[test]
fn issue1902_stdin_bound_to_regular_file_follows_the_binding() {
    let (stdout, _stderr, code) = run_with_fixture(
        "exec 0<$F\n\
         [ -w /dev/stdin ] && echo w=T || echo w=F\n\
         [ -r /dev/stdin ] && echo r=T || echo r=F\n\
         [ -e /dev/stdin ] && echo e=T || echo e=F\n\
         [ -f /dev/stdin ] && echo f=T || echo f=F\n\
         [ -s /dev/stdin ] && echo s=T || echo s=F\n\
         [ -c /dev/stdin ] && echo c=T || echo c=F\n",
    );
    assert_eq!(code, Some(0), "script must run clean");
    // GNU: fd 0 is the fixture — writable, readable, regular, nonempty,
    // NOT a character device (644 mode bits through /proc/self/fd/0).
    assert_verdicts(
        &stdout,
        &[
            ("w", true),
            ("r", true),
            ("e", true),
            ("f", true),
            ("s", true),
            ("c", false),
        ],
    );
}

#[test]
fn issue1902_stdin_bound_to_null_device_follows_the_binding() {
    let (stdout, _stderr, code) = run_c(
        "exec 0</dev/null\n\
         [ -c /dev/stdin ] && echo c=T || echo c=F\n\
         [ -w /dev/stdin ] && echo w=T || echo w=F\n\
         [ -r /dev/stdin ] && echo r=T || echo r=F\n\
         [ -s /dev/stdin ] && echo s=T || echo s=F\n\
         [ -f /dev/stdin ] && echo f=T || echo f=F\n",
    );
    assert_eq!(code, Some(0));
    // GNU: the bound inode is the null device — character device, r+w
    // mode bits (crw-rw-rw-), zero size, not a regular file.
    assert_verdicts(
        &stdout,
        &[
            ("c", true),
            ("w", true),
            ("r", true),
            ("s", false),
            ("f", false),
        ],
    );
}

#[test]
fn issue1902_stdout_stderr_names_follow_their_bindings() {
    let (stdout, _stderr, code) = run_with_fixture(
        "exec 5>&1\n\
         exec 1>$F\n\
         [ -w /dev/stdout ] && w=T || w=F\n\
         [ -r /dev/stdout ] && r=T || r=F\n\
         exec 2>$F\n\
         [ -w /dev/stderr ] && e=T || e=F\n\
         exec 1>&5\n\
         printf 'w=%s\\nr=%s\\ne=%s\\n' \"$w\" \"$r\" \"$e\"\n",
    );
    assert_eq!(code, Some(0));
    // GNU: fd 1 and fd 2 are the fixture — writable; -r follows the
    // referent's mode bits (644: owner read). The verdicts are captured
    // in variables and printed through the saved script stdout on fd 5,
    // since the very binds under test own fds 1 and 2 while they hold.
    assert_verdicts(&stdout, &[("w", true), ("r", true), ("e", true)]);
}

#[test]
fn issue1902_dev_fd_n_answers_from_the_bound_descriptor() {
    let (stdout, _stderr, code) = run_with_fixture(
        "exec 3<$F\n\
         [ -e /dev/fd/3 ] && echo e3=T || echo e3=F\n\
         [ -w /dev/fd/3 ] && echo w3=T || echo w3=F\n\
         [ -f /dev/fd/3 ] && echo f3=T || echo f3=F\n\
         exec 3<&-\n\
         [ -e /dev/fd/3 ] && echo e3c=T || echo e3c=F\n\
         [ -e /dev/fd/9 ] && echo e9=T || echo e9=F\n",
    );
    assert_eq!(code, Some(0));
    // GNU: /dev/fd/3 exists while fd 3 is open (writable by mode bits,
    // regular); once closed /proc/self/fd/3 is gone, and fd 9 never
    // existed.
    assert_verdicts(
        &stdout,
        &[
            ("e3", true),
            ("w3", true),
            ("f3", true),
            ("e3c", false),
            ("e9", false),
        ],
    );
}

#[test]
fn issue1902_pipeline_stage_stdin_is_the_fifo() {
    let (stdout, _stderr, code) = run_c(
        "printf x | { [ -p /dev/stdin ] && echo p=T || echo p=F; }\n\
         printf x | { [ -w /dev/stdin ] && echo w=T || echo w=F; }\n\
         printf x | { [[ -p /dev/stdin ]] && echo pp=T || echo pp=F; }\n",
    );
    assert_eq!(code, Some(0));
    // GNU: the stage's fd 0 is the pipe — a fifo (prw-r--r--), so -p and
    // -w both true, on both the [ ] and [[ ]] faces.
    assert_verdicts(&stdout, &[("p", true), ("w", true), ("pp", true)]);
}

#[test]
fn issue1902_string_operands_stay_literal() {
    let (stdout, _stderr, code) = run_c(
        "[ -z /dev/stdin ] && echo z=T || echo z=F\n\
         [ -n /dev/stdin ] && echo n=T || echo n=F\n",
    );
    // GNU: -z/-n judge the LITERAL word (non-empty string) — the alias is
    // never path-resolved for them (test.c unary_test only hands file-op
    // words to sh_stat/sh_eaccess). The -t operand-validation face
    // (integer expected) is a separate family and stays unpinned here.
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("z", false), ("n", true)]);
}

#[test]
fn issue1902_leading_zero_spelling_stays_rejected() {
    // rubash#1973 contract unchanged: GNU rejects /dev/fd/00 on every
    // face (the registry's canonical-fd judgment), so the operand falls
    // to the filesystem where no such node exists.
    let (stdout, _stderr, code) = run_c("[ -e /dev/fd/00 ] && echo e=T || echo e=F");
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("e", false)]);
}

#[test]
fn issue1902_unbound_names_keep_the_filesystem_face() {
    // Controls: with no binding in play the spellings behave exactly as
    // before — /dev/null answers its device table, a plain path answers
    // from the filesystem.
    let (stdout, _stderr, code) = run_c(
        "[ -w /dev/null ] && echo n=T || echo n=F\n\
         [ -e /nonexistent-x-1902 ] && echo m=T || echo m=F\n",
    );
    assert_eq!(code, Some(0));
    assert_verdicts(&stdout, &[("n", true), ("m", false)]);
}
