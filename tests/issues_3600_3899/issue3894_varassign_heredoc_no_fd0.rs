//! rubash#3894 — a `{var}<<` / `{var}<<<` heredoc body on a simple
//! command was fed to fd 0 as if it were a plain `<<` body; GNU binds
//! the body only to the freshly allocated descriptor and leaves the
//! command's stdin untouched.
//!
//! GNU anchor: redir.c:963-968 `do_redirection_internal` (RX_ACTIVE arm)
//! — for REDIR_VARASSIGN the redirector is `fcntl (fd, F_DUPFD,
//! SHELL_FD_BASE)`, a fresh descriptor >= 10, and redir_varassign
//! (redir.c:1470) binds the variable to it; fd 0 never participates.
//! The heredoc arm (redir.c:423 `here_document_to_fd`, :1065-1072)
//! materializes the body onto that allocated fd.
//!
//! The rubash defect was the fd-0 stdin mirror: the parser stored every
//! fd-None heredoc body into `cmd.heredoc` (src/parser/redirections.rs
//! `fill_pending_heredoc_body`), and every stdin derivation — the funnel
//! staging (`command_input_redirect`), the stdin-string resolvers
//! (`stdin_string_for_command[_mut]`), the fd-0 winner
//! (`fd0_stdin_redirect_winner`), and the has-own-stdin predicates —
//! treated an fd-None twin as an fd-0 body without excluding
//! `fd_var.is_some()`. The fix: the mirror only records plain fd-0
//! bodies, and every fd-0 derivation excludes `{var}` twins (the
//! `{var}<<<` precedent in `assign_here_string_redirect_raw`, which
//! already skips the `here_string` mirror, is the in-tree model).
//!
//! Every expectation below is measured against GNU bash 5.3.20
//! (script-file oracle probes, 2026-11-18, stdin /dev/null). The
//! mixed-order fd numbers are deliberately NOT pinned: which free
//! descriptor a `{var}` entry gets depends on the heredoc transport
//! (GNU parks the plain heredoc's pipe on fd 10, so `read <<A {v}<<B`
//! allocates 11 there); the defect under test is the stdin binding,
//! so those cases assert the stdin outcome only.

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!("i3894_vh_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn varassign_heredoc_does_not_feed_external_stdin() {
    // GNU: cat reads the real stdin (/dev/null here), not the body;
    // v does not survive the fork of an external command.
    let (stdout, _stderr, code) = run_file("cat {v}<<A\nvar-body\nA\necho \"v=$v\"\n", "ext");
    assert_eq!(stdout, "v=\n");
    assert_eq!(code, 0);
}

#[test]
fn varassign_heredoc_does_not_feed_builtin_read() {
    // GNU: read hits EOF on the untouched fd 0 (rc=1, x empty) while
    // v still gets its allocation (builtins redirect in-process).
    let (stdout, _stderr, code) = run_file(
        "read x {v}<<A\nrb-body\nA\necho \"x=$x v=$v rc=$?\"\n",
        "read",
    );
    assert_eq!(stdout, "x= v=10 rc=1\n");
    assert_eq!(code, 0);
}

#[test]
fn varassign_heredoc_does_not_feed_external_stdin_here_string() {
    let (stdout, _stderr, code) = run_file("cat {v}<<<hello\necho \"v=$v\"\n", "ext-hs");
    assert_eq!(stdout, "v=\n");
    assert_eq!(code, 0);
}

#[test]
fn varassign_here_string_does_not_feed_builtin_read() {
    let (stdout, _stderr, code) =
        run_file("read x {v}<<<hello\necho \"x=$x v=$v rc=$?\"\n", "read-hs");
    assert_eq!(stdout, "x= v=10 rc=1\n");
    assert_eq!(code, 0);
}

#[test]
fn plain_heredoc_still_feeds_stdin_with_varassign_sibling() {
    // GNU: the plain `<<A` body is stdin; the `{v}<<B` body goes to
    // the allocated fd and is never read. Both orders.
    let (stdout, _stderr, code) = run_file(
        "cat <<A {v}<<B\nbody-A\nA\nbody-B\nB\necho \"v=$v\"\n",
        "mix1",
    );
    assert_eq!(stdout, "body-A\nv=\n");
    assert_eq!(code, 0);
    let (stdout, _stderr, code) = run_file(
        "cat {v}<<B <<A\nbody-B\nB\nbody-A\nA\necho \"v=$v\"\n",
        "mix2",
    );
    assert_eq!(stdout, "body-A\nv=\n");
    assert_eq!(code, 0);
}

#[test]
fn plain_heredoc_still_feeds_builtin_read_with_varassign_sibling() {
    // GNU: read takes the plain body (x=body-A, rc=0); the `{var}`
    // entry only allocates. The allocated number is transport
    // dependent, so only the stdin outcome is pinned.
    let (stdout, _stderr, code) = run_file(
        "read x <<A {v}<<B\nbody-A\nA\nbody-B\nB\necho \"x=$x v=$v rc=$?\"\n",
        "mix-read1",
    );
    assert!(
        stdout.starts_with("x=body-A v=") && stdout.ends_with(" rc=0\n"),
        "stdout: {stdout}"
    );
    assert_eq!(code, 0);
    let (stdout, _stderr, code) = run_file(
        "read x {v}<<B <<A\nbody-B\nB\nbody-A\nA\necho \"x=$x v=$v rc=$?\"\n",
        "mix-read2",
    );
    assert!(
        stdout.starts_with("x=body-A v=") && stdout.ends_with(" rc=0\n"),
        "stdout: {stdout}"
    );
    assert_eq!(code, 0);
}

#[test]
fn two_varassign_heredocs_leave_stdin_untouched() {
    // GNU: neither body binds fd 0; read hits EOF (rc=1, x empty).
    // The second allocation wins the variable; its number is
    // transport dependent, so only the stdin outcome is pinned.
    let (stdout, _stderr, code) = run_file(
        "read x {v}<<A {v}<<B\nbody1\nA\nbody2\nB\necho \"x=$x v=$v rc=$?\"\n",
        "two-var",
    );
    assert!(
        stdout.starts_with("x= v=") && stdout.ends_with(" rc=1\n"),
        "stdout: {stdout}"
    );
    assert_eq!(code, 0);
}

#[test]
fn varassign_heredoc_does_not_feed_function_call_stdin() {
    // GNU: the function's read hits EOF (rc=1, x empty); v=10
    // persists because the call redirects in-process.
    let (stdout, _stderr, code) = run_file(
        "f() { read x; echo \"x=$x rc=$? v=$v\"; }; f {v}<<A\nbody\nA\n",
        "func",
    );
    assert_eq!(stdout, "x= rc=1 v=10\n");
    assert_eq!(code, 0);
}

#[test]
fn varassign_heredoc_does_not_feed_if_condition_read() {
    // GNU: the condition's read fails, so the else branch runs.
    let (stdout, _stderr, code) = run_file(
        "if read x {v}<<A; then echo then-branch; else echo else-branch; fi\nbody\nA\necho \"v=$v\"\n",
        "ifcond",
    );
    assert_eq!(stdout, "else-branch\nv=10\n");
    assert_eq!(code, 0);
}

#[test]
fn varassign_heredoc_does_not_override_pipeline_stdin() {
    // GNU: the stage keeps the pipe as its stdin (prints "hi"); a
    // plain `<<` still overrides the pipe (prints the body).
    let (stdout, _stderr, code) =
        run_file("echo hi | cat {v}<<A\nbody\nA\necho \"v=$v\"\n", "pipe-var");
    assert_eq!(stdout, "hi\nv=\n");
    assert_eq!(code, 0);
    let (stdout, _stderr, code) = run_file("echo hi | cat <<A\nbody\nA\n", "pipe-plain");
    assert_eq!(stdout, "body\n");
    assert_eq!(code, 0);
}

#[test]
fn numbered_and_plain_heredoc_controls_still_feed() {
    // `cat 9<<A` (fd 9, fd 0 untouched) and plain `cat <<A` agree
    // with GNU before and after the fix; `read <<A` still feeds.
    let (stdout, _stderr, code) = run_file("cat 9<<A\nnine\nA\necho done\n", "ctl-9");
    assert_eq!(stdout, "done\n");
    assert_eq!(code, 0);
    let (stdout, _stderr, code) = run_file("cat <<A\nplain\nA\necho done\n", "ctl-plain");
    assert_eq!(stdout, "plain\ndone\n");
    assert_eq!(code, 0);
    let (stdout, _stderr, code) =
        run_file("read x <<A\nhello\nA\necho \"x=$x rc=$?\"\n", "ctl-read");
    assert_eq!(stdout, "x=hello rc=0\n");
    assert_eq!(code, 0);
}

#[test]
fn unterminated_varassign_heredoc_still_warns() {
    // GNU warns for `{var}<<` at EOF exactly like for plain `<<`:
    // the warning is about the gather, not fd 0 — the parser fix
    // must not silence it. The body must still not feed stdin.
    //
    // Oracle (GNU bash 5.3.20, script file, stdin /dev/null,
    // 2026-11-18, captain re-verified for the #3915 review):
    // `cat {v}<<A` + `var-body` + EOF prints NOTHING on stdout
    // (cat reads the untouched fd 0), warns on stderr, rc 0. The
    // `v=` expectation the first version of this test carried was
    // wrong: no `echo "v=$v"` line can follow an unterminated
    // heredoc — the rest of the file IS the body — so nothing
    // ever prints `v=`. Empty stdout is itself the assertion
    // that the body did not feed cat's stdin (it would have
    // printed `var-body`).
    let (stdout, stderr, code) = run_file("cat {v}<<A\nvar-body\n", "eof-warn");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("delimited by end-of-file (wanted `A')"),
        "stderr: {stderr}"
    );
    assert_eq!(code, 0);
}
