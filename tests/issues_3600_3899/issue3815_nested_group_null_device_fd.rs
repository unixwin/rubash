//! rubash#3815: a compound group's `N</dev/null` must bind a real
//! /dev/null descriptor, and nested groups must restore fd N to the
//! enclosing scope's binding — not to a stray carrier pipe.
//!
//! GNU ground truth (bash 5.3.20, script-file mode): redir.c
//! do_redirection_internal opens the target with redir_open — /dev/null
//! is an ordinary file open, so the descriptor a child inspects via
//! /proc/self/fd/N is /dev/null itself. Rubash's compound input scope
//! shortcut the null device to an empty text endpoint, which the child
//! delivery materializes as a synthetic carrier (pipe/temp file), so
//! `ls -l /proc/self/fd/8` reported pipe:[…] where GNU reports
//! /dev/null. The same shortcut sat in compound_fd0_stream_file for
//! the fd-0 `< /dev/null` form.
//!
//! Every expectation below was diffed byte-for-byte against
//! /usr/local/bin/bash 5.3.20 under WSL.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3815-{}-{tag}", std::process::id()));
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
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue reproducer: nested groups both redirecting fd 8 from
/// /dev/null. Inside the outer group fd 8 is /dev/null; after both
/// scopes close it is the original file again. GNU prints
/// `-> /dev/null` then `-> f8.txt`; the broken build printed a
/// `pipe:[…]` target for the inner inspection.
#[test]
#[cfg(unix)]
fn nested_group_dev_null_restores_scope_chain() {
    let (stdout, stderr, rc) = run_driver(
        "nested",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { { echo g; } 8</dev/null; ls -l /proc/self/fd/8; } 8</dev/null\n\
         ls -l /proc/self/fd/8\n\
         exec 8<&-\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "g");
    assert!(
        lines[1].contains("-> /dev/null"),
        "inner group fd 8 must be /dev/null, got: {}",
        lines[1]
    );
    assert!(
        lines[2].contains("f8.txt") && !lines[2].contains("dev/null") && !lines[2].contains("pipe"),
        "outer restore must return fd 8 to the original file, got: {}",
        lines[2]
    );
}

/// fd-0 form: `( ls -l /proc/self/fd/0 ) < /dev/null` — the compound's
/// stdin bind must hand the child a real /dev/null descriptor, not the
/// empty-text carrier the text mirror materializes.
#[test]
#[cfg(unix)]
fn compound_fd0_dev_null_binds_real_device() {
    let (stdout, stderr, rc) = run_driver("fd0", "( ls -l /proc/self/fd/0 ) < /dev/null\n");
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert!(
        stdout.contains("-> /dev/null"),
        "compound fd 0 must be /dev/null, got: {stdout}"
    );
    assert!(
        !stdout.contains("rubash-process-subst"),
        "no synthetic carrier may appear, got: {stdout}"
    );
}

/// Simple-command control: `ls -l /proc/self/fd/8 < /dev/null` already
/// bound the real device; keep it pinned so the scope fix cannot
/// regress the non-compound face.
#[test]
#[cfg(unix)]
fn simple_command_dev_null_still_real_device() {
    let (stdout, stderr, rc) = run_driver("simple", "ls -l /proc/self/fd/0 < /dev/null\n");
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert!(stdout.contains("-> /dev/null"), "got: {stdout}");
}

/// Read-offset semantics survive the real-device bind: a group that
/// binds `8</dev/null` must not consume the outer fd 8's position.
#[test]
fn group_dev_null_preserves_outer_offset() {
    let (stdout, stderr, rc) = run_driver(
        "offset",
        "printf 'l1\\nl2\\n' > f8.txt\n\
         exec 8<f8.txt\n\
         { echo hi; } 8</dev/null\n\
         IFS= read -r l1 <&8; rc1=$?\n\
         IFS= read -r l2 <&8; rc2=$?\n\
         echo \"rc1=$rc1 l1=[$l1] rc2=$rc2 l2=[$l2]\"\n\
         exec 8<&-\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "hi\nrc1=0 l1=[l1] rc2=0 l2=[l2]\n");
}

/// Windows keeps the synthetic empty bind (no /dev/null path): the
/// in-process read still sees immediate EOF exactly like GNU's
/// /dev/null read.
#[test]
fn dev_null_input_reads_eof() {
    let (stdout, stderr, rc) = run_driver(
        "eof",
        "{ IFS= read -r l <&0; echo \"rc=$? l=[$l]\"; } < /dev/null\n",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "rc=1 l=[]\n");
}
