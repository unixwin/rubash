//! rubash#3231 — `[[ == ]]` quoted extglob group with a parenthesis (or
//! pipe) inside the quotes does not match: `@("a(b")` and `@("a)b")`
//! print N on rubash where GNU prints M.
//!
//! GNU anchors: the `[[ == ]]` RHS is prepared by
//! `subst.c:4077 cond_expand_word` (special == 1), which finishes with
//! `pathexp.c:212 quote_string_for_globbing`: every quoted (CTLESC-marked)
//! character in `pathexp.c:164 glob_char_p`'s set — under EXTENDED_GLOB
//! that includes `(`, `|`, `)` — is emitted backslash-escaped, so a quoted
//! paren reaches the matcher as `\(` / `\)` and a quoted pipe as `\|`.
//! The matcher's own scanner, `lib/glob/sm_loop.c:676 PATSCAN`, skips
//! `\` + char pairs while hunting the group end and the `|` split points
//! (sm_loop.c:692-696 `case L('\\'): skip = 1`), so escaped structural
//! characters never open, close, or split a group. Quoting inside a
//! group therefore makes only the quoted characters literal; the group
//! itself stays live (the #3100 routing already relies on this).
//!
//! Harness: script files (extglob must be enabled at parse time; a `-c`
//! single-line form parses before `shopt -s extglob` runs). Same
//! `rubash_file` shape as issue3100. Expectations measured cell-by-cell
//! against the GNU bash 5.3.20 oracle (probe-gnu.sh, 23 cells).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3231-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("cond.sh"), script).expect("write cond.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("cond.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C.UTF-8")
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
fn quoted_paren_group_core_shapes() {
    // Issue repro C1/C2 verbatim (variable subject form), plus the
    // issue's plain-quoted controls K1/K2/K4, which already agree.
    check(
        "shopt -s extglob\n\
         s='a(b'\n\
         [[ $s == @(\"a(b\") ]] && echo C1-M || echo C1-N\n\
         s='a)b'\n\
         [[ $s == @(\"a)b\") ]] && echo C2-M || echo C2-N\n\
         [[ \"a(b\" == \"a(b\" ]] && echo K1-M || echo K1-N\n\
         u='@(a)'\n\
         [[ $u == \"@(a)\" ]] && echo K2-M || echo K2-N\n\
         [[ \"a)b\" == \"a)b\" ]] && echo K4-M || echo K4-N\n",
        "C1-M\nC2-M\nK1-M\nK2-M\nK4-M\n",
    );
}

#[test]
fn quoted_pipe_inside_group_is_literal() {
    // The class is the extglob group grammar, not just parens: a quoted
    // `|` is literal text (P1), never an alternative split (P2 must NOT
    // match `a`), while an unquoted `|` still splits (A1/A2).
    check(
        "shopt -s extglob\n\
         s='a|b'\n\
         [[ $s == @(\"a|b\") ]] && echo P1-M || echo P1-N\n\
         s='a'\n\
         [[ $s == @(\"a|b\") ]] && echo P2-M || echo P2-N\n\
         s='a(b'\n\
         [[ $s == @(x|\"a(b\") ]] && echo A1-M || echo A1-N\n\
         s='x'\n\
         [[ $s == @(x|\"a(b\") ]] && echo A2-M || echo A2-N\n",
        "P1-M\nP2-N\nA1-M\nA2-M\n",
    );
}

#[test]
fn quoted_paren_all_operators() {
    // Every extglob operator takes the same quoted body: `+`/`*` repeat
    // it, `?` makes it optional, `!` negates the literal body (O3/O4 are
    // a discriminating pair), and single quotes protect like double
    // quotes (Q1).
    check(
        "shopt -s extglob\n\
         s='a(ba(b'\n\
         [[ $s == +(\"a(b\") ]] && echo O1-M || echo O1-N\n\
         [[ $s == *(\"a(b\") ]] && echo O2-M || echo O2-N\n\
         s='a(b'\n\
         [[ $s == !(\"a)b\") ]] && echo O3-M || echo O3-N\n\
         s='a)b'\n\
         [[ $s == !(\"a)b\") ]] && echo O4-M || echo O4-N\n\
         s=''\n\
         [[ $s == ?(\"a(b\") ]] && echo O5-M || echo O5-N\n\
         s='a(b'\n\
         [[ $s == ?(\"a(b\") ]] && echo O6-M || echo O6-N\n\
         [[ $s == @('a(b') ]] && echo Q1-M || echo Q1-N\n",
        "O1-M\nO2-M\nO3-M\nO4-N\nO5-M\nO6-M\nQ1-M\n",
    );
}

#[test]
fn quoted_group_like_text_stays_literal() {
    // Group-shaped text inside quotes is data: balanced parens (B1),
    // a nested-looking `@(` (B2), and a quoted body followed by an
    // unquoted literal tail (T1). E1 is the backslash-spelling control
    // that already works; N1/N2 are wrong-subject negative controls;
    // K3 is the issue's negation control.
    check(
        "shopt -s extglob\n\
         s='a(b)c'\n\
         [[ $s == @(\"a(b)c\") ]] && echo B1-M || echo B1-N\n\
         s='a@(b'\n\
         [[ $s == @(\"a@(b\") ]] && echo B2-M || echo B2-N\n\
         s='a(b'\n\
         [[ $s == @(a\\(b) ]] && echo E1-M || echo E1-N\n\
         s='a(bx'\n\
         [[ $s == @(\"a(b\")x ]] && echo T1-M || echo T1-N\n\
         s='a(b'\n\
         [[ $s == @(\"a)b\") ]] && echo N1-M || echo N1-N\n\
         s='axb'\n\
         [[ $s == @(\"a(b\") ]] && echo N2-M || echo N2-N\n\
         [[ \"a*b\" == !(\"a*b\") ]] && echo K3-M || echo K3-N\n",
        "B1-M\nB2-M\nE1-M\nT1-M\nN1-N\nN2-N\nK3-N\n",
    );
}
