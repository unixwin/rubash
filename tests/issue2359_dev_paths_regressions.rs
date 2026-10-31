//! rubash#2359 — engine special-path faces: `/dev/fd/N`,
//! `/dev/tcp|udp/host/port`, `/dev/stdin|stdout|stderr` across plain
//! redirects, pipeline stages, and the noclobber ladder.
//!
//! GNU contract (bash 5.3 C source, read first):
//! - redir.c:598-615 `_redir_special_filenames` claims a net spelling
//!   only in the FULL `/dev/tcp/*/*` two-segment shape; redir_open
//!   (redir.c:725-731) tries the special table first, then noclobber,
//!   then a plain open — so a host-only spelling (`/dev/tcp/127.0.0.1`)
//!   is a literal filename open and fails ENOENT, never netopen's
//!   EINVAL.
//! - lib/sh/netopen.c `_netopen6`: an EMPTY service segment resolves
//!   through getaddrinfo to port 0, so `/dev/tcp/127.0.0.1/` fails at
//!   connect with ECONNREFUSED — it is not an unknown service.
//! - redir.c:676-712 `noclobber_open`: under `set -C`, the open behind
//!   a `/dev/fd/N` or `/dev/std*` spelling stats THROUGH the spelling;
//!   an existing REGULAR file refuses (`cannot overwrite existing
//!   file`), a pipe/device target opens. GNU evaluates the redirect
//!   inside the command's own context: a pipeline element's fd 1 is
//!   the stage pipe (execute_cmd.c execute_pipeline binds it before
//!   do_redirections), so a stage spelling naming fd 1 is judged
//!   against the pipe, not the shell's ambient stdout.
//! - The refusal is ONE diagnostic from the single do_redirections
//!   pass (redir.c:246 stops at the first failing redirect).
//! - The `$(<file)` cat shortcut is optimize_cat_file (subst.c:6666)
//!   over open_redir_file (builtins/evalstring.c:759): a plain
//!   open(fn, O_RDONLY) (evalstring.c:781) that never consults the
//!   special-filename table — a net spelling fails there ENOENT, and
//!   connecting is a bug (a UDP spelling even blocked forever).
//! - A redirection failure on a brace group is the group's failure:
//!   status 1, `&&` short-circuits, `||` runs, `set -e` exits — the
//!   group is not re-dispatched after the failure is reported.
//! - Opening /dev/stdin when stdin is a character device (/dev/null)
//!   opens the device fresh; the write succeeds and is discarded.
//!
//! Expectations measured against GNU bash 5.3.20 (script files,
//! loopback listeners, 2026-10-31).

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn run_script(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let (tx, rx) = mpsc::channel();
    let script = script.to_string();
    let panic_tag = tag.to_string();
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash2359-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let case = dir.join("case.sh");
        std::fs::write(&case, &script).expect("write fixture");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .current_dir(&dir)
            .arg(&case)
            .stdin(Stdio::null())
            .output()
            .expect("run rubash");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = tx.send((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ));
    });
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{panic_tag}: rubash run timed out"))
}

/// Twin of [`run_script`] with the shell's stdout bound to a regular
/// file (the harness shape that makes `/dev/stdout` name a regular
/// file). Returns (file content, stderr, status).
fn run_script_stdout_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let (tx, rx) = mpsc::channel();
    let script = script.to_string();
    let panic_tag = tag.to_string();
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash2359-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let case = dir.join("case.sh");
        std::fs::write(&case, &script).expect("write fixture");
        let cap = dir.join("cap.txt");
        let cap_file = std::fs::File::create(&cap).expect("create capture file");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .current_dir(&dir)
            .arg(&case)
            .stdin(Stdio::null())
            .stdout(Stdio::from(cap_file))
            .stderr(Stdio::piped())
            .output()
            .expect("run rubash");
        let file_content = std::fs::read_to_string(&cap).expect("read capture file");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = tx.send((
            file_content,
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ));
    });
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{panic_tag}: rubash run timed out"))
}

fn refusal_count(stderr: &str) -> usize {
    stderr.matches("cannot overwrite existing file").count()
}

/// (a) Host-only net spelling: not the special shape, so the open is a
/// literal filename open — ENOENT, never netopen's EINVAL wording.
#[test]
fn issue2359_host_only_net_spelling_is_a_literal_open() {
    let (_out, err, _rc) = run_script(
        "tcp-noport-redir",
        "cat < /dev/tcp/127.0.0.1\necho \"rc:$?\"\n",
    );
    assert!(
        err.contains("No such file or directory"),
        "redirect face should fail ENOENT, stderr: {err}"
    );
    assert!(
        !err.contains("Invalid argument"),
        "redirect face must not engage netopen, stderr: {err}"
    );

    let (_out, err, _rc) = run_script(
        "tcp-noport-operand",
        "cat /dev/tcp/127.0.0.1\necho \"rc:$?\"\n",
    );
    assert!(
        err.contains("No such file or directory"),
        "operand face should fail ENOENT, stderr: {err}"
    );
    assert!(
        !err.contains("Invalid argument"),
        "operand face must not engage netopen, stderr: {err}"
    );

    let (_out, err, _rc) = run_script(
        "udp-noport-redir",
        "cat < /dev/udp/127.0.0.1\necho \"rc:$?\"\n",
    );
    assert!(
        err.contains("No such file or directory"),
        "udp twin should fail ENOENT, stderr: {err}"
    );
    assert!(
        !err.contains("Invalid argument"),
        "udp twin must not engage netopen, stderr: {err}"
    );
}

/// (b) Empty service segment: getaddrinfo resolves it to port 0, so
/// the failure is the connect's ECONNREFUSED, not an unknown service.
#[test]
fn issue2359_empty_service_resolves_to_port_zero() {
    let (_out, err, _rc) = run_script(
        "tcp-empty-port",
        "cat < /dev/tcp/127.0.0.1/\necho \"rc:$?\"\n",
    );
    assert!(
        err.contains("Connection refused"),
        "empty service should reach connect and be refused, stderr: {err}"
    );
    assert!(
        !err.contains("Servname not supported"),
        "empty service is not an unknown service, stderr: {err}"
    );
}

/// (h, control) The full two-segment shape still engages netopen, and
/// the empty-HOST shape still yields the getaddrinfo failure.
#[test]
fn issue2359_full_net_shape_still_connects() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(b"GREETING-LINE\n");
            let _ = stream.flush();
        }
    });
    let script = format!("cat < /dev/tcp/127.0.0.1/{port}\n");
    let (out, err, _rc) = run_script("tcp-full-shape", &script);
    assert_eq!(
        out, "GREETING-LINE\n",
        "full shape must connect, stderr: {err}"
    );

    let script = format!("cat < /dev/tcp//{port}\necho \"rc:$?\"\n");
    let (_out, err, _rc) = run_script("tcp-empty-host", &script);
    assert!(
        err.contains("Name or service not known"),
        "empty host keeps the getaddrinfo failure, stderr: {err}"
    );
}

/// Twin of [`run_script`] with the shell's stderr bound to a regular
/// file (the harness shape that makes `/dev/stderr` name a regular
/// file). Returns (stderr file content, stdout, status).
fn run_script_stderr_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let (tx, rx) = mpsc::channel();
    let script = script.to_string();
    let panic_tag = tag.to_string();
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash2359-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let case = dir.join("case.sh");
        std::fs::write(&case, &script).expect("write fixture");
        let cap = dir.join("caperr.txt");
        let cap_file = std::fs::File::create(&cap).expect("create capture file");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .current_dir(&dir)
            .arg(&case)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(cap_file))
            .output()
            .expect("run rubash");
        let file_content = std::fs::read_to_string(&cap).expect("read capture file");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = tx.send((
            file_content,
            String::from_utf8_lossy(&output.stdout).into_owned(),
            output.status.code(),
        ));
    });
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{panic_tag}: rubash run timed out"))
}

/// (c/d) Under `set -C`, `> /dev/stdout`, `> /dev/fd/1` and
/// `> /dev/stderr` refuse when the stdio fd is backed by a regular
/// file — exactly one diagnostic, and the refused command's bytes
/// never land in the file. (/dev/stderr needs the stderr-file
/// harness: with stderr a pipe the same spelling is writable —
/// the pipe sub-case lives in
/// issue2359_noclobber_allows_stdio_spelling_to_pipe's family.)
#[test]
fn issue2359_noclobber_refuses_stdio_spellings_to_regular_file() {
    for (tag, spelling) in [("nc-stdout", "/dev/stdout"), ("nc-fd1", "/dev/fd/1")] {
        let script = format!("set -C\necho hi > {spelling}\necho \"rc:$?\"\n");
        let (file, err, _rc) = run_script_stdout_file(tag, &script);
        assert_eq!(
            refusal_count(&err),
            1,
            "{spelling}: refusal must be reported exactly once, stderr: {err}"
        );
        assert!(
            err.contains(&format!("{spelling}: cannot overwrite existing file")),
            "{spelling}: refusal names the spelling, stderr: {err}"
        );
        assert_eq!(
            file, "rc:1\n",
            "{spelling}: only the following echo's line lands in the file, file: {file:?}"
        );
    }

    // The /dev/stderr spelling under the stderr-file harness: the
    // refusal diagnostic is written to fd 2 — the file itself — and
    // stdout carries only the status line (GNU 5.3.20 oracle: same
    // shape; with stderr a PIPE the spelling is writable instead,
    // which is why this sub-case cannot share the harness above).
    let (err_file, out, _rc) = run_script_stderr_file(
        "nc-stderr",
        "set -C\necho hi > /dev/stderr\necho \"rc:$?\"\n",
    );
    assert_eq!(
        refusal_count(&err_file),
        1,
        "/dev/stderr: refusal must be reported exactly once, stderr file: {err_file}"
    );
    assert!(
        err_file.contains("/dev/stderr: cannot overwrite existing file"),
        "/dev/stderr: refusal names the spelling, stderr file: {err_file}"
    );
    assert_eq!(
        out, "rc:1\n",
        "/dev/stderr: only the status line reaches stdout, stdout: {out:?}"
    );
}

/// (f) The numbered-fd twin: `echo > /dev/fd/3` with fd 3 on a regular
/// file refuses under `set -C`, reported exactly once, file untouched.
#[test]
fn issue2359_noclobber_refuses_dev_fd3_once() {
    let (out, err, _rc) = run_script(
        "nc-fd3",
        "exec 3> out.txt\nset -C\necho via3 > /dev/fd/3\necho \"rc:$?\"\nexec 3>&-\necho \"file:[$(cat out.txt)]\"\n",
    );
    assert_eq!(
        refusal_count(&err),
        1,
        "refusal must be reported exactly once, stderr: {err}"
    );
    assert!(
        out.contains("rc:1"),
        "redirect failed with status 1, stdout: {out}"
    );
    assert!(
        out.contains("file:[]"),
        "the refused write never lands, stdout: {out}"
    );
}

/// (e) The same spellings stay writable when the stdio fd is a pipe:
/// noclobber_open's stat sees a non-regular target and opens.
#[test]
fn issue2359_noclobber_allows_stdio_spelling_to_pipe() {
    let (out, err, _rc) = run_script(
        "nc-stdout-pipe",
        "set -C\necho via-stdout > /dev/stdout\necho \"rc:$?\"\n",
    );
    assert!(
        out.contains("via-stdout"),
        "pipe-backed /dev/stdout stays writable, stdout: {out}"
    );
    assert!(
        out.contains("rc:0"),
        "no failure, stdout: {out} stderr: {err}"
    );
    assert_eq!(
        refusal_count(&err),
        0,
        "no refusal against a pipe, stderr: {err}"
    );
}

/// Pipeline stage context: the element's fd 1 is the stage pipe, so
/// `> /dev/fd/1` inside a stage is judged against the pipe even when
/// the shell's own stdout is a regular file — allowed, bytes flow.
#[test]
fn issue2359_pipeline_stage_dev_fd1_names_the_pipe() {
    let (out, err, _rc) = run_script(
        "stage-fd1",
        "set -C\necho piped > /dev/fd/1 | cat\necho \"rc:$?\"\n",
    );
    assert!(
        out.contains("piped"),
        "stage bytes must flow through the pipe, stdout: {out}"
    );
    assert!(out.contains("rc:0"), "pipeline succeeds, stdout: {out}");
    assert_eq!(
        refusal_count(&err),
        0,
        "no noclobber refusal against the stage pipe, stderr: {err}"
    );
}

/// Pipeline stage with a numbered fd on a regular file: the refusal
/// still lands exactly once and the stage produces nothing.
#[test]
fn issue2359_pipeline_stage_dev_fd3_refuses_once() {
    let (out, err, _rc) = run_script(
        "stage-fd3",
        "exec 3> out.txt\nset -C\necho via3 > /dev/fd/3 | cat\necho \"rc:$?\"\nexec 3>&-\necho \"file:[$(cat out.txt)]\"\n",
    );
    assert_eq!(
        refusal_count(&err),
        1,
        "stage refusal must be reported exactly once, stderr: {err}"
    );
    assert!(
        out.contains("file:[]"),
        "the refused stage write never lands, stdout: {out}"
    );
}

/// (g) A spelling-open of a regular-file fd is a FRESH open: each
/// redirect truncates, so sequential writes replace, not append at
/// the aliased descriptor's offset.
#[test]
fn issue2359_stdio_spelling_open_truncates_fresh() {
    let (file, err, _rc) = run_script_stdout_file(
        "stdout-fresh-open",
        "echo one > /dev/stdout\necho two > /dev/stdout\n",
    );
    assert_eq!(
        file, "two\n",
        "each spelling-open truncates fresh, file: {file:?} stderr: {err}"
    );
}

/// The `$(<file)` cat shortcut is a plain open (evalstring.c:781):
/// a net spelling fails ENOENT even with a live server answering,
/// for TCP and UDP alike — it must never connect.
#[test]
fn issue2359_comsub_cat_shortcut_net_spelling_is_a_plain_open() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(b"BANNER\n");
            let _ = stream.flush();
        }
    });
    let script = format!("x=$(</dev/tcp/127.0.0.1/{port})\necho \"rc:$? len:${{#x}}\"\n");
    let (out, err, _rc) = run_script("comsub-tcp-plain-open", &script);
    assert!(
        out.contains("rc:1 len:0"),
        "cat shortcut must fail, not read the banner, stdout: {out}"
    );
    assert!(
        err.contains(&format!(
            "/dev/tcp/127.0.0.1/{port}: No such file or directory"
        )),
        "failure is the plain open's ENOENT, stderr: {err}"
    );

    let script = format!("x=$(</dev/udp/127.0.0.1/{port})\necho \"rc:$? len:${{#x}}\"\n");
    let (out, err, _rc) = run_script("comsub-udp-plain-open", &script);
    assert!(
        out.contains("rc:1 len:0"),
        "UDP cat shortcut must fail fast (it used to block), stdout: {out}"
    );
    assert!(
        err.contains(&format!(
            "/dev/udp/127.0.0.1/{port}: No such file or directory"
        )),
        "UDP failure is the plain open's ENOENT, stderr: {err}"
    );
}

/// A brace group whose redirect open is refused keeps the failure:
/// status 1, `&&` short-circuits, `||` runs its right-hand side, and
/// `set -e` exits — the group is not silently re-dispatched to a
/// success (the failure used to surface with status 0).
#[test]
fn issue2359_brace_group_redirect_failure_keeps_status() {
    let script = "set -C\n{ echo A; } > /dev/stdout\necho \"grp:$?\"\n{ echo B; } > /dev/stdout && echo AND-RAN\n{ echo C; } > /dev/stdout || echo OR-RAN\n";
    let (file, err, _rc) = run_script_stdout_file("grp-nc-status", script);
    assert_eq!(
        file, "grp:1\nOR-RAN\n",
        "group failure is status 1 and drives the lists, file: {file:?} stderr: {err}"
    );
    assert_eq!(
        refusal_count(&err),
        3,
        "each refused group reports exactly once, stderr: {err}"
    );

    let script = "set -Ce\necho before\n{ echo A; } > /dev/stdout\necho after\n";
    let (file, err, rc) = run_script_stdout_file("grp-nc-errexit", script);
    assert_eq!(
        file, "before\n",
        "errexit stops the list at the refused group, file: {file:?} stderr: {err}"
    );
    assert_eq!(rc, Some(1), "shell exits with the group's failure status");
    assert_eq!(refusal_count(&err), 1, "one refusal, stderr: {err}");
}

/// Writing through /dev/stdin (or /dev/fd/0) with stdin bound to the
/// null device: GNU opens the device fresh and the write succeeds —
/// the dup of a read endpoint failed EBADF at the builtin's flush.
#[test]
fn issue2359_dev_stdin_write_to_null_device_succeeds() {
    let (out, err, _rc) = run_script(
        "stdin-null-write",
        "echo x > /dev/stdin\necho \"stdin:$?\"\necho x > /dev/fd/0\necho \"fd0:$?\"\n{ echo y; } > /dev/stdin\necho \"grp:$?\"\n",
    );
    assert!(
        out.contains("stdin:0") && out.contains("fd0:0") && out.contains("grp:0"),
        "device opens succeed on every face, stdout: {out} stderr: {err}"
    );
    assert!(
        !err.contains("Bad file descriptor"),
        "no EBADF write error, stderr: {err}"
    );
}

/// The stage refusal also lands for an APPEND-mode numbered fd:
/// the alias open is a fresh open regardless of the source's mode.
#[test]
fn issue2359_pipeline_stage_append_fd3_refuses_once() {
    let (out, err, _rc) = run_script(
        "stage-append-fd3",
        "printf 'orig\\n' > out.txt\nexec 3>> out.txt\nset -C\necho via3 > /dev/fd/3 | cat\necho \"stage:${PIPESTATUS[0]}\"\nexec 3>&-\necho \"file:[$(cat out.txt)]\"\n",
    );
    assert_eq!(
        refusal_count(&err),
        1,
        "stage refusal must be reported exactly once, stderr: {err}"
    );
    assert!(
        out.contains("stage:1"),
        "the stage fails with status 1, stdout: {out}"
    );
    assert!(
        out.contains("file:[orig]"),
        "the refused stage write never lands, stdout: {out}"
    );
}

/// Twin of [`run_script`] with the shell's stdin bound to a regular
/// file. Returns (stdout, stderr, status, stdin file content after).
fn run_script_stdin_file(
    tag: &str,
    script: &str,
    stdin_content: &str,
) -> (String, String, Option<i32>, String) {
    let (tx, rx) = mpsc::channel();
    let script = script.to_string();
    let stdin_content = stdin_content.to_string();
    let panic_tag = tag.to_string();
    let tag = tag.to_string();
    std::thread::spawn(move || {
        let dir = std::env::temp_dir().join(format!("rubash2359-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let case = dir.join("case.sh");
        std::fs::write(&case, &script).expect("write fixture");
        let feed = dir.join("feed.txt");
        std::fs::write(&feed, &stdin_content).expect("write stdin fixture");
        let feed_file = std::fs::File::open(&feed).expect("open stdin fixture");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .current_dir(&dir)
            .arg(&case)
            .stdin(Stdio::from(feed_file))
            .output()
            .expect("run rubash");
        let after = std::fs::read_to_string(&feed).expect("read stdin fixture after");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = tx.send((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
            after,
        ));
    });
    rx.recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("{panic_tag}: rubash run timed out"))
}

/// A numbered fd DUP'd from stdout (`exec 3>&1`) names stdout's open
/// file description: inside a pipeline stage, `> /dev/fd/3` with
/// stdout a regular file takes the same noclobber refusal as
/// /dev/stdout itself — the stage pipe is not the target.
#[test]
fn issue2359_pipeline_stage_stdout_dup_fd3_refuses_once() {
    let (file, err, _rc) = run_script_stdout_file(
        "stage-stdout-dup-fd3",
        "exec 3>&1\nset -C\necho via3 > /dev/fd/3 | cat\necho \"stage:$?\"\n",
    );
    assert_eq!(
        refusal_count(&err),
        1,
        "refusal must be reported exactly once, stderr: {err}"
    );
    assert_eq!(
        file, "stage:0\n",
        "the refused stage write never reaches the pipe, file: {file:?}"
    );
}

/// A numbered fd DUP'd from stdin (`exec 3<&0`) names stdin's open
/// file description: `> /dev/fd/3` with stdin a regular file opens
/// that file fresh for writing, like /dev/stdin itself.
#[test]
fn issue2359_stdin_dup_fd3_write_reopens_regular_file() {
    let (out, err, _rc, after) = run_script_stdin_file(
        "stdin-dup-fd3-write",
        "exec 3<&0\necho via3 > /dev/fd/3\necho \"w3:$?\"\ncat < /dev/fd/3\necho \"r3:$?\"\n",
        "seed\n",
    );
    assert!(
        out.contains("w3:0") && out.contains("r3:0"),
        "write and read through the dup both succeed, stdout: {out} stderr: {err}"
    );
    assert!(
        out.contains("via3"),
        "the fresh write reads back, stdout: {out}"
    );
    assert_eq!(
        after, "via3\n",
        "the stdin file is rewritten, after: {after:?}"
    );
}

/// `> /dev/stdin` with stdin a regular file is a fresh open of that
/// file: it truncates and replaces the content (GNU, oracle probe).
#[test]
fn issue2359_stdin_spelling_write_reopens_regular_file() {
    let (out, err, _rc, after) = run_script_stdin_file(
        "stdin-file-write",
        "echo via-stdin-spelling > /dev/stdin\necho \"rc:$?\"\n",
        "seed-data\n",
    );
    assert!(
        out.contains("rc:0"),
        "write succeeds, stdout: {out} stderr: {err}"
    );
    assert_eq!(
        after, "via-stdin-spelling\n",
        "the stdin file is truncated and rewritten, after: {after:?}"
    );
}
