//! rubash#3806: rebinding an fd that holds an output process
//! substitution with an INPUT redirect must still run the deferred
//! body — GNU's dup2 onto the redirector closes the old write
//! descriptor, the substitution reader sees EOF, and the body
//! finishes.
//!
//! GNU ground truth (bash 5.3.20, script-file mode):
//! `exec 2> >(cat > body2.txt); exec 2<input.txt; exec 2>&-` creates
//! body2.txt. Rubash registers the deferred body in the fd's output
//! ledger; the input bind's `clear_output_fd_mirrors` removed that
//! registration before the final close could run it, so the body was
//! abandoned and body2.txt never appeared. The persistent exec input
//! arm now flushes the substitution before the bind retires the write
//! arm, and compound scopes save/restore the output ledger with the
//! rest of the slot state.
//!
//! Expectations verified byte-for-byte against /usr/local/bin/bash
//! 5.3.20 under WSL, script-file mode.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3806-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let mut file = std::fs::File::create(dir.join("driver.sh")).expect("write driver");
    file.write_all(driver.as_bytes()).expect("write driver");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("driver.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (
        stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue reproducer verbatim: output procsub on fd 2, an input
/// rebind, then the close. GNU creates body2.txt.
#[test]
#[cfg(unix)]
fn exec_input_rebind_runs_deferred_body() {
    let (stdout, stderr, rc) = run_driver(
        "rebind",
        "printf 'in\\n' > input.txt\n\
         rm -f body2.txt\n\
         exec 2> >(cat > body2.txt)\n\
         exec 2<input.txt\n\
         exec 2>&-\n\
         sleep 0.3\n\
         if [ -f body2.txt ]; then echo \"body2 EXISTS size=$(wc -c < body2.txt)\"; else echo \"body2 MISSING\"; fi\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "body2 EXISTS size=0\n");
}

/// Content written before the rebind must reach the body — GNU's
/// reader drains everything written until the last write descriptor
/// closes.
#[test]
#[cfg(unix)]
fn rebind_preserves_written_content() {
    let (stdout, stderr, rc) = run_driver(
        "content",
        "printf 'in\\n' > input.txt\n\
         rm -f body3.txt\n\
         exec 2> >(cat > body3.txt)\n\
         echo hello >&2\n\
         exec 2<input.txt\n\
         exec 2>&-\n\
         sleep 0.3\n\
         echo \"body3=[$(cat body3.txt 2>/dev/null)]\"\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "body3=[hello]\n");
}

/// Compound-scope face: `{ :; } 2<input.txt` binds the input only for
/// the group — the procsub registration must survive the scope so the
/// body still runs when the shell finally closes fd 2.
#[test]
#[cfg(unix)]
fn compound_input_bind_preserves_procsub() {
    let (stdout, stderr, rc) = run_driver(
        "compound",
        "printf 'in\\n' > input.txt\n\
         rm -f body4.txt\n\
         exec 4> >(cat > body4.txt)\n\
         { :; } 4<input.txt\n\
         echo after >&4\n\
         exec 4>&-\n\
         sleep 0.3\n\
         echo \"body4=[$(cat body4.txt 2>/dev/null)]\"\n\
         exec 4<&- 2>/dev/null\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "body4=[after]\n");
}

/// ReadWrite rebind (`<>`) retires the write arm the same way — the
/// body must still run.
#[test]
#[cfg(unix)]
fn readwrite_rebind_runs_deferred_body() {
    let (stdout, stderr, rc) = run_driver(
        "rw",
        "printf 'in\\n' > input.txt\n\
         rm -f body5.txt\n\
         exec 2> >(cat > body5.txt)\n\
         exec 2<>input.txt\n\
         exec 2>&-\n\
         sleep 0.3\n\
         if [ -f body5.txt ]; then echo \"body5 EXISTS\"; else echo \"body5 MISSING\"; fi\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "body5 EXISTS\n");
}
