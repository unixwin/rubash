//! Issue unixwin/rubash#3491: numbered-fd (>=3) output process
//! substitution `N> >(...)` delivers nothing / `Bad file descriptor`.
//!
//! Split from #2234 (whose PR #3490 fixed only the fd-1/2 chain on
//! the simple-command path): the fd>=3 faces have their own root
//! causes, all grounded in GNU's one ordered walk —
//! `do_redirections` (redir.c:237) walks the owning command's
//! redirect list in order, and `process_substitute` (subst.c:6362)
//! forks each consumer at its own entry's turn, so the consumer
//! inherits the fd table exactly as the entries BEFORE its own left
//! it, and the owning command's fd N is bound to the substitution's
//! pipe at that same turn.
//!
//! The three defects, all reproduced on master be610327 (GNU 5.2.21
//! oracle, same shapes as the issue; the box three-way is in the
//! lane REPORT):
//!
//! (A) Compound commands never materialize a list-only output
//! procsub: `compound_redirect_needs_procsub_rewrite` only scans
//! the stdio mirror fields (`redirect_out`/`append`/`redirect_err`/
//! `redirect_err_append`), but the parser keeps every fd>=3 entry
//! — and any fd-1/2 entry that loses the mirror race to a later
//! same-fd redirect — solely in the ordered `redirects` list
//! (nodes.rs `is_list_only_redirect`). The unmaterialized `>(...)`
//! target reached the compound fd walk verbatim, which opened a
//! LITERAL file named `>(...)` and never ran the consumer:
//! `{ echo X >&3; } 3> >(wc -c > fa)` left `fa` empty and a stray
//! `>(wc -c > fa)` file behind; the first link of a compound fd-1
//! chain (`{ printf hi; } > >(wc -c > f1) > >(wc -c > f2)`)
//! never ran either (GNU f1 `0`, rubash f1 missing).
//!
//! (B) Compound consumers run without their prefix fd bindings:
//! the persistent (in-process) finish runs each consumer after the
//! compound's fd scope has been restored, so a body `>&3` fails
//! EBADF even though GNU's fork inherited fd 3 bound to the
//! earlier substitution's pipe:
//! `{ echo X >&3; printf hi; } 3> >(wc -c) > >(wc -c >&3)`
//! reported `3: Bad file descriptor` where GNU prints `4`.
//!
//! (C) The simple-command live-link pre-scan (#2234,
//! `finish_process_substitutions`) only links fds 1/2, so an
//! fd>=3 chain races the same way #2234 fixed for fd 1/2: the
//! reader's input is snapshotted before the writer appends:
//! `printf hi 3> >(wc -c) > >(wc -c >&3)` printed `0` where GNU
//! prints `2`.
//!
//! The fix (see REPORT.md in the lane workdir): (A) materialize
//! list-only output procsubs in
//! `materialize_compound_output_process_substitutions`, (B) run
//! each persistent compound consumer under its redirect-prefix fd
//! bindings, in writer-before-reader order, (C) extend the live
//! link to fd>=3 `CarrierWrite` dispositions.
//!
//! Harness note: same as tests/issue2234_*: the shell runs as a
//! subprocess, file sinks are polled to a deadline, stdout pins
//! `wait` for the detached consumers. No pin depends on fd 9 or
//! on the test process's own fd table shape.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3491-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let mut out = child.stdout.take().expect("stdout pipe");
    let mut err = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let status = child.wait().expect("wait rubash");
    (
        out_thread.join().unwrap_or_default(),
        err_thread.join().unwrap_or_default(),
        status.code(),
    )
}

/// Spell a path so it can be interpolated into a script on any host.
/// Windows paths carry `\` separators, and an unquoted `\U` / `\r` /
/// ... in a shell word is an escape to the parser, so the sink file is
/// never created. POSIX separators are inert for both parsers; on
/// Linux/macOS this is a no-op.
fn posix_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Poll a file until its content equals `expected` or the deadline
/// passes; returns the last observed content.
fn wait_file_content(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = String::new();
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(path) {
            last = content;
            if last == expected {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    last
}

// ---------------- #3491 (A): compound materialization ----------------

#[test]
fn issue3491_brace_numbered_fd_delivers_to_file() {
    // The issue's Q1, verbatim: the brace group's fd 3 is bound to
    // the substitution, `echo X >&3` writes 2 bytes, `wc -c` counts
    // them into fa. Pre-fix fa stayed empty and a literal file named
    // `>(wc -c > fa)` appeared in the cwd.
    let dir = test_dir("q1");
    let fa = dir.join("fa");
    let script = format!(
        "{{ echo X >&3; }} 3> >(wc -c > {})\nsleep 0.5\n",
        posix_path(&fa)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
    assert!(
        !dir.join(">(wc -c > fa)").exists()
            && !std::fs::read_dir(&dir)
                .expect("read test dir")
                .any(|entry| entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(">(")),
        "no literal >(...) file may remain in {:?}",
        dir
    );
}

#[test]
fn issue3491_brace_numbered_fd_cat_delivers() {
    // Q1b: the consumer is `cat > fb`, so the bytes arrive verbatim.
    let dir = test_dir("q1b");
    let fb = dir.join("fb");
    let script = format!(
        "{{ echo AB >&3; }} 3> >(cat > {})\nsleep 0.5\n",
        posix_path(&fb)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fb, "AB\n");
    assert_eq!(content, "AB\n");
}

#[test]
fn issue3491_subshell_numbered_fd_delivers() {
    // S3: the same shape on a subshell — the subshell wrapper shares
    // the compound materialization gate. GNU prints `2`.
    let script = "(echo X >&3) 3> >(wc -c)\nwait\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
}

#[test]
fn issue3491_if_numbered_fd_delivers() {
    // G1: an `if` compound with a numbered-fd output procsub.
    let dir = test_dir("g1");
    let fa = dir.join("fa");
    let script = format!(
        "if true; then echo X >&3; fi 3> >(wc -c > {})\nsleep 0.5\n",
        posix_path(&fa)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue3491_loop_numbered_fd_delivers() {
    // H1: a `while read` loop with a numbered-fd output procsub;
    // `hello\n` is 6 bytes.
    let dir = test_dir("h1");
    let fa = dir.join("fa");
    let script = format!(
        "while read -r l; do echo \"$l\" >&3; done <<< hello 3> >(wc -c > {})\nsleep 0.5\n",
        posix_path(&fa)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "6\n");
    assert_eq!(content, "6\n");
}

#[test]
fn issue3491_compound_fd1_chain_first_link_runs() {
    // K1: the fd-1 face of the same list-only gap — the FIRST of two
    // same-fd redirects loses the mirror race, so only the second
    // consumer was materialized. GNU: f1 `0` (the second consumer's
    // body writes to a file, so the first sees EOF) and f2 `2`.
    // Pre-fix f1 never appeared at all (literal `>(...)` file).
    let dir = test_dir("k1");
    let f1 = dir.join("f1");
    let f2 = dir.join("f2");
    let script = format!(
        "{{ printf hi; }} > >(wc -c > {}) > >(wc -c > {})\nsleep 0.5\n",
        posix_path(&f1),
        posix_path(&f2)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(wait_file_content(&f1, "0\n"), "0\n");
    assert_eq!(wait_file_content(&f2, "2\n"), "2\n");
}

// ---------------- #3491 (B): persistent prefix bindings ----------------

#[test]
fn issue3491_brace_numbered_fd_chain_counts_four() {
    // The issue's Q2, verbatim: the second consumer's `>&3` writes
    // its `2\n` into the first consumer's stdin, which already holds
    // `X\n` — GNU prints `4`. Pre-fix the persistent finish ran the
    // body after the compound's fd scope had been restored, so the
    // `>&3` failed with `3: Bad file descriptor`.
    let script = "{ echo X >&3; printf hi; } 3> >(wc -c) > >(wc -c >&3)\nwait\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "4\n", "stderr: {stderr}");
}

#[test]
fn issue3491_brace_numbered_fd_chain_file_sink_counts_four() {
    // K7: Q2 with the reader's count in a file, so the pin does not
    // depend on stdout plumbing. GNU f1 is `4`.
    let dir = test_dir("k7");
    let f1 = dir.join("f1");
    let script = format!(
        "{{ echo X >&3; printf hi; }} 3> >(wc -c > {}) > >(wc -c >&3)\nsleep 0.5\n",
        posix_path(&f1)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&f1, "4\n");
    assert_eq!(content, "4\n");
}

#[test]
fn issue3491_brace_fd1_chain_counts_two() {
    // C1: the fd-1 face of the same persistent-prefix mechanism —
    // the second consumer's `>&1` writes its `2\n` into the first
    // consumer's stdin, which otherwise sees only the body's `hi`
    // through the second pipe... precisely: the first consumer sees
    // only the second consumer's `2\n`, so GNU prints `2`. (Pre-fix
    // the first consumer never ran and the second printed its `2`
    // straight to stdout — same digit, wrong mechanism — so this pin
    // also asserts the first consumer ran, via the file sink.)
    let dir = test_dir("c1");
    let f1 = dir.join("f1");
    let script = format!(
        "{{ printf hi; }} > >(wc -c > {}) > >(wc -c >&1)\nsleep 0.5\n",
        posix_path(&f1)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&f1, "2\n");
    assert_eq!(content, "2\n");
}

// ---------------- #3491 (C): simple-path live link, fd>=3 ----------------

#[test]
fn issue3491_simple_numbered_fd_chain_counts_two() {
    // C3: the simple-command fd>=3 chain — the #2234 live link only
    // covered fds 1/2, so the reader's input was snapshotted before
    // the writer appended and rubash printed `0`. GNU prints `2`.
    let script = "printf hi 3> >(wc -c) > >(wc -c >&3)\nwait\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
}

// ---------------- controls (already green, must not regress) ----------------

#[test]
fn issue3491_simple_numbered_fd_still_delivers() {
    // S1 control: the simple command's numbered-fd delivery already
    // worked (its materialization covers the full redirect list).
    let script = "echo X 3> >(wc -c) >&3\nwait\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
}

#[test]
fn issue3491_pipeline_stage_brace_still_delivers() {
    // Q1c control: a brace group as a pipeline stage already
    // delivered (the stage executor materializes it). GNU prints `3`.
    let script = "echo hi | { cat >&3; } 3> >(wc -c)\nwait\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "3\n", "stderr: {stderr}");
}

#[test]
fn issue3491_exec_persistent_fd_still_delivers() {
    // E1 control: the `exec 3> >(...)` persistent face already
    // delivered through the exec-time opener.
    let dir = test_dir("e1");
    let fe = dir.join("fe");
    let script = format!(
        "exec 3> >(cat > {})\necho hi >&3\nexec 3>&-\nsleep 0.5\n",
        posix_path(&fe)
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fe, "hi\n");
    assert_eq!(content, "hi\n");
}
