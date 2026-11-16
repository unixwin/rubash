//! rubash#2664 — GLOBIGNORE (pathname-expansion side) regressions from
//! the Murase `run-glob-bracket` suite (`tests/glob-bracket.tests`,
//! GNU bash 5.3.20 oracle; the `$GLOBIGNORE vs fnmatch(3)` section).
//!
//! GNU matches a GLOBIGNORE pattern against a candidate pathname with
//! ONE strmatch call carrying FNM_PATHNAME (pathexp.c
//! `glob_name_is_acceptable`), and rubash had two divergences in its
//! port of that matcher (`strmatch_pathname` in src/executor/glob.rs):
//!
//! 1. Brackets can NEVER match `/` under FNM_PATHNAME (sm_loop.c:341
//!    bars `sc == '/'` before BRACKMATCH runs), and a class body with
//!    an unescaped `/` in member position is voided entirely
//!    (`[a/]`, `[c/d]`, `[a-c/]` match nothing). So `ab[/]cd/efg`,
//!    `ab[/a]cd/efg`, `ab[!a]cd/efg` and `ab[.-0]cd/efg` all KEEP
//!    `ab/cd/efg`, while `ab@(/)cd/efg` still excludes it through the
//!    extglob arm. An earlier revision of this fix had brackets match
//!    `/` — the opposite of GNU; corrected in the rework.
//! 2. An extglob alternative after a `*` cannot match `/`: the `*`
//!    never advances past the slash for the group to try there, so
//!    `*@(/)cd/efg` does NOT exclude `ab/cd/efg`. Rubash routed
//!    GLOBIGNORE extglob patterns through the case-pattern engine,
//!    which has no FNM_PATHNAME, so its `*` crossed `/` and the
//!    alternative matched.
//!
//! NOTE on the suite halt (issue #2664, second half): after the
//! pcheck section the suite builds the `strmatch` loadable builtin
//! and calls it for every remaining case. Rubash's dynamic loader
//! accepts the `.so` (RTLD_LAZY, as in GNU) but the host exports no
//! `strmatch` symbol, so the first call dies with silent exit 127.
//! That is a loader/export limitation, not a glob-matcher defect —
//! the bracket cases themselves all match GNU (see the wave-2 pins in
//! tests/issue2664_extglob_slash_bracket_regressions.rs) — and is NOT
//! covered by this fix; it needs a maintainer decision (fail the load
//! loudly vs. export `strmatch`).
//!
//! Expectations verified against GNU bash 5.3.20(1)-release (script
//! files, not -c: extglob must be set before the pattern is parsed).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2664g-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Fixture: directory tree holding exactly `files` (relative paths);
/// the probe script lives outside the globbed directory and runs with
/// the fixture as cwd.
fn run_probe(tag: &str, files: &[&str], body: &str) -> (String, String, bool) {
    let dir = tempdir(tag);
    let fix = dir.join("fix");
    for f in files {
        let p = fix.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).expect("fixture dirs");
        std::fs::write(p, "").expect("fixture file");
    }
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

/// GLOBIGNORE probe over `*/*/efg*`: prints `excluded` when the file
/// is ignored, `kept [...]` otherwise.
const GLOBIGNORE_PROBE: &str = "shopt -s extglob\n\
t() { local GLOBIGNORE=$1; local -a f=(*/*/efg*); if [[ $f == '*/*/efg*' ]]; then echo \"G<$1>: excluded\"; else echo \"G<$1>: kept [${f[*]}]\"; fi; }\n";

#[test]
fn globignore_bracket_never_matches_slash() {
    // Suite lines #1-#21 (pcheck): a bracket can never match `/`
    // (sm_loop.c:341), so every `*`/bracket combination that would
    // need the bracket at a slash KEEPS the file; plain `*` and the
    // extglob arm still exclude where GNU does.
    let (stdout, stderr, ok) = run_probe(
        "bracket",
        &["ab/cd/efg"],
        &format!(
            "{GLOBIGNORE_PROBE}\
            t 'ab/cd/efg'\n\
            t 'ab[/]cd/efg'\n\
            t 'ab[/a]cd/efg'\n\
            t 'ab[a/]cd/efg'\n\
            t 'ab[!a]cd/efg'\n\
            t 'ab[.-0]cd/efg'\n\
            t '*/*/efg'\n\
            t '*[/]*/efg'\n\
            t '*[/a]*/efg'\n\
            t '*[a/]*/efg'\n\
            t '*[!a]*/efg'\n\
            t '*[.-0]*/efg'\n\
            t '*[b]/*/efg'\n\
            t '*[ab]/*/efg'\n\
            t '*[ba]/*/efg'\n\
            t '*[!a]/*/efg'\n\
            t '*[a-c]/*/efg'\n\
            t 'ab@(/)cd/efg'\n\
            t '*@(/)cd/efg'\n\
            t '*/cd/efg'\n"
        ),
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "G<ab/cd/efg>: excluded\n\
        G<ab[/]cd/efg>: kept [ab/cd/efg]\n\
        G<ab[/a]cd/efg>: kept [ab/cd/efg]\n\
        G<ab[a/]cd/efg>: kept [ab/cd/efg]\n\
        G<ab[!a]cd/efg>: kept [ab/cd/efg]\n\
        G<ab[.-0]cd/efg>: kept [ab/cd/efg]\n\
        G<*/*/efg>: excluded\n\
        G<*[/]*/efg>: kept [ab/cd/efg]\n\
        G<*[/a]*/efg>: kept [ab/cd/efg]\n\
        G<*[a/]*/efg>: kept [ab/cd/efg]\n\
        G<*[!a]*/efg>: kept [ab/cd/efg]\n\
        G<*[.-0]*/efg>: kept [ab/cd/efg]\n\
        G<*[b]/*/efg>: excluded\n\
        G<*[ab]/*/efg>: excluded\n\
        G<*[ba]/*/efg>: excluded\n\
        G<*[!a]/*/efg>: excluded\n\
        G<*[a-c]/*/efg>: excluded\n\
        G<ab@(/)cd/efg>: excluded\n\
        G<*@(/)cd/efg>: kept [ab/cd/efg]\n\
        G<*/cd/efg>: excluded\n"
    );
}

#[test]
fn globignore_bracket_with_slash_member_voided() {
    // A `/` anywhere in member position voids the whole bracket for
    // every test char (BRACKMATCH member-scan poison), so `[a/]`,
    // `[c/d]` and even `[a-c/]` — a range matched before the slash —
    // all keep the file. Slash-free classes still work.
    let (stdout, stderr, ok) = run_probe(
        "veto",
        &["aba/cd/efg"],
        &format!(
            "{GLOBIGNORE_PROBE}\
            t 'ab[a/]/cd/efg'\n\
            t 'ab[c/d]/cd/efg'\n\
            t 'ab[a-c/]/cd/efg'\n\
            t 'ab[/]/cd/efg'\n\
            t 'ab[a-c]/cd/efg'\n\
            t 'ab[.-0]/cd/efg'\n\
            t 'ab[!a]/cd/efg'\n"
        ),
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "G<ab[a/]/cd/efg>: kept [aba/cd/efg]\n\
        G<ab[c/d]/cd/efg>: kept [aba/cd/efg]\n\
        G<ab[a-c/]/cd/efg>: kept [aba/cd/efg]\n\
        G<ab[/]/cd/efg>: kept [aba/cd/efg]\n\
        G<ab[a-c]/cd/efg>: excluded\n\
        G<ab[.-0]/cd/efg>: kept [aba/cd/efg]\n\
        G<ab[!a]/cd/efg>: kept [aba/cd/efg]\n"
    );
}

#[test]
fn globignore_extglob_alternative_slash() {
    // An alternative MAY match a literal `/` (`@(ab/cd)`, `@(/)`) —
    // but a `*` before a group cannot carry it across a slash:
    // `*@(/)cd/efg` is the #20 bug (GNU keeps the file).
    let (stdout, stderr, ok) = run_probe(
        "extglob",
        &["ab/cd/efg"],
        &format!(
            "{GLOBIGNORE_PROBE}\
            t '@(ab/cd)/efg'\n\
            t '*@(ab/cd)/efg'\n\
            t '*(ab/cd)/efg'\n\
            t 'ab*(/)cd/efg'\n\
            t 'ab?(/)cd/efg'\n\
            t 'ab+(/)cd/efg'\n\
            t 'ab!(x)cd/efg'\n\
            t 'ab@(/)cd/efg'\n\
            t '*@(/)cd/efg'\n\
            t 'ab@(x|/)cd/efg'\n\
            t 'ab@(cd|/)cd/efg'\n"
        ),
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "G<@(ab/cd)/efg>: excluded\n\
        G<*@(ab/cd)/efg>: excluded\n\
        G<*(ab/cd)/efg>: excluded\n\
        G<ab*(/)cd/efg>: excluded\n\
        G<ab?(/)cd/efg>: excluded\n\
        G<ab+(/)cd/efg>: excluded\n\
        G<ab!(x)cd/efg>: excluded\n\
        G<ab@(/)cd/efg>: excluded\n\
        G<*@(/)cd/efg>: kept [ab/cd/efg]\n\
        G<ab@(x|/)cd/efg>: excluded\n\
        G<ab@(cd|/)cd/efg>: excluded\n"
    );
}

#[test]
fn globignore_extglob_star_run_groups() {
    // sm_loop.c collapse-loop arms: `**(x)y` and `**(/)y` match
    // (the `*` run can consume up to the group), but `*?(x)y`,
    // `*@(x)y` and `a**(x)y` never match (the group op at a slash
    // fails the match).
    let (stdout, stderr, ok) = run_probe(
        "stargroup",
        &["a/bx", "a/bxy"],
        "shopt -s extglob\n\
        u() { local GLOBIGNORE=$1; local -a f=(*/*); if [[ ${#f[@]} == 0 ]]; then echo \"U<$1>: excluded\"; else echo \"U<$1>: kept [${f[*]}]\"; fi; }\n\
        u '**(x)y'\n\
        u '**(/)y'\n\
        u '*?(x)y'\n\
        u '*@(x)y'\n\
        u 'a**(x)y'\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "U<**(x)y>: kept [a/bx]\n\
        U<**(/)y>: kept [a/bx]\n\
        U<*?(x)y>: kept [a/bx a/bxy]\n\
        U<*@(x)y>: kept [a/bx a/bxy]\n\
        U<a**(x)y>: kept [a/bx a/bxy]\n"
    );
}

#[test]
fn globignore_extglob_literal_when_off() {
    // With extglob off, GLOBIGNORE extglob patterns are literal.
    let (stdout, stderr, ok) = run_probe(
        "off",
        &["ab/cd/efg"],
        "t() { local GLOBIGNORE=$1; local -a f=(*/*/efg*); if [[ $f == '*/*/efg*' ]]; then echo \"G<$1>: excluded\"; else echo \"G<$1>: kept [${f[*]}]\"; fi; }\n\
        t 'ab@(/)cd/efg'\n\
        t 'ab@(x|/)cd/efg'\n\
        t 'ab*(/)cd/efg'\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(
        stdout,
        "G<ab@(/)cd/efg>: kept [ab/cd/efg]\n\
        G<ab@(x|/)cd/efg>: kept [ab/cd/efg]\n\
        G<ab*(/)cd/efg>: kept [ab/cd/efg]\n"
    );
}

#[test]
fn plain_glob_and_conditional_match_unchanged() {
    // Boundary locks: the fix is GLOBIGNORE-only. Plain pathname
    // expansion still treats `*@(/)cd/efg` as a literal (no match),
    // and `[[ == ]]` (no FNM_PATHNAME) still matches it.
    let (stdout, stderr, ok) = run_probe(
        "boundary",
        &["ab/cd/efg"],
        "shopt -s extglob\n\
        printf 'p1: '; for f in *@(/)cd/efg; do printf '<%s> ' \"$f\"; done; echo\n\
        if [[ ab/cd/efg == *@(/)cd/efg ]]; then echo 's1: yes'; else echo 's1: no'; fi\n",
    );
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "p1: <*@(/)cd/efg> \ns1: yes\n");
}
