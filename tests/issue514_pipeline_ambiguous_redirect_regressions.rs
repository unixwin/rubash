//! Issue #514: an unquoted unset variable in a redirect word expands to
//! ZERO fields — GNU reports `$TMPF: ambiguous redirect` (the raw word)
//! and a failed pipeline stage dies alone, rc staying the last stage's.
//!
//! GNU spec (redir.c:298-333 redirection_expand NULL → AMBIGUOUS_REDIRECT
//! at redir.c:911 reporting the raw word via redirection_error
//! redir.c:186-190; execute_cmd.c pipeline element fork boundary): the
//! stage's do_redirections fail inside the forked element — its diagnostic
//! rides the element's fd 1 (the pipe), the element exits 1, downstream
//! stages run, and the pipeline status is the LAST stage's (cat reads EOF:
//! rc 0). Corpus F043 (builtin_stage_stderr_diff_check.sh:42):
//! `printf '%d\n' abc 2>&1 >$TMPF | cat` → one
//! `$TMPF: ambiguous redirect` line, rc 0. rubash fell into
//! open("")/ENOENT (`: No such file or directory`, empty word) AND unwound
//! the whole pipeline with rc 1.
//!
//! The zero-field rule is distinct from the quoted-null case (#265, kept):
//! `> ""` anchors one empty field — open("") proceeds and fails ENOENT —
//! and `$v` with v="a b" (multi-field) is ambiguous with the raw word.
//!
//! Rust semantic owner: executor/redirection.rs
//! apply_ordered_output_redirects Output/Append/Clobber arm (the
//! apply-time ambiguous gate — the pipeline stream routing runs the
//! ordered walk after the preflight), and executor/pipeline_exec.rs
//! execute_pipeline_stage_inner (the preflight rejection marks the stage
//! pre-wired so the driver's routing does not re-apply the same list and
//! print the diagnostic twice).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue514-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue514 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("create probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let code = output.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Corpus F043: `2>&1` applies before the failing word, so the element's
/// fd 2 is the pipe — the diagnostic rides it into cat's stdout (GNU
/// measured: the message lands in cat's output), and the pipeline rc is
/// the consumer's 0.
#[test]
fn issue514_pipeline_stage_redirect_failure_continues() {
    let out = run_rubash_script("printf '%d\\n' abc 2>&1 >$TMPF | cat\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert!(
        out.stdout
            .ends_with("line 1: $TMPF: ambiguous redirect\nrc=0\n"),
        "expected the diagnostic through the pipe, got {:?}",
        out.stdout
    );
    assert_eq!(out.stdout.matches("ambiguous redirect").count(), 1);
    assert_eq!(out.stderr, "");
}

/// The dup-first ordering (`2>&1` before the failing word) behaves the
/// same: one diagnostic through the pipe, consumer rc (GNU 0).
#[test]
fn issue514_dup_then_failing_word_single_diagnostic() {
    let out = run_rubash_script("printf x 2>&1 >$TMPF | cat\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert!(
        out.stdout
            .ends_with("line 1: $TMPF: ambiguous redirect\nrc=0\n"),
        "expected the diagnostic through the pipe, got {:?}",
        out.stdout
    );
    assert_eq!(out.stdout.matches("ambiguous redirect").count(), 1);
    assert_eq!(out.stderr, "");
}

/// A plain pipeline producer with the failing word: the diagnostic has no
/// fd-2 rebind (the failure IS the first redirect), so it goes to the
/// script's stderr — not the pipe (GNU measured) — and the consumer runs.
#[test]
fn issue514_producer_stage_dies_consumer_runs() {
    let out = run_rubash_script("echo hi >$TMPF | cat\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "rc=0\n");
    assert!(
        out.stderr.ends_with("line 1: $TMPF: ambiguous redirect\n"),
        "expected the diagnostic on the script stderr, got {:?}",
        out.stderr
    );
    assert_eq!(out.stderr.matches("ambiguous redirect").count(), 1);
}

/// Outside a pipeline the failed redirect aborts the command with
/// status 1 and the raw word (GNU parity; the pre-existing gate).
#[test]
fn issue514_standalone_redirect_reports_raw_word_rc_one() {
    let out = run_rubash_script("echo hi >$TMPF\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "rc=1\n");
    assert!(out.stderr.contains("$TMPF: ambiguous redirect"));
}

/// #265 shape preserved: a QUOTED null is one empty field — open("")
/// proceeds and fails ENOENT, not ambiguous (GNU parity).
#[test]
fn issue514_quoted_null_stays_enoent() {
    let out = run_rubash_script("echo hi > \"\"\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "rc=1\n");
    assert!(out.stderr.contains(": No such file or directory"));
    assert!(!out.stderr.contains("ambiguous"));
}

/// A quoted unset variable anchors one empty field the same way (GNU
/// `: No such file or directory`).
#[test]
fn issue514_quoted_unset_stays_enoent() {
    let out = run_rubash_script("echo x > \"$TMPF\"\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "rc=1\n");
    assert!(out.stderr.contains(": No such file or directory"));
    assert!(!out.stderr.contains("ambiguous"));
}

/// Multi-field unquoted expansion is ambiguous with the raw word (GNU
/// `$v: ambiguous redirect`).
#[test]
fn issue514_multi_field_expansion_is_ambiguous() {
    let out = run_rubash_script("v=\"a b\"\necho x >$v\necho \"rc=$?\"\n");
    assert_eq!(out.code, Some(0));
    assert_eq!(out.stdout, "rc=1\n");
    assert!(out.stderr.contains("$v: ambiguous redirect"));
}
