//! rubash#3100 — `[[ == ]]` / `[[ != ]]` with a quoted RHS never routes an
//! extglob group to the extglob matcher: `quoted_conditional_pattern_status`
//! (`src/executor/conditional.rs`) sends every quoted RHS through
//! `quote_aware_glob_rhs` + `case_pattern_matches`, which has no extglob
//! handling, so `@("a*b")` is compared as literal text.
//!
//! GNU anchors: `[[ == ]]` pattern matching is uniform —
//! `execute_cmd.c:4043-4046` marks `=`/`==`/`!=` as patmatch,
//! `execute_cmd.c:4068` expands the RHS with mode 1 (pattern: quoting only
//! decides which characters are literal), `execute_cmd.c:4116-4118` forces
//! `extended_glob = 1` for the `cond_test` call, and `test.c:396-401`
//! (`patcomp`) runs `strmatch(..., FNMATCH_EXTFLAG|FNMATCH_IGNCASE)`.
//! Quoting inside a group therefore keeps the group's operator live and
//! makes only the quoted characters literal.
//!
//! Harness: script files (extglob must be enabled at parse time; a `-c`
//! single-line form parses before `shopt -s extglob` runs). Same
//! `rubash_file` shape as issue3050. Expectations verified byte-for-byte
//! against GNU bash 5.3.20 script-file probes.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3100-{}-{}",
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
fn conditional_quoted_rhs_extglob_group_core_shapes() {
    // Issue repro B1–B4, verbatim: quoted group alternatives are literal,
    // the group itself is live. B2 (plain quoted pattern) and B3 (unquoted
    // group) are controls that already work.
    check(
        "shopt -s extglob\n\
         s='a*b'\n\
         [[ $s == @(\"a*b\") ]] && echo B1-MATCH || echo B1-NOMATCH\n\
         [[ $s == \"a*b\" ]] && echo B2-MATCH || echo B2-NOMATCH\n\
         [[ $s == @(a*b) ]] && echo B3-MATCH || echo B3-NOMATCH\n\
         [[ $s == @('a*b') ]] && echo B4-MATCH || echo B4-NOMATCH\n",
        "B1-MATCH\nB2-MATCH\nB3-MATCH\nB4-MATCH\n",
    );
}

#[test]
fn conditional_quoted_rhs_extglob_group_not_equal() {
    // The `!=` arm shares the routing: a matching group makes `!=` false.
    check(
        "shopt -s extglob\n\
         s='a*b'\n\
         [[ $s != @(\"a*b\") ]] && echo N1-NE || echo N1-EQ\n\
         [[ $s != @(\"x\") ]] && echo N2-NE || echo N2-EQ\n",
        "N1-EQ\nN2-NE\n",
    );
}

#[test]
fn conditional_quoted_rhs_extglob_alternatives_stay_literal() {
    // Inside the group, quoting keeps `*`/`?` literal in both directions:
    // `@("a?b")` rejects `a*b` and accepts `a?b`.
    check(
        "shopt -s extglob\n\
         s='a*b'\n\
         [[ $s == @(\"a*b\"|x) ]] && echo A1-MATCH || echo A1-NOMATCH\n\
         [[ $s == @(\"a?b\") ]] && echo A2-MATCH || echo A2-NOMATCH\n\
         s2='a?b'\n\
         [[ $s2 == @(\"a?b\") ]] && echo A3-MATCH || echo A3-NOMATCH\n\
         v='b'\n\
         [[ $v == @(\"a*b\"|b) ]] && echo A4-MATCH || echo A4-NOMATCH\n",
        "A1-MATCH\nA2-NOMATCH\nA3-MATCH\nA4-MATCH\n",
    );
}

#[test]
fn conditional_quoted_rhs_extglob_all_operators() {
    // Every extglob operator routes, not just `@`: `+`, `?`, `*` accept the
    // single literal occurrence; `!` negates the group.
    check(
        "shopt -s extglob\n\
         s='a*b'\n\
         [[ $s == +(\"a*b\") ]] && echo P1-MATCH || echo P1-NOMATCH\n\
         [[ $s == ?(\"a*b\") ]] && echo Q1-MATCH || echo Q1-NOMATCH\n\
         [[ $s == *(\"a*b\") ]] && echo T1-MATCH || echo T1-NOMATCH\n\
         [[ $s == !(\"a*b\") ]] && echo G1-MATCH || echo G1-NOMATCH\n\
         [[ $s == !(\"x\") ]] && echo G2-MATCH || echo G2-NOMATCH\n",
        "P1-MATCH\nQ1-MATCH\nT1-MATCH\nG1-NOMATCH\nG2-MATCH\n",
    );
}

#[test]
fn conditional_quoted_rhs_extglob_affixes_and_fully_quoted_control() {
    // A group may sit between literal affixes; a fully quoted `@(a)` is
    // literal text, not a group (the routing decision reads the raw
    // spelling's unquoted spans, never the cooked pattern).
    check(
        "shopt -s extglob\n\
         t='xa*by'\n\
         [[ $t == x@(\"a*b\")y ]] && echo S1-MATCH || echo S1-NOMATCH\n\
         u='@(a)'\n\
         [[ $u == \"@(a)\" ]] && echo F1-MATCH || echo F1-NOMATCH\n",
        "S1-MATCH\nF1-MATCH\n",
    );
}
