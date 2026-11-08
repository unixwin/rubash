//! rubash#3147 — describing a command found by a FRESH PATH walk must
//! print the walk's as-written join (`<PATH element>/<name>`), not a
//! cwd-anchored absolute form with `/./` residue.
//!
//! GNU anchor: findcmd.c:553 `find_user_command_in_path` constructs
//! exactly one string per hit, `sh_makepath (xpath, name, 0)` — the
//! PATH element verbatim + `/` + name (no second `/` when the element
//! already ends with one; an empty element becomes `.` via MP_DOCWD).
//! builtins/type.def:211 `describe_command` prints that constructed
//! name for `command -v`, `type` and `type -a`; after the command has
//! run, the hash table records the same constructed name
//! (`foo is hashed (./rel1/foo)`). Only `command -V` absolutizes —
//! that is a separate describe face and is NOT pinned here.
//!
//! rubash face at the pin's base: `command_path_with_bridge`
//! (src/executor/lookup_paths.rs) maps the fresh walk through
//! `find_user_command`'s anchored PathBuf + `shell_display_path`, so
//! every relative-element describe prints `/tmp/<fixture>/./rel1/foo`.
//!
//! Harness: script files run with cwd = a fresh fixture dir (the
//! issue977 shape), `LC_ALL=C.UTF-8` pinned on the child. All PATH
//! tails use a nonexistent sentinel element so the walk is hermetic.
//! Expectations are byte-exact from GNU bash 5.3.20 probes
//! (2026-11-09). Unix-only: shebang fixtures + exec bits.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "rubash-3147-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join("rel1")).expect("fixture rel1");
        std::fs::create_dir_all(dir.join("abs1")).expect("fixture abs1");
        for (rel, body) in [
            ("rel1/foo", "#!/bin/sh\necho ran-foo-rel\n"),
            ("abs1/foo", "#!/bin/sh\necho ran-foo-abs\n"),
            ("foo", "#!/bin/sh\necho ran-foo-cwd\n"),
        ] {
            let path = dir.join(rel);
            std::fs::write(&path, body).expect("fixture script");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("fixture chmod");
        }
        Fixture { dir }
    }

    fn run(&self, script: &str) -> Outcome {
        std::fs::write(self.dir.join("case.sh"), script).expect("write case.sh");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("case.sh")
            .current_dir(&self.dir)
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn rubash");
        Outcome {
            stdout: output.stdout,
            stderr: output.stderr,
            code: output.status.code().unwrap_or(-1),
        }
    }

    fn display(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn expect(script: &str, want_stdout: &str, want_code: i32, ctx: &str) -> Outcome {
    let fix = Fixture::new(ctx);
    let script = script.replace("@FIX@", &fix.display());
    let out = fix.run(&script);
    assert_eq!(
        out.stdout,
        want_stdout.replace("@FIX@", &fix.display()).as_bytes(),
        "{ctx}: stdout mismatch; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.code,
        want_code,
        "{ctx}: rc mismatch; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{ctx}: stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// The issue's headline shape: `./rel1` element, all describe verbs.
#[test]
fn dot_slash_element_describe_verbs() {
    expect(
        "PATH=./rel1:/nonexistent3147\ncommand -v foo\ntype foo\ntype -a foo\ntype -t foo\n",
        "./rel1/foo\nfoo is ./rel1/foo\nfoo is ./rel1/foo\nfile\n",
        0,
        "dot-slash",
    );
}

/// A bare relative element joins verbatim — no `./` is invented.
#[test]
fn bare_relative_element() {
    expect(
        "PATH=rel1:/nonexistent3147\ncommand -v foo\ntype foo\n",
        "rel1/foo\nfoo is rel1/foo\n",
        0,
        "bare-relative",
    );
}

/// An empty PATH element means cwd and spells `./foo` (MP_DOCWD).
#[test]
fn empty_element_means_cwd() {
    expect(
        "PATH=:/nonexistent3147\ncommand -v foo\ntype foo\n",
        "./foo\nfoo is ./foo\n",
        0,
        "empty-element",
    );
}

/// An explicit `.` element spells the same `./foo`.
#[test]
fn dot_element_means_cwd() {
    expect(
        "PATH=.:/nonexistent3147\ncommand -v foo\n",
        "./foo\n",
        0,
        "dot-element",
    );
}

/// A trailing-slash element must not double the join slash.
#[test]
fn trailing_slash_element() {
    expect(
        "PATH=./rel1/:/nonexistent3147\ncommand -v foo\n",
        "./rel1/foo\n",
        0,
        "trailing-slash",
    );
}

/// Control: an absolute element already IS the as-written join.
#[test]
fn absolute_element_control() {
    expect(
        "PATH=@FIX@/abs1:/nonexistent3147\ncommand -v foo\ntype foo\n",
        "@FIX@/abs1/foo\nfoo is @FIX@/abs1/foo\n",
        0,
        "absolute-control",
    );
}

/// `type -a` enumerates every element's own join, in PATH order.
#[test]
fn type_a_mixed_elements() {
    expect(
        "PATH=./rel1:@FIX@/abs1\ntype -a foo\n",
        "foo is ./rel1/foo\nfoo is @FIX@/abs1/foo\n",
        0,
        "type-a-mixed",
    );
}

/// Chain: fresh describe -> execute (records the name) -> describe
/// again. Every step must keep the constructed spelling; the hashed
/// `type` form parenthesizes the same string.
#[test]
fn chain_fresh_execute_describe_again() {
    expect(
        "PATH=./rel1:/nonexistent3147\ncommand -v foo\nfoo\ncommand -v foo\ntype foo\n",
        "./rel1/foo\nran-foo-rel\n./rel1/foo\nfoo is hashed (./rel1/foo)\n",
        0,
        "chain",
    );
}

/// A relative element found in a later PATH position spells the same.
#[test]
fn relative_element_second_position() {
    expect(
        "PATH=/nonexistent3147:./rel1\ncommand -v foo\n",
        "./rel1/foo\n",
        0,
        "second-position",
    );
}

/// Control: a miss stays a silent rc-1 miss.
#[test]
fn not_found_control() {
    expect(
        "PATH=./rel1:/nonexistent3147\ncommand -v nosuchcmd3147\necho rc=$?\n",
        "rc=1\n",
        0,
        "not-found",
    );
}
