//! Regression pins for rubash#3147 — describing a NOT-yet-hashed
//! command (a fresh PATH walk) printed the session-cwd-anchored
//! resolution (`/tmp/<fixture>/./rel1/foo`, `/./` residue included)
//! instead of the walk's as-written join.
//!
//! GNU ownership: the walk builds ONE string —
//! find_in_path_element's `full_path = sh_makepath (xpath, name, 0)`
//! (findcmd.c:553), the PATH element verbatim joined with the name —
//! and describe_command prints exactly that string for the
//! CDESC_SHORTDESC (`type`) and CDESC_REUSABLE (`command -v`) faces
//! (builtins/type.def:379 find_user_command / :381
//! user_command_matches for `-a`, printed at :425/:427). The hashed
//! face was already fixed by rubash#2696 (the recorded spelling IS
//! the same join); this file pins the fresh-walk faces: `command -v`,
//! `type`, `type -p`, `type -P`, `type -a`, relative / dot / empty /
//! trailing-slash elements, and the post-`cd` element anchor.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8 (captain-lane probes,
//! 2026-11-09).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3147-{tag}-{}", std::process::id()));
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

fn write_exec(path: &Path, marker: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("fixture dir");
    std::fs::write(path, format!("#!/bin/sh\necho {marker}\n")).expect("fixture file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("fixture mode");
}

/// rel1/foo, rel2/foo, cwdfoo in the root, and an empty sub/.
fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    write_exec(&root.join("rel1").join("foo"), "FOO-RAN");
    write_exec(&root.join("rel2").join("foo"), "FOO2-RAN");
    write_exec(&root.join("cwdfoo"), "CWDFOO-RAN");
    std::fs::create_dir_all(root.join("sub")).expect("sub dir");
    root
}

#[test]
fn fresh_describe_prints_as_written_join() {
    let root = fixture("fresh");
    let out = run(
        &root,
        "PATH=./rel1:/usr/bin:/bin\ncommand -v foo\ntype foo\ntype -p foo\n",
    );
    assert_eq!(out.stdout, "./rel1/foo\nfoo is ./rel1/foo\n./rel1/foo\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn type_all_prints_each_element_join() {
    let root = fixture("all");
    let out = run(&root, "PATH=./rel1:./rel2:/usr/bin:/bin\ntype -a foo\n");
    assert_eq!(out.stdout, "foo is ./rel1/foo\nfoo is ./rel2/foo\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn plain_relative_element_join_verbatim() {
    let root = fixture("plainrel");
    let out = run(
        &root,
        "PATH=rel1:/usr/bin:/bin\ncommand -v foo\ntype foo\n",
    );
    assert_eq!(out.stdout, "rel1/foo\nfoo is rel1/foo\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn dot_and_empty_elements_join_as_dot_slash() {
    let root = fixture("dotempty");
    let out = run(
        &root,
        "PATH=.:/usr/bin:/bin\ncommand -v cwdfoo\ntype cwdfoo\nPATH=:/usr/bin:/bin\ncommand -v cwdfoo\ntype cwdfoo\n",
    );
    assert_eq!(
        out.stdout,
        "./cwdfoo\ncwdfoo is ./cwdfoo\n./cwdfoo\ncwdfoo is ./cwdfoo\n"
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn absolute_element_join_coincides() {
    let root = fixture("abs");
    let body = format!(
        "PATH=\"$FIX/rel1:/usr/bin:/bin\"\ncommand -v foo\ntype foo\n"
    );
    let out = run(&root, &body);
    let abs = format!("{}/rel1/foo", root.display());
    assert_eq!(out.stdout, format!("{abs}\nfoo is {abs}\n"));
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn trailing_slash_element_join() {
    let root = fixture("trailslash");
    let out = run(
        &root,
        "PATH=./rel1/:/usr/bin:/bin\ncommand -v foo\ntype -a foo\n",
    );
    assert_eq!(out.stdout, "./rel1/foo\nfoo is ./rel1/foo\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn hashed_describe_keeps_recorded_spelling() {
    let root = fixture("hashed");
    let out = run(
        &root,
        "PATH=./rel1:/usr/bin:/bin\nfoo\ntype foo\ntype -a foo\n",
    );
    // The hashed line is the #2696 recorded spelling; `type -a`
    // without -P does not prepend the hash entry (type.def's
    // `all == 0 || CDESC_FORCE_PATH` guard) and lists the walk hit.
    assert_eq!(
        out.stdout,
        "FOO-RAN\nfoo is hashed (./rel1/foo)\nfoo is ./rel1/foo\n"
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn type_force_path_prints_walk_join() {
    let root = fixture("forcepath");
    let out = run(&root, "PATH=./rel1:/usr/bin:/bin\nfoo\ntype -P foo\n");
    assert_eq!(out.stdout, "FOO-RAN\n./rel1/foo\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn describe_after_cd_uses_new_cwd_join() {
    let root = fixture("cdjoin");
    let out = run(
        &root,
        "cd sub\nPATH=../rel1:/usr/bin:/bin\ncommand -v foo\ntype foo\ntype -a foo\n",
    );
    assert_eq!(
        out.stdout,
        "../rel1/foo\nfoo is ../rel1/foo\nfoo is ../rel1/foo\n"
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn not_found_stays_not_found() {
    let root = fixture("notfound");
    let out = run(
        &root,
        "PATH=./rel1:/usr/bin:/bin\ncommand -v nosuch3147\necho \"rc=$?\"\n",
    );
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}
