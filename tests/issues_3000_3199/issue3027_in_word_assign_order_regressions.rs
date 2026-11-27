//! rubash#3027 — within one unquoted word, `${var=word}`'s assignment
//! leaks into a preceding `${var:+word}`.
//!
//! GNU evaluates a word's fragments strictly left-to-right
//! (subst.c:11229 `expand_word_internal` scans the word once, calling
//! `param_expand` per `$`; subst.c:9777 `parameter_brace_expand` binds
//! the `=` operator's variable inside
//! `parameter_brace_expand_rhs`, subst.c:8096-8151, at the moment the
//! fragment is reached). At `${a:+b}` time `a` is still unset, so the
//! fragment expands empty; only the later `${a=b}` binds.
//!
//! Root cause: rubash's assignment pre-scan
//! (`apply_parameter_assignment_expansions_in_word`,
//! parameter_words.rs) applies every `:=`/`=` in the word before the
//! real pass evaluates any fragment, and the rubash#1169 freeze is
//! supposed to compensate — an earlier pure fragment that reads the
//! assigned variable is frozen at its pre-assignment value, keyed by
//! its site ((word-ctx, [frag])). But `expand_word`
//! (expand_word.rs) installed a spurious site guard (0) for ANY word
//! starting with `${` and ending with `}` — including a word holding
//! several `${...}` fragments, whose naive `${`/`}` strip yields a
//! bogus single name — and `expand_braced_parameter_word` then declined
//! such a word to the embedded walker while the guard stayed alive, so
//! the walker keyed every fragment one level deeper ([0, frag]) than
//! the freeze ([frag]) and the freeze was never found. The fix gates
//! that guard on the word really spanning one braced parameter, the
//! same gate the mutable whole-word entry already applies.
//!
//! The leak only shows in UNQUOTED multi-fragment words: quoted words
//! take the mutable whole-word entry (which spans-checks before its
//! guard), so `"${a:+b}${a=b}"` was already correct. Every cell below
//! is therefore unquoted, and each expected value was read off GNU bash
//! 5.3.20.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level
//! stdout/stderr assertions. `LC_ALL=C.UTF-8` is set on the child
//! explicitly so the pin is hermetic under any ambient locale.

use std::process::{Command, Stdio};

fn run(script: &str) -> (Vec<u8>, Vec<u8>, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        output.stdout,
        output.stderr,
        output.status.code().unwrap_or(-1),
    )
}

fn run_out(script: &str) -> Vec<u8> {
    let (stdout, stderr, code) = run(script);
    assert!(
        code == 0,
        "rubash -c {script:?} exited with {code}; stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    stdout
}

/// The issue's repro, in an unquoted argument: GNU `b`, pre-fix `bb`.
#[test]
fn plus_before_assign_in_one_word() {
    assert_eq!(run_out("unset a; echo ${a:+b}${a=b}"), b"b\n");
}

/// The issue's repro as the command word itself: GNU tries to run `b`.
#[test]
fn plus_before_assign_as_command_word() {
    let (stdout, stderr, code) = run("unset a; ${a:+b}${a=b}");
    assert_eq!(code, 127);
    assert!(stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&stderr).contains(": b: command not found"),
        "stderr was: {}",
        String::from_utf8_lossy(&stderr)
    );
}

/// The t0238 three-expansion concatenation: GNU `bb`
/// (`${a:+b}` empty, `${a:-b}` -> `b`, `${a=b}` -> `b`).
#[test]
fn three_expansions_concatenated() {
    assert_eq!(run_out("unset a; echo ${a:+b}${a:-b}${a=b}"), b"bb\n");
}

/// The `:=` (colon) spelling of the assignment leaks the same way.
#[test]
fn plus_before_colon_assign_in_one_word() {
    assert_eq!(run_out("unset a; echo ${a:+b}${a:=b}"), b"b\n");
}

/// `${var:-word}` before the assignment: the default still applies.
#[test]
fn minus_before_assign_in_one_word() {
    assert_eq!(run_out("unset a; echo ${a:-b}${a=b}"), b"bb\n");
}

/// Reversed order: the assignment is visible to LATER fragments.
#[test]
fn assign_before_plus_in_one_word() {
    assert_eq!(run_out("unset a; echo ${a=b}${a:+b}"), b"bb\n");
}

/// Split words were never affected: the first word sees unset `a`.
/// (The empty unquoted `${a:+b}` yields zero fields, so there is no
/// leading space — GNU prints plain `b`.)
#[test]
fn plus_and_assign_as_separate_words() {
    assert_eq!(run_out("unset a; echo ${a:+b} ${a=b}"), b"b\n");
}

/// The assignment still takes effect for the rest of the command.
#[test]
fn assign_binds_for_later_reads() {
    assert_eq!(run_out("unset a; echo ${a:+b}${a=b} $a"), b"b b\n");
}

/// The bare `+` (set-ness, not non-emptiness) operator before `=`.
#[test]
fn bare_plus_before_assign_in_one_word() {
    assert_eq!(run_out("unset a; echo ${a+b}${a=b}"), b"b\n");
}

/// An earlier fragment reading the variable itself (unset) keeps the
/// pre-assignment value when the assignment follows it.
#[test]
fn plain_read_before_later_assign() {
    assert_eq!(run_out("unset a; echo ${a}${a=b} $a"), b"b b\n");
}

/// Subscript side-effect dedup (the site guard's other job, SUB_RES_XPASS):
/// each `${arr[$((i++))]}` evaluates its subscript exactly once, in order.
#[test]
fn subscript_side_effects_stay_ordered() {
    assert_eq!(
        run_out("i=0; arr=(p q); echo ${arr[$((i++))]}${arr[$((i++))]}"),
        b"pq\n"
    );
}
