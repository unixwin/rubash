//! rubash#3359 — a command hashed through a relative PATH entry is
//! re-validated on every read of the hash table.
//!
//! GNU hashcmd.c:126 phash_search is the one read of a remembered
//! location, used by execution (findcmd.c:365 search_for_command),
//! `type`/`command -v` (type.def:353 describe_command) and `hash -t`
//! (hash.def:284). For a stored relative path (HASH_RELPATH,
//! hashcmd.c:115) it returns the `./`-prefixed form when executable
//! (hashcmd.c:146-161); otherwise, when the stored path starts with `.`
//! and its directory is the cwd itself (hashcmd.c:176-187 same_file), it
//! returns NULL and search_for_command walks $PATH again — so a deleted
//! `./mycmd` is "command not found" (127), `type`/`hash -t` report not
//! found, and the entry itself stays in the table until a walk
//! re-inserts it (findcmd.c:416-426 phash_insert, times_found = 1).
//! `hash NAME` stores the search's as-written join too
//! (hash.def:213-225 add_hashed_command -> find_user_command), not the
//! cwd-anchored path. rubash served the stored spelling unvalidated and
//! `hash NAME` stored the anchored path.
//!
//! Expected stdout/stderr are the GNU bash 5.3.20 outputs of the same
//! script file run from an empty scratch directory.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

/// Run `script` as `case.sh` from a fresh empty scratch dir; return
/// (stdout, stderr).
fn run(tag: &str, script: &str) -> (String, String) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash-issue3359-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case script");
    let output = Command::new(RUBASH)
        .arg("case.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// `hash NAME` records `./mycmd`; once deleted, run / `hash -t` / `type` /
/// `command -v` / subshell / `command` all miss, and the entry stays listed.
#[test]
fn issue3359_explicit_hash() {
    let (out, err) = run(
        "explicit_hash",
        r###"mk() { printf '#!/bin/sh\necho REL\n' > mycmd; chmod +x mycmd; }
PATH=.:/usr/bin:/bin
echo "== explicit"
mk; hash mycmd; hash -t mycmd; type mycmd; hash -l | grep mycmd
rm mycmd
mycmd; echo "run rc=$?"
hash -t mycmd; echo "hash-t rc=$?"
type mycmd; echo "type rc=$?"
command -v mycmd; echo "command-v rc=$?"
(mycmd); echo "subshell rc=$?"
command mycmd; echo "command rc=$?"
hash -l | grep mycmd
"###,
    );
    assert_eq!(
        out,
        r###"== explicit
./mycmd
mycmd is hashed (./mycmd)
builtin hash -p ./mycmd mycmd
run rc=127
hash-t rc=1
type rc=1
command-v rc=1
subshell rc=127
command rc=127
builtin hash -p ./mycmd mycmd
"###
    );
    assert_eq!(
        err,
        r###"case.sh: line 6: mycmd: command not found
case.sh: line 7: hash: mycmd: not found
case.sh: line 8: type: mycmd: not found
case.sh: line 10: mycmd: command not found
case.sh: line 11: mycmd: command not found
"###
    );
}

/// An execution-recorded `./mycmd`: deleted -> every read misses; recreated
/// -> the same entry serves again.
#[test]
fn issue3359_run_once() {
    let (out, err) = run(
        "run_once",
        r###"mk() { printf '#!/bin/sh\necho REL\n' > mycmd; chmod +x mycmd; }
PATH=.:/usr/bin:/bin
echo "== runonce"
hash -r; mk; mycmd; hash -t mycmd
rm mycmd
mycmd; echo "run rc=$?"
mycmd; echo "run2 rc=$?"
type mycmd; echo "type rc=$?"
hash -t mycmd; echo "hash-t rc=$?"
echo "== recreated"
mk; hash -t mycmd; type mycmd; mycmd; echo "rc=$?"
"###,
    );
    assert_eq!(
        out,
        r###"== runonce
REL
./mycmd
run rc=127
run2 rc=127
type rc=1
hash-t rc=1
== recreated
./mycmd
mycmd is hashed (./mycmd)
REL
rc=0
"###
    );
    assert_eq!(
        err,
        r###"case.sh: line 6: mycmd: command not found
case.sh: line 7: mycmd: command not found
case.sh: line 8: type: mycmd: not found
case.sh: line 9: hash: mycmd: not found
"###
    );
}

/// A miss walks $PATH and the hit elsewhere overwrites the entry
/// (phash_insert, times_found = 1).
#[test]
fn issue3359_refound() {
    let (out, err) = run(
        "refound",
        r###"mk() { printf '#!/bin/sh\necho REL\n' > mycmd; chmod +x mycmd; }
PATH=.:/usr/bin:/bin
echo "== re-found elsewhere"
rm mycmd; mkdir -p d2; printf '#!/bin/sh\necho D2\n' > d2/mycmd; chmod +x d2/mycmd
PATH=.:/usr/bin:/bin:$PWD/d2; mk; hash mycmd; rm mycmd
mycmd; echo "rc=$?"; hash -t mycmd | sed "s|$PWD|W|"; hash | grep mycmd | sed "s|$PWD|W|"
"###,
    );
    assert_eq!(
        out,
        r###"== re-found elsewhere
D2
rc=0
W/d2/mycmd
   1	W/d2/mycmd
"###
    );
    assert_eq!(
        err,
        r###"rm: cannot remove 'mycmd': No such file or directory
"###
    );
}

/// Controls: a relative entry whose directory is NOT the cwd is served
/// as stored (ENOENT at exec); a plain `rel1` entry is served `./`-dotted.
#[test]
fn issue3359_relative_entries() {
    let (out, err) = run(
        "relative_entries",
        r###"mk() { printf '#!/bin/sh\necho REL\n' > mycmd; chmod +x mycmd; }
PATH=.:/usr/bin:/bin
echo "== other-dir relative entry keeps its spelling"
mkdir -p rel1; printf '#!/bin/sh\necho FOO\n' > rel1/foo; chmod +x rel1/foo
PATH=./rel1:/usr/bin:/bin; foo; rm rel1/foo
foo; echo "rc=$?"; type foo; hash -t foo
echo "== plain relative entry is served dotted"
printf '#!/bin/sh\necho FOO\n' > rel1/foo; chmod +x rel1/foo
PATH=rel1:/usr/bin:/bin; foo; hash -l | grep foo; hash -t foo; type foo
"###,
    );
    assert_eq!(
        out,
        r###"== other-dir relative entry keeps its spelling
FOO
rc=127
foo is hashed (./rel1/foo)
./rel1/foo
== plain relative entry is served dotted
FOO
builtin hash -p rel1/foo foo
./rel1/foo
foo is hashed (./rel1/foo)
"###
    );
    assert_eq!(
        err,
        r###"case.sh: line 6: ./rel1/foo: No such file or directory
"###
    );
}
