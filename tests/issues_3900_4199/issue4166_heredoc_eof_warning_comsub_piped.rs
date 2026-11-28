//! rubash#4166 regressions: an EOF-token-terminated heredoc inside a
//! command substitution must warn at parse/gather time even when the `cat`
//! fast-path plan later declines the command shape, and the diagnostic must
//! belong to the exact heredoc occurrence GNU gathered.
//!
//! GNU ownership: parse_comsub sets PST_EOFTOKEN and shell_eof_token
//! (parse.y:4513-4519); pending heredocs are gathered at the end of the
//! substitution parse (parse.y:4563-4567); make_here_document's EOFTOKEN arm
//! accepts delimiter-prefix + EOF-token but leaves full_line clear
//! (make_cmd.c:606-611), so make_cmd.c:626-627 emits
//! `here-document at line N delimited by end-of-file`. `lineno` is the line
//! gather_here_documents passed for THAT redirect (parse.y:3120-3131), not
//! necessarily the first heredoc/header of the batch. Body lines keep their
//! newline bytes in the document (make_cmd.c:622-624), and redir.c:348-404
//! writes the expanded document verbatim to stdin.
//!
//! Every expectation below is byte-verified against the GNU Bash 5.3.20
//! oracle through LF script files.

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-heredoc4166-{}-{}",
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

#[test]
fn three_stage_pipeline_warns_even_when_plan_declines() {
    assert_both_locales(
        "echo \"$(cat <<E | sort | tr a-z A-Z\nbody\nE)\"\n",
        "BODY\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n",
    );
}

#[test]
fn non_cat_comsub_warns_even_when_plan_declines() {
    assert_both_locales(
        "echo \"$(sort <<E\nb\na\nE)\"\n",
        "a\nb\n",
        "case.sh: line 4: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n",
    );
}

#[test]
fn warning_targets_later_cat_not_first_command() {
    assert_both_locales(
        "echo \"$(echo a; cat <<E\nbody\nE)\"\n",
        "a\nbody\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n",
    );
}

#[test]
fn later_heredoc_in_one_batch_owns_warning_and_body_newlines_survive() {
    assert_both_locales(
        "echo \"$(cat <<E; cat <<F\nbody\nE\nfbody\nF)\"\n",
        "body\nfbody\n",
        "case.sh: line 5: warning: here-document at line 3 delimited by end-of-file (wanted `F')\n",
    );
}

#[test]
fn properly_closed_heredoc_stays_silent() {
    assert_both_locales("echo \"$(cat <<E\nbody\nE\n)\"\n", "body\n", "");
}

#[test]
fn preceding_lines_keep_exact_warning_and_gather_lines() {
    assert_both_locales(
        "echo 1\necho 2\necho \"$(sort <<E\nb\na\nE)\"\n",
        "1\n2\na\nb\n",
        "case.sh: line 6: warning: here-document at line 3 delimited by end-of-file (wanted `E')\n",
    );
}
