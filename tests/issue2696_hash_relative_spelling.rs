//! Regression pins for rubash#2696 — the command hash table stored
//! the absolutized (session-cwd-anchored) resolution instead of the
//! as-found spelling, so `type` / `hash` printed the absolute path
//! and a post-`cd` re-execution reported plain "command not found".
//!
//! GNU ownership: findcmd.c find_in_path_element builds ONE string —
//! the as-written `<PATH element>/<name>` join (sh_makepath with the
//! element verbatim) — and that string is what phash_insert records
//! (findcmd.c:416) and what shell_execve later executes verbatim.
//! With `PATH=./rel1:$PATH` the recorded spelling is `./rel1/foo`:
//! `type foo` prints `foo is hashed (./rel1/foo)`, `hash` lists
//! `./rel1/foo`, and after `cd /` the verbatim name fails ENOENT as
//! `./rel1/foo: No such file or directory` with status 127
//! (execute_cmd.c:6143). An absolute PATH element records the
//! absolute join, which coincides with the anchored form.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2696-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env("FIX", root)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let foo = root.join("rel1").join("foo");
    std::fs::create_dir_all(foo.parent().expect("parent")).expect("rel1");
    std::fs::write(&foo, "#!/bin/sh\necho FOO-RAN\n").expect("foo");
    std::fs::set_permissions(&foo, std::fs::Permissions::from_mode(0o755)).expect("foo mode");
    root
}

#[test]
fn hash_records_the_as_found_relative_spelling() {
    let root = fixture("relative");
    let out = run(
        &root,
        "PATH=./rel1:/usr/bin:/bin\nfoo\ntype foo\nhash\ncd /\nfoo\necho \"rc=$?\"\n",
    );
    assert!(
        out.stdout.contains("FOO-RAN\nfoo is hashed (./rel1/foo)\n"),
        "stdout={:?}",
        out.stdout
    );
    // The `hash` listing names the recorded spelling, never the
    // absolutized form.
    assert!(out.stdout.contains("./rel1/foo"), "stdout={:?}", out.stdout);
    assert!(
        !out.stdout.contains(&format!("{}/rel1/foo", root.display())),
        "absolutized spelling leaked: stdout={:?}",
        out.stdout
    );
    assert!(out.stdout.ends_with("rc=127\n"), "stdout={:?}", out.stdout);
    // The post-cd failure is the verbatim recorded name failing
    // ENOENT (line 6), not "command not found".
    assert_eq!(
        out.stderr,
        "case.sh: line 6: ./rel1/foo: No such file or directory\n"
    );
    assert_eq!(out.rc, 0);
}

#[test]
fn hash_records_absolute_element_verbatim() {
    let root = fixture("absolute");
    let body = format!("PATH=\"$FIX/rel1:/usr/bin:/bin\"\nfoo\ntype foo\n");
    let out = run(&root, &body);
    let abs = format!("{}/rel1/foo", root.display());
    assert!(
        out.stdout
            .contains(&format!("FOO-RAN\nfoo is hashed ({abs})\n")),
        "stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}
