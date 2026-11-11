//! rubash#2732 (B5-02) — a function defined at the interactive top
//! level must report `BASH_SOURCE[0] == "main"`. GNU make_cmd.c:
//! 746-762 (`make_function_def`): the definition records
//! BASH_SOURCE[0] when the array is non-empty (a sourced file's
//! frame); when it is empty the default is `"environment"` before
//! shell initialization, `"main"` for an interactive shell, and
//! `dollar_vars[0]` for the `-c` route. The call frame then reports
//! the definition record (execute_cmd.c `execute_function` pushes
//! `shell_fn->source_file`).
//!
//! Measured against GNU bash 5.3.20 (setsid, no controlling tty):
//! | form | BASH_SOURCE[0] inside the function |
//! |---|---|
//! | defined at piped `-i` top level | `main` |
//! | g (REPL) calls f (REPL) | f sees `main main` |
//! | defined in a sourced file | the sourced path, as passed |
//! | defined on `-c` | argv[0], as invoked |
//! | defined in a script (plain or `-i`) | the script path, as invoked |
//!
//! Top-level `${BASH_SOURCE[*]}` at the interactive prompt is empty
//! in GNU — that emptiness is exactly what makes the `main` default
//! fire, so it is asserted in the same run.
//!
//! Note (face check): GNU `caller 0` inside an interactively called
//! function prints NOTHING and returns 1, so the caller face carries
//! no `main` evidence and is deliberately not pinned.
//!
//! Harness shape: same setsid/no-ctty + cleared-env + scratch-HOME
//! shape as issue2731.

use std::io::Write;
use std::process::{Command, Stdio};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2732-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn rubash_no_tty(
    args: &[&str],
    stdin_bytes: &[u8],
    home: &std::path::Path,
) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_bytes);
    }
    let out = child.wait_with_output().expect("wait for rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn repl_defined_function_reports_main() {
    let home = scratch_dir("repl");
    let input = b"f() { echo \"SRC0=${BASH_SOURCE[0]}\"; echo \"SRCALL=${BASH_SOURCE[*]}\"; }\nf\necho \"TOP=[${BASH_SOURCE[*]}]\"\nexit\n";
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], input, &home);
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        stdout.contains("SRC0=main"),
        "GNU: interactive definitions record `main`, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("SRCALL=main"),
        "GNU: the whole frame stack is the single `main` frame, got stdout: {stdout:?}"
    );
    assert!(
        stdout.contains("TOP=[]"),
        "GNU: top-level BASH_SOURCE is empty at the prompt, got stdout: {stdout:?}"
    );
}

#[test]
fn repl_defined_chain_reports_main_main() {
    let home = scratch_dir("chain");
    let input = b"f() { echo \"F_SRC=[${BASH_SOURCE[*]}]\"; }\ng() { f; }\ng\nexit\n";
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], input, &home);
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        stdout.contains("F_SRC=[main main]"),
        "GNU: caller and definition frames are both `main`, got stdout: {stdout:?} stderr: {stderr:?}"
    );
}

#[test]
fn repl_source_chain_keeps_file_source() {
    // Chain: a function defined in a SOURCED file keeps the file as
    // its source, while the REPL-defined one keeps `main` — the two
    // records must not contaminate each other.
    let home = scratch_dir("source");
    let lib = home.join("lib.sh");
    std::fs::write(&lib, "h() { echo \"H_SRC=${BASH_SOURCE[0]}\"; }\n").expect("write lib");
    let input = format!(
        "f() {{ echo \"F_SRC=${{BASH_SOURCE[0]}}\"; }}\nsource {}\nh\nf\nexit\n",
        lib.to_string_lossy()
    );
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], input.as_bytes(), &home);
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        stdout.contains(&format!("H_SRC={}", lib.to_string_lossy())),
        "GNU: sourced definitions record the file, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("F_SRC=main"),
        "GNU: the REPL definition still records `main` after sourcing, got stdout: {stdout:?}"
    );
}

#[test]
fn command_string_defined_function_reports_argv0() {
    // Control (make_cmd.c: the `-c` default is dollar_vars[0]).
    let home = scratch_dir("dashc");
    let (stdout, stderr, _) = rubash_no_tty(
        &["-c", "f() { echo \"SRC0=${BASH_SOURCE[0]}\"; }; f"],
        b"",
        &home,
    );
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        stdout.contains(&format!("SRC0={}", env!("CARGO_BIN_EXE_rubash"))),
        "GNU: -c definitions record argv[0] as invoked, got stdout: {stdout:?} stderr: {stderr:?}"
    );
}

#[test]
fn script_defined_function_reports_script_path() {
    // Control: script files seed BASH_SOURCE with the file
    // (shell.c:1640 push_source in open_shell_script), so the
    // definition record is the path — on both the plain and the
    // `-i` script routes.
    let home = scratch_dir("script");
    let script = home.join("defs.sh");
    std::fs::write(
        &script,
        "f() { echo \"SRC0=${BASH_SOURCE[0]}\"; }\nf\necho \"TOP=[${BASH_SOURCE[*]}]\"\n",
    )
    .expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    for args in [vec![script_arg.as_str()], vec!["-i", script_arg.as_str()]] {
        let (stdout, stderr, _) = rubash_no_tty(&args, b"", &home);
        assert!(
            stdout.contains(&format!("SRC0={script_arg}")),
            "GNU: script definitions record the script path (args {args:?}), got stdout: {stdout:?} stderr: {stderr:?}"
        );
        assert!(
            stdout.contains(&format!("TOP=[{script_arg}]")),
            "GNU: top-level BASH_SOURCE is the script (args {args:?}), got stdout: {stdout:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&home);
}
