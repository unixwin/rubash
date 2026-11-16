//! Regression pins for rubash#3358 — explicit `hash NAME` recorded the
//! absolutized (session-cwd-anchored) resolution instead of the as-found
//! spelling for relative PATH entries, so `hash -t` / `hash -l` / bare
//! `hash` printed `<cwd>/./mycmd` where GNU prints `./mycmd`.
//!
//! GNU ownership: builtins/hash.def:223-225 — the bare-name loop calls
//! `find_user_command (w)` and `phash_insert (w, full_path, ...)`; GNU's
//! walk builds ONE string, the as-written `<PATH element>/<name>` join
//! (findcmd.c sh_makepath with the element verbatim), and that string is
//! what gets recorded. The run-once recording path already stores the
//! as-found spelling (rubash#2696, PR #3120); only the explicit-`hash`
//! builtin still stored the anchored form. The fresh-walk describe path
//! (`command -v` / `type` for unhashed commands) was fixed separately
//! (rubash#3147, PR #3215).
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same probe
//! scripts under LC_ALL=C.UTF-8 (seed_r93a2.sh in the issue: GNU 6/6,
//! rubash f5f65d35 3 FAIL + 3 PASS).
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3358-{tag}-{}", std::process::id()));
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
    let mycmd = root.join("mycmd");
    std::fs::write(&mycmd, "#!/bin/sh\necho REL\n").expect("mycmd");
    std::fs::set_permissions(&mycmd, std::fs::Permissions::from_mode(0o755)).expect("mycmd mode");
    let dcmd = root.join("d").join("mycmd");
    std::fs::create_dir_all(dcmd.parent().expect("parent")).expect("d");
    std::fs::write(&dcmd, "#!/bin/sh\necho REL\n").expect("d/mycmd");
    std::fs::set_permissions(&dcmd, std::fs::Permissions::from_mode(0o755)).expect("d/mycmd mode");
    root
}

#[test]
fn explicit_hash_records_relative_spelling() {
    let root = fixture("relative");
    let out = run(
        &root,
        "PATH=.:/usr/bin:/bin\nhash mycmd\nhash -t mycmd\nhash -l\nhash\ntype mycmd\n",
    );
    // `hash -t` prints the stored spelling verbatim.
    assert!(
        out.stdout.contains("./mycmd\n"),
        "hash -t must print ./mycmd: stdout={:?}",
        out.stdout
    );
    // `hash -l` replays the stored spelling.
    assert!(
        out.stdout.contains("builtin hash -p ./mycmd mycmd\n"),
        "hash -l must replay ./mycmd: stdout={:?}",
        out.stdout
    );
    // Bare `hash` lists the stored spelling (hits is 1: `hash -t`
    // above bumped times_found, hashlib.c:254).
    assert!(
        out.stdout.contains("\t./mycmd\n"),
        "hash listing must name ./mycmd: stdout={:?}",
        out.stdout
    );
    // `type` reads the same stored spelling.
    assert!(
        out.stdout.contains("mycmd is hashed (./mycmd)\n"),
        "type must report the hashed ./mycmd: stdout={:?}",
        out.stdout
    );
    // The anchored form must leak nowhere.
    assert!(
        !out.stdout.contains(&format!("{}/./mycmd", root.display())),
        "absolutized spelling leaked: stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}

#[test]
fn explicit_hash_controls_agree() {
    // The seed's three controls, which already agreed before the fix:
    // run-once recording, `hash -p` with a relative spelling, and an
    // absolute PATH element.
    let root = fixture("controls");
    let out = run(
        &root,
        "PATH=.:/usr/bin:/bin\n\
         mycmd >/dev/null\n\
         hash -t mycmd\n\
         hash -r\n\
         hash -p ./mycmd mycmd\n\
         hash -t mycmd\n\
         hash -r\n\
         PATH=\"$FIX/d:/usr/bin:/bin\"\n\
         hash mycmd\n\
         hash -t mycmd\n",
    );
    let abs = format!("{}/d/mycmd", root.display());
    assert_eq!(
        out.stdout,
        format!("./mycmd\n./mycmd\n{abs}\n"),
        "controls: run-once, hash -p, absolute: stdout={:?}",
        out.stdout
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.rc, 0);
}
