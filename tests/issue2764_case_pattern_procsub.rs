//! rubash#2764 — case-pattern words carrying an unquoted process
//! substitution are EXECUTED at match time, like GNU.
//!
//! GNU anchor: execute_cmd.c:3712 (`expand_word_leave_quoted`, per
//! pattern attempt) + subst.c:11365-11385 (the `<`/`>` arms of
//! `expand_word_internal`) + subst.c:6362 (`process_substitute`):
//! an unquoted `<(...)` / `>(...)` in a case pattern expands to the
//! `/dev/fd/N` word form (the body runs as an async child), and the
//! fd path — NOT the body's output — is the pattern. Every
//! expectation below is a GNU bash 5.3.20 measurement taken with
//! `~/workspace/rubash-audit/build-gnu/bash-5.3/bash` on the probe
//! scripts `probes/gnu{1,2,3,5}.sh` (2026-11-12/13); the rubash
//! baseline carried `<(...)` as literal text, so no body ever ran
//! and no fd-path pattern ever matched.
//!
//! Unix only: the faces ride `/dev/fd` paths and async children.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-2764-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    let canonical = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    ScratchDir(canonical)
}

fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

fn assert_script(dir: &Path, script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = run_rubash(dir, script);
    assert_eq!(
        code,
        Some(0),
        "script rc: {code:?}\nscript:\n{script}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stderr, "",
        "script stderr must be empty\nscript:\n{script}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        stdout, expected_stdout,
        "script stdout\nscript:\n{script}\nstderr:\n{stderr}"
    );
}

/// GNU probe B (gnu1.sh): the first input procsub in a fresh shell
/// is `/dev/fd/63`, so the fd-path word matches the pattern.
#[test]
fn issue2764_case_pattern_input_procsub_matches_fd_path() {
    let dir = scratch("fd-path");
    assert_script(
        &dir.0,
        "case /dev/fd/63 in <(true)) echo MATCH;; *) echo NOMATCH;; esac\n",
        "MATCH\n",
    );
}

/// GNU probe S (gnu3.sh): same match with the word coming from a
/// variable.
#[test]
fn issue2764_case_pattern_input_procsub_matches_variable_word() {
    let dir = scratch("var-word");
    assert_script(
        &dir.0,
        "p='/dev/fd/63'\ncase \"$p\" in <(true)) echo MATCH;; *) echo NOMATCH;; esac\n",
        "MATCH\n",
    );
}

/// GNU probe A/A2 (gnu1.sh/gnu2.sh): the `<(...)` body runs (async
/// child) even though the fd-path pattern does not match `x`; the
/// marker appears after a short sleep.
#[test]
fn issue2764_case_pattern_input_procsub_side_effect() {
    let dir = scratch("input-side-effect");
    assert_script(
        &dir.0,
        "case x in <(touch m1; printf 'x')) echo MATCH;; *) echo NOMATCH;; esac\nsleep 1\ntest -e m1 && echo MARKER_CREATED || echo MARKER_MISSING\n",
        "NOMATCH\nMARKER_CREATED\n",
    );
}

/// GNU probe D (gnu1.sh) + Q1/Q4 (gnu5.sh): a quoted
/// `"<(...)"` / `'<(...)'` stays literal text — no body runs, no
/// fd-path pattern.
#[test]
fn issue2764_case_pattern_quoted_procsub_stays_literal() {
    let dir = scratch("quoted-literal");
    assert_script(
        &dir.0,
        "case x in \"<(touch m3)\") echo MATCH;; *) echo NOMATCH;; esac\nsleep 1\ntest -e m3 && echo MARKER_CREATED || echo MARKER_MISSING\ncase x in '<(touch m3b)') echo MATCH;; *) echo NOMATCH;; esac\nsleep 1\ntest -e m3b && echo MARKER2_CREATED || echo MARKER2_MISSING\n",
        "NOMATCH\nMARKER_MISSING\nNOMATCH\nMARKER2_MISSING\n",
    );
}

/// GNU probe F2 (gnu2.sh): an unquoted glob suffix after the
/// procsub still globs (`/dev/fd/63*` matches); GNU probe F
/// (gnu1.sh): a QUOTED `'*'` is a literal star, so
/// `/dev/fd/63*`-with-literal-star does not match `/dev/fd/63`.
#[test]
fn issue2764_case_pattern_procsub_glob_suffix() {
    let dir = scratch("glob-suffix");
    assert_script(
        &dir.0,
        "case /dev/fd/63 in <(true)*) echo MATCH;; *) echo NOMATCH;; esac\ncase /dev/fd/63 in <(true)'*') echo MATCH;; *) echo NOMATCH;; esac\n",
        "MATCH\nNOMATCH\n",
    );
}

/// GNU probes J + K2 (gnu2.sh/gnu3.sh): the procsub splices
/// mid-word and after a glob prefix.
#[test]
fn issue2764_case_pattern_procsub_midword_and_prefix() {
    let dir = scratch("midword-prefix");
    assert_script(
        &dir.0,
        "case /dev/fd/63x in <(true)x) echo MATCH;; *) echo NOMATCH;; esac\ncase z/dev/fd/63 in *<(true)) echo MATCH;; *) echo NOMATCH;; esac\n",
        "MATCH\nMATCH\n",
    );
}

/// GNU probe L (gnu2.sh/gnu3.sh): two procsubs in one pattern get
/// two distinct fds (`/dev/fd/63/dev/fd/62`), so the
/// `/dev/fd/63/dev/fd/63` word does not match.
#[test]
fn issue2764_case_pattern_two_procsubs_distinct_fds() {
    let dir = scratch("two-procsubs");
    assert_script(
        &dir.0,
        "case /dev/fd/63/dev/fd/63 in <(true)/<(true)) echo MATCH;; *) echo NOMATCH;; esac\n",
        "NOMATCH\n",
    );
}

/// GNU probe C (gnu1.sh): the `>(...)` body runs even though the
/// fd-path pattern does not match `x`. The marker check follows
/// the case IMMEDIATELY (no intervening command): the body is
/// finished by the case command itself, so it must already exist
/// here — a later command's drain must not be what creates it.
#[test]
fn issue2764_case_pattern_output_procsub_side_effect() {
    let dir = scratch("output-side-effect");
    assert_script(
        &dir.0,
        "case x in >(touch m2)) echo MATCH;; *) echo NOMATCH;; esac\ntest -e m2 && echo MARKER_CREATED || echo MARKER_MISSING\n",
        "NOMATCH\nMARKER_CREATED\n",
    );
}

/// GNU probe R (gnu3.sh): an output procsub's fd path matches —
/// the `>(true)` materialized for the case reuses the freed fd 63.
#[test]
fn issue2764_case_pattern_output_procsub_matches_fd_path() {
    let dir = scratch("output-fd-path");
    assert_script(
        &dir.0,
        "fd=$(echo >(true))\ncase \"$fd\" in >(true)) echo MATCH;; *) echo NOMATCH;; esac\n",
        "MATCH\n",
    );
}

/// GNU probe Q (gnu3.sh) + Q5 (gnu5.sh): the body's OUTPUT is not
/// the pattern — `hello` does not match `<(printf 'hello')`, whose
/// pattern is the fd path.
#[test]
fn issue2764_case_pattern_procsub_output_not_pattern() {
    let dir = scratch("output-not-pattern");
    assert_script(
        &dir.0,
        "case hello in <(printf 'hello')) echo MATCH;; *) echo NOMATCH;; esac\n",
        "NOMATCH\n",
    );
}

/// GNU probe E (gnu1.sh), control: command substitution in the
/// same position already worked — it keeps working, and its side
/// effect runs.
#[test]
fn issue2764_case_pattern_comsub_control() {
    let dir = scratch("comsub-control");
    assert_script(
        &dir.0,
        "case x in $(touch m4; printf x)) echo MATCH;; *) echo NOMATCH;; esac\nsleep 1\ntest -e m4 && echo MARKER_CREATED || echo MARKER_MISSING\n",
        "MATCH\nMARKER_CREATED\n",
    );
}

/// GNU re-expansion per attempt (probe G shape): a word matching
/// no arm still expands EVERY arm's pattern, so both arms' bodies
/// run. If expansion happened only once, `a2` would be missing.
#[test]
fn issue2764_case_pattern_each_arm_expanded() {
    let dir = scratch("each-arm");
    assert_script(
        &dir.0,
        "case y in <(touch a1; printf 'x')) echo ARM1;; <(touch a2; printf 'y')) echo ARM2;; *) echo NOMATCH;; esac\nsleep 1\ntest -e a1 && echo A1_CREATED || echo A1_MISSING\ntest -e a2 && echo A2_CREATED || echo A2_MISSING\n",
        "NOMATCH\nA1_CREATED\nA2_CREATED\n",
    );
}

/// GNU probe P (gnu3.sh): a procsub as one extglob alternative
/// matches the fd path.
#[test]
fn issue2764_case_pattern_procsub_extglob_alternative() {
    let dir = scratch("extglob-alt");
    assert_script(
        &dir.0,
        "shopt -s extglob\ncase /dev/fd/63 in <(true)|/dev/fd/64) echo MATCH;; *) echo NOMATCH;; esac\n",
        "MATCH\n",
    );
}

/// rubash#2764 F2 rework — count-in (probe-threeway.sh): the
/// `<(...)` body runs exactly ONCE per attempt. GNU 5.3.20: `N|1`.
/// The first-round code materialized both `pattern.text` and
/// `pattern.raw_text`, so the body ran twice (`cnt` held 2 lines).
#[test]
fn issue2764_case_pattern_procsub_counted_once_input() {
    let dir = scratch("count-in");
    assert_script(
        &dir.0,
        "rm -f cnt\ncase x in <(echo x >> cnt)) echo M;; *) echo N;; esac\nsleep 1\ntest -e cnt && wc -l < cnt || echo 0\n",
        "N\n1\n",
    );
}

/// rubash#2764 F2 rework — count-out (probe-threeway.sh): the
/// `>(...)` body runs exactly ONCE per attempt. GNU 5.3.20: `N|1`.
/// The first-round code registered two different fd-words for the
/// same body and finished both (`cnt` held 2 lines).
#[test]
fn issue2764_case_pattern_procsub_counted_once_output() {
    let dir = scratch("count-out");
    assert_script(
        &dir.0,
        "rm -f cnt\ncase x in >(echo x >> cnt)) echo M;; *) echo N;; esac\nsleep 1\ntest -e cnt && wc -l < cnt || echo 0\n",
        "N\n1\n",
    );
}

/// rubash#2764 F2 rework — count-two-arms (probe-threeway.sh):
/// a word matching no arm still expands EVERY arm's pattern, but
/// each arm's body runs exactly once. GNU 5.3.20:
/// `N|c1=1 c2=1`.
#[test]
fn issue2764_case_pattern_procsub_counted_once_two_arms() {
    let dir = scratch("count-two-arms");
    assert_script(
        &dir.0,
        "rm -f c1 c2\ncase y in <(echo x >> c1)) echo A1;; <(echo x >> c2)) echo A2;; *) echo N;; esac\nsleep 1\necho \"c1=$(test -e c1 && wc -l < c1 || echo 0) c2=$(test -e c2 && wc -l < c2 || echo 0)\"\n",
        "N\nc1=1 c2=1\n",
    );
}
