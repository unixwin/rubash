//! rubash#4169 regressions: one reader unit containing two heredoc
//! command substitutions in two commands, where the first
//! substitution's closing `)` sits on its heredoc delimiter line and
//! that same line opens the second substitution, warned only for the
//! LAST substitution. The earlier substitution's extracted source
//! never carried the delimiter-line IFS_GLUE record, so its plan saw
//! `closed_by_paren=false` and the `delimited by end-of-file` warning
//! never fired.
//!
//! GNU warns per occurrence: make_cmd.c:602-611 records the delimiter
//! line carrying the closing `)` for every here-document, and
//! make_cmd.c:627 issues the warning when the delimiter line is not
//! found before end-of-input. In the lexer, the record insert in
//! `Lexer::push_line` existed only in the arm where no substitution
//! stays open after the appended line; the line that closes heredoc #1
//! while opening comsub #2 (e.g. `E); w=$(cat <<E`) took the other arm,
//! which popped the heredoc without inserting the record. The insert
//! is now one helper shared by both arms.
//!
//! Split note (captain ruling, 2026-11-24): the sibling shapes with
//! two or more comsubs inside ONE command (two echo arguments, the
//! quoted form, three arguments) are NOT pinned here. With this lexer
//! fix their records do reach every plan, but two consumer-side
//! defects still falsify the warnings — attribution anchored at the
//! outer command's start line, and identical-source dedup suppressing
//! a distinct occurrence — tracked as rubash#4190; their pins land
//! with that fix.
//!
//! Every expectation below is byte-verified against the GNU Bash
//! 5.3.20 oracle (script-file batteries; the C.UTF-8 and C locales
//! agree on all of these shapes).

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-heredoc4169-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .env("LC_ALL", lc_all)
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_both_locales(script: &str, expected_stdout: &str, expected_stderr: &str) {
    for lc_all in ["C.UTF-8", "C"] {
        let (stdout, stderr, code) = rubash_file(script, lc_all);
        assert_eq!(stdout, expected_stdout, "stdout mismatch ({lc_all})");
        assert_eq!(stderr, expected_stderr, "stderr mismatch ({lc_all})");
        assert_eq!(code, Some(0), "exit code mismatch ({lc_all})");
    }
}

/// r1: two assignment comsubs in one reader unit; the first comsub's
/// delimiter line also opens the second comsub. GNU warns for both.
const R1: &str = r#"v=$(cat <<E
body1
E); w=$(cat <<E
body2
E)
"#;

#[test]
fn two_assignment_comsubs_one_unit_warn_for_both() {
    assert_both_locales(
        R1,
        "",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n\
         case.sh: line 5: warning: here-document at line 3 delimited by end-of-file (wanted `E')\n",
    );
}

/// r2: the echo form — the first comsub is an echo argument, the second
/// an assignment on the same delimiter line. GNU warns for both and the
/// echo prints the first body.
const R2: &str = r#"echo $(cat <<E
body1
E); w=$(cat <<E
body2
E)
"#;

#[test]
fn echo_then_assignment_comsub_one_unit_warn_for_both() {
    assert_both_locales(
        R2,
        "body1\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n\
         case.sh: line 5: warning: here-document at line 3 delimited by end-of-file (wanted `E')\n",
    );
}
