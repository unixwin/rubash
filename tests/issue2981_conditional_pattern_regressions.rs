//! rubash#2981 regressions: a double-quoted `[[ == ]]` / `case`
//! pattern containing BOTH a backslash-escape (`` \` `` or `\$`) AND
//! a single-quote failed to match on rubash while GNU matched. The
//! pattern builder re-expanded the double-quoted segment as a bare
//! fragment, losing the double-quote context — `\$` misread as `\` +
//! a live `$name`, `\\` never collapsed — and a `'` hand-tagged with
//! the U+E010 decoded-quote marker survived whenever a backslash
//! routed the fragment through the mutable walker (GNU cond.c
//! cond_expand_word expands the pattern operand as a word WITH its
//! quoting intact; parse.y's double-quote scanner never opens
//! single-quote state). The same bare-fragment expansion also
//! corrupted the `case` subject word and the `case` pattern, and the
//! double-quote span scanner split at inner quotes instead of
//! skipping nested `$(...)`/`${...}`/backtick units (GNU parse.y:
//! 5416-5432 skip_double_quoted).
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2981-{}-{}",
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

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(code, Some(0), "script failed: {script}\nstderr: {stderr}");
    assert_eq!(stderr, "", "script {script} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "script: {script}");
}

#[test]
fn issue2981_double_quoted_backtick_plus_quote_matches() {
    check(
        r#"L=$'`x\''
if [[ "$L" == "\`x'" ]]; then echo MATCH; else echo NOMATCH; fi
"#,
        "MATCH\n",
    );
}

#[test]
fn issue2981_double_quoted_dollar_escape_plus_quote_matches() {
    check(
        r#"L="a\$b'c"
if [[ "$L" == "a\$b'c" ]]; then echo MATCH; else echo NOMATCH; fi
"#,
        "MATCH\n",
    );
}

#[test]
fn issue2981_double_quoted_backslash_plus_quote_matches() {
    check(
        r#"L="a\\b'c"
if [[ "$L" == "a\\b'c" ]]; then echo MATCH; else echo NOMATCH; fi
"#,
        "MATCH\n",
    );
}

#[test]
fn issue2981_case_double_quoted_backtick_plus_quote_matches() {
    check(
        r#"L=$'`x\''
case "$L" in "\`x'") echo MATCH;; *) echo NOMATCH;; esac
"#,
        "MATCH\n",
    );
}

#[test]
fn issue2981_double_quoted_star_is_literal_not_glob() {
    check(
        r#"if [[ "a*b" == "a*b" ]]; then echo MATCH; else echo NOMATCH; fi
if [[ "a?b" == "a?b" ]]; then echo MATCH; else echo NOMATCH; fi
"#,
        "MATCH\nMATCH\n",
    );
}

#[test]
fn issue2981_double_quoted_comsub_inner_quotes_match() {
    check(
        r#"if [[ "x" == "$(echo "x")" ]]; then echo MATCH; else echo NOMATCH; fi
"#,
        "MATCH\n",
    );
}
