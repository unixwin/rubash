//! rubash PR #2643 (jhermelink) — aliases were expanded inside `$(...)`
//! but not inside backquote command substitutions, unlike GNU bash.
//!
//! Root cause (from the PR): the stream-level alias pass
//! (`expand_comsub_alias_bodies`) splices `$(` bodies but skips backquote
//! units, and `comsub_body_alias_splice_extracted` then skipped the
//! per-body splice whenever the line was marked `__RUBASH_ALIAS_STREAMED`,
//! so a backquote body was never expanded.
//!
//! GNU anchor (oracle = GNU bash 5.3.20, probed 2026-11-03): with
//! `shopt -s expand_aliases` and `alias ll='echo ALIAS-USED'` —
//! `echo \`ll\`` -> ALIAS-USED; inside double quotes -> ALIAS-USED;
//! nested in `$(...)` -> ALIAS-USED; after `shopt -u expand_aliases`
//! both substitution forms fail identically (`ll: command not found`,
//! empty substitution, rc 0 for the enclosing echo).
//!
//! The fix narrows the streamed-skip in
//! `comsub_body_alias_splice_extracted` so a backquote body
//! (`backtick_body_diagnostic` set) still receives its per-body splice.
//! Regression pin added by the captain on top of the contributor's fix.

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
        let dir = std::env::temp_dir().join(format!(
            "rubash-i2643-{tag}-{}-{nanos}",
            std::process::id()
        ));
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

#[test]
fn pr2643_backquote_body_expands_alias() {
    let s = Scratch::new("core");
    let (stdout, _stderr, rc) = s.run(
        "shopt -s expand_aliases\nalias ll='echo ALIAS-USED'\necho \"backquote: `ll`\"\n",
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "backquote: ALIAS-USED\n");
}

#[test]
fn pr2643_backquote_matches_dollar_form() {
    let s = Scratch::new("equiv");
    let (stdout, _stderr, rc) = s.run(
        "shopt -s expand_aliases\nalias ll='echo ALIAS-USED'\necho \"dollar: $(ll)\"\necho \"backquote: `ll`\"\n",
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "dollar: ALIAS-USED\nbackquote: ALIAS-USED\n");
}

#[test]
fn pr2643_alias_with_args_in_argument_position() {
    // The reporter's shape: an alias carrying its own words, invoked
    // inside backquotes in argument position — the alias's words must
    // survive (they were silently dropped before the fix).
    let s = Scratch::new("args");
    let (stdout, _stderr, rc) = s.run(
        "shopt -s expand_aliases\nalias greet='echo GREET'\necho \"arg: `greet hi`\"\n",
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "arg: GREET hi\n");
}

#[test]
fn pr2643_backquote_inside_double_quotes_and_nested() {
    let s = Scratch::new("nested");
    let (stdout, _stderr, rc) = s.run(
        "shopt -s expand_aliases\nalias ll='echo ALIAS-USED'\necho \"dq: \\\"`ll`\\\"\"\necho \"nested: $(echo `ll`)\"\n",
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "dq: \"ALIAS-USED\"\nnested: ALIAS-USED\n");
}

#[test]
fn pr2643_expand_aliases_off_backquote_matches_dollar() {
    // Negative parity: with expand_aliases unset after the definition,
    // GNU expands in NEITHER form; the two forms must stay equivalent
    // (the fix must not make backquotes expand where $() does not).
    let s = Scratch::new("off");
    let (stdout, _stderr, rc) = s.run(
        "shopt -s expand_aliases\nalias ll='echo ALIAS-USED'\nshopt -u expand_aliases\necho \"off-bq: `ll`\"\necho \"off-dollar: $(ll)\"\n",
    );
    assert_eq!(rc, 0);
    assert_eq!(stdout, "off-bq: \noff-dollar: \n");
}
