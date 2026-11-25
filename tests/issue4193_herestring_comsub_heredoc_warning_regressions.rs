//! rubash#4193 regressions: a command substitution fed by a here-string
//! (`<<< $(cat <<E ... E)`) lost its here-document end-of-file warning
//! entirely — GNU bash 5.3.20 warns, rubash emitted nothing.
//!
//! Root cause: the lexer's comsub-heredoc header scan
//! (`scan_line_for_comsub_heredoc_headers`) stepped over only the first
//! `<` of `<<<`, so the remaining `<<` was misread as a heredoc header
//! whose delimiter was the here-string word itself (`$(cat`). That bogus
//! header queued ahead of the real one in `comsub_heredocs`, so when the
//! delimiter line (`E)`) arrived it matched neither the queue front's
//! close nor its delimiter-line mark, the IFS_GLUE record byte was never
//! spliced, and the substitution's heredoc plan never saw the record the
//! warning keys on. GNU tokenizes the operator by maximal munch at the
//! first `<` (parse.y:3689-3703 read_token: `<<` + `<` is
//! LESS_LESS_LESS, one here-string token); the scan now skips all three
//! bytes the same way, and the warning flows through the family's single
//! existing emission site (make_cmd.c:627 per-occurrence warning).
//!
//! Expectations are byte-verified against the GNU Bash 5.3.20 oracle
//! (script-file batteries; the C.UTF-8 and C locales agree on all of
//! these shapes), except the while-loop pin's warning LINE NUMBERS: on
//! compound commands a comsub in a redirect word still takes the legacy
//! line seed (the #4190 frontier for unstamped redirect words — the
//! same attribution is produced by a plain `< $(...)` redirect word,
//! which this fix does not touch), so that pin asserts the warning's
//! presence and text, not its line numbers. Its stdout is byte-exact.

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-heredoc4193-{}-{}",
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

const WARNING_E_3_1: &str =
    "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n";

/// The reported shape: a while loop whose stdin is a here-string built
/// from a comsub whose heredoc delimiter line carries the closing `)`.
/// On the unfixed tree stderr was EMPTY (no warning at all); GNU warns.
/// The warning's line numbers ride the legacy redirect-word seed (see
/// the module doc), so this pin asserts presence + warning text.
const HS_WHILE: &str = "while read x; do echo \"got $x\"; done <<< $(cat <<E\nbody\nE)\n";

#[test]
fn herestring_while_loop_comsub_warns() {
    for lc_all in ["C.UTF-8", "C"] {
        let (stdout, stderr, code) = rubash_file(HS_WHILE, lc_all);
        assert_eq!(stdout, "got body\n", "stdout mismatch ({lc_all})");
        assert!(
            stderr.contains("warning: here-document at line ")
                && stderr.contains("delimited by end-of-file (wanted `E')"),
            "warning missing or malformed ({lc_all}): {stderr:?}"
        );
        assert_eq!(code, Some(0), "exit code mismatch ({lc_all})");
    }
}

/// Same shape on a simple command: byte-exact against GNU, line
/// numbers included.
const HS_SIMPLE: &str = "read x <<< $(cat <<E\nbody\nE)\necho \"hs-simple:[$x]\"\n";

#[test]
fn herestring_simple_command_comsub_warns_byte_exact() {
    assert_both_locales(HS_SIMPLE, "hs-simple:[body]\n", WARNING_E_3_1);
}

/// The here-string word double-quoted: the comsub still owns a heredoc
/// whose delimiter line carries the `)`.
const HS_QUOTED: &str = "read x <<< \"$(cat <<E\nbody\nE)\"\necho \"hs-quoted:[$x]\"\n";

#[test]
fn herestring_quoted_word_comsub_warns_byte_exact() {
    assert_both_locales(HS_QUOTED, "hs-quoted:[body]\n", WARNING_E_3_1);
}

/// The `<<-` strip-tabs variant inside the here-string's comsub: the
/// delimiter line is tab-indented and carries the `)`.
const HS_DASH: &str = "read x <<< $(cat <<-E\n\tbody\n\tE)\necho \"hs-dash:[$x]\"\n";

#[test]
fn herestring_strip_tabs_comsub_warns_byte_exact() {
    assert_both_locales(HS_DASH, "hs-dash:[body]\n", WARNING_E_3_1);
}

/// Guard: a properly closed heredoc inside a here-string comsub (the
/// `)` sits on its own line) warns on neither side.
const HS_CLOSED: &str = "read x <<< $(cat <<E\nbody\nE\n)\necho \"hs-closed:[$x]\"\n";

#[test]
fn herestring_closed_heredoc_no_warning() {
    assert_both_locales(HS_CLOSED, "hs-closed:[body]\n", "");
}

/// Guard: the same comsub shape reached through a command word (an
/// assignment value) warned before this fix and must keep warning,
/// byte-identically.
const ASG: &str = "x=$(cat <<E\nbody\nE)\necho \"asg:[$x]\"\n";

#[test]
fn command_word_same_shape_guard() {
    assert_both_locales(ASG, "asg:[body]\n", WARNING_E_3_1);
}
