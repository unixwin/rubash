//! rubash#3046 — `exec` in a pipeline/subshell position loses its
//! redirections: stdin redirected from a character device reads EOF
//! (0 bytes), and stderr redirections are dropped entirely (never
//! applied, target file never created).
//!
//! Production death: modernish's `use sys/base/mktemp` builds its
//! random suffix with `exec dd … </dev/urandom | exec tr …` — the
//! first `exec dd` reads 0 bytes, the suffix comes out empty, and
//! mktemp fails rc=128 (see the #2902 closing-chain report §3). This
//! issue is one of the two gates for re-running that chain.
//!
//! GNU semantics: a pipeline element (or `( )` subshell) is forked
//! (execute_cmd.c:664/725 → execute_in_subshell, execute_cmd.c:1576);
//! the child applies the element's FULL redirection list onto its
//! real descriptors (execute_cmd.c:4845-4855 already_forked →
//! execute_subshell_builtin_or_function → execute_cmd.c:5522
//! do_redirections (redirects, RX_ACTIVE)) and the exec builtin then
//! replaces the image with those descriptors in place
//! (builtins/exec.def:231 shell_execve). redir.c's redir_open opens
//! the target and dup2's it uniformly — the file TYPE of the stdin
//! target (regular file vs character device) is never consulted, and
//! fd 2 is wired exactly like fd 0/1.
//!
//! rubash surface: at pipeline/subshell position the element's stdio
//! is assembled from the executor's staged/virtual descriptor model
//! (src/executor/pipeline_exec.rs stage machinery;
//! src/executor/trap_exec.rs execute_exec / execute_exec_spawn
//! resolve the child stdio through exec_inherited_stdin /
//! exec_inherited_stdio from the fd table;
//! src/executor/command_input_scope.rs bind_command_fd0_input
//! decides between a real FileFd endpoint and FUNCTION_STDIN text
//! staging). On that path a character-device stdin binding collapses
//! to an empty staged input (the child sees EOF while a regular file
//! binds fine) and the element's `2>` redirect never reaches the
//! child's stderr at all. Top-level `exec` goes through the spawn
//! arms in trap_exec.rs and is pinned here as the contrast face.
//!
//! Oracle evidence: GNU bash 5.3.20, probes under
//! `~/workspace/fix-W2b/probes/gnu/` (p1-p15). All expected values
//! below are the GNU outputs byte-for-byte.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level
//! stdout/stderr assertions; `LC_ALL=C.UTF-8` is set on the child
//! explicitly so the pin is hermetic under any ambient locale.
//! File-carrying cells run in a per-test temp dir. None of the cells
//! depend on descriptor NUMBERS, so running under the build slot
//! (whose flock holds fd 9) is representative.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Out {
    rc: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run(script: &str, cwd: Option<&PathBuf>) -> Out {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let output = command
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    Out {
        rc: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3046-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn expect_stdout(out: &Out, expected: &[u8], ctx: &str) {
    assert_eq!(
        out.stdout,
        expected,
        "{ctx}: stdout got {:?}, want {expected:?}; stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.rc,
        0,
        "{ctx}: rc {} != 0; stderr: {}",
        out.rc,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr not empty: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Pipeline position, stdin from /dev/urandom (issue repro 1).
#[test]
fn pipeline_exec_stdin_urandom() {
    let out = run(
        "exec dd bs=160 count=1 </dev/urandom 2>/dev/null | wc -c",
        None,
    );
    expect_stdout(&out, b"160\n", "exec dd </dev/urandom | wc -c");
}

/// Pipeline position, head from /dev/urandom (issue repro 2).
#[test]
fn pipeline_exec_stdin_head_urandom() {
    let out = run("exec head -c 10 </dev/urandom | wc -c", None);
    expect_stdout(&out, b"10\n", "exec head -c 10 </dev/urandom | wc -c");
}

/// Pipeline position, stdin from /dev/zero (issue repro 3).
#[test]
fn pipeline_exec_stdin_dev_zero() {
    let out = run(
        "exec dd bs=160 count=1 </dev/zero 2>/dev/null | wc -c",
        None,
    );
    expect_stdout(&out, b"160\n", "exec dd </dev/zero | wc -c");
}

/// Control: the same pipeline WITHOUT exec reads the device fine.
#[test]
fn pipeline_no_exec_stdin_urandom_control() {
    let out = run("dd bs=160 count=1 </dev/urandom 2>/dev/null | wc -c", None);
    expect_stdout(&out, b"160\n", "dd </dev/urandom | wc -c (no exec)");
}

/// Control: pipeline exec with a REGULAR file as stdin works —
/// the defect is specific to character devices.
#[test]
fn pipeline_exec_stdin_regular_file_control() {
    let dir = temp_dir("regfile");
    fs::write(dir.join("regfile"), b"0123456789ABCDEFGHIJ").expect("write regfile");
    let out = run(
        "exec dd bs=10 count=1 < regfile 2>/dev/null | wc -c",
        Some(&dir),
    );
    expect_stdout(&out, b"10\n", "exec dd < regfile | wc -c");
    let _ = fs::remove_dir_all(&dir);
}

/// Control: /dev/null as stdin is genuinely EOF — a fix must not
/// fabricate bytes here.
#[test]
fn pipeline_exec_stdin_dev_null_control() {
    let out = run(
        "exec dd bs=160 count=1 </dev/null 2>/dev/null | wc -c",
        None,
    );
    expect_stdout(&out, b"0\n", "exec dd </dev/null | wc -c");
}

/// Pipeline position, stderr to /dev/null must silence the child
/// (issue repro 6): GNU prints the count only, nothing leaks.
#[test]
fn pipeline_exec_stderr_to_devnull() {
    let out = run("exec cat /nonexistent-42 2>/dev/null | wc -c", None);
    expect_stdout(&out, b"0\n", "exec cat /nonexistent-42 2>/dev/null | wc -c");
}

/// Pipeline position, stderr to a FILE (issue repro 7): GNU creates
/// the file and dd's record summary lands in it; nothing leaks to
/// the outer stderr.
#[test]
fn pipeline_exec_stderr_to_file() {
    let dir = temp_dir("errfile");
    fs::write(dir.join("regfile"), b"0123456789ABCDEFGHIJ").expect("write regfile");
    let out = run(
        "exec dd bs=10 count=1 < regfile 2>errfile | wc -c",
        Some(&dir),
    );
    expect_stdout(&out, b"10\n", "exec dd < regfile 2>errfile | wc -c");
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("records in"),
        "dd's stderr leaked to the outer stderr instead of errfile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let errfile =
        fs::read(dir.join("errfile")).expect("errfile must be created by the 2> redirect");
    let text = String::from_utf8_lossy(&errfile);
    assert!(
        text.contains("records in") && text.contains("bytes copied"),
        "errfile must carry dd's record summary, got: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Subshell position, stdin from /dev/urandom.
#[test]
fn subshell_exec_stdin_urandom() {
    let out = run(
        "( exec dd bs=160 count=1 </dev/urandom 2>/dev/null ) | wc -c",
        None,
    );
    expect_stdout(&out, b"160\n", "( exec dd </dev/urandom ) | wc -c");
}

/// Subshell position, stderr to /dev/null.
#[test]
fn subshell_exec_stderr_to_devnull() {
    let out = run("( exec cat /nonexistent-42 2>/dev/null ) | wc -c", None);
    expect_stdout(
        &out,
        b"0\n",
        "( exec cat /nonexistent-42 2>/dev/null ) | wc -c",
    );
}

/// Chain form: EVERY pipeline stage is an exec — the middle stage's
/// stdin is the pipe, the first stage's stdin is the device; both
/// bindings must hold at once.
#[test]
fn pipeline_exec_chain_all_stages() {
    let out = run(
        "exec dd bs=160 count=1 </dev/urandom 2>/dev/null | exec cat | wc -c",
        None,
    );
    expect_stdout(&out, b"160\n", "exec dd </dev/urandom | exec cat | wc -c");
}

/// Chain form, the production death shape (modernish mktemp STEP 4):
/// three exec stages inside a command substitution turn 10 device
/// bytes into a 20-char hex suffix; the intermediate length is the
/// state the next step consumes.
#[test]
fn comsub_mktemp_style_chain() {
    let out = run(
        "X=$(exec dd bs=10 count=1 </dev/urandom 2>/dev/null | exec od -An -tx1 | exec tr -d ' \\n'); echo ${#X}",
        None,
    );
    expect_stdout(&out, b"20\n", "mktemp-style exec chain in comsub");
}

/// Contrast face: TOP-LEVEL exec (no pipeline) with a character
/// device stdin — GNU delivers the 160 bytes as the process output.
#[test]
fn toplevel_exec_stdin_urandom() {
    let out = run("exec dd bs=160 count=1 </dev/urandom 2>/dev/null", None);
    assert_eq!(
        out.rc,
        0,
        "top-level exec dd rc {}; stderr: {}",
        out.rc,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout.len(),
        160,
        "top-level exec dd </dev/urandom must emit 160 bytes, got {}",
        out.stdout.len()
    );
    assert!(
        out.stderr.is_empty(),
        "stderr not empty: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Contrast face: TOP-LEVEL exec with stderr to /dev/null — GNU
/// exits 1 in silence.
#[test]
fn toplevel_exec_stderr_to_devnull() {
    let out = run("exec cat /nonexistent-42 2>/dev/null", None);
    assert_eq!(out.rc, 1, "top-level exec cat rc {} != 1", out.rc);
    assert!(
        out.stdout.is_empty(),
        "stdout not empty: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "stderr must be silenced by 2>/dev/null, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The issue's honest exception, pinned as a control: stdout
/// redirected to a file while in a pipeline — GNU writes the file,
/// the pipe sees nothing, stderr (to /dev/null) stays silent.
#[test]
fn pipeline_exec_stdout_to_file_exception_shape() {
    let dir = temp_dir("outfile");
    fs::write(dir.join("regfile"), b"0123456789ABCDEFGHIJ").expect("write regfile");
    let out = run(
        "exec dd bs=10 count=1 < regfile 2>/dev/null > outfile | cat",
        Some(&dir),
    );
    expect_stdout(&out, b"", "exec dd > outfile | cat");
    let written = fs::read(dir.join("outfile")).expect("outfile must be created");
    assert_eq!(written, b"0123456789", "outfile must carry dd's 10 bytes");
    let _ = fs::remove_dir_all(&dir);
}
