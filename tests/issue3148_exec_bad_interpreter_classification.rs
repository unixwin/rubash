//! rubash#3148 — the `exec` builtin's spawn-failure report must run
//! the shell_execve classification, not dump the raw OS error.
//!
//! GNU anchor (execute_cmd.c shell_execve, :6136-6202): after a failed
//! execve (non-ENOEXEC), `:6143` sets the status switch
//! (ENOENT -> EX_NOTFOUND 127, else EX_NOEXEC 126), then:
//!   - file missing or not executable -> `:6157` file_error: a
//!     `name: line N: <fullpath>: <strerror>` line (WITH line segment);
//!   - executable with a `#!` line whose interpreter is missing ->
//!     `:6184` sys_error `"%s: %s: %s", command, interp,
//!     "bad interpreter"` — NO line segment, status 126; the
//!     interpreter name ends at the first blank/tab (getinterp) and a
//!     trailing CR displays as `^M`;
//!   - executable, no `#!`, ENOENT (missing PT_INTERP loader) ->
//!     `:6192` internal_error "cannot execute: required file not
//!     found", status 127.
//! The reported file name is exec.def:172 `full_pathname (command)` —
//! the cwd-joined absolute form, never the as-written `./x` join and
//! never with `/./` residue. A PATH search failure never reaches
//! shell_execve: it is the builtin_error shape
//! `name: line N: exec: <word>: not found`, status 127 — the `exec:`
//! infix belongs ONLY to that arm.
//!
//! rubash face at the pin's base: `run_external_exec` /
//! `run_external_exec_with_stdio` (src/builtins/exec.rs) print
//! `{diagnostic_prefix}exec: {program}: {raw io::Error}` for every
//! spawn failure — line segment + `exec:` infix + anchored name with
//! `/./` + unclassified OS text. The classification itself already
//! exists for the ordinary invocation face
//! (`spawn_error_diagnostic`, src/executor/external_inner.rs).
//!
//! Harness: script files run with cwd = a fresh fixture dir,
//! `LC_ALL=C.UTF-8` pinned on the child; expectations byte-exact from
//! GNU bash 5.3.20 probes (2026-11-09). Unix-only fixtures.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "rubash-3148-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join("sub")).expect("fixture sub");
        for (rel, body, mode) in [
            ("badinterp", "#!/nonexistent-interp\necho never\n", 0o755),
            (
                "sub/badinterp2",
                "#!/nonexistent-interp2\necho never\n",
                0o755,
            ),
            (
                "badinterp_arg",
                "#!/nonexistent-interp -x\necho never\n",
                0o755,
            ),
            (
                "badinterp_crlf",
                "#!/nonexistent-interp\r\necho never\n",
                0o755,
            ),
            ("textscript", "echo ran-text\n", 0o755),
            ("goodscript", "#!/bin/sh\necho ran-good\n", 0o755),
            ("notexec", "#!/nonexistent-interp\necho never\n", 0o644),
        ] {
            let path = dir.join(rel);
            std::fs::write(&path, body).expect("fixture script");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .expect("fixture chmod");
        }
        Fixture { dir }
    }

    fn run(&self, script: &str) -> Outcome {
        std::fs::write(self.dir.join("case.sh"), script).expect("write case.sh");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("case.sh")
            .current_dir(&self.dir)
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn rubash");
        Outcome {
            stdout: output.stdout,
            stderr: output.stderr,
            code: output.status.code().unwrap_or(-1),
        }
    }

    fn display(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn expect(script: &str, want_stdout: &str, want_stderr: &str, want_code: i32, ctx: &str) {
    let fix = Fixture::new(ctx);
    let out = fix.run(script);
    assert_eq!(
        out.stdout,
        want_stdout.replace("@FIX@", &fix.display()).as_bytes(),
        "{ctx}: stdout mismatch; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stderr,
        want_stderr.replace("@FIX@", &fix.display()).as_bytes(),
        "{ctx}: stderr mismatch; stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        out.code,
        want_code,
        "{ctx}: rc mismatch; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The issue's headline shape: bad interpreter via `exec`, sys_error
/// form (no line segment, no `exec:` infix), shell exits 126 and the
/// rest of the script never runs.
#[test]
fn bad_interpreter_line1() {
    expect(
        "exec ./badinterp\necho AFTER\n",
        "",
        "case.sh: @FIX@/badinterp: /nonexistent-interp: bad interpreter: \
         No such file or directory\n",
        126,
        "badinterp-line1",
    );
}

/// The sys_error prolog never gains a line segment, wherever the
/// `exec` sits in the script; earlier commands' output stands.
#[test]
fn bad_interpreter_no_line_segment_at_line4() {
    expect(
        "echo one\necho two\necho three\nexec ./badinterp\n",
        "one\ntwo\nthree\n",
        "case.sh: @FIX@/badinterp: /nonexistent-interp: bad interpreter: \
         No such file or directory\n",
        126,
        "badinterp-line4",
    );
}

/// The report name is full_pathname: cwd-joined for a subdirectory
/// target too, with no `/./` residue.
#[test]
fn bad_interpreter_subdir_report_name() {
    expect(
        "exec sub/badinterp2\n",
        "",
        "case.sh: @FIX@/sub/badinterp2: /nonexistent-interp2: \
         bad interpreter: No such file or directory\n",
        126,
        "badinterp-subdir",
    );
}

/// A target found by the PATH walk reports the same full_pathname.
#[test]
fn bad_interpreter_via_path_walk() {
    expect(
        "PATH=.:/nonexistent3148\nexec badinterp\n",
        "",
        "case.sh: @FIX@/badinterp: /nonexistent-interp: bad interpreter: \
         No such file or directory\n",
        126,
        "badinterp-pathwalk",
    );
}

/// Missing file: the file_error arm — WITH line segment, no `exec:`
/// infix, full_pathname name, status 127.
#[test]
fn missing_file_file_error_shape() {
    expect(
        "exec ./nonexistent\n",
        "",
        "case.sh: line 1: @FIX@/nonexistent: No such file or directory\n",
        127,
        "missing-file",
    );
}

/// Not-executable file: file_error arm again, EACCES text, status 126.
#[test]
fn not_executable_file_error_shape() {
    expect(
        "exec ./notexec\n",
        "",
        "case.sh: line 1: @FIX@/notexec: Permission denied\n",
        126,
        "notexec",
    );
}

/// ENOEXEC (no `#!`): the shell falls back to interpreting the file.
#[test]
fn enoexec_text_fallback() {
    expect(
        "exec ./textscript\necho AFTER\n",
        "ran-text\n",
        "",
        0,
        "enoexec",
    );
}

/// getinterp ends the interpreter name at the first blank.
#[test]
fn interp_arg_truncated_at_blank() {
    expect(
        "exec ./badinterp_arg\n",
        "",
        "case.sh: @FIX@/badinterp_arg: /nonexistent-interp: \
         bad interpreter: No such file or directory\n",
        126,
        "interp-arg",
    );
}

/// A trailing CR in the interpreter name displays as `^M`.
#[test]
fn interp_crlf_caret_m() {
    expect(
        "exec ./badinterp_crlf\n",
        "",
        "case.sh: @FIX@/badinterp_crlf: /nonexistent-interp^M: \
         bad interpreter: No such file or directory\n",
        126,
        "interp-crlf",
    );
}

/// Control: PATH search failure keeps the builtin_error shape —
/// the `exec:` infix lives only here.
#[test]
fn search_failure_control() {
    expect(
        "exec nosuchcmd3148\n",
        "",
        "case.sh: line 1: exec: nosuchcmd3148: not found\n",
        127,
        "search-failure",
    );
}

/// Chain: an `exec` failure is fatal even inside a `||` list — the
/// shell exits 126, neither the handler nor later lines run.
#[test]
fn or_list_does_not_catch_exec_failure() {
    expect(
        "exec ./badinterp || echo caught\necho AFTER\n",
        "",
        "case.sh: @FIX@/badinterp: /nonexistent-interp: bad interpreter: \
         No such file or directory\n",
        126,
        "or-list",
    );
}

/// Chain: inside a subshell the failure kills only the subshell; the
/// parent observes status 126 and continues.
#[test]
fn subshell_exec_failure_propagates_status() {
    expect(
        "(exec ./badinterp); echo rc=$?\necho DONE\n",
        "rc=126\nDONE\n",
        "case.sh: @FIX@/badinterp: /nonexistent-interp: bad interpreter: \
         No such file or directory\n",
        0,
        "subshell",
    );
}

/// Control: a working target replaces the shell and runs.
#[test]
fn good_script_control() {
    expect(
        "exec ./goodscript\necho AFTER\n",
        "ran-good\n",
        "",
        0,
        "good-script",
    );
}

/// The `-c` form: the sys_error prolog is the shell's $0 (the path it
/// was invoked by), still with no line segment.
#[test]
fn dash_c_form_prolog_is_zero() {
    let fix = Fixture::new("dash-c");
    let bin = env!("CARGO_BIN_EXE_rubash");
    let output = Command::new(bin)
        .args(["-c", "exec ./badinterp"])
        .current_dir(&fix.dir)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash -c");
    let want = format!(
        "{bin}: {}/badinterp: /nonexistent-interp: bad interpreter: \
         No such file or directory\n",
        fix.display()
    );
    assert_eq!(
        output.stderr,
        want.as_bytes(),
        "dash-c: stderr mismatch; stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stdout.is_empty(), "dash-c: stdout must be empty");
    assert_eq!(output.status.code().unwrap_or(-1), 126, "dash-c: rc");
}
