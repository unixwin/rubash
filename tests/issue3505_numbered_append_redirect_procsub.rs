//! Issue unixwin/rubash#3505: `N>>` append redirect rejected when its
//! target is a process substitution after a compound command.
//!
//! Narrowing (reproduced on master be610327): the issue body's simple
//! example does NOT fail — `echo hello 3>>/tmp/f` and `true 3>>fe` both
//! parse and run. The failing face is strictly `N>> >(...)` trailing a
//! compound command:
//!
//! ```sh
//! { echo X >&3; } 3>> >(wc -c > fa)
//! # GNU (oracle bash 5.3.20, per the issue): fa is `2`
//! # rubash: rc=2, `syntax error near unexpected token '3>>'`
//! ```
//!
//! Root cause (not the lexer, despite the issue title): the lexer
//! produces the correct `Append("3>>")` token, and `>(...)` after a
//! redirect operator lexes as `RedirectOut(">")` + `Keyword("(")` — a
//! split procsub target. `collect_trailing_redirections`
//! (src/parser/redirections.rs) is the shared collector for redirects
//! trailing compound commands (`}` / `)` / `fi` / ...). Its
//! `RedirectOut` arm tries both
//! `combined_process_substitution_redirect_target` (`&>`/`&>>`) and
//! `output_process_substitution_redirect_target` (`N>` + `>` + `(`),
//! which is why `{ ...; } 3> >(...)` works — but its `Append` arm
//! tried ONLY the `combined_...` intake (which requires `&>`/`&>>`),
//! so `3>>` fell through to the generic target lookup, which cannot
//! take `>` as a target, and the `Append` token was left unconsumed.
//!
//! The fix adds the missing `append_process_substitution_redirect_target`
//! arm to that `Append` arm, mirroring the `assign_append_target`
//! simple-command path (src/parser/redirect_assign.rs), which is why
//! `echo X 3>> >(...)` already parsed. No lexer change.
//!
//! Harness note: same as tests/issue3491_*: the shell runs as a
//! subprocess, file sinks are polled to a deadline, stdout pins `wait`
//! for the detached consumers.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3505-{tag}-{}", std::process::id()));
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

// ---------------- #3505: the F1 face ----------------

#[test]
fn issue3505_brace_numbered_append_delivers_to_file() {
    // The issue's F1, verbatim: the brace group's fd 3 is bound to
    // the substitution, `echo X >&3` writes 2 bytes, `wc -c` counts
    // them into fa. Pre-fix this was a parse error (rc=2, `syntax
    // error near unexpected token '3>>'`). GNU fa is `2`.
    let dir = test_dir("f1");
    let fa = dir.join("fa");
    let script = format!(
        "{{ echo X >&3; }} 3>> >(wc -c > {})\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
    assert!(
        !std::fs::read_dir(&dir)
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
fn issue3505_brace_bare_append_delivers_to_file() {
    // Bare `>>` after a group: the fd-1 face of the same collector
    // arm (the intake accepts an empty digit prefix too). GNU fa
    // is `2`.
    let dir = test_dir("bare");
    let fa = dir.join("fa");
    let script = format!(
        "{{ printf hi; }} >> >(wc -c > {})\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue3505_brace_numbered_append_preserves_plain_file() {
    // The plain-file append face: `3>>` after a group must open
    // with O_APPEND, so a pre-seeded file keeps its content and
    // gains the new bytes. GNU: `seed\nX\n`.
    let dir = test_dir("plain");
    let plain = dir.join("plain.txt");
    std::fs::write(&plain, "seed\n").expect("seed plain file");
    let script = format!("{{ echo X >&3; }} 3>> {}\nsleep 0.5\n", plain.display());
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        std::fs::read_to_string(&plain).expect("read plain file"),
        "seed\nX\n"
    );
}

#[test]
fn issue3505_subshell_numbered_append_delivers() {
    // The same shape on a subshell — the subshell wrapper shares
    // the trailing-redirect collector. GNU prints `2`.
    let script = "(echo X >&3) 3>> >(wc -c)\nwait\n";
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "2\n", "stderr: {stderr}");
}

#[test]
fn issue3505_if_numbered_append_delivers() {
    // An `if` compound with a numbered-append output procsub.
    // GNU fa is `2`.
    let dir = test_dir("if");
    let fa = dir.join("fa");
    let script = format!(
        "if true; then echo X >&3; fi 3>> >(wc -c > {})\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue3505_multidigit_numbered_append_delivers() {
    // Multi-digit fd: `10>>` goes through `finish_number_token`
    // rather than the single-digit scanner arms. GNU fa is `2`.
    let dir = test_dir("fd10");
    let fa = dir.join("fa");
    let script = format!(
        "{{ echo X >&10; }} 10>> >(wc -c > {})\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

// ---------------- controls (already green, must not regress) ----------------

#[test]
fn issue3505_simple_numbered_append_still_delivers() {
    // The simple-command path already had the intake
    // (`assign_append_target`); it must keep working. `echo` writes
    // fd 1, so the `>&3` suffix (after the `3>>` bind) dups stdout
    // onto the bound fd 3 — the `>&3`-first order is wrong, GNU
    // reports `3: Bad file descriptor` for it (redirects run
    // left-to-right, fd 3 is not yet bound). GNU fa is `2`, and the
    // base binary already delivers it: this is a control, green on
    // base and head. (The first draft of this test omitted the
    // `>&3` and expected `2` for a bare `echo X 3>> ...`; GNU gives
    // `0` for that form because nothing ever writes fd 3.)
    let dir = test_dir("simple");
    let fa = dir.join("fa");
    let script = format!("echo X 3>> >(wc -c > {}) >&3\nsleep 0.5\n", fa.display());
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue3505_brace_numbered_truncate_still_delivers() {
    // The `3> >(...)` Q1 control from #3491: the neighboring
    // `RedirectOut` arm must not regress. GNU fa is `2`.
    let dir = test_dir("trunc");
    let fa = dir.join("fa");
    let script = format!(
        "{{ echo X >&3; }} 3> >(wc -c > {})\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "2\n");
    assert_eq!(content, "2\n");
}

#[test]
fn issue3505_brace_stderr_append_still_delivers() {
    // The `2>> >(...)` control: the neighboring `RedirectErrAppend`
    // arm already handled the stderr procsub. `oops\n` is 5 bytes,
    // so GNU fa is `5`.
    let dir = test_dir("err");
    let fa = dir.join("fa");
    let script = format!(
        "{{ echo oops >&2; }} 2>> >(wc -c > {})\nsleep 0.5\n",
        fa.display()
    );
    let (_stdout, stderr, code) = rubash_stdin(&script);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let content = wait_file_content(&fa, "5\n");
    assert_eq!(content, "5\n");
}
