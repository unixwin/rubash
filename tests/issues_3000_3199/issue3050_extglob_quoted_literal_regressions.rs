//! rubash#3050 — extglob group quoted literals never match:
//! `extglob_match_literal_with_case` (`src/executor/conditional/pattern.rs`)
//! lacks the CTLESC escape arm the ordinary matcher
//! (`case_pattern_atom_matches`, same file) already has, so a
//! CTLESC-protected literal metacharacter inside an extglob group falls
//! into the catch-all and is compared as a literal CTLESC character.
//!
//! GNU anchors: a `case` pattern word is expanded with
//! `expand_word_leave_quoted` and then passed through
//! `quote_string_for_globbing(..., QGLOB_CVTNULL|QGLOB_CTLESC)`
//! (execute_cmd.c:3695-3729), which turns each CTLESC-quoted character
//! into a backslash escape; the comment there states "the strmatch()
//! call must be able to recognize backslashes as escape characters".
//! The matcher honours that in lib/glob/sm_loop.c:121-135 (GMATCH
//! `case L('\\')`: "backslash escape removes special meaning" — the
//! escaped char is matched literally). In rubash's CTLESC port the
//! escape carrier is CTLESC, not backslash, so the extglob literal
//! matcher needs the CTLESC arm isomorphic to the ordinary matcher's.
//!
//! Harness: script files (extglob must be enabled at parse time; a `-c`
//! single-line form parses before `shopt -s extglob` runs). Same
//! `rubash_file` shape as issue3000. Expectations verified byte-for-byte
//! against GNU bash 5.3.20 script-file probes.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3050-{}-{}",
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
fn extglob_quoted_star_matches_literal_star() {
    // Issue repro, verbatim: quoted `*` inside @(…) is a literal star.
    check(
        "shopt -s extglob\n\
         s='a*b'\n\
         case \"$s\" in @(\"a*b\"|x)) echo MATCH1;; *) echo NOMATCH1;; esac\n\
         case \"$s\" in @(\"a*b\")) echo MATCH2;; *) echo NOMATCH2;; esac\n\
         case \"$s\" in @(\"a?b\"|x)) echo MATCH3;; *) echo NOMATCH3;; esac\n",
        "MATCH1\nMATCH2\nNOMATCH3\n",
    );
}

#[test]
fn extglob_quoted_literal_pua_value_matches() {
    // #3014 lane probe shape: the quoted value is a PUA character.
    check(
        "shopt -s extglob\n\
         v=$'\\uE002'\n\
         case \"$v\" in @(\"$v\"|x)) echo MATCH4;; *) echo NOMATCH4;; esac\n",
        "MATCH4\n",
    );
}

#[test]
fn extglob_quoted_metachars_are_literal_not_glob() {
    // A quoted `*`/`?` must NOT act as a glob operator: 'axb' is not
    // the literal string 'a*b', and 'aXb' is not the literal 'a?b'.
    // (The third arm re-checks the MATCH5 shape from the GNU probe:
    // s='a*b' matches the first quoted alternative literally.)
    check(
        "shopt -s extglob\n\
         s='a*b'\n\
         case 'axb' in @(\"a*b\")) echo MATCH6;; *) echo NOMATCH6;; esac\n\
         case 'aXb' in @(\"a?b\")) echo MATCH7;; *) echo NOMATCH7;; esac\n\
         case \"$s\" in @(\"a*b\"|\"a?b\")) echo MATCH5;; *) echo NOMATCH5;; esac\n",
        "NOMATCH6\nNOMATCH7\nMATCH5\n",
    );
}

#[test]
fn extglob_unquoted_star_still_globs() {
    // Control: an unquoted `*` inside @(…) keeps its glob meaning
    // (GNU: `@(a*b)` matches 'a*b' itself, since `*` consumes the
    // literal `*`; 'xb' does not match it).
    check(
        "shopt -s extglob\n\
         case 'axb' in @(a*b)) echo GLOB1;; *) echo NOGLOB1;; esac\n\
         case 'aXb' in @(a?b)) echo GLOB2;; *) echo NOGLOB2;; esac\n\
         case 'xb' in @(a*b)) echo GLOB4;; *) echo NOGLOB4;; esac\n",
        "GLOB1\nGLOB2\nNOGLOB4\n",
    );
}
