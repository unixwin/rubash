use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// `/dev/std*` / `/dev/fd/N` / `/proc/self/fd/N` operand semantics, checked
// against WSL GNU Bash 5.3.0. GNU hands these words to the child literally
// (execute_cmd.c shell_execve); the child resolves them through the OS
// /dev/fd layer (open ≡ dup). Windows has no such OS layer, so rubash
// resolves the operand against its fd table — in-process for emulated file
// builtins, materialized paths for real children.

fn run(script: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash")
}

fn run_with_stdin(script: &str, stdin: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");
    child.stdin.as_mut().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().expect("wait rubash")
}

#[test]
fn dev_stdin_operand_reads_fd0() {
    // GNU: `printf 'ab\n' | cat /dev/stdin` prints "ab".
    let output = run_with_stdin("cat /dev/stdin", b"ab\n");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"ab\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn dev_stdin_operand_reads_pipeline_stage_input() {
    // In a pipeline stage fd 0 is the upstream pipe, not the process
    // stdin — this previously hung on the CONIN$ translation.
    let output = run("printf '1\\n2\\n3\\n' | cat /dev/stdin");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n2\n3\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn dev_stdin_operand_inside_command_substitution() {
    // GNU: fd 0 inside $(...) is shared with the parent (subst.c:7143).
    let output = run_with_stdin("x=$(cat /dev/stdin); echo \"got:$x\"", b"ab\n");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "got:ab\n");
}

#[test]
fn dev_stdout_operand_reads_fd1_endpoint() {
    // GNU: reading the descriptor bound to fd 1 when fd 1 is a capture
    // pipe yields no bytes and exits 0 — it must not try to open a file
    // named /dev/stdout.
    let output = run("printf '1\\n2\\n3\\n' | cat /dev/stdout");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");
}

#[test]
fn dev_fd_numbered_operand_reads_bound_descriptor() {
    // GNU redir.c do_redirections: `3<<<x` binds fd 3 for the command;
    // cat opens /dev/fd/3 and reads the here-string.
    let output = run("cat /dev/fd/3 3<<<'fd3data'");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "fd3data\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn proc_self_fd_operand_reads_fd0() {
    let output = run_with_stdin("cat /proc/self/fd/0", b"procfd\n");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "procfd\n");
}

#[test]
fn dev_null_operand_and_redirects() {
    let output = run("cat /dev/null; printf 'x\\n' >/dev/null; echo rc=$?");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "rc=0\n");
}

#[test]
fn dev_stdout_stderr_redirect_targets() {
    // As redirection targets /dev/stdout and /dev/stderr keep their fd
    // meaning (redir.c), not filesystem paths.
    let output = run("printf 'x\\n' >/dev/stdout; printf 'y\\n' >/dev/stderr");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "x\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "y\n");
}

#[test]
fn dev_fd_operand_flows_to_external_child() {
    // A real child (a second rubash) resolves /dev/stdin through the
    // materialized endpoint the parent handed it.
    let rubash = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
    let output = run_with_stdin(&format!("'{rubash}' -c 'cat /dev/stdin'"), b"child-io\n");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "child-io\n");
}

#[test]
fn etc_paths_never_leak_translated_names() {
    // /etc/passwd and /etc/hosts are environment-only differences: a host
    // with a real file (e.g. Git Bash's /etc) reads it, one without gets a
    // clean ENOENT. What must never happen is the operand leaking as a
    // mangled translated path like winuxcmd\etc\passwd.
    for path in ["/etc/passwd", "/etc/hosts"] {
        let output = run(&format!("cat {path}"));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("winuxcmd") && !stderr.contains("\\etc\\"),
            "{path} leaked a translated path: {stderr:?}"
        );
    }
}

#[test]
fn dev_tty_may_block_or_fail_like_gnu() {
    // GNU: `cat /dev/tty` blocks when the shell has a controlling tty and
    // exits with ENXIO when it does not — both are correct. The contract
    // pinned here is the absence of a path-translation failure: a fast
    // exit must carry a device-style diagnostic, not ENOENT for a mangled
    // path like winuxcmd\dev\tty.
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("cat /dev/tty </dev/null")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => break None,
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    match status {
        // Blocked like GNU under a controlling tty — terminate.
        None => {
            let _ = child.kill();
            let _ = child.wait();
        }
        Some(status) => {
            let output = child.wait_with_output().expect("collect rubash output");
            assert!(!status.success());
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                !stderr.contains("winuxcmd") && !stderr.contains("\\dev\\"),
                "tty diagnostic leaked a translated path: {stderr:?}"
            );
        }
    }
}
