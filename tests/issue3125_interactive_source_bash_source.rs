//! rubash#3125 — interactive `source` does not push a BASH_SOURCE frame.
//!
//! GNU anchors (bash 5.3.20, verified against the oracle in this lane):
//! - builtins/evalfile.c:253 `source_file()` unconditionally
//!   `push_source (bash_source_a, filename)`;
//! - variables.c:5721 `push_source` is a plain `array_push` — the array
//!   starts EMPTY in every mode (shell.c:1640 pushes the main script
//!   file only inside `shell_execscript`); there is no `$0` seed frame
//!   in GNU's BASH_SOURCE at all;
//! - execute_cmd.c:5313 `execute_function` pushes the DEFINING file
//!   (`shell_fn->source_file`);
//! - variables.c:1892 BASH_SOURCE is `att_noassign|att_nounset`, so an
//!   empty array reads as UNBOUND under `set -u` (subst.c:7500 is the
//!   symptom outlet, not the fix face).
//!
//! The defect: rubash's BASH_SOURCE views (`dynamic_stack_array_values`,
//! `parameter_array_storage`, and the `caller` array fetch) pop the
//! bottom stack entry whenever there is no script main frame, assuming
//! it is the `$0` seed installed by `set_env(__RUBASH_SCRIPT_NAME)`.
//! An interactive session never installs that seed (no script file,
//! no `-c` string, no stdin-script name), so the bottom entry is a
//! REAL frame — the sourced file — and the pop eats the only frame:
//! `${#BASH_SOURCE[@]}` reads 0 at the sourced file's top level, 1
//! instead of 2 in a nested source, and 0 inside a function defined
//! in the sourced file. `-c` and script-file forms are unaffected
//! (their seed really is at the bottom).
//!
//! Pins run through a real PTY (`script(1)`, the issue1016 harness
//! shape): `-c` cannot reproduce this — the interactive driver path
//! is the only one without a seed. Fixtures are written by the test
//! into a scratch cwd so `source ./sourced_outer.sh` resolves there.
//! Expected values are the GNU oracle's, byte for byte.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-3125-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

const SOURCED_OUTER: &str = r#"echo "OUTER-pre BS0=[${BASH_SOURCE[0]}] BS=[${BASH_SOURCE}] N=${#BASH_SOURCE[@]}"
source ./sourced_inner.sh
echo "OUTER-post BS0=[${BASH_SOURCE[0]}] BS=[${BASH_SOURCE}] N=${#BASH_SOURCE[@]}"
probe_fn() { echo "FN BS0=[${BASH_SOURCE[0]}] BS1=[${BASH_SOURCE[1]:-}] N=${#BASH_SOURCE[@]} FUNC=[${FUNCNAME[0]}]"; }
"#;

const SOURCED_INNER: &str = r#"echo "INNER BS0=[${BASH_SOURCE[0]}] BS=[${BASH_SOURCE}] N=${#BASH_SOURCE[@]} FUNC=[${FUNCNAME[0]:-}]"
"#;

const NOUNSET_CELL: &str = r#"echo "NS BS=[$BASH_SOURCE] N=${#BASH_SOURCE[@]}"
"#;

/// Drive an interactive session through `script(1)` and return the
/// full transcript. Each line is paced 1s (issue1016 shape); the
/// session ends with `exit`.
fn run_interactive_session(lines: &[&str]) -> String {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return String::new();
    }
    let dir = scratch_tempdir();
    std::fs::write(dir.0.join("sourced_outer.sh"), SOURCED_OUTER).expect("write outer");
    std::fs::write(dir.0.join("sourced_inner.sh"), SOURCED_INNER).expect("write inner");
    std::fs::write(dir.0.join("nounset_cell.sh"), NOUNSET_CELL).expect("write nounset cell");

    let cmdline = format!("{} --norc -i", env!("CARGO_BIN_EXE_rubash"));
    let mut child = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env("TERM", "dumb")
        .env("PS1", "/@ ")
        .current_dir(&dir.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn script(1)");

    let mut stdin = child.stdin.take().expect("child stdin");
    let owned_lines: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    let writer = std::thread::spawn(move || {
        for line in owned_lines {
            if stdin.write_all(line.as_bytes()).is_err() {
                break;
            }
            if stdin.write_all(b"\n").is_err() {
                break;
            }
            let _ = stdin.flush();
            std::thread::sleep(Duration::from_secs(1));
        }
        let _ = stdin.write_all(b"exit\n");
        let _ = stdin.flush();
    });

    let mut stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let buf = match rx.recv_timeout(Duration::from_secs(25)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            panic!(
                "interactive session did not finish within 25s (input lines lost or shell hung)"
            );
        }
    };
    let status = child.wait().expect("wait for script(1)");
    let _ = writer.join();
    let _ = reader.join();
    assert_eq!(status.code(), Some(0), "session did not exit 0");

    String::from_utf8_lossy(&buf).into_owned()
}

fn require(transcript: &str, needle: &str) {
    assert!(
        transcript.contains(needle),
        "expected {needle:?} in session transcript: {transcript:?}"
    );
}

/// Cell 1 — a directly sourced file sees its own frame: scalar,
/// element 0 and length all agree (GNU: BS0=[./sourced_outer.sh],
/// BS=[./sourced_outer.sh], N=1). The top level stays empty after
/// the source returns (guard: the fix must not leak a frame).
#[test]
fn interactive_source_pushes_bash_source_frame() {
    let text = run_interactive_session(&[
        "source ./sourced_outer.sh",
        "echo \"TOP N=${#BASH_SOURCE[@]} BS=[${BASH_SOURCE}]\"",
    ]);
    if text.is_empty() {
        return;
    }
    require(
        &text,
        "OUTER-pre BS0=[./sourced_outer.sh] BS=[./sourced_outer.sh] N=1",
    );
    require(
        &text,
        "OUTER-post BS0=[./sourced_outer.sh] BS=[./sourced_outer.sh] N=1",
    );
    require(&text, "TOP N=0 BS=[]");
}

/// Cell 2 — a nested source sees depth 2 (GNU: INNER N=2; the
/// outer frame must survive under the inner one). FUNCNAME stays
/// empty at a sourced file's top level (the "source" frame is
/// invisible outside functions).
#[test]
fn interactive_nested_source_depth_is_two() {
    let text = run_interactive_session(&["source ./sourced_outer.sh"]);
    if text.is_empty() {
        return;
    }
    require(
        &text,
        "INNER BS0=[./sourced_inner.sh] BS=[./sourced_inner.sh] N=2 FUNC=[]",
    );
}

/// Cell 3 — a function defined in the sourced file, called from the
/// REPL, sees its defining file as BASH_SOURCE[0] with depth 1
/// (execute_cmd.c:5313 pushes `shell_fn->source_file`).
#[test]
fn interactive_sourced_function_call_sees_defining_file() {
    let text = run_interactive_session(&["source ./sourced_outer.sh", "probe_fn"]);
    if text.is_empty() {
        return;
    }
    require(
        &text,
        "FN BS0=[./sourced_outer.sh] BS1=[] N=1 FUNC=[probe_fn]",
    );
}

/// Cell 4 — under `set -u`, `"$BASH_SOURCE"` inside a sourced file
/// is bound (variables.c:1892 `att_nounset` fires only on a truly
/// empty array): no `unbound variable` error, exit status 0.
#[test]
fn interactive_source_bash_source_bound_under_nounset() {
    let text = run_interactive_session(&[
        "set -u",
        "source ./nounset_cell.sh",
        "echo \"NS-RC=$?\"",
        "set +u",
    ]);
    if text.is_empty() {
        return;
    }
    require(&text, "NS BS=[./nounset_cell.sh] N=1");
    require(&text, "NS-RC=0");
    assert!(
        !text.contains("unbound variable"),
        "nounset must not fire on BASH_SOURCE inside a sourced file: {text:?}"
    );
}
