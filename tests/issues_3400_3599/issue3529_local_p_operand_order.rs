//! rubash#3529 — `local -p NAME...` reports every operand in operand
//! order, option-shaped words included.
//!
//! GNU builtins/declare.def:389-406 (declare_internal, pflag branch with
//! local_var set) walks the operand list once, in order, and calls
//! builtins/setattr.def:555 show_localname_attributes() on each word
//! verbatim: a word that names a local of the CURRENT variable context is
//! printed (`local -` for the option-snapshot local), anything else —
//! a missing name, an option-shaped word after the first operand (`-x`,
//! `--help`, `+x`, `--`), a valued word (`x=1`), a subscripted word —
//! is reported `not found` and fails the command. `-g` does not change
//! the lookup (show_localname_attributes ignores it).
//!
//! The pre-fix tree reported missing plain names in a pre-pass (before
//! every other diagnostic), then dropped the core call whenever no plain
//! name was left — so `local -p nope1 -x` never reported `-x` — printed
//! `local -` before every other operand, and under `-g` printed the
//! global instead of the frame's local.
//!
//! Expectations verified against GNU bash 5.3.20 with script files.
//! stdout and stderr are asserted separately: the merged-stream
//! interleave is the repo-wide buffered-output limitation, not this lane.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3529-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn missing_name_then_option_shaped_word() {
    let (stdout, stderr) = rubash_file("nope-x", "f() { local -p nope1 -x; echo \"rc:$?\"; }\nf\n");
    assert_eq!(stdout, "rc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: local: nope1: not found\n\
         t.sh: line 1: local: -x: not found\n"
    );
}

#[test]
fn found_name_then_help_word() {
    let (stdout, stderr) = rubash_file(
        "x-help",
        "f() { local x=1; local -p x --help; echo \"rc:$?\"; }\nf\n",
    );
    assert_eq!(stdout, "declare -- x=\"1\"\nrc:1\n");
    assert_eq!(stderr, "t.sh: line 1: local: --help: not found\n");
}

#[test]
fn missing_names_and_option_word_keep_operand_order() {
    let (stdout, stderr) = rubash_file(
        "order",
        "f() { local -p nope1 nope2 -x nope3; echo \"rc:$?\"; }\nf\n\
         g() { local a=1 b=2; local -p a -x b nope; echo \"rc:$?\"; }\ng\n\
         h() { local v=2; local -p v nope1 -x; echo \"rc:$?\"; }\nh\n",
    );
    assert_eq!(
        stdout,
        "rc:1\ndeclare -- a=\"1\"\ndeclare -- b=\"2\"\nrc:1\ndeclare -- v=\"2\"\nrc:1\n"
    );
    assert_eq!(
        stderr,
        "t.sh: line 1: local: nope1: not found\n\
         t.sh: line 1: local: nope2: not found\n\
         t.sh: line 1: local: -x: not found\n\
         t.sh: line 1: local: nope3: not found\n\
         t.sh: line 3: local: -x: not found\n\
         t.sh: line 3: local: nope: not found\n\
         t.sh: line 5: local: nope1: not found\n\
         t.sh: line 5: local: -x: not found\n"
    );
}

#[test]
fn plus_and_double_dash_operands_are_names() {
    let (stdout, stderr) = rubash_file(
        "plus-dd",
        "f() { local x=1; local -p nope +x --; echo \"rc:$?\"; }\nf\n\
         g() { local x=1; local -p -- x -- -p; echo \"rc:$?\"; }\ng\n",
    );
    assert_eq!(stdout, "rc:1\ndeclare -- x=\"1\"\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: local: nope: not found\n\
         t.sh: line 1: local: +x: not found\n\
         t.sh: line 1: local: --: not found\n\
         t.sh: line 3: local: --: not found\n\
         t.sh: line 3: local: -p: not found\n"
    );
}

#[test]
fn dash_local_prints_in_operand_order() {
    let (stdout, stderr) = rubash_file(
        "dash",
        "f() { local x=1; local -; local -p x - x; echo \"rc:$?\"; }\nf\n",
    );
    assert_eq!(
        stdout,
        "declare -- x=\"1\"\nlocal -\ndeclare -- x=\"1\"\nrc:0\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn valued_and_subscripted_words_are_looked_up_verbatim() {
    let (stdout, stderr) = rubash_file(
        "verbatim",
        "f() { local x=1 a=(1 2); local -p x=1 'a[0]' a x; echo \"rc:$?\"; }\nf\n",
    );
    assert_eq!(
        stdout,
        "declare -a a=([0]=\"1\" [1]=\"2\")\ndeclare -- x=\"1\"\nrc:1\n"
    );
    assert_eq!(
        stderr,
        "t.sh: line 1: local: x=1: not found\n\
         t.sh: line 1: local: a[0]: not found\n"
    );
}

#[test]
fn global_flag_does_not_change_the_frame_lookup() {
    let (stdout, stderr) = rubash_file(
        "g",
        "x=g\nf() { local x=1; local -g -p x; echo \"rc:$?\"; }\nf\n\
         g() { x=g; local -p x nope; echo \"rc:$?\"; }\ng\n",
    );
    assert_eq!(stdout, "declare -- x=\"1\"\nrc:0\nrc:1\n");
    assert_eq!(
        stderr,
        "t.sh: line 4: local: x: not found\n\
         t.sh: line 4: local: nope: not found\n"
    );
}
