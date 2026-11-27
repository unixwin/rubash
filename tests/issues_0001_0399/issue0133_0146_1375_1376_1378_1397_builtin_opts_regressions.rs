//! W4 backfill slice 1 — builtin option-parsing faces.
//!
//! * rust#133 — `complete -D` / `complete -E` (default / empty-line
//!   completion actions) are accepted with rc 0.
//! * rust#146 — `compgen -F func` really invokes the function with the
//!   completion variables bound (COMP_CWORD is -1 in this context) and
//!   prints the COMPREPLY candidates it produced.
//! * rust#1375 — a lone `--` ends option parsing for builtins:
//!   `hash -- ls` and `enable -- echo` both succeed.
//! * rust#1376 — `builtin -- echo hi` dispatches to the builtin.
//! * rust#1378 — `shopt -- -p NAME` prints the option state (name field
//!   padded to 20 columns, tab, value) and then fails rc 1 with the GNU
//!   "-p: invalid shell option name" diagnostic.
//! * rust#1397 — `enable -p BADNAME` tolerates the unknown name, still
//!   prints the full builtin list to stdout, and returns rc 0.
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
fn issue133_complete_default_and_empty_actions_accepted() {
    let s = Scratch::new("i133");
    let (out, err, rc) =
        s.run("complete -D -F myfn\necho rc=$?\ncomplete -E -F myfn\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=0\nrc=0\n");
    assert_eq!(err, "");
}

#[test]
fn issue146_compgen_f_invokes_function_with_comp_vars() {
    let s = Scratch::new("i146");
    let (out, err, rc) = s.run(
        "mycomp() { echo \"inside cword=[$COMP_CWORD]\"; COMPREPLY=(one two); }\ncompgen -F mycomp -- a\necho rc=$?\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "inside cword=[-1]\none\ntwo\nrc=0\n");
}

#[test]
fn issue1375_double_dash_ends_builtin_options() {
    let s = Scratch::new("i1375");
    let (out, err, rc) = s.run("hash -- ls\necho \"h=$?\"\nenable -- echo\necho \"e=$?\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "h=0\ne=0\n");
}

#[test]
fn issue1376_builtin_double_dash_dispatches() {
    let s = Scratch::new("i1376");
    let (out, err, rc) = s.run("builtin -- echo hi\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\nrc=0\n");
}

#[test]
fn issue1378_shopt_double_dash_dash_p_prints_state_then_errors() {
    let s = Scratch::new("i1378");
    let (out, err, rc) = s.run("shopt -- -p expand_aliases\necho rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "expand_aliases      \toff\nrc=1\n");
    assert_eq!(
        err,
        "$S/case.sh: line 1: shopt: -p: invalid shell option name\n"
    );
}

#[test]
fn issue1397_enable_p_bad_name_still_lists_and_succeeds() {
    let s = Scratch::new("i1397");
    let (out, err, rc) = s.run(
        "enable -p nosuchzz > out.txt\necho rc=$?\nhead -1 out.txt\ngrep -c '^enable ' out.txt\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "rc=0");
    assert_eq!(lines[1], "enable .");
    let count: i32 = lines[2].parse().expect("grep count line");
    assert!(
        count >= 55,
        "GNU lists 61 enable lines; got {count} (full output: {out})"
    );
}
