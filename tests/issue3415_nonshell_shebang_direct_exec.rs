//! rubash#3415: a directly-executed script whose `#!` line names a
//! NON-shell interpreter must be run by that interpreter, not parsed
//! as bash.
//!
//! GNU anchor: on a host with hash-bang exec the kernel execs the named
//! interpreter (execute_cmd.c:6126 shell_execve execs first and only
//! falls back to shell execution for a shebangless text file,
//! :6225-6260); where the OS has no hash-bang exec, GNU re-execs
//! through the NAMED interpreter (execute_shell_script,
//! execute_cmd.c:6001-6043). Windows has no kernel `#!`, so rubash
//! stands in for both: resolve the named interpreter and spawn it with
//! the script as its operand (the MSYS/Git Bash model).
//!
//! Base behavior on Windows (master ffc09fb7): the in-process shell
//! fallback swallowed the file — a non-shell body died with a bash
//! syntax error (rc 2, diagnostic headed by the script name) and a
//! shebang-only file exited 0 in silence.
//!
//! Windows only: unix lets the kernel own `#!`, so the new spawn path
//! is unreachable there (the `should_run_with_shell` gate). The
//! interpreter fixtures are chosen to exist on every Windows host:
//! a batch file in the scratch dir (spawned through cmd by the Rust
//! std runtime, which proves the optional-argument and operand argv
//! byte-for-byte), `C:/Windows/System32/hostname.exe` (ignores its
//! argv, prints the host name), and `cmd.exe /c` (the single optional
//! argument arm). `#!/usr/bin/env hostname` exercises the env-launcher
//! arm. Output is asserted line-wise: cmd children print CRLF.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3415-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue3415 scratch dir");
    ScratchDir(dir)
}

fn write(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("write fixture");
}

/// Run `parent_body` as a rubash script file in `dir`; returns
/// (exit code, stdout, stderr).
fn run_parent(dir: &Path, parent_body: &str) -> (Option<i32>, String, String) {
    std::fs::write(dir.join("parent.sh"), parent_body).expect("write parent script");
    let output = Command::new(BIN)
        .arg("parent.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash parent script");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A batch-file interpreter fixture: echoes its whole argv, so the
/// optional shebang argument, the script operand and the caller's
/// operands are all visible in the output.
fn write_batch_interpreter(dir: &Path) {
    write(dir, "interp.cmd", "@echo off\necho interp-got:%*\n");
}

/// Lines of `stdout` with CR stripped (cmd children print CRLF).
fn lines(stdout: &str) -> Vec<&str> {
    stdout.lines().collect()
}

/// The issue's shape, with a batch interpreter standing in for node:
/// the interpreter gets the optional argument, the script operand and
/// the caller's operands; the non-shell body is never parsed as bash.
/// Base: `./t_x: line 2: syntax error ...`, rc 2.
#[test]
fn foreign_interp_direct_with_optarg_and_operands() {
    let dir = scratch("direct");
    write_batch_interpreter(&dir.0);
    write(&dir.0, "t_x", "#!./interp.cmd MYOPT\nconsole.log(1)\n");
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_x A B; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    assert_eq!(lines(&stdout), ["interp-got:MYOPT ./t_x A B", "rc=0"]);
}

/// The issue's second shape: a shebang-only file must not exit 0 in
/// silence — the interpreter runs and its output is visible.
/// Base: silent rc 0.
#[test]
fn foreign_interp_shebang_only_file_is_not_silent() {
    let dir = scratch("shebang-only");
    write_batch_interpreter(&dir.0);
    write(&dir.0, "t_y", "#!./interp.cmd\n");
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_y; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    assert_eq!(lines(&stdout), ["interp-got:./t_y", "rc=0"]);
}

/// The `/usr/bin/env` launcher arm: env resolves the target on PATH
/// and the target runs the script. Base: line 2 syntax error, rc 2.
#[test]
fn foreign_interp_env_launcher() {
    let dir = scratch("env");
    write(
        &dir.0,
        "t_host",
        "#!/usr/bin/env hostname\nconsole.log(1)\n",
    );
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_host; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    let out_lines = lines(&stdout);
    assert_eq!(out_lines.len(), 2, "stdout lines: {out_lines:?}");
    assert!(!out_lines[0].is_empty(), "hostname line must print");
    assert_eq!(out_lines[1], "rc=0");
}

/// A direct absolute interpreter path: the script runs under it.
/// Base: line 2 syntax error, rc 2.
#[test]
fn foreign_interp_direct_absolute_path() {
    let dir = scratch("abspath");
    write(
        &dir.0,
        "t_h2",
        "#!C:/Windows/System32/hostname.exe\nconsole.log(1)\n",
    );
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_h2; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    let out_lines = lines(&stdout);
    assert_eq!(out_lines.len(), 2, "stdout lines: {out_lines:?}");
    assert!(!out_lines[0].is_empty(), "hostname line must print");
    assert_eq!(out_lines[1], "rc=0");
}

/// The single optional shebang argument reaches the interpreter:
/// `cmd.exe /c` runs the script as a batch file. Base: the batch body
/// is parsed as bash (`@echo: command not found` on stderr).
#[test]
fn foreign_interp_single_optional_argument() {
    let dir = scratch("optarg");
    write(
        &dir.0,
        "t_cmd",
        "#!C:/Windows/System32/cmd.exe /c\n@echo off\necho batch-ran\n",
    );
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_cmd; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    assert_eq!(lines(&stdout), ["batch-ran", "rc=0"]);
}

/// Boundary that must not move: sh/bash shebangs keep the shell path.
#[test]
fn shell_shebang_unchanged() {
    let dir = scratch("shell");
    write(&dir.0, "t_sh", "#!/usr/bin/env sh\necho shell-ok\n");
    write(&dir.0, "t_bash", "#!/usr/bin/env bash\necho shell-ok\n");
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_sh; ./t_bash; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    assert_eq!(lines(&stdout), ["shell-ok", "shell-ok", "rc=0"]);
}

/// Boundary that must not move: a shebangless text file still runs as
/// a shell script (the ENOEXEC tail, execute_cmd.c:6237-6260).
#[test]
fn shebangless_script_still_runs_as_shell() {
    let dir = scratch("nosheb");
    write(
        &dir.0,
        "t_nosheb",
        "echo \"runner=$0\"\narr=(a b)\necho \"len=${#arr[@]}\"\n",
    );
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_nosheb; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    assert_eq!(lines(&stdout), ["runner=./t_nosheb", "len=2", "rc=0"]);
}

/// The `exec` builtin face shares the shell wrap, so it shares the
/// fix: `exec ./t_x` replaces the shell with the interpreter.
/// Base: sh parses the body (`console.log` syntax error, rc 2).
#[test]
fn foreign_interp_exec_builtin() {
    let dir = scratch("exec");
    write_batch_interpreter(&dir.0);
    write(&dir.0, "t_x", "#!./interp.cmd MYOPT\nconsole.log(1)\n");
    let (code, stdout, stderr) = run_parent(&dir.0, "exec ./t_x A B; echo UNREACHABLE\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    assert!(
        stdout.contains("interp-got:MYOPT ") && stdout.contains("t_x A B"),
        "stdout must carry the interpreter argv, saw: {stdout}"
    );
    assert!(
        !stdout.contains("UNREACHABLE"),
        "exec must replace: {stdout}"
    );
}

/// The pipeline-stage face spawns through the same funnel, so a
/// foreign-shebang stage runs under its interpreter too.
/// Base: the stage is shell-wrapped, line 2 syntax error, rc 2.
#[test]
fn foreign_interp_pipeline_stage() {
    let dir = scratch("pipeline");
    write(
        &dir.0,
        "t_host",
        "#!/usr/bin/env hostname\nconsole.log(1)\n",
    );
    let (code, stdout, stderr) = run_parent(&dir.0, "./t_host | cat; echo \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert!(stderr.is_empty(), "stderr must be empty, saw: {stderr}");
    let out_lines = lines(&stdout);
    assert_eq!(out_lines.len(), 2, "stdout lines: {out_lines:?}");
    assert!(!out_lines[0].is_empty(), "hostname line must print");
    assert_eq!(out_lines[1], "rc=0");
}
