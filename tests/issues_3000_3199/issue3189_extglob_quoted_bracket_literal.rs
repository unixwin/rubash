//! rubash#3189 — extglob group quoted literal `[` never matches:
//! the extglob group-structure scanners in `src/executor/conditional/extglob.rs`
//! do not honour the CTLESC escape, so a quoted structural character inside
//! an extglob group is processed as pattern structure before the literal
//! matcher ever sees the alternative.
//!
//! Root cause (two faces, one invariant — every pattern scanner must treat
//! escape+char as one opaque unit before interpreting structure):
//! - `extglob_group_end` (extglob.rs) has a `'\\'` skip arm but no CTLESC
//!   arm: a quoted `[` starts its bracket skip and runs past the group end
//!   when no unquoted `]` follows (returns None, the group path is
//!   abandoned for whole-pattern literal matching, which fails on `@`);
//!   a quoted `(` inflates the paren depth; a quoted `)` ends the group
//!   early at the quoted paren.
//! - `extglob_split_alternatives` (extglob.rs) has no escape arm at all:
//!   a quoted `|` splits the alternative (leaving a dangling CTLESC), and
//!   quoted parens corrupt its depth count.
//! The literal matchers themselves are correct: `extglob_match_literal_with_case`
//! gained the CTLESC arm in #3098 (pattern.rs), and the ordinary matcher
//! `case_pattern_atom_matches` already had it — which is why alternatives
//! whose group scan is accidentally rescued by a later `]` do match.
//!
//! GNU anchors: the case pattern is passed through
//! `quote_string_for_globbing(..., QGLOB_CVTNULL|QGLOB_CTLESC)`
//! (execute_cmd.c:3724), whose convert_to_backslash branch
//! (pathexp.c:246-272) rewrites every CTLESC-quoted glob char — `[`
//! included (glob_char_p, pathexp.c:164) — as a backslash escape; every
//! GNU pattern scanner then checks the escape first (PATSCAN,
//! lib/glob/sm_loop.c:697-700 before the bracket case at :709; GMATCH
//! `case L('\\')` at sm_loop.c:121 before `case L('[')` at :327).
//!
//! Harness: script files (extglob must be enabled at parse time). Same
//! `rubash_file` shape as issue3050. Expectations verified byte-for-byte
//! against GNU bash 5.3.20 script-file probes, three-way against the
//! pre-#3098 base (4295c853) and the #3098 head (310d3a8b): cells are
//! marked RED (fail on the post-#3098 base, must turn green with the fix)
//! or LOCK (already green on that base; guards against overcorrection).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3189-{}-{}",
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
fn extglob_quoted_bracket_never_matches() {
    // RED — the issue repro (MATCH8, from the #3098 review probes) plus
    // quoting variants of the same shape: prefix inside the alternative,
    // single quotes, only the `[` quoted, two quoted brackets.
    check(
        "shopt -s extglob\n\
         case 'a[b' in @(\"a[b\")) echo MATCH8;; *) echo NOMATCH8;; esac\n\
         case 'prea[b' in @(pre\"a[b\")) echo MATCHD7;; *) echo NOMATCHD7;; esac\n\
         case 'a[b' in @('a[b')) echo MATCHD8;; *) echo NOMATCHD8;; esac\n\
         case 'a[b' in @(a\"[b\")) echo MATCHD10;; *) echo NOMATCHD10;; esac\n\
         case 'a[[b' in @(\"a[[b\")) echo MATCHD11;; *) echo NOMATCHD11;; esac\n",
        "MATCH8\nMATCHD7\nMATCHD8\nMATCHD10\nMATCHD11\n",
    );
}

#[test]
fn extglob_quoted_bracket_group_positions() {
    // RED — the group is not at the pattern start, the pattern continues
    // after the group, the `[` sits in a later alternative, and a second
    // alternative follows the bracket alternative.
    check(
        "shopt -s extglob\n\
         case 'xa[b' in x@(\"a[b\")) echo MATCHG1;; *) echo NOMATCHG1;; esac\n\
         case 'a[bZ' in @(\"a[b\")Z) echo MATCHG2;; *) echo NOMATCHG2;; esac\n\
         case 'a[b' in @(a[b]b|\"a[b\")) echo MATCHE11;; *) echo NOMATCHE11;; esac\n\
         case 'a[b' in @(\"a[b\"|x)) echo MATCHSD;; *) echo NOMATCHSD;; esac\n",
        "MATCHG1\nMATCHG2\nMATCHE11\nMATCHSD\n",
    );
}

#[test]
fn extglob_quoted_bracket_all_operators() {
    // RED — all five group operators share the same two scanners
    // (extglob_group_end / extglob_split_alternatives). The !(...) cell
    // with the non-matching word must MATCH (negation of a literal that
    // the word is not); the matching word stays a NOMATCH lock.
    check(
        "shopt -s extglob\n\
         case 'a[b' in *(\"a[b\")) echo MATCHS;; *) echo NOMATCHS;; esac\n\
         case 'a[b' in +(\"a[b\")) echo MATCHP;; *) echo NOMATCHP;; esac\n\
         case 'a[b' in ?(\"a[b\")) echo MATCHQ;; *) echo NOMATCHQ;; esac\n\
         case 'xb' in !(\"a[b\")) echo MATCHN1;; *) echo NOMATCHN1;; esac\n\
         case 'a[b' in !(\"a[b\")) echo MATCHN2;; *) echo NOMATCHN2;; esac\n",
        "MATCHS\nMATCHP\nMATCHQ\nMATCHN1\nNOMATCHN2\n",
    );
}

#[test]
fn extglob_quoted_paren_group_end() {
    // RED — quoted parens are structural to the group-end scanner too:
    // a quoted `(` inflates its depth (the real closer only restores it
    // to 1, so no group end is found) and a quoted `)` ends the group
    // early. NOTE: the #3098 review record claimed the `(` shape was
    // already fixed (MATCH12); that reading could not be reproduced —
    // 170+ runs of the #3098 head binary give NOMATCH12 deterministically
    // (binary and probe file hashes unchanged). If this cell is ever
    // observed green on the unfixed base, treat it as the recorded
    // anomaly, not as a reason to weaken the cell.
    check(
        "shopt -s extglob\n\
         case 'a(b' in @(\"a(b\")) echo MATCH12;; *) echo NOMATCH12;; esac\n\
         case 'a)b' in @(\"a)b\")) echo MATCHD1;; *) echo NOMATCHD1;; esac\n",
        "MATCH12\nMATCHD1\n",
    );
}

#[test]
fn extglob_quoted_bracket_conditional_face() {
    // RED — the [[ == ]] face routes through the same extglob scanners
    // (conditional.rs -> extglob_case_pattern_matches).
    check(
        "shopt -s extglob\n\
         if [[ 'a[b' == @(\"a[b\") ]]; then echo MATCHE9; else echo NOMATCHE9; fi\n",
        "MATCHE9\n",
    );
}

#[test]
fn extglob_quoted_pipe_is_one_literal() {
    // RED (face B) — a quoted `|` is literal data, not an alternative
    // separator: split_alternatives must not cut inside the quotes.
    // The backslash spelling has the same expectation. The 'a' cell is
    // a negative lock: it must NOT match (the quoted alternative is the
    // single literal `a|b`).
    check(
        "shopt -s extglob\n\
         case 'a|b' in @(\"a|b\")) echo MATCHSB;; *) echo NOMATCHSB;; esac\n\
         case 'a|b' in @(a\\|b)) echo MATCHSF;; *) echo NOMATCHSF;; esac\n\
         case 'a' in @(\"a|b\")) echo MATCHSC;; *) echo NOMATCHSC;; esac\n",
        "MATCHSB\nMATCHSF\nNOMATCHSC\n",
    );
}

#[test]
fn extglob_escaped_paren_alternative_split() {
    // RED (face B, with A for the second shape) — a backslash-escaped
    // `(` must not raise the split depth so the real `|` is skipped;
    // the quoted-`(` shape additionally needs the group-end fix.
    check(
        "shopt -s extglob\n\
         case 'a(b' in @(a\\(b|x)) echo MATCHSE;; *) echo NOMATCHSE;; esac\n\
         case 'a(b' in @(\"a(b\"|x)) echo MATCHSA;; *) echo NOMATCHSA;; esac\n",
        "MATCHSE\nMATCHSA\n",
    );
}

#[test]
fn extglob_quoted_bracket_rescued_shapes_stay_green() {
    // LOCK — these already match on the post-#3098 base because a later
    // unquoted `]` rescues the group-end bracket skip, or the quoted
    // parens cancel out before the split point. The fix must not break
    // them (no overcorrection of the scanners).
    check(
        "shopt -s extglob\n\
         case 'a[b]c' in @(\"a[b]c\")) echo MATCHE7;; *) echo NOMATCHE7;; esac\n\
         case 'a[b' in @(\"a[b\"|a[b]b)) echo MATCHE8;; *) echo NOMATCHE8;; esac\n\
         case 'a[b]' in @(\"a[b]\")) echo MATCHD12;; *) echo NOMATCHD12;; esac\n\
         case 'a(b)c' in @(\"a(b)c\")) echo MATCHPC;; *) echo NOMATCHPC;; esac\n\
         case 'a(b)c' in @(\"a(b)c\"|x)) echo MATCHSA2;; *) echo NOMATCHSA2;; esac\n\
         case 'x' in @(\"a(b)c\"|x)) echo MATCHSA3;; *) echo NOMATCHSA3;; esac\n",
        "MATCHE7\nMATCHE8\nMATCHD12\nMATCHPC\nMATCHSA2\nMATCHSA3\n",
    );
}

#[test]
fn extglob_quoted_metachars_fixed_by_3098_stay_green() {
    // LOCK — the two p8 shapes #3098 verifiably fixed (its CTLESC arm in
    // the extglob literal matcher): quoted `]` and quoted `!`.
    check(
        "shopt -s extglob\n\
         case 'a]b' in @(\"a]b\")) echo MATCH11;; *) echo NOMATCH11;; esac\n\
         case 'a!b' in @(\"a!b\")) echo MATCH13;; *) echo NOMATCH13;; esac\n",
        "MATCH11\nMATCH13\n",
    );
}

#[test]
fn extglob_bracket_class_controls() {
    // LOCK — unquoted bracket expressions keep their class meaning, in
    // a group and in a plain pattern; the plain (non-extglob) case face
    // with a quoted `[` already agrees with GNU on the base.
    check(
        "shopt -s extglob\n\
         case 'ab' in @([a]b)) echo MATCHE4;; *) echo NOMATCHE4;; esac\n\
         case 'xb' in @([a]b)) echo MATCHE5;; *) echo NOMATCHE5;; esac\n\
         case 'abb' in a[b]b) echo MATCHF1;; *) echo NOMATCHF1;; esac\n\
         case 'a[b' in \"a[b\") echo MATCHE1;; *) echo NOMATCHE1;; esac\n",
        "MATCHE4\nNOMATCHE5\nMATCHF1\nMATCHE1\n",
    );
}

#[test]
fn extglob_quoted_bracket_negative_controls() {
    // LOCK — words that are not the quoted literal must not match:
    // a `]` the word does not have, and a different middle character.
    check(
        "shopt -s extglob\n\
         case 'a[b' in @(\"a[b]\")) echo MATCHD13;; *) echo NOMATCHD13;; esac\n\
         case 'axb' in @(\"a[b\")) echo MATCHD14;; *) echo NOMATCHD14;; esac\n",
        "NOMATCHD13\nNOMATCHD14\n",
    );
}
