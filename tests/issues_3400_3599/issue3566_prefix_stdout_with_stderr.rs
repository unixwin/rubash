//! rubash#3566 — the Windows prefix-concurrent pipeline path
//! (`execute_external_prefix_concurrently`, pipeline_exec.rs)
//! dropped the prefix's stdout payload whenever the last prefix
//! member also produced stderr: the `else if !prefix_stderr.is_empty()`
//! arm relayed stderr through the fd-table binding but never assigned
//! `input = prefix_output`, so the shell-side tail read an empty
//! stream.
//!
//! GNU execute_pipeline (execute_cmd.c:2620) gives every element an
//! independent fd-1 pipe (`pipe(fildes)` at :2646, `do_piping` in the
//! forked child) while fd 2 stays the shell's own stderr — stdout
//! and stderr are separate objects, so prefix stderr can never
//! consume prefix stdout. The fix assigns `input = prefix_output`
//! unconditionally after the stderr routing (ambient `exec 2>&1`
//! merge still diverts the bytes to ambient_stderr per rubash#1552;
//! the rubash#3308 fd-table relay is unchanged).
//!
//! Shape: >=2 admitted external prefix members + >=2 shell-side tail
//! members (function stages), which is exactly the admission contract
//! of the Windows-only path (pipeline_exec.rs:4896-4924). On unix the
//! function declines and the sequential loop produces the same GNU
//! output, so the pin is cross-platform; the fail-first red is the
//! Windows leg. Expectations byte-verified against GNU bash 5.3.20
//! (/usr/local/bin/bash, target/scratch-i3/gnu-3566.sh): stderr
//! `errmsg`, stdout `g:g:two`, pipeline rc=0.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3566-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `body` as a script FILE under rubash with cwd = dir.
/// Returns (stdout, stderr, status).
fn rubash_script(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join("case.sh");
    fs::write(&script, body).expect("write script fixture");
    let output = Command::new(RUBASH)
        .arg(&script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's reference pipeline: two external prefix members (the
/// second writes stdout AND stderr) feeding two shell-side stages.
/// GNU: `errmsg` on stderr, `g:g:two` on stdout, rc=0.
#[test]
fn issue3566_prefix_stdout_survives_prefix_stderr() {
    let dir = scratch("prefix");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "g() { while read l; do echo \"g:$l\"; done; }\n",
            "\"$THIS_SH\" -c 'echo one' | \"$THIS_SH\" -c 'echo two; echo errmsg >&2' | g | g\n",
            "echo \"pipeline rc=$?\"\n",
        ),
    );
    assert_eq!(stdout, "g:g:two\npipeline rc=0\n");
    assert_eq!(stderr, "errmsg\n");
}

/// Same decoupling with the stderr producer in the FIRST prefix
/// member: only the last prefix member's payload becomes the tail's
/// input, but every member's stdout must still flow through the
/// inter-member pipes — a mid-prefix stderr must not eat the chain.
#[test]
fn issue3566_mid_prefix_stderr_keeps_chain() {
    let dir = scratch("mid");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "g() { while read l; do echo \"g:$l\"; done; }\n",
            "\"$THIS_SH\" -c 'echo one; echo e1 >&2' | \"$THIS_SH\" -c 'cat; echo two' | g\n",
            "echo \"pipeline rc=$?\"\n",
        ),
    );
    assert_eq!(stdout, "g:one\ng:two\npipeline rc=0\n");
    assert_eq!(stderr, "e1\n");
}

/// Control: prefix stderr alone (no stdout payload) still relays and
/// the tail sees an empty stream — GNU `printf '' | g` shape.
#[test]
fn issue3566_stderr_only_prefix_still_relays() {
    let dir = scratch("stderronly");
    let (stdout, stderr, _) = rubash_script(
        &dir,
        concat!(
            "g() { while read l; do echo \"g:$l\"; done; echo \"g-done\"; }\n",
            "\"$THIS_SH\" -c 'echo one' | \"$THIS_SH\" -c 'echo errmsg >&2' | g | g\n",
            "echo \"pipeline rc=$?\"\n",
        ),
    );
    // The last prefix member's stdout is empty, so the first tail
    // stage emits only its trailing marker; the second stage reads
    // that marker and echoes it prefixed, then emits its own —
    // GNU's stream order (gnu-3566-extra.sh pipeline2).
    assert_eq!(stdout, "g:g-done\ng-done\npipeline rc=0\n");
    assert_eq!(stderr, "errmsg\n");
}
