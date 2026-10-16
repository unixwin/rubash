//! Issue unixwin/rubash#675: a double-quoted for-list word (`for p in "f*" "b*"`) must
//! bind plain data like assignment does — GNU subst.c:4807 dequote_string strips the
//! CTLESC quoting pairs off every word once pathname expansion has decided. The loop
//! variable kept its CTLESC carrier (${#p} was 3), so an unquoted `$p` holding glob
//! characters stayed glob-dead in pattern position: per-arm expand_word_leave_quoted
//! (execute_cmd.c:3712) saw quoted text where GNU sees plain data, and `case foobar
//! in $p)` missed. The for-list finalizer now dequotes bound values; the #662 static
//! classifier is untouched.

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
fn loop_var_unquoted_pattern_stays_glob_active() {
    let (stdout, _stderr, code) = rubash(
        r#"for p in "f*" "b*"; do case foobar in $p) echo HIT:$p;; *) echo MISS:$p;; esac; done"#,
    );
    assert_eq!(stdout, "HIT:f*\nMISS:b*\n");
    assert_eq!(code, Some(0));
}

#[test]
fn double_quoted_for_word_binds_plain_data() {
    let (stdout, _stderr, code) = rubash(r#"for p in "f*"; do echo "${#p}"; done"#);
    assert_eq!(stdout, "2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn quoted_for_word_still_literal_under_quotes() {
    // Dequoting must not make the value glob-active where GNU keeps it literal:
    // a quoted `"$p"` expansion never participates in pattern matching.
    let (stdout, _stderr, code) = rubash(
        r#"for p in "f*" "b*"; do case foobar in "$p") echo LIT:$p;; *) echo NOTLIT:$p;; esac; done"#,
    );
    assert_eq!(stdout, "NOTLIT:f*\nNOTLIT:b*\n");
    assert_eq!(code, Some(0));
}
