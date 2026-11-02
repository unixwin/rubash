//! Issue rubash#2492: a process-substitution PRODUCER child did not
//! inherit ambient fds whose endpoints name one of the shell's own
//! standard streams — after `exec 8<&0`, a `<( ... <&8 ... )` body
//! failed with `8: Bad file descriptor` (the modernish `_Msh_loopgen`
//! `exec 0<&8` face).
//!
//! GNU basis: `exec 8<&0` is the dup branch of
//! `do_redirection_internal` (`redir.c:1192` — `dup2 (redir_fd,
//! redirector)`), leaving fd 8 on the process stdin's open file
//! description, and `process_substitute` (`subst.c:6362`, `make_child`
//! at `subst.c:6412`) is a pure fork: the child inherits that
//! descriptor whole, reads through it consume the same description,
//! and `exec 0<&8` inside the child is a legal dup.
//!
//! Root cause (residual of #2471's half (a)): the input-procsub spawn
//! (`spawn_input_process_substitution_child`, executor/trap_exec.rs)
//! had its ambient `__RUBASH_FD_*` pass behind `#[cfg(unix)]` —
//! issue2212 pin 08 covers the collected comsub route (which runs the
//! body in-process), so the Windows streamed spawn never delivered
//! ANY ambient fd, std-stream or File. The pass now runs on every
//! platform: on Windows the stamped inheritable dups cross the plain
//! Command::spawn at their own handle values (std sets no
//! PROC_THREAD_ATTRIBUTE_HANDLE_LIST there), which is exactly what
//! the child's init trusts verbatim (init.rs), and std-stream
//! endpoints arrive materialized through `ambient_procsub_spawn_fd`'s
//! `standard_backing` dup (#2212).
//!
//! Expectations are GNU bash 5.3.20 behavior, measured with the same
//! script files (WSL `/usr/local/bin/bash`, stdin fed on both sides —
//! the fd is a dup of stdin, so a terminal/empty stdin blocks GNU in
//! the child's `read <&8`): artifacts under
//! target/issue-suites/results/issue2492/.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2492-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Run a script file with stdin FED — the fd under test is a dup of
/// stdin, so both the oracle run and this pin must provide data or
/// the child blocks in `read <&8` (see the issue's oracle note).
fn run_script_with_stdin(
    dir: &Path,
    name: &str,
    body: &str,
    stdin_data: &str,
) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    let mut child = rubash_command()
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    {
        let stdin = child.stdin.as_mut().expect("stdin pipe");
        stdin.write_all(stdin_data.as_bytes()).expect("feed stdin");
    }
    drop(child.stdin.take());
    child.wait_with_output().expect("wait rubash")
}

/// Poll a side file until `needle` lands or the deadline passes —
/// the `>(...)` consumer faces report through a file because the
/// consumer body's own stdout surface is a separate, known residual
/// (its writes DO arrive; see the sibling probes for the stderr and
/// file channels, which are the observable faces for this family).
fn await_file_contents(dir: &Path, name: &str, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut delivered = String::new();
    while Instant::now() < deadline {
        delivered = fs::read_to_string(dir.join(name)).unwrap_or_default();
        if delivered.contains(needle) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    delivered
}

// The issue's exact repro: `exec 8<&0` + a streamed `<( ... )` body
// that reads `<&8` and then dups it (`exec 0<&8`). GNU (bash 5.3.20,
// stdin fed): the child reads the parent's stdin through fd 8 and
// the exec dup succeeds. Before the fix rubash printed
// `line 3: 8: Bad file descriptor` / `child read-EOF` / FAILED.
#[test]
fn issue2492_producer_reads_stdin_dup_fd_and_exec_dups() {
    let dir = test_dir("repro");
    let body = "exec 8<&0\necho \"parent fd8 ok\"\nexec 9< <(\n  if IFS= read -r line <&8 2>/dev/null; then echo \"child sees fd8: [$line]\"; else echo \"child read-EOF\"; fi\n  exec 0<&8 && echo \"  child exec 0<&8 ok\" || echo \"  child exec 0<&8 FAILED\"\n)\nIFS= read -r got <&9 && echo \"parent got: [$got]\"\nIFS= read -r got <&9 && echo \"parent got: [$got]\"\n";
    let output = run_script_with_stdin(&dir, "main.sh", body, "DATA-LINE\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "parent fd8 ok\nparent got: [child sees fd8: [DATA-LINE]]\nparent got: [  child exec 0<&8 ok]\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert_eq!(output.status.code(), Some(0));
    let _ = fs::remove_dir_all(&dir);
}

// Matrix face `8<&1` x `<( ... )` producer: fd 8 is a dup of the
// script's stdout description. GNU: a read through it fails (the
// description is write-only — the builtin reports rc 1, here
// silenced) while `exec 0<&8` — dup2 of a legal descriptor —
// SUCCEEDS. Rubash must reproduce both halves, not just the failure.
#[test]
fn issue2492_producer_stdout_dup_read_fails_but_exec_dup_ok() {
    let dir = test_dir("lt1");
    let body = "exec 8<&1\nexec 9< <(\n  if IFS= read -r line <&8 2>/dev/null; then echo \"child read fd8: [$line]\"; else echo \"child read rc=$?\"; fi\n  exec 0<&8 2>/dev/null && echo \"  exec 0<&8 ok\" || echo \"  exec 0<&8 FAILED\"\n)\nIFS= read -r got <&9 && echo \"got: [$got]\"\nIFS= read -r got <&9 && echo \"got: [$got]\"\n";
    let output = run_script_with_stdin(&dir, "main.sh", body, "DATA-LINE\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "got: [child read rc=1]\ngot: [  exec 0<&8 ok]\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let _ = fs::remove_dir_all(&dir);
}

// Matrix face `8>&2` x `<( ... )` producer: fd 8 is a dup of the
// script's stderr description. GNU: the parent's `>&8` write, the
// body's `>&8` write, and the body's `exec 2>&8` dup all land on the
// same stderr stream; the script's status is the failed fourth read
// (rc 1). Pinned byte-for-byte on BOTH streams.
#[test]
fn issue2492_producer_stderr_dup_write_and_exec_dup() {
    let dir = test_dir("gt2");
    let body = "exec 8>&2\necho \"producer8-line\" >&8 2>/dev/null && echo \"wrote >&8 ok\" || echo \"wrote >&8 FAILED\"\nexec 9< <( echo \"body9\" >&8 2>/dev/null && echo \"  body wrote >&8 ok\" || echo \"  body wrote >&8 FAILED\"; exec 2>&8 2>/dev/null && echo \"  body exec 2>&8 ok\" || echo \"  body exec 2>&8 FAILED\"; echo body-done )\nIFS= read -r got <&9 && echo \"got: [$got]\"\nIFS= read -r got <&9 && echo \"got: [$got]\"\nIFS= read -r got <&9 && echo \"got: [$got]\"\nIFS= read -r got <&9 && echo \"got: [$got]\"\n";
    let output = run_script_with_stdin(&dir, "main.sh", body, "DATA-LINE\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "wrote >&8 ok\ngot: [  body wrote >&8 ok]\ngot: [  body exec 2>&8 ok]\ngot: [body-done]\n",
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "producer8-line\nbody9\n"
    );
    assert_eq!(output.status.code(), Some(1));
    let _ = fs::remove_dir_all(&dir);
}

// Matrix face `8<&0` x `>( ... )` consumer: the consumer body reads
// its payload off the procsub pipe, then the parent's stdin through
// the inherited fd 8. Both reads must deliver their own stream's
// data (GNU: the fork shares fd 8's description with stdin).
#[test]
fn issue2492_consumer_reads_stdin_dup_fd() {
    let dir = test_dir("cons-lt0");
    let body = "exec 8<&0\necho start > >( IFS= read -r payload; printf 'payload:[%s]\\n' \"$payload\" > cons.txt; IFS= read -r eight <&8 2>/dev/null && printf 'cons8:[%s]\\n' \"$eight\" >> cons.txt || printf 'cons8readrc=%s\\n' \"$?\" >> cons.txt )\nwait\n";
    let output = run_script_with_stdin(&dir, "main.sh", body, "DATA-LINE\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("Bad file descriptor"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let delivered = await_file_contents(&dir, "cons.txt", "cons8");
    assert_eq!(delivered, "payload:[start]\ncons8:[DATA-LINE]\n");
    let _ = fs::remove_dir_all(&dir);
}

// Matrix face `8<&1` x `>( ... )` consumer: the read through the
// stdout dup fails rc 1 (write-only description) on both shells —
// the same verdict the producer face pins above.
#[test]
fn issue2492_consumer_stdout_dup_read_fails() {
    let dir = test_dir("cons-lt1");
    let body = "exec 8<&1\necho start > >( IFS= read -r payload; printf 'payload:[%s]\\n' \"$payload\" > cons.txt; IFS= read -r eight <&8 2>/dev/null && printf 'cons8:[%s]\\n' \"$eight\" >> cons.txt || printf 'cons8readrc=%s\\n' \"$?\" >> cons.txt )\nwait\n";
    let output = run_script_with_stdin(&dir, "main.sh", body, "DATA-LINE\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let delivered = await_file_contents(&dir, "cons.txt", "cons8readrc");
    assert_eq!(delivered, "payload:[start]\ncons8readrc=1\n");
    let _ = fs::remove_dir_all(&dir);
}

// Matrix face `8>&2` x `>( ... )` consumer: the consumer body writes
// through the inherited stderr dup; the bytes land on the script's
// stderr stream exactly as GNU's do, and the body's own status line
// confirms the fd was open, not silently dropped.
#[test]
fn issue2492_consumer_writes_stderr_dup_fd() {
    let dir = test_dir("cons-gt2");
    let body = "exec 8>&2\necho start > >( IFS= read -r payload; printf 'payload:[%s]\\n' \"$payload\" > cons.txt; echo cons8-line >&8 2>/dev/null && printf 'wrote8ok\\n' >> cons.txt || printf 'wrote8FAIL\\n' >> cons.txt )\nwait\n";
    let output = run_script_with_stdin(&dir, "main.sh", body, "DATA-LINE\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let delivered = await_file_contents(&dir, "cons.txt", "wrote8");
    assert_eq!(delivered, "payload:[start]\nwrote8ok\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "cons8-line\n");
    let _ = fs::remove_dir_all(&dir);
}
