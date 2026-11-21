//! rubash#3972 — same-line blank-only `${ }` funsub bodies.
//!
//! GNU bash 5.3.20 treats `${ }` / `${  }` / `${\t}` as a nofork command
//! substitution whose command list is empty: the expansion is the empty
//! string, rc 0. rubash misjudged the shape twice over:
//! - the executor's funsub routing gate (`funsub_body_is_terminated`)
//!   demanded a `;`/newline terminator even when the body holds no
//!   command at all, so the word fell through to the parameter machinery
//!   and reported `bad substitution`;
//! - the unclosed-input scan's funsub delimiter started with
//!   `term_ready: false` and blanks never arm it, so the closing `}` was
//!   refused and the whole input reported `unexpected EOF while looking
//!   for matching '}'` (exit 2).
//!
//! GNU anchors: parser.h:85 FUNSUB_CHAR; parse.y:5506 parse_comsub — the
//! body is a command list, and an empty list needs no terminator before
//! the closing `}` (the terminator rule binds a PRECEDING command, and
//! there is none). Boundary probed against the cloud oracle
//! (~/oracle/bash-5.3.20): the four empty forms `$()`, `$(\n)`,
//! `${ \n}`, `${  }` all print `blank ----`; `${ echo hi }` (a command
//! with no terminator) is an error on both sides and stays one here.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i3972-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn assert_blank_form(scratch: &Scratch, script: &str) {
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "rc mismatch for {script:?}; stderr:\n{err}");
    assert_eq!(out, "blank ----\n", "stdout mismatch for {script:?}");
    assert_eq!(err, "", "stderr mismatch for {script:?}");
}

/// The comsub.tests line itself: two-space body glued inside a word.
#[test]
fn issue3972_two_space_body_glued() {
    let scratch = Scratch::new("two");
    assert_blank_form(&scratch, "echo blank --${  }--\n");
}

/// One-space body.
#[test]
fn issue3972_one_space_body_glued() {
    let scratch = Scratch::new("one");
    assert_blank_form(&scratch, "echo blank --${ }--\n");
}

/// Tab-only body.
#[test]
fn issue3972_tab_body_glued() {
    let scratch = Scratch::new("tab");
    assert_blank_form(&scratch, "echo blank --${\t}--\n");
}

/// Mixed blanks (space/tab/space) body.
#[test]
fn issue3972_mixed_blank_body_glued() {
    let scratch = Scratch::new("mixed");
    assert_blank_form(&scratch, "echo blank --${ \t }--\n");
}

/// Three-space body.
#[test]
fn issue3972_three_space_body_glued() {
    let scratch = Scratch::new("three");
    assert_blank_form(&scratch, "echo blank --${   }--\n");
}

/// Whole-word blank funsub: the script must run to completion (the EOF
/// face killed the entire input before any line executed).
#[test]
fn issue3972_whole_word_blank_body_runs() {
    let scratch = Scratch::new("whole");
    let (out, err, rc) = scratch.run("echo ${  }\necho done\n");
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert_eq!(out, "\ndone\n");
    assert_eq!(err, "");
}

/// Blank funsub as an assignment value.
#[test]
fn issue3972_blank_body_assignment() {
    let scratch = Scratch::new("assign");
    let (out, err, rc) = scratch.run("x=${  }\nrc=$?\necho \"x=[${x}] rc=$rc\"\n");
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert_eq!(out, "x=[] rc=0\n");
    assert_eq!(err, "");
}

/// Blank funsub glued on both sides inside a word.
#[test]
fn issue3972_blank_body_glued_both_sides() {
    let scratch = Scratch::new("glued");
    let (out, err, rc) = scratch.run("echo a${ }b\n");
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert_eq!(out, "ab\n");
    assert_eq!(err, "");
}

/// Blank funsub inside double quotes.
#[test]
fn issue3972_blank_body_in_double_quotes() {
    let scratch = Scratch::new("dquote");
    let (out, err, rc) = scratch.run("echo \"[${  }]\"\n");
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert_eq!(out, "[]\n");
    assert_eq!(err, "");
}

/// Two adjacent blank funsubs in one word stream.
#[test]
fn issue3972_adjacent_blank_bodies() {
    let scratch = Scratch::new("adjacent");
    let (out, err, rc) = scratch.run("echo ${  }${ }\necho end\n");
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert_eq!(out, "\nend\n");
    assert_eq!(err, "");
}

/// Blank funsub nested as a command inside a normal funsub body.
#[test]
fn issue3972_blank_body_nested_in_funsub() {
    let scratch = Scratch::new("nested");
    let (out, err, rc) = scratch.run("echo ${ echo ${  }; }\n");
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert_eq!(out, "\n");
    assert_eq!(err, "");
}

/// Control: a normal terminated funsub still runs its body.
#[test]
fn issue3972_control_terminated_funsub() {
    let scratch = Scratch::new("ctrlterm");
    let (out, err, rc) = scratch.run("echo ${ echo hi; }\n");
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert_eq!(out, "hi\n");
    assert_eq!(err, "");
}

/// Control: newline-carrying blank bodies already worked and must not
/// change (the `${ \n}` empty form from comsub.tests).
#[test]
fn issue3972_control_newline_blank_body() {
    let scratch = Scratch::new("ctrlnl");
    assert_blank_form(&scratch, "echo blank --${ \n}--\n");
}
