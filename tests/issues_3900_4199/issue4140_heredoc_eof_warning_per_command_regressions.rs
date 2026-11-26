//! rubash#4140 regressions: the `delimited by end-of-file` warning for an
//! unterminated heredoc inside a command substitution was deduplicated
//! by source text in `last_heredoc_warning_source`, and the record was
//! never scoped to a command — two DISTINCT commands in one script whose
//! comsub sources were textually identical warned only once.
//!
//! GNU warns per heredoc occurrence per parse: make_cmd.c:627
//! make_here_document issues the warning unconditionally when the
//! delimiter line is not found before end-of-input, and gather state is
//! per parse — there is no cross-command dedup anywhere in GNU. The
//! dedup's only legitimate domain is one command dispatch, where the
//! same occurrence can be planned through several expansion paths
//! (assignment-value pass, embedded-parameter pass, typed→readback
//! retry); the record is now stamped with the dispatch's
//! preopened_generation so it cannot outlive that command.
//!
//! Every expectation below is byte-verified against the GNU Bash 5.3.20
//! oracle (script-file batteries; the C.UTF-8 and C locales agree on all
//! of these shapes).

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-heredoc4140-{}-{}",
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

/// Two separate assignment commands whose comsub sources are textually
/// identical: GNU warns for each command's heredoc.
const TWO_ASSIGNMENTS: &str = r#"v="$(cat <<E
body
E)"
echo "v=$v"
v2="$(cat <<E
body
E)"
echo "v2=$v2"
"#;

#[test]
fn two_identical_assignment_commands_warn_per_command() {
    assert_both_locales(
        TWO_ASSIGNMENTS,
        "v=body\nv2=body\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n\
         case.sh: line 7: warning: here-document at line 5 delimited by end-of-file (wanted `E')\n",
    );
}

/// The unquoted echo form of the same pair (the readback/typed entries
/// differ from the assignment form; both commands must warn).
const TWO_ECHOES: &str = r#"echo $(cat <<E
body
E)
echo $(cat <<E
body
E)
"#;

#[test]
fn two_identical_echo_commands_warn_per_command() {
    assert_both_locales(
        TWO_ECHOES,
        "body\nbody\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n\
         case.sh: line 6: warning: here-document at line 4 delimited by end-of-file (wanted `E')\n",
    );
}

/// Control: ONE assignment command whose single comsub occurrence is
/// planned through the command's several expansion paths still warns
/// exactly once (one GNU parse, one warning).
const SINGLE_ASSIGNMENT: &str = r#"v="$(cat <<E
body
E)"
echo "v=$v"
"#;

#[test]
fn single_assignment_command_warns_once() {
    assert_both_locales(
        SINGLE_ASSIGNMENT,
        "v=body\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n",
    );
}

/// Control: two commands with DIFFERENT comsub bodies warn twice —
/// the dedup never merged distinct sources, and must not start to.
const TWO_DIFFERENT: &str = r#"v=$(cat <<E
body1
E)
w=$(cat <<E
body2
E)
echo "v=$v w=$w"
"#;

#[test]
fn two_different_commands_warn_twice() {
    assert_both_locales(
        TWO_DIFFERENT,
        "v=body1 w=body2\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n\
         case.sh: line 6: warning: here-document at line 4 delimited by end-of-file (wanted `E')\n",
    );
}
