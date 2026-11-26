//! rubash#4141 regressions: a quoted command substitution whose heredoc
//! feeds a pipeline (`"$(cat <<E | sort ...)"`) leaked the delimiter line
//! into the captured output as `E` + IFS_GLUE (0x1c).
//!
//! Route map (measured): the `cat` heredoc comsub plan models at most
//! the `[sort, -u]` pipeline arm, so a plain `sort` (or `cat`, or a
//! third stage) declines at produce time and the body falls to the real
//! executor, which re-tokenizes the extracted body text. In that text
//! the delimiter line carries the reader's IFS_GLUE record byte — the
//! outer reader splices it in front of the closing `)` to record that
//! the substitution closed on the delimiter line (make_cmd.c:602-611),
//! and extraction consumes the `)` itself, leaving `E\x1c`. The plan
//! route strips IFS_GLUE wholesale before tokenizing, so it never saw
//! the byte; the real route's gatherer (`feed_awaiting_body`) compared
//! the raw line against the delimiter, its in-comsub trailing-residue
//! arm required the rest after the delimiter to trim to empty, and
//! `\x1c` is not whitespace — so the line failed every arm and was
//! pushed into the body.
//!
//! GNU: gather_here_documents (make_cmd.c:602) ends the document at the
//! exact delimiter line, and the PST_EOFTOKEN arm (make_cmd.c:606-611)
//! ends it at the delimiter followed by the substitution's EOF token.
//! In the extracted body the EOF token is already consumed; the glue
//! record is its only trace, so delimiter + glue is a terminator line,
//! not body text.
//!
//! Warning behaviour is unchanged by the fix and already matches GNU
//! for every shape asserted with stderr below: one delimited-by-end-of
//! -file warning per `E)` shape, emitted by the plan's closed-by-paren
//! arm during admission (which runs before the produce decline). The
//! three-stage shape is asserted on stdout only: its missing warning
//! (the plan declines it before the warning arm) is a pre-existing
//! warning-side gap of the same family as the unquoted form's, not
//! this issue.
//!
//! Every expectation below is byte-verified against the GNU Bash
//! 5.3.20 oracle (script-file batteries; the C.UTF-8 and C locales
//! agree on all of these shapes).

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-comsub4141-{}-{}",
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

const BATTERY: &str = r#"echo "$(cat <<E | sort
body
E)"
echo "$(cat <<E | sort
zebra
apple
E)"
echo "$(cat <<E | cat
body
E)"
echo "$(cat <<-E | sort
	body
	E)"
x=world
echo "$(cat <<E | sort
hello $x
E)"
echo "$(cat <<'E' | sort
body
E)"
"#;

const BATTERY_EXPECTED: &str = "\
body
apple
zebra
body
body
hello world
body
";

const BATTERY_EXPECTED_STDERR: &str = "\
case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')
case.sh: line 7: warning: here-document at line 4 delimited by end-of-file (wanted `E')
case.sh: line 10: warning: here-document at line 8 delimited by end-of-file (wanted `E')
case.sh: line 13: warning: here-document at line 11 delimited by end-of-file (wanted `E')
case.sh: line 17: warning: here-document at line 15 delimited by end-of-file (wanted `E')
case.sh: line 20: warning: here-document at line 18 delimited by end-of-file (wanted `E')
";

#[test]
fn quoted_comsub_piped_heredoc_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C.UTF-8");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert_eq!(stderr, BATTERY_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn quoted_comsub_piped_heredoc_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert_eq!(stderr, BATTERY_EXPECTED_STDERR, "stderr mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// A third pipeline stage also declines the plan (it models at most
/// two) and leaks the same way through the real route. Stdout only —
/// see the header note on this shape's pre-existing warning gap.
#[test]
fn three_stage_pipeline_delimiter_does_not_leak() {
    let script = "echo \"$(cat <<E | sort | tr a-z A-Z\nbody\nE)\"\n";
    let (stdout, _stderr, code) = rubash_file(script, "C.UTF-8");
    assert_eq!(stdout, "BODY\n", "stdout mismatch");
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// The widened terminator arm must still refuse a line that is the
/// delimiter plus real words: `E more` is body text, not a terminator.
#[test]
fn delimiter_plus_words_line_stays_body() {
    let script = "echo \"$(cat <<E | sort\nE more\nE)\"\n";
    let (stdout, stderr, code) = rubash_file(script, "C.UTF-8");
    assert_eq!(stdout, "E more\n", "stdout mismatch");
    assert_eq!(
        stderr,
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n",
        "stderr mismatch"
    );
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// Adjacent routes that were already correct must stay correct: the
/// plan-produced `[sort, -u]` arm, the non-piped quoted form, the
/// assignment form, and the unquoted form (stdout only — the unquoted
/// form's missing end-of-file warning is a separate pre-existing face,
/// not this issue).
#[test]
fn adjacent_routes_unchanged() {
    let script = r#"echo "$(cat <<E | sort -u
body
E)"
echo "$(cat <<E
body
E
)"
v="$(cat <<E | sort
body
E)"
echo "v=$v"
"#;
    let (stdout, stderr, code) = rubash_file(script, "C.UTF-8");
    assert_eq!(stdout, "body\nbody\nv=body\n", "stdout mismatch");
    assert_eq!(
        stderr,
        "\
case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')
case.sh: line 10: warning: here-document at line 8 delimited by end-of-file (wanted `E')
",
        "stderr mismatch"
    );
    assert_eq!(code, Some(0), "exit code mismatch");

    let (stdout, _stderr, code) = rubash_file("echo $(cat <<E | sort\nbody\nE)\n", "C.UTF-8");
    assert_eq!(stdout, "body\n", "unquoted stdout mismatch");
    assert_eq!(code, Some(0), "unquoted exit code mismatch");
}
