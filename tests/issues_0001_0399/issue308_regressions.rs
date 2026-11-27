//! Issue rubash#308 regressions (wt13/fixpack lane).
//!
//! `sys/cmd/harden.mm: line 404: syntax error near unexpected token `}'`
//! — rubash rejected modernish's harden.mm (GNU accepts) because reserved
//! words inside CASE PATTERNS opened phantom compounds in the
//! compound-boundary scanners, desynchronizing every enclosing construct
//! scan (function bodies, clause bodies).
//!
//! GNU anchors:
//! - parse.y:3177-3186 CHECK_FOR_RESERVED_WORD: inside a case pattern list
//!   (PST_CASEPAT, parser.h:29, entered when the case's `in' is read at
//!   parse.y:3379/3396 and cleared at the clause `)' at 3788) reserved
//!   words are NOT recognized — `while' in `case x in (while|break))' is
//!   pattern text; only `esac' can match, and only when the previous token
//!   is not `|' (Posix grammar rule 4, parse.y:3181).
//! - parse.y:1225-1236 pattern_list: newline_list is legal before a
//!   clause's first pattern token; the optional clause-opening `(' is
//!   grammar (parse.y:1231), not extglob nesting.
//!
//! Root causes fixed (parser/support.rs update_compound_boundary_stack):
//! 1. Keywords between `in' and the clause `)' pushed fi/done/esac expects
//!    like real compound openers. The stack now tracks the pattern region
//!    with PST_CASEPAT-equivalent sentinels; keywords there stay inert.
//! 2. A newline right after `in' (or a clause terminator) consumed the
//!    "may-open" slot, so the clause's real `(' was misread as extglob
//!    nesting and desynchronized nested case bodies. newline_list tokens
//!    are now transparent in the pattern region.
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 (script-file probes
//! 2026-09-28 under target/issue-suites/results/fixpack308/; GNU case and
//! parser suites zero-diff after the fix).

use std::process::Command;

/// Run `rubash -n` on a script FILE in its own scratch directory and
/// return (stderr, code).
fn rubash_n(script: &str) -> (String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i308n-{}-{}",
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
        "rubash-i308-{}-{}",
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

/// The harden.mm shape: a reserved word inside a case pattern, in a
/// function body, followed by another definition. GNU parses this; the
/// phantom loop used to swallow the function's `}'.
#[test]
fn reserved_word_in_case_pattern_parses() {
    let (stderr, code) = rubash_n(
        "f() {\n\tcase ${x} in\n\t(while|break)\n\t\techo hit ;;\n\tesac\n}\ng() {\n\t:\n}\necho ok\n",
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Every compound-opening keyword in pattern position, inside a function,
/// with a multi-line follower (the full class, not one symptom).
#[test]
fn every_compound_keyword_in_pattern_is_inert() {
    for keyword in ["while", "until", "for", "if", "case", "select"] {
        let (stderr, code) = rubash_n(&format!(
            "f() {{\n\tcase ${{x}} in\n\t(a|{keyword})\n\t\techo hit ;;\n\tesac\n}}\ng() {{\n\t:\n}}\necho ok\n"
        ));
        assert_eq!(stderr, "", "keyword {keyword} must be pattern text");
        assert_eq!(code, Some(0), "keyword {keyword}");
    }
}

/// newline_list before the first pattern token: the clause `(' is still
/// the optional opener, not extglob nesting (parse.y:1225/1231).
#[test]
fn newlines_around_pattern_list_parse() {
    let (stderr, code) = rubash_n("case a in\n\n( x )\n\n\techo b ;;\n\nesac\necho ok\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Nested case as a bodyless clause's body (harden.mm's inner shape):
/// three levels of case nesting with the middle clause unterminated.
#[test]
fn nested_case_in_bodyless_clause_parses() {
    let (stderr, code) = rubash_n(
        "case a in\n( x )\n\tcase c2 in\n\t( y )\n\t\tcase p in ( z ) v=1 ;; esac\n\tesac ;;\nesac\necho done\n",
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The real harden.mm pattern: escaped metachars, the full reserved-word
/// list split across a backslash-newline continuation, plus an empty-
/// string clause, inside functions back to back.
#[test]
fn harden_mm_reserved_word_pattern_continuation_parses() {
    let (stderr, code) = rubash_n(concat!(
        "f() {\n",
        "\tcase ${v} in\n",
        "\t(\\!|\\{|\\}|case|do|done|elif|else|\\esac|fi|for|if|in|then|until|while \\\n",
        "\t|break|:|continue|.|eval|exec|exit|export|readonly|return|set|shift|times|trap|unset)\n",
        "\t\tdie \"reserved\" ;;\n",
        "\t( '' )\n",
        "\t\techo empty ;;\n",
        "\tesac\n",
        "}\n",
        "g() { :; }\n",
        "echo ok\n"
    ));
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Runtime semantics: a reserved-word pattern still MATCHES its word —
/// the fix must not turn patterns into something the matcher drops.
#[test]
fn reserved_word_pattern_still_matches() {
    let (stdout, stderr, code) = rubash_file(
        "v=while\ncase $v in\n(a|while) echo matched-while ;;\n(*) echo no ;;\nesac\nv=fi\ncase $v in\n(fi|z) echo matched-fi ;;\n(*) echo no ;;\nesac\necho end\n",
    );
    assert_eq!(stdout, "matched-while\nmatched-fi\nend\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
