//! rubash#2664 — two glob defects from the Murase pattern table
//! (run-glob-bracket, cases #20 and #22+):
//!
//! 1. An extglob alternative containing `/` must never match in
//!    pathname expansion: GNU matches pathname components one at a
//!    time (glob.c:774 passes FNM_PATHNAME to strmatch at glob.c:889,
//!    and a component name contains no `/`), so `*@(/)cd/efg` cannot
//!    match the path `ab/cd/efg`. GNU still treats the word as a
//!    pattern (nullglob drops it, failglob errors). In string
//!    matching (`[[ == ]]`, `case`) there is no pathname restriction
//!    and the same pattern DOES match — those cells are locks.
//! 2. Slash-in-bracket patterns (`ab[/]ef`, `ab[c/d]ef`,
//!    `ab[[=/=]]ef`, ...): GNU completes the whole table. The shell
//!    word predicate (pathexp.c unquoted_glob_pattern_p) voids a
//!    bracket at an unquoted `/`, so in glob context these words are
//!    literals; in string matching the bracket classes match `/`
//!    normally (sm_loop.c:172-173 only bars wildcards from matching
//!    `/` under FNM_PATHNAME).
//!
//! Expectations verified against GNU bash 5.3.20 (script files, not
//! -c: extglob must be set before the pattern is parsed).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2664-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Fixture: directory tree holding exactly `ab/cd/efg`; the probe
/// script lives outside the globbed directory and runs with the
/// fixture as cwd.
fn run_in_ab_tree(tag: &str, body: &str) -> (String, String, bool) {
    let dir = tempdir(tag);
    let fix = dir.join("fix");
    std::fs::create_dir_all(fix.join("ab/cd")).expect("fixture dirs");
    std::fs::write(fix.join("ab/cd/efg"), "").expect("fixture file");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("../probe.sh")
        .current_dir(&fix)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let ok = output.status.success();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, ok)
}

#[test]
fn extglob_slash_alternative_never_matches_a_pathname() {
    let (stdout, _stderr, ok) = run_in_ab_tree(
        "glob",
        "shopt -s extglob\n\
         printf 'c1: '; for f in *@(/)cd/efg; do printf '<%s> ' \"$f\"; done; echo\n\
         printf 'c2: '; for f in @(ab/cd)/efg; do printf '<%s> ' \"$f\"; done; echo\n\
         printf 'c3: '; for f in *(ab/cd)/efg; do printf '<%s> ' \"$f\"; done; echo\n\
         printf 'c4: '; for f in ?(ab/)cd/efg; do printf '<%s> ' \"$f\"; done; echo\n\
         printf 'c5: '; for f in ab@(/cd)/efg; do printf '<%s> ' \"$f\"; done; echo\n\
         printf 'c6: '; for f in @(ab)/cd/efg; do printf '<%s> ' \"$f\"; done; echo\n",
    );
    assert!(ok);
    assert_eq!(
        stdout,
        "c1: <*@(/)cd/efg> \n\
         c2: <@(ab/cd)/efg> \n\
         c3: <*(ab/cd)/efg> \n\
         c4: <?(ab/)cd/efg> \n\
         c5: <ab@(/cd)/efg> \n\
         c6: <ab/cd/efg> \n"
    );
}

#[test]
fn extglob_slash_alternative_is_a_pattern_under_nullglob() {
    // GNU: the word IS a pattern (unlike slash-in-bracket words), so
    // nullglob drops it when the per-component match fails.
    let (stdout, _stderr, ok) = run_in_ab_tree(
        "nullglob",
        "shopt -s extglob nullglob\n\
         echo B: X *@(/)cd/efg Y\n\
         echo C: X @(ab/cd)/efg Y\n\
         echo D: X @(zz)/cd/efg Y\n\
         echo E: X @(ab)/cd/efg Y\n",
    );
    assert!(ok);
    assert_eq!(
        stdout,
        "B: X Y\n\
         C: X Y\n\
         D: X Y\n\
         E: X ab/cd/efg Y\n"
    );
}

#[test]
fn extglob_slash_alternative_still_matches_as_a_string() {
    // String matching has no FNM_PATHNAME restriction: locks against
    // over-fixing the matcher itself.
    let (stdout, _stderr, ok) = run_in_ab_tree(
        "strmatch",
        "shopt -s extglob\n\
         [[ ab/cd/efg == *@(/)cd/efg ]] && echo 's1: yes' || echo 's1: no'\n\
         [[ ab/cd/efg == @(ab/cd)/efg ]] && echo 's2: yes' || echo 's2: no'\n\
         case ab/cd/efg in *@(/)cd/efg) echo 's3: yes';; *) echo 's3: no';; esac\n",
    );
    assert!(ok);
    assert_eq!(stdout, "s1: yes\ns2: yes\ns3: yes\n");
}

#[test]
fn bracket_slash_string_matching_completes_the_chain() {
    // Murase "slash in bracket expressions" section: every case runs
    // and the script reaches its tail (on the defect tree the output
    // stopped entirely at the first such case).
    let (stdout, _stderr, ok) = run_in_ab_tree(
        "brackets",
        "shopt -s extglob\n\
         [[ ab/ef == ab[/]ef ]] && echo 'br1 [/]: yes' || echo 'br1 [/]: no'\n\
         [[ ab/ef == ab[c/d]ef ]] && echo 'br2 [c/d]: yes' || echo 'br2 [c/d]: no'\n\
         [[ ab/def == ab[c/d]ef ]] && echo 'br3 [c/d] d: yes' || echo 'br3 [c/d] d: no'\n\
         [[ ab=ef == ab[[=/=]]ef ]] && echo 'br4 [[=/=]]: yes' || echo 'br4 [[=/=]]: no'\n\
         [[ ab/ef == ab[[=/=]]ef ]] && echo 'br5 [[=/=]] slash: yes' || echo 'br5 [[=/=]] slash: no'\n\
         echo 'reached-end rc=0'\n",
    );
    assert!(ok);
    assert_eq!(
        stdout,
        "br1 [/]: yes\n\
         br2 [c/d]: yes\n\
         br3 [c/d] d: no\n\
         br4 [[=/=]]: no\n\
         br5 [[=/=]] slash: yes\n\
         reached-end rc=0\n"
    );
}

#[test]
fn bracket_slash_words_are_literals_in_glob_context() {
    // pathexp.c unquoted_glob_pattern_p: an unquoted `/` voids the
    // bracket, so the word is not a pattern at all — it survives
    // nullglob and failglob untouched.
    let (stdout, _stderr, ok) = run_in_ab_tree(
        "bracketglob",
        "shopt -s extglob\n\
         printf 'g1: '; for f in ab[/]ef; do printf '<%s> ' \"$f\"; done; echo\n\
         printf 'g2: '; for f in ab[c/d]ef; do printf '<%s> ' \"$f\"; done; echo\n\
         shopt -s nullglob\n\
         echo A: X ab[/]ef Y\n\
         shopt -u nullglob; shopt -s failglob\n\
         echo F: X ab[/]ef Y\n\
         echo \"F-rc=$?\"\n\
         echo tail-reached\n",
    );
    assert!(ok);
    assert_eq!(
        stdout,
        "g1: <ab[/]ef> \n\
         g2: <ab[c/d]ef> \n\
         A: X ab[/]ef Y\n\
         F: X ab[/]ef Y\n\
         F-rc=0\n\
         tail-reached\n"
    );
}

#[test]
fn bracket_slash_case_statement_matches() {
    let (stdout, _stderr, ok) = run_in_ab_tree(
        "casebr",
        "case ab/ef in ab[/]ef) echo 'case-br1: yes';; *) echo 'case-br1: no';; esac\n",
    );
    assert!(ok);
    assert_eq!(stdout, "case-br1: yes\n");
}
