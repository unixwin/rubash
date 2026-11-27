//! rubash#3964 (face 1): the `$(< file)` quick-read — GNU subst.c
//! optimize_cat_file over open_redir_file — resolved an fd-0 alias
//! target (`/dev/stdin`, `/dev/fd/0`) only through the fd table
//! (buffered snapshot / live File endpoint). When the enclosing
//! context's stdin rides the FUNCTION_STDIN channel — a pipeline
//! stage's input for in-process stages (brace groups, subshells),
//! a function's stdin — the fd table carries no fd-0 endpoint, so
//! the ladder fell through to a filesystem read of the literal
//! `/dev/stdin`: the rubash PROCESS's own stdin (empty under
//! /dev/null; a hang while a live ambient stdin stays open), never
//! the context's stream. `read x < /dev/stdin` and
//! `$(cat /dev/stdin)` in the same positions already read the
//! channel; only the quick-read ladder was blind to it.
//!
//! GNU model: command_substitute forks (subst.c:7143), so the
//! substitution reads the fd 0 in effect in the ENCLOSING context,
//! sharing its offset; the enclosing simple command's OWN
//! redirections are not yet applied during its word expansion
//! (the `x=$(</dev/stdin) <<<hi` control below is empty in GNU
//! too — pinned so the fix cannot overreach into the command's
//! own redirect walk).
//!
//! Oracle: GNU bash 5.3.20, script files, byte-compared
//! (stdout/stderr/status).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3964a-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("data.txt"), "filedata\n").expect("write data.txt");
    dir
}

fn rubash_c(script: &str, dir: &Path) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_run(script: &str, expected_stdout: &str) {
    let dir = tempdir("case");
    let run = rubash_c(script, &dir);
    assert_eq!(run.stdout, expected_stdout, "stdout for {script:?}");
    assert_eq!(run.stderr, "", "stderr for {script:?}");
    assert_eq!(run.code, Some(0), "status for {script:?}");
}

/// Collect a child under a hard deadline. The issue-4229 defect is a
/// blocking read of the wrong Windows device, so every new probe must kill
/// and reap a stuck child rather than hanging the suite.
fn collect_bounded(mut child: std::process::Child, label: &str) -> Run {
    let mut stdout_pipe = child.stdout.take().expect("stdout pipe");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout_pipe, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });
    let mut stderr_pipe = child.stderr.take().expect("stderr pipe");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr_pipe, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut timed_out = false;
    let mut code = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                code = status.code();
                break;
            }
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => panic!("try_wait failed for {label:?}: {error}"),
        }
    }
    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    assert!(!timed_out, "rubash timed out for {label:?}");
    Run {
        stdout,
        stderr,
        code,
    }
}

fn spawn_c(tag: &str, script: &str) -> (std::process::Child, PathBuf) {
    let dir = tempdir(tag);
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_FUNCTION_STDIN")
        .env_remove("__RUBASH_FUNCTION_STDIN_OFFSET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    (child, dir)
}

fn rubash_c_piped_stdin(tag: &str, script: &str, input: &[u8]) -> Run {
    let (mut child, _dir) = spawn_c(tag, script);
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input)
        .expect("write piped stdin");
    drop(child.stdin.take());
    collect_bounded(child, script)
}

fn assert_piped_stdin(tag: &str, script: &str, input: &[u8], expected_stdout: &str) {
    let run = rubash_c_piped_stdin(tag, script, input);
    assert_eq!(run.stdout, expected_stdout, "stdout for {script:?}");
    assert_eq!(run.stderr, "", "stderr for {script:?}");
    assert_eq!(run.code, Some(0), "status for {script:?}");
}

fn rubash_c_file_stdin(tag: &str, script: &str, input: &[u8]) -> Run {
    let dir = tempdir(tag);
    std::fs::write(dir.join("stdin.txt"), input).expect("write stdin file");
    let stdin_file = std::fs::File::open(dir.join("stdin.txt")).expect("open stdin file");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_FUNCTION_STDIN")
        .env_remove("__RUBASH_FUNCTION_STDIN_OFFSET")
        .stdin(Stdio::from(stdin_file))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    collect_bounded(child, script)
}

fn assert_file_stdin(tag: &str, script: &str, input: &[u8], expected_stdout: &str) {
    let run = rubash_c_file_stdin(tag, script, input);
    assert_eq!(run.stdout, expected_stdout, "stdout for {script:?}");
    assert_eq!(run.stderr, "", "stderr for {script:?}");
    assert_eq!(run.code, Some(0), "status for {script:?}");
}

fn rubash_c_null_stdin(tag: &str, script: &str) -> Run {
    let dir = tempdir(tag);
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_FUNCTION_STDIN")
        .env_remove("__RUBASH_FUNCTION_STDIN_OFFSET")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    collect_bounded(child, script)
}

fn rubash_script_piped_stdin(tag: &str, script_text: &str, input: &[u8]) -> Run {
    let dir = tempdir(tag);
    std::fs::write(dir.join("case.sh"), script_text).expect("write script file");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_FUNCTION_STDIN")
        .env_remove("__RUBASH_FUNCTION_STDIN_OFFSET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash script");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(input)
        .expect("write piped stdin");
    drop(child.stdin.take());
    collect_bounded(child, script_text)
}

/// The pipeline-group position from the report: the stage input is
/// the context fd 0 the substitution must read.
#[test]
fn quickread_dev_stdin_in_pipeline_group() {
    assert_run("echo hi | { x=$(</dev/stdin); echo \"[$x]\"; }", "[hi]\n");
}

/// A subshell pipeline stage feeds its body through the same
/// channel.
#[test]
fn quickread_dev_stdin_in_pipeline_subshell() {
    assert_run("echo hi | (x=$(</dev/stdin); echo \"[$x]\")", "[hi]\n");
}

/// The function position from the report: the call's here-string
/// binds the function's stdin.
#[test]
fn quickread_dev_stdin_in_function_herestring() {
    assert_run("f(){ x=$(</dev/stdin); echo \"[$x]\"; }; f <<<hi", "[hi]\n");
}

/// A function as a pipeline stage reads the stage input.
#[test]
fn quickread_dev_stdin_in_function_pipeline_stage() {
    assert_run(
        "f(){ x=$(</dev/stdin); echo \"[$x]\"; }; echo hi | f",
        "[hi]\n",
    );
}

/// The `/dev/fd/0` spelling names the same fd 0.
#[test]
fn quickread_dev_fd0_in_pipeline_group() {
    assert_run("echo hi | { x=$(</dev/fd/0); echo \"[$x]\"; }", "[hi]\n");
}

/// Shared cursor: a `read` inside the function consumes the first
/// line; the substitution sees only the remainder (GNU's shared
/// fd 0 offset).
#[test]
fn quickread_dev_stdin_after_read_shares_cursor() {
    assert_run(
        "f(){ read a; x=$(</dev/stdin); echo \"[$a|$x]\"; }; printf 'l1\\nl2\\n' | f",
        "[l1|l2]\n",
    );
}

/// The substitution drains the context stream: a later `read`
/// in the same function sees EOF.
#[test]
fn quickread_dev_stdin_drains_for_later_read() {
    assert_run(
        "f(){ x=$(</dev/stdin); read a; echo \"[$x|$a]\"; }; printf 'l1\\nl2\\n' | f",
        "[l1\nl2|]\n",
    );
}

/// Control: an enclosing redirect that rebinds fd 0 to a real
/// file shadows the channel — the file is read, not the
/// function's stdin.
#[test]
fn quickread_dev_stdin_group_redirect_file_shadows_channel() {
    assert_run(
        "f(){ { x=$(</dev/stdin); echo \"[$x]\"; } < data.txt; }; f <<<hi",
        "[filedata]\n",
    );
}

/// Control: the plain-file quick-read is unchanged.
#[test]
fn quickread_plain_file_unchanged() {
    assert_run("x=$(<data.txt); echo \"[$x]\"", "[filedata]\n");
}

/// Control (GNU order): the assignment command's OWN here-string
/// is not yet applied while its value expands — GNU yields empty
/// here too.
#[test]
fn quickread_own_herestring_not_yet_applied() {
    assert_run("x=$(</dev/stdin) <<<hi; echo \"[$x]\"", "[]\n");
}

/// The substitution must never touch the shell PROCESS's stdin:
/// with the process stdin held open (a pipe nobody writes to),
/// the pipeline-group form still completes from the stage input.
/// On the defective ladder this read the process's /dev/stdin
/// and blocked until EOF.
#[test]
fn quickread_dev_stdin_never_reads_process_stdin() {
    let dir = tempdir("held");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "echo hi | { x=$(</dev/stdin); echo \"[$x]\"; }"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    // Hold the process stdin open without writing: take the writer
    // into a thread that simply keeps it alive past the deadline.
    let held_stdin = child.stdin.take().expect("child stdin");
    let holder = std::thread::spawn(move || {
        let mut held_stdin = held_stdin;
        std::thread::sleep(Duration::from_secs(30));
        let _ = held_stdin.write_all(b"");
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break Some(status),
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    drop(holder);
    let status = status.expect("rubash blocked reading its own process stdin");
    assert!(status.success(), "status {status:?}");
    let output = child.wait_with_output().expect("collect output");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[hi]\n",
        "stdout with held-open process stdin"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "stderr with held-open process stdin"
    );
}

/// rubash#4229: at top level fd 0 is the inherited process stdin. With a
/// pipe whose writer closes, the quick-read drains that pipe instead of
/// opening Windows CONIN$.
#[test]
fn quickread_top_level_pipe_stdin() {
    assert_piped_stdin(
        "top-pipe",
        r#"x=$(</dev/stdin); echo "[$x]""#,
        b"hi\n",
        "[hi]\n",
    );
}

/// The headline brace-group form at top level uses the same inherited fd 0.
#[test]
fn quickread_top_level_brace_group_pipe_stdin() {
    assert_piped_stdin(
        "top-group-pipe",
        r#"{ x=$(</dev/stdin); echo "[$x]"; }"#,
        b"hi\n",
        "[hi]\n",
    );
}

/// White space after `<` is still the same optimized redirection operand.
#[test]
fn quickread_top_level_spaced_operand_pipe_stdin() {
    assert_piped_stdin(
        "top-space-pipe",
        r#"x=$(< /dev/stdin); echo "[$x]""#,
        b"hi\n",
        "[hi]\n",
    );
}

/// A regular file on process stdin is read through the same fd alias.
#[test]
fn quickread_top_level_regular_file_stdin() {
    assert_file_stdin(
        "top-file",
        r#"x=$(</dev/stdin); echo "[$x]""#,
        b"hi\n",
        "[hi]\n",
    );
}

/// NUL stdin is immediate EOF: empty output, status 0, and no console read.
#[test]
fn quickread_top_level_null_stdin() {
    let run = rubash_c_null_stdin("top-null", r#"x=$(</dev/stdin); echo "[$x]""#);
    assert_eq!(run.stdout, "[]\n");
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

/// `/dev/fd/0` and `/proc/self/fd/0` are the same fd-0 spelling family.
#[test]
fn quickread_top_level_fd0_spellings_pipe_stdin() {
    for (tag, spelling) in [
        ("top-devfd0", "/dev/fd/0"),
        ("top-procfd0", "/proc/self/fd/0"),
    ] {
        let script = format!(r#"x=$(<{spelling}); echo "[$x]""#);
        assert_piped_stdin(tag, &script, b"hi\n", "[hi]\n");
    }
}

/// A pipe cannot be re-opened: `read` and the quick-read share the one
/// stream cursor, so the substitution sees only the unread remainder.
#[test]
fn quickread_top_level_pipe_after_read_shares_cursor() {
    assert_piped_stdin(
        "pipe-read-first",
        r#"read a; x=$(</dev/stdin); echo "[$a|$x]""#,
        b"l1\nl2\n",
        "[l1|l2]\n",
    );
}

/// Draining the pipe inside the substitution advances the shared cursor:
/// a later `read` sees EOF.
#[test]
fn quickread_top_level_pipe_drains_for_later_read() {
    assert_piped_stdin(
        "pipe-drain-first",
        r#"x=$(</dev/stdin); read a; echo "[$x|$a]""#,
        b"l1\nl2\n",
        "[l1\nl2|]\n",
    );
}

/// A regular file's magic-link open is fresh (offset 0), so the
/// substitution sees the whole file even after `read` consumed a line.
#[test]
fn quickread_top_level_regular_file_reopens_fresh() {
    assert_file_stdin(
        "file-fresh-reopen",
        r#"read a; x=$(</dev/stdin); echo "[$a|$x]""#,
        b"l1\nl2\n",
        "[l1|l1\nl2]\n",
    );
}

/// Closing fd 0 makes the alias OPEN fail ENOENT; it must not fall through
/// to the process stdin handle or the console device.
#[test]
fn quickread_closed_stdin_fails_enoent() {
    let run = rubash_c_piped_stdin(
        "closed-stdin",
        r#"exec 0<&-; x=$(</dev/stdin); echo "[$x] rc=$?""#,
        b"hi\n",
    );
    assert_eq!(run.stdout, "[] rc=1\n");
    assert!(
        run.stderr.contains("No such file or directory"),
        "stderr should carry ENOENT, got: {:?}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

/// A dup of inherited stdin carries the same stream endpoint.
#[test]
fn quickread_dup_stdin_fd_reads_pipe() {
    assert_piped_stdin(
        "dup-stdin",
        r#"exec 5<&0; x=$(</dev/fd/5); echo "[$x]""#,
        b"hi\n",
        "[hi]\n",
    );
}

/// The Claude Code hook shape that reported the hang: the quick-read in a
/// redirected brace group consumes the JSON payload without falling back.
#[test]
fn quickread_claude_hook_pattern_pipe_stdin() {
    assert_piped_stdin(
        "hook-json",
        r#"{ input=$(</dev/stdin); } 2>/dev/null || input=$(cat); echo "[$input]""#,
        br#"{"a":1}
"#,
        "[{\"a\":1}]\n",
    );
}

/// Script-file invocation has the same inherited-stdin fd 0 as `-c`.
#[test]
fn quickread_script_file_pipe_stdin() {
    let run =
        rubash_script_piped_stdin("script-file", "x=$(</dev/stdin); echo \"[$x]\"\n", b"hi\n");
    assert_eq!(run.stdout, "[hi]\n");
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}
