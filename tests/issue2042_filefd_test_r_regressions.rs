//! rubash#2042 item ⑤: `test -r /proc/<pid>/fd/N` on a file-backed
//! open descriptor. GNU's sh_eaccess (test.c:553 ->
//! lib/sh/eaccess.c:194) answers TRUE for every live fd magic link
//! regardless of the fd's open mode (uid-1 procfs measurement:
//! `lr-x` read-only regular, `l-wx` write-only append, `lr-x`
//! /dev/null all yes). The pipe-mark face covered pipes only; the
//! file class fell to a filesystem with no such names, and the
//! fused-scan mark refresh skipped the numeric-pid spelling
//! (rubash#1974) so the first dispatch read stale marks.

use std::process::Command;

fn run(script: &str) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2042_file_fd_link_is_readable_in_both_spellings() {
    // Read-only and write-only binds; the self and the numeric-pid
    // spellings must agree (GNU: both name the same fd directory).
    for bind in ["exec 4</dev/null", "exec 4>/dev/null"] {
        let (stdout, stderr, code) = run(&format!(
            "{bind}; test -r /proc/self/fd/4 && echo s || echo n; test -r /proc/$$/fd/4 && echo p || echo q"
        ));
        assert_eq!(code, 0, "stderr: {stderr}");
        assert_eq!(stdout.trim(), "s\np", "bind {bind}: stdout {stdout}");
    }
}

#[test]
fn issue2042_pid_spelling_first_dispatch_is_not_stale() {
    // The numeric-pid spelling as the FIRST fd operand of the shell
    // run: the mark refresh must trigger for it (deterministic —
    // this exact order used to answer from a cold mark table).
    let (stdout, stderr, code) = run("exec 4</dev/null; test -r /proc/$$/fd/4 && echo t || echo f");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "t", "stdout: {stdout} stderr: {stderr}");
}

#[test]
fn issue2042_closed_fd_link_stays_unreadable_and_pipes_keep_their_matrix() {
    // A closed fd's link dangles (ENOENT in procfs): the marks are
    // absent and the filesystem face answers false.
    let (stdout, stderr, code) =
        run("exec 4</dev/null; exec 4<&-; test -r /proc/self/fd/4 && echo t || echo f");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "f", "stdout: {stdout} stderr: {stderr}");

    // The pipe class keeps its #1467 verdicts (-r still true there).
    let (stdout, stderr, code) = run(
        "exec 3< <(printf 'x\\n'); test -r /proc/self/fd/3 && echo t || echo f; test -p /proc/self/fd/3 && echo p || echo q",
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "t\np", "stdout: {stdout} stderr: {stderr}");
}
