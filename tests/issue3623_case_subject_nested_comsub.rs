//! rubash#3623 — a single-command `$(cmd)` nested in a `case` inside
//! `$(...)` corrupted the outer substitution's delimiter scan.
//!
//! `dollar_command_substitution` does not count `(`/`)` while
//! `case_depth > 0` (they are case-pattern syntax), so the inner
//! comsub was scanned inline: its command name completed as the outer
//! subject word and its `)` satisfied the case-close arm, truncating
//! `$(case $(ls) in ...)` to `$(case $(ls` and surfacing the inner
//! `)` as a syntax error.
//!
//! GNU mechanism (parse.y:4451 parse_comsub): every `$(` is a fresh
//! parse; the case subject/pattern state is per-level, so the nested
//! unit is opaque to the outer case machine. The fix recursively
//! skips `$(` as a unit while `case_depth > 0` — same shape as the
//! existing double-quote nested-`$(` skip.
//!
//! GNU bash 5.3.20 oracle (verified script-file/-c probes):
//!   x=$(case $(printf x) in x) echo hit;; *) echo miss;; esac) -> hit
//!   x=$( case $(printf y) in x) echo hit;; *) echo miss;; esac ) -> miss
//!   x=$(case v in $(printf v)) echo inpat;; esac) -> inpat
//!   x=$(case v in v) echo $(printf ok);; esac) -> ok
//!   x=$(case $(printf z) in z|y) echo zw;; esac) -> zw

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn single_command_comsub_in_case_subject_scans_clean() {
    let (stdout, stderr, code) =
        rubash("x=$(case $(printf x) in x) echo hit;; *) echo miss;; esac); echo \"$x\"");
    assert_eq!(stdout, "hit\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn subject_comsub_miss_falls_to_star_pattern() {
    let (stdout, stderr, code) =
        rubash("x=$( case $(printf y) in x) echo hit;; *) echo miss;; esac ); echo \"$x\"");
    assert_eq!(stdout, "miss\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn multiline_comsub_case_subject_form() {
    // The original comsub-posix.tests shape.
    let (stdout, stderr, code) =
        rubash("x=$(\n    case $(printf e) in\n\te) echo foobix;;\n    esac\n)\necho \"$x\"");
    assert_eq!(stdout, "foobix\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn comsub_in_case_pattern_and_body() {
    let (stdout, _, code) = rubash("x=$(case v in $(printf v)) echo inpat;; esac); echo \"$x\"");
    assert_eq!(stdout, "inpat\n");
    assert_eq!(code, Some(0));

    let (stdout, _, code) = rubash("x=$(case v in v) echo $(printf ok);; esac); echo \"$x\"");
    assert_eq!(stdout, "ok\n");
    assert_eq!(code, Some(0));
}

#[test]
fn alternation_pattern_after_subject_comsub() {
    let (stdout, _, code) = rubash("x=$(case $(printf z) in z|y) echo zw;; esac); echo \"$x\"");
    assert_eq!(stdout, "zw\n");
    assert_eq!(code, Some(0));
}

// The sharpest failing shape at the pre-fix tree is a ONE-WORD subject
// comsub with no space inside it (`$(true)`, `$(ls)`, `$(pwd)`): the
// word stays in `word` when the inner `)` arrives, so the truncated-case
// close arm fired (`$(case $(ls` -> syntax error, rc 2). A two-word
// comsub (`$(printf e)`) cleared the case stage at the blank and did not
// fail, which is why this class needs its own pins.
#[test]
fn single_word_subject_comsub_keeps_outer_substitution_closed() {
    let (stdout, stderr, code) = rubash("x=$(case $(true) in *) echo hit;; esac); echo \"$x\"");
    assert_eq!(stdout, "hit\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn single_word_subject_comsub_multiline_form() {
    let (stdout, stderr, code) =
        rubash("y=$(\n    case $(true) in\n\t*) echo got;;\n    esac\n)\necho \"$y\"");
    assert_eq!(stdout, "got\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn single_word_subject_comsub_empty_subject_patterns() {
    // `$(true)` has an empty subject: GNU matches it against `""`.
    let (stdout, stderr, code) =
        rubash("x=$(case $(true) in \"\") echo empty;; *) echo other;; esac); echo \"$x\"");
    assert_eq!(stdout, "empty\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
