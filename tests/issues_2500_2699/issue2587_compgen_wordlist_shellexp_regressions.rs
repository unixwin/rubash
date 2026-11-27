//! rubash#2587 — `compgen -W` expanded its wordlist with the
//! for-list expander, which pathname-expands every element: glob
//! characters in an element (literal, from a parameter, or from an
//! array splice) were replaced by filename matches, and under
//! failglob a non-matching element failed the expansion into an
//! `Err` that `unwrap_or_default` silently swallowed — the element
//! vanished and the builtin reported a truncated success.
//!
//! GNU ownership: gen_wordlist_matches (pcomplete.c:863-920) expands
//! the split wordlist with expand_words_shellexp (pcomplete.c:884) =
//! expand_word_list_internal with WEXP_SHELLEXP
//! (BRACEEXP|TILDEEXP|PARAMEXP, subst.c:12575); the pathname stage
//! runs only under WEXP_PATHEXP (subst.c:13219), so wordlist
//! elements are NEVER pathname-expanded and failglob cannot fail
//! them. Oracle (bash 5.3.20): in a directory holding one.txt and
//! two.txt, `compgen -W '*.txt'` prints the literal `*.txt`;
//! `shopt -s failglob; compgen -W '/no/such/*.zzz'` prints the
//! literal pattern with status 0.
//!
//! The port: a shellexp entry point on the shared word-expansion
//! walker (the pathname stage forced off) and Result propagation out
//! of expand_compgen_wordlist — a failure aborts the whole wordlist
//! generation (diagnostic + status 1 for the compgen builtin; no
//! candidates from the spec on the live-completion path) instead of
//! dropping one element silently.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_script_in(dir: &std::path::Path, script: &str) -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn fixture_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2587-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("create fixture dir");
    fs::write(dir.join("one.txt"), "").expect("fixture file");
    fs::write(dir.join("two.txt"), "").expect("fixture file");
    dir
}

fn lines(stdout: &str) -> BTreeSet<&str> {
    stdout.lines().collect()
}

#[test]
fn glob_characters_in_wordlist_elements_stay_literal() {
    let dir = fixture_dir();
    // Literal glob element: NOT replaced by the directory's matches.
    let (stdout, _) = run_script_in(&dir, "compgen -W '*.txt' -- ''; echo rc=$?");
    assert_eq!(stdout, "*.txt\nrc=0\n");
    // A parameter expansion RESULT is not pathname-expanded either.
    let (stdout, _) = run_script_in(&dir, "p='*.txt'; compgen -W '$p' -- ''; echo rc=$?");
    assert_eq!(stdout, "*.txt\nrc=0\n");
    // Array splice elements are literal candidates, glob chars and all.
    let (stdout, _) = run_script_in(
        &dir,
        "a=('*.txt' plain); compgen -W '${a[@]}' -- ''; echo rc=$?",
    );
    assert!(stdout.contains("rc=0\n"), "compgen succeeds: {stdout:?}");
    assert_eq!(
        lines(&stdout.replace("rc=0\n", "")),
        BTreeSet::from(["*.txt", "plain"])
    );
    // Real expansion still happens (shellexp expands; it just never globs).
    let (stdout, _) = run_script_in(&dir, "v=yz; compgen -W 'x$v' -- ''; echo rc=$?");
    assert_eq!(stdout, "xyz\nrc=0\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn failglob_cannot_collapse_a_wordlist_element() {
    let dir = fixture_dir();
    let (stdout, stderr) = run_script_in(
        &dir,
        "shopt -s failglob; compgen -W '/no/such/*.zzz' -- ''; echo rc=$?",
    );
    assert_eq!(
        stdout, "/no/such/*.zzz\nrc=0\n",
        "the literal pattern is the candidate and compgen succeeds (stderr {stderr:?})"
    );
    // A failing element in a longer list takes nothing with it.
    let (stdout, _) = run_script_in(
        &dir,
        "shopt -s failglob; compgen -W 'pre /no/such/*.zzz post' -- ''; echo rc=$?",
    );
    assert!(stdout.contains("rc=0\n"), "compgen succeeds: {stdout:?}");
    assert_eq!(
        lines(&stdout.replace("rc=0\n", "")),
        BTreeSet::from(["pre", "/no/such/*.zzz", "post"])
    );
    let _ = fs::remove_dir_all(&dir);
}
