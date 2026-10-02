//! Issue rubash#396 regression (item 2, K018/K019): `source <(cmd)` and
//! `. <(cmd)` used to fail with `/dev/fd/N: No such file or directory`
//! because the source builtin went straight to the Windows filesystem.
//!
//! GNU spec:
//! - The word `<(...)` expands to `/dev/fd/N` naming the pipe read end
//!   process_substitute parks at fd >= 64 (subst.c:6392 move_to_high_fd).
//! - builtins/source.def hands the expanded word to _evalfile;
//!   builtins/evalfile.c:104 `fd = open (filename, O_RDONLY)` reopens
//!   that pipe on Linux. rubash's endpoints live in the executor fd
//!   table, so the read consults `input_snapshot_bytes` — the same
//!   lookup the comsub operand path uses.
//! - Diagnostics prefix with the /dev/fd word (`/dev/fd/N: line 1:
//!   cmd: command not found`), rc 127; args after the word become the
//!   sourced text's positional parameters; an empty stream sources
//!   nothing with rc 0.
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 script-file probes
//! (target/issue396/{srcproc,srcproc2}.sh — stdout, stderr and rc all
//! identical; the fd NUMBER in the diagnostic is an allocation detail).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue396-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue396 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    // LF line endings: probes run identically under WSL GNU Bash for
    // baseline comparison.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `source <(echo ...)` runs the stream in the current shell (GNU
/// srcproc.sh probe: sourced-inline, rc 0).
#[test]
fn source_procsub_runs_stream_in_current_shell() {
    let outcome =
        run_rubash_script("source <(echo echo sourced-inline)\necho src-rc=$?\n. <(echo echo dotted-inline)\necho dot-rc=$?\n");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(
        outcome.stdout,
        "sourced-inline\nsrc-rc=0\ndotted-inline\ndot-rc=0\n"
    );
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// Assignments and args in the sourced stream affect the current shell;
/// args after the word become positional parameters (GNU srcproc2.sh).
#[test]
fn source_procsub_args_and_assignments_apply() {
    let outcome = run_rubash_script(
        "source <(echo 'echo \"arg1=$1 arg2=$2\"') one two\necho args-rc=$?\n\
         . <(echo 'v=fromprocsub')\necho v=$v\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(
        outcome.stdout,
        "arg1=one arg2=two\nargs-rc=0\nv=fromprocsub\n"
    );
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// A failing command inside the stream reports with the /dev/fd word as
/// the script prefix and rc 127 (GNU: `/dev/fd/N: line 1: ...: command
/// not found`); an empty stream sources nothing, rc 0.
#[test]
fn source_procsub_notfound_and_empty_shapes() {
    let outcome = run_rubash_script(
        "source <(echo not-a-command-xyz)\necho notfound-rc=$?\nsource <(true)\necho empty-rc=$?\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "notfound-rc=127\nempty-rc=0\n");
    assert!(
        outcome
            .stderr
            .contains(": line 1: not-a-command-xyz: command not found"),
        "stderr: {}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.starts_with("/dev/fd/"),
        "stderr prefix must be the /dev/fd word: {}",
        outcome.stderr
    );
}

/// The neighboring `/dev/stdin` pipeline route must keep its GNU shape
/// (`echo x | . /dev/stdin` sources the pipe; GNU srcproc2.sh probe:
/// command-not-found with the /dev/stdin prefix, rc 127).
#[test]
fn source_dev_stdin_pipe_still_works() {
    let outcome = run_rubash_script("echo piped-content | . /dev/stdin\necho stdin-rc=$?\n");
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "stdin-rc=127\n");
    assert!(
        outcome
            .stderr
            .contains("/dev/stdin: line 1: piped-content: command not found"),
        "stderr: {}",
        outcome.stderr
    );
}
