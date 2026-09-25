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
fn dev_fd1_aliases_read_fd1_not_stdin() {
    // The reported silent family: `/dev/fd/1` and `/proc/self/fd/1` name
    // the command's fd 1 — in a pipeline that is the downstream pipe, not
    // the upstream stdin. GNU 5.3 reopens the fd readably
    // (open("/proc/self/fd/1", O_RDONLY)) and BLOCKS on the pipe's read
    // end; the well-defined Windows approximation returns empty without
    // hanging. Bare `cat` reads fd 0 and gets the data; `tee /dev/stdout`
    // writes fd 1 and forwards it — the write-vs-read asymmetry is
    // GNU-correct.
    for operand in ["/dev/fd/1", "/proc/self/fd/1"] {
        let output = run(&format!("printf '1\\n2\\n3\\n' | cat {operand}"));
        assert!(output.status.success(), "{operand} failed");
        assert_eq!(output.stdout, b"", "{operand} read upstream data");
    }
    // GNU: tee writes fd 1 AND the /dev/stdout file operand — the same
    // downstream pipe twice — so two input lines land as four.
    let output = run("printf 'a\\nb\\n' | tee /dev/stdout | wc -l");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "4");
}

fn temp_seed(content: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "rubash-devfd-seed-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, content).expect("write seed file");
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn dev_stdin_operand_obeys_command_input_redirect() {
    // redir.c do_redirections binds the command's own `<file` before the
    // operand opens /dev/stdin — GNU reads the file, not the pipeline.
    let seed = temp_seed("seeddata\n");
    let output = run_with_stdin(
        &format!("cat /dev/stdin <'{seed}'"),
        b"pipe-data-that-must-not-appear\n",
    );
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "seeddata\n");
}

#[test]
fn dev_stdin_operand_reads_unnumbered_here_string() {
    // Unnumbered `<<<` binds fd 0 through `cmd.here_string` — GNU adds the
    // trailing newline (parse.y here-string; redir.c makes it fd 0).
    let output = run("cat /dev/stdin <<<'hstr'");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hstr\n");
}

#[test]
fn dev_fd2_operand_obeys_stderr_redirect() {
    // `2>file` is kept only in `redirect_err` (not the ordered list) —
    // GNU still binds fd 2 to the file, so /dev/fd/2 reads it (empty for
    // a fresh truncate, seeded bytes for append).
    let fresh = temp_seed("");
    let output = run_with_stdin(&format!("cat /dev/fd/2 2>'{fresh}'"), b"pipe\n");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");

    let seeded = temp_seed("seeddata\n");
    let output = run(&format!("cat /dev/fd/2 2>>'{seeded}'"));
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "seeddata\n");
}

#[test]
fn dev_fd1_operand_obeys_stdout_redirect() {
    // `1>file` binds fd 1 to the file; GNU reads the (just-truncated,
    // hence empty) file rather than ENOENT-ing.
    let fresh = temp_seed("");
    let output = run(&format!("cat /dev/fd/1 1>'{fresh}'"));
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");
}

#[test]
fn dev_fd_dup_chain_follows_fd0_redirect() {
    // `3>&0` then `<file`: the dup chain lands on fd 0, whose own redirect
    // binds the file — GNU reads "seeddata", not the pipe.
    let seed = temp_seed("seeddata\n");
    let output = run_with_stdin(&format!("cat /dev/fd/3 3>&0 <'{seed}'"), b"pipe\n");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "seeddata\n");
}

#[test]
fn dev_stdout_operand_same_file_diagnostic() {
    // GNU cat.c: an input file identical to the output file reports
    // "input file is output file" and exits 1, leaving the file
    // untouched (a fresh `>` truncate has nothing to overlap, so the
    // diagnostic only fires when the input file holds bytes).
    let seed = temp_seed("seed\n");
    let output = run(&format!("cat /dev/stdout >>'{seed}'"));
    assert!(!output.status.success(), "expected rc 1: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("input file is output file"),
        "expected same-file diagnostic: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(&seed).unwrap(), "seed\n");

    // Empty target: GNU reopens the truncated file, reads nothing, exits 0
    // silently — `>` truncates before cat runs, so there is no overlap to
    // report.
    let empty = temp_seed("");
    let output = run(&format!("cat /dev/stdout >'{empty}'"));
    assert!(output.status.success());
    assert_eq!(output.stderr, b"");
    assert_eq!(std::fs::read_to_string(&empty).unwrap(), "");
}

#[test]
fn dev_stdout_reopens_inherited_file_handle() {
    // GNU /dev/stdout ≡ open("/proc/self/fd/1", O_RDONLY) — a fresh open
    // of fd 1's target. When the process's inherited stdout is a seeded
    // file (parent `>>`), cat's reopened input IS the output file:
    // "input file is output file", rc 1.
    let seed = temp_seed("parent-seed\n");
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(&seed)
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("cat /dev/stdout")
        .stdout(Stdio::from(file))
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    assert!(!output.status.success(), "expected rc 1: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("input file is output file"),
        "expected same-file diagnostic: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The same reopen on fd 2's file IS readable input (fd 1 is the
    // harness pipe, not that file): GNU `cat /dev/stderr 2>>seeded`
    // prints the seeded bytes.
    let err_seed = temp_seed("err-seed\n");
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(&err_seed)
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("cat /dev/stderr")
        .stdout(Stdio::piped())
        .stderr(Stdio::from(file))
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "err-seed\n");
    assert_eq!(std::fs::read_to_string(&err_seed).unwrap(), "err-seed\n");
}

#[test]
fn cat_continues_past_failed_operands() {
    // GNU cat.c: a failed operand prints a diagnostic and sets exit 1,
    // but later operands still produce output.
    let ok = temp_seed("tail-ok\n");
    let output = run(&format!("cat /dev/fd/9 '{ok}'"));
    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "tail-ok\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("/dev/fd/9"));
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
fn external_stage0_inherits_process_stdin() {
    // GNU execute_pipeline forks the first member with the shell's fd 0:
    // `printf | sh -c 'ext | cat'` feeds `ext` the real pipe, not an empty
    // buffered payload. `rubash -c` stands in for a non-whitelisted
    // external so the inherit path — not the pre-drain — is exercised.
    let rubash = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
    let output = run_with_stdin(&format!("'{rubash}' -c 'cat' | cat"), b"inherited-stdin\n");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "inherited-stdin\n");
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
