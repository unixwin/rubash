//! W4 backfill slice 1 — parameter expansion / shell state faces.
//!
//! * rust#67 — under `set -u`, `$((a))` for a set-but-zero variable must
//!   not raise an unbound error.
//! * rust#150 — `${x:-$(...)}` must not leak a stray `}` into the value.
//! * rust#208 — under `set -u`, `${#b[@]}` for an unset array prints the
//!   unbound diagnostic but is non-fatal in a script (GNU continues).
//! * rust#209 — a backslash-escaped `@` inside `${x:+...}` stays a literal
//!   `@` in the result (`a\@b` -> `a@b`).
//! * rust#212 — word splitting of `${v:-"a b"}` keeps the quoted default
//!   as ONE array element.
//!
//! All expectations were measured against GNU bash 5.3.20. Tests only.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-bfw4-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        let norm = |b: &[u8]| String::from_utf8_lossy(b).replace(self.dir.to_str().unwrap(), "$S");
        (
            norm(&out.stdout),
            norm(&out.stderr),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        self.finish(out)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue67_nounset_arith_on_set_zero_var() {
    let s = Scratch::new("i67");
    let (out, err, rc) = s.run("set -u\na=0\necho $((a))\necho done\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "0\ndone\n");
    assert_eq!(err, "");
}

#[test]
fn issue150_default_comsub_no_stray_brace() {
    let s = Scratch::new("i150");
    let (out, err, rc) = s.run("x=y\nv=${x:-$(echo sub)}\necho \"$v\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "y\n");
}

#[test]
fn issue208_nounset_array_length_is_nonfatal_with_diagnostic() {
    let s = Scratch::new("i208");
    let (out, err, rc) = s.run("set -u\nunset b\necho ${#b[@]}\necho after\n");
    // GNU: the expansion fails with a diagnostic, the echo never runs, but
    // the script continues with the next command and exits 0.
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "after\n");
    assert_eq!(err, "$S/case.sh: line 3: b: unbound variable\n");
}

#[test]
fn issue209_escaped_at_in_alternate_value() {
    let s = Scratch::new("i209");
    let (out, err, rc) = s.run("x=set\nv=${x:+a\\@b}\necho \"$v\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "a@b\n");
}

#[test]
fn issue212_quoted_default_is_one_array_element() {
    let s = Scratch::new("i212");
    let (out, err, rc) = s.run("unset v\na=(${v:-\"a b\"})\necho \"n=${#a[@]} e0=[${a[0]}]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "n=1 e0=[a b]\n");
}
