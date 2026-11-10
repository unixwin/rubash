//! Regression pins for rubash#3060 — a `for` loop over a locale
//! variable fired the setlocale failure warning TWICE per bind, where
//! GNU fires it once.
//!
//! GNU ownership: execute_cmd.c:2990 execute_for_command binds the
//! loop variable with bind_variable (execute_cmd.c:3093) and then
//! calls stupidly_hack_special_variables(identifier) once
//! (execute_cmd.c:3097-3100); for the locale family that reaches
//! variables.c:6272 sv_locale -> locale.c:185 set_locale_var, which
//! warns on a failed setlocale (locale.c:225), appending
//! `: <strerror(errno)>` only when errno is set — glibc leaves errno
//! set only on the FIRST failed lookup of a given locale name in the
//! process, so repeat binds of the same bad name warn without the
//! suffix. GNU keeps NO save/restore of the loop variable (the
//! lexical_scoping block in execute_for_command is `#if 0`): after
//! the loop the variable holds its last bound value.
//!
//! rubash root cause: the loop's plain-arm bind delegates the store
//! to apply_shell_assignment, whose tail already notes the bind
//! (rubash#1692), and execute_for_command then noted it a second
//! time at the loop level. The fix keeps exactly one note per bind,
//! owned by the code that performs the store: the funnel for the
//! plain arm, the loop itself for the nameref-cell arm (which stores
//! directly, bypassing the funnel).
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files on a system where the bogus names do not
//! resolve. The bogus names are chosen so no real locale can match.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3060-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Base command with the whole locale family removed from the
/// inherited environment so each case starts from a known state.
fn base() -> Command {
    let mut cmd = Command::new(BIN);
    for var in [
        "LANG",
        "LC_ALL",
        "LC_COLLATE",
        "LC_CTYPE",
        "LC_MESSAGES",
        "LC_NUMERIC",
        "LC_TIME",
    ] {
        cmd.env_remove(var);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn run_script(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = base()
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue3060_single_bad_value_warns_exactly_once() {
    // The issue's repro: one bogus word, one bind, one warning (with
    // the strerror suffix — first failed lookup of the name).
    let (path, stdout, stderr, code) = run_script(
        "single",
        "for LC_ALL in bogus_FOR_LOCALE; do :; done\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_FOR_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3060_good_then_bad_warns_once_for_the_bad_bind() {
    let (path, stdout, stderr, code) = run_script(
        "goodbad",
        "for LC_ALL in C bogus_FOR_LOCALE; do :; done\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_FOR_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3060_bad_good_bad_warns_per_bind_like_gnu() {
    // Two bogus binds: GNU warns for each — the first with the
    // strerror suffix, the second without (glibc errno survives only
    // the first failed lookup of the name in the process). Two
    // warnings total, not four.
    let (path, stdout, stderr, code) = run_script(
        "badgoodbad",
        "for LC_ALL in bogus_FOR_LOCALE C bogus_FOR_LOCALE; do :; done\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_FOR_LOCALE): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_FOR_LOCALE)\n"
        )
    );
}

#[test]
fn issue3060_loop_variable_keeps_last_value() {
    // GNU runs no restore after a for loop: the variable keeps the
    // last bound value. The single warning carries the for line.
    let (path, stdout, stderr, code) = run_script(
        "keep",
        "LC_ALL=C\nfor LC_ALL in bogus_FOR_LOCALE; do :; done\necho \"[$LC_ALL]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[bogus_FOR_LOCALE]\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: warning: setlocale: LC_ALL: cannot change locale (bogus_FOR_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3060_function_local_loop_variable_warns_once() {
    let (path, stdout, stderr, code) = run_script(
        "funclocal",
        "f() { local LC_ALL; for LC_ALL in bogus_FOR_LOCALE; do :; done; }\nf\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_FOR_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3060_nested_loop_warns_once() {
    let (path, stdout, stderr, code) = run_script(
        "nested",
        "for LC_ALL in bogus_FOR_LOCALE; do for x in a b; do :; done; done\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (bogus_FOR_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3060_category_loop_variable_warns_once_with_its_own_name() {
    let (path, stdout, stderr, code) = run_script(
        "category",
        "for LC_CTYPE in bogus_CTYPE_LOCALE; do :; done\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_CTYPE: cannot change locale (bogus_CTYPE_LOCALE): No such file or directory\n"
        )
    );
}

#[test]
fn issue3060_silent_loop_cases_stay_silent() {
    // A non-locale loop variable never reaches the funnel, and LANG
    // binds are silent by GNU design (set_lang never warns).
    let (_path, stdout, stderr, code) = run_script(
        "silent",
        "for x in bogus_FOR_LOCALE; do :; done\nfor LANG in bogus_LANG_LOCALE; do :; done\necho done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "", "no setlocale warning in the silent cases");
}

#[test]
fn issue3060_nameref_loop_variable_retargets_silently() {
    // GNU execute_cmd.c:3081-3095: a nameref loop variable is bound
    // with ASS_NAMEREF — the iteration word RETARGETS the cell (here
    // to a variable named bogus_FOR_LOCALE); LC_ALL itself is never
    // bound, and the sv_ hack runs on the cell's own name, so no
    // warning fires and LC_ALL stays unset.
    let (_path, stdout, stderr, code) = run_script(
        "nameref",
        "declare -n ref=LC_ALL\nfor ref in bogus_FOR_LOCALE; do :; done\necho \"[$LC_ALL]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "", "nameref retarget must not warn");
}
