//! Deep-audit round 2, lane B1 (expansion engine) regression pins.
//!
//! Every expectation below is the literal GNU bash 5.3.20 output for the
//! same source, captured from the oracle binary
//! (~/workspace/rubash-audit/gnu-bash-5.3.20/bin/bash). Test names carry
//! the lane finding IDs from ~/workspace/deep-audit/b1/REPORT.md.
//! UNVERIFIED-BY-CAPTAIN.

use std::process::{Command, Output};

fn run(source: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", source])
        .env_remove("BASH_ENV")
        .output()
        .expect("run rubash")
}

fn stdout_of(source: &str) -> String {
    let out = run(source);
    assert!(
        out.status.success(),
        "expected success for {source:?}, got {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout is utf8")
}

fn check(cases: &[(&str, &str)]) {
    for (source, expected) in cases {
        assert_eq!(&stdout_of(source), expected, "source: {source}");
    }
}

/// B1-02: case-modification patterns are always extended-glob patterns
/// (GNU lib/sh/casemod.c:99 sh_modcase matches each character with
/// strmatch(pat, s, FNM_EXTMATCH), independent of the extglob shopt).
/// Before the fix every extglob group in a `^`/`^^`/`,`/`,,` pattern
/// silently matched nothing.
#[test]
fn b1_02_casemod_extglob_patterns() {
    check(&[
        (
            r#"v='aAbB xX'; printf '<%s>\n' "${v^^!(a)}""#,
            "<aABB XX>\n",
        ),
        (
            r#"v='aAbB xX'; printf '<%s>\n' "${v^^@(a|b)}""#,
            "<AABB xX>\n",
        ),
        (
            r#"v='aAbB xX'; printf '<%s>\n' "${v^^+([a-z])}""#,
            "<AABB XX>\n",
        ),
        (r#"v='aAbB xX'; printf '<%s>\n' "${v^@(a)}""#, "<AAbB xX>\n"),
        (
            r#"v='aAbB xX'; printf '<%s>\n' "${v^^?(x)}""#,
            "<aAbB XX>\n",
        ),
        (
            r#"v='aAbB xX'; printf '<%s>\n' "${v,,!(a)}""#,
            "<aabb xx>\n",
        ),
        (
            r#"v='aAbB xX'; printf '<%s>\n' "${v^^*(a)}""#,
            "<AAbB xX>\n",
        ),
        (
            r#"shopt -s extglob; v='aAbB'; printf '<%s>\n' "${v^^!(a)}""#,
            "<aABB>\n",
        ),
        // Plain (non-extglob) patterns must keep their old verdicts.
        (
            r#"v='aAbB xX'; printf '<%s>\n' "${v^^[a-z]}""#,
            "<AABB XX>\n",
        ),
        (r#"v='aAbB xX'; printf '<%s>\n' "${v^^a}""#, "<AAbB xX>\n"),
        (
            r#"v='hello world'; printf '<%s>\n' "${v^^}""#,
            "<HELLO WORLD>\n",
        ),
        (r#"v='HeLLo'; printf '<%s>\n' "${v,,}""#, "<hello>\n"),
    ]);
}

/// B1-05: the string form of the positional-parameter transforms
/// (GNU subst.c parameter_brace_transform VT_POSPARAMS case;
/// pos_params_assignment, subst.c:8724, for @A; list_transform for
/// @a/@K/@k). Before the fix the fragment/assignment walks resolved
/// "@" as a variable name: @A and @a rendered empty and @K/@k quoted
/// the JOINED positional string once.
#[test]
fn b1_05_positional_transforms() {
    check(&[
        (r#"set -- 'a b' c; echo "<${@@A}>""#, "<set -- 'a b' 'c'>\n"),
        (r#"set -- 'a b' c; echo "<${*@A}>""#, "<set -- 'a b' 'c'>\n"),
        (
            r#"set -- 'a b' c; x=${@@A}; echo "<$x>""#,
            "<set -- 'a b' 'c'>\n",
        ),
        (r#"set --; echo "<${@@A}>""#, "<>\n"),
        (r#"set -- "c'd"; echo "<${@@A}>""#, "<set -- 'c'\\''d'>\n"),
        (r#"set -- 'a b' c; echo "<${@@K}>""#, "<'a b' 'c'>\n"),
        (r#"set -- 'a b' c; echo "<${@@k}>""#, "<'a b' 'c'>\n"),
        (r#"set -- 'a b' c; echo "<${*@K}>""#, "<'a b' 'c'>\n"),
        (r#"set -- 'a b' c; echo "<${@@a}>""#, "< >\n"),
        (r#"set -- x; echo "<${@@a}>""#, "<>\n"),
        (
            r#"IFS=:; set -- 'a b' c; x=${@@K}; echo "<$x>""#,
            "<'a b':'c'>\n",
        ),
        (
            r#"IFS=:; set -- 'a b' c; x=${*@A}; echo "<$x>""#,
            "<set -- 'a b':'c'>\n",
        ),
        (r#"IFS=:; set -- 'a b' c; x=${@@a}; echo "<$x>""#, "<:>\n"),
        (r#"IFS=:; set -- 'a b' c; echo "<${*@K}>""#, "<'a b':'c'>\n"),
    ]);
}
