//! Issue rubash#372 (issue308 residue): compound keywords in case PATTERN
//! position must stay word-inert when a clause-opening `(` precedes them.
//!
//! `case x in (a|case) ... esac` parsed fine but the batch driver's
//! unclosed-input gate (lexer close-char scan) reported a phantom
//! `unexpected end of file from `(' command`, because the scan's case-word
//! machine has no pattern-region state: a pattern-position `case` counted
//! as a real case keyword and desynchronized the delimiter accounting.
//!
//! GNU anchors:
//! - parse.y:3177-3186 CHECK_FOR_RESERVED_WORD: inside a case pattern list
//!   (PST_CASEPAT, parser.h:29, set at parse.y:3379/3396 when the case's
//!   `in' is read, cleared at the clause `)' at parse.y:3788) reserved
//!   words are NOT recognized — pattern text.
//! - parse.y:1225-1236 pattern_list: the optional clause-opening `(' is
//!   grammar; keywords after it are still pattern text.
//! - GNU decides completeness by PARSING (read_token drives the grammar);
//!   the fix verifies the text-level scanner's unclosed verdict against
//!   the real parser before reporting EOF (src/script_driver.rs
//!   `unclosed_verdict_disproved_by_parser`, rubash#117 move 2 — converge
//!   to the real parser).
//!
//! Every accepted shape below was verified byte-for-byte against WSL GNU
//! Bash 5.3.0 from script files (target/issue372-*.sh in the lane
//! worktree); the rejected shapes still produce GNU's EOF diagnostics.

use std::process::Command;

/// Run `rubash -n` on a script FILE in its own scratch directory and
/// return (stderr, code).
fn rubash_n(script: &str) -> (String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i372n-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash -n");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Run a script FILE and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i372-{}-{}",
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

/// The exact rubash#372 reproducer: `case' in pattern position behind a
/// clause-opening `(' must not trip the unclosed-input gate.
#[test]
fn case_keyword_in_parenthesized_pattern_parses() {
    let (stderr, code) = rubash_n("case x in\n(a|case)\n\techo hit ;;\nesac\necho ok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The same shape executes and still MATCHES its pattern word.
#[test]
fn case_keyword_in_parenthesized_pattern_executes() {
    let (stdout, stderr, code) = rubash_file(
        "v=case\ncase $v in\n(a|case)\n\techo hit ;;\n(*)\n\techo no ;;\nesac\necho ok\n",
    );
    assert_eq!(stdout, "hit\nok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The full compound-keyword class in pattern position with the clause
/// paren (the issue308 `every_compound_keyword...` matrix plus the `(`
/// grouping that exposes the scanner gap).
#[test]
fn every_compound_keyword_in_grouped_pattern_is_inert() {
    for keyword in [
        "while", "until", "for", "if", "case", "select", "esac", "do", "done", "fi", "then",
        "elif", "else", "in", "coproc", "time", "function",
    ] {
        let (stderr, code) = rubash_n(&format!(
            "f() {{\n\tcase ${{x}} in\n\t(a|{keyword})\n\t\techo hit ;;\n\tesac\n}}\ng() {{\n\t:\n}}\necho ok\n"
        ));
        assert_eq!(stderr, "", "keyword {keyword} must be pattern text");
        assert_eq!(code, Some(0), "keyword {keyword}");
    }
}

/// A keyword pattern inside a nested case's clause body, with grouped
/// patterns before and after it on the OUTER case.
#[test]
fn keyword_patterns_across_nested_case_and_groups_parse() {
    let (stderr, code) = rubash_n(concat!(
        "case outer in\n",
        "(x|y)\n\techo g1 ;;\n",
        "(case)\n",
        "\tcase inner in\n",
        "\t(b|while)\n\t\t\techo nested ;;\n",
        "\tesac ;;\n",
        "(z)\n\techo g2 ;;\n",
        "esac\n",
        "echo done\n"
    ));
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The harden.mm reserved-word list split across a backslash-newline
/// continuation inside a function (the file-level shape of the second
/// issue308 residual red).
#[test]
fn harden_mm_keyword_continuation_file_parses_and_runs() {
    let (stdout, stderr, code) = rubash_file(concat!(
        "f() {\n",
        "\tcase ${v} in\n",
        "\t(\\!|\\{|\\}|case|do|done|elif|else|\\esac|fi|for|if|in|then|until|while \\\n",
        "\t|break|:|continue|.|eval|exec|exit|export|readonly|return|set|shift|times|trap|unset)\n",
        "\t\techo reserved ;;\n",
        "\t( '' )\n",
        "\t\techo empty ;;\n",
        "\tesac\n",
        "}\n",
        "g() { :; }\n",
        "v=case\n",
        "f\n",
        "v=\n",
        "f\n",
        "echo ok\n"
    ));
    assert_eq!(stdout, "reserved\nempty\nok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Negative control: a genuinely truncated case (no `esac') still reports
/// the EOF syntax error — the parser-verification must not swallow real
/// unclosed-input diagnostics.
#[test]
fn truncated_case_still_reports_eof_syntax_error() {
    let (stderr, code) = rubash_n("case x in\n(a|case)\n\techo hi ;;\n");
    assert!(
        stderr.contains("syntax error"),
        "truncated case must error, got: {stderr}"
    );
    assert_eq!(code, Some(2));
}

/// Negative control: an unclosed subshell and an unclosed quote keep
/// their EOF diagnostics.
#[test]
fn unclosed_subshell_and_quote_still_error() {
    let (stderr, code) = rubash_n("(echo hi\n");
    assert!(
        stderr.contains("syntax error") || stderr.contains("EOF"),
        "{stderr}"
    );
    assert_eq!(code, Some(2));

    let (stderr, code) = rubash_n("echo 'x\n");
    assert!(
        stderr.contains("EOF") || stderr.contains("syntax error"),
        "{stderr}"
    );
    assert_eq!(code, Some(2));
}

/// A file whose scanner verdict is a false positive but whose real parse
/// is clean executes its commands normally (the driver falls through to
/// the real parser path).
#[test]
fn false_positive_verdict_executes_whole_script() {
    let (stdout, stderr, code) =
        rubash_file("case y in\n(b|case)\n\techo in-case ;;\nesac\necho after\n");
    assert_eq!(stdout, "after\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
