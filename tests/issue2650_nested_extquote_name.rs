//! rubash#2650 — a nested `$'name'` (extquote) as a parameter name was
//! rejected inside `${...#...}` patterns; GNU accepts it there
//! (upstream posixexp7.sub: "extquote makes these work").
//!
//! GNU anchors (bash 5.3):
//! - At read time, `parse_matched_pair` (parse.y:4053) translates a
//!   `$'...'` span inside a `${ }` body when `extended_quote` is set,
//!   when the word is not double-quoted, or in the DOLBRACE_QUOTE /
//!   DOLBRACE_QUOTE2 states — the nested expansion's NAME included.
//!   The twin extraction-time gate is subst.c:1586-1610
//!   (`string_extract_verbatim`): DOLBRACE_PARAM + extended_quote
//!   translates, so the inner expansion dispatches on the decoded name.
//! - Observable context rule (GNU 5.3.20 probes, 2026-11-08): the
//!   `$'name'` form is honored in double-quoted words and heredoc
//!   bodies, at top level and nested, in word and pattern position.
//!   In an UNQUOTED word GNU rejects it with bad substitution in both
//!   positions — the same verdict as the no-`$` quoted name
//!   `${'x1'%'t'}`, which is a bad substitution in every position.
//!
//! Oracle: GNU bash 5.3.20 probe scripts (2026-11-08), byte-compared.
//! Harness: script FILES via CARGO_BIN_EXE_rubash (the upstream suite
//! shape), LC_ALL=C.UTF-8 pinned on the child.

use std::io::Write;
use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run_file(script: &str) -> Outcome {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "rubash_issue2650_{}_{}_{}.sh",
        std::process::id(),
        script.len(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::write(&path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_file(&path);
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn run_stdin(script: &str) -> Outcome {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -s");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("wait");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

fn expect(script: &str, want_stdout: &[u8], ctx: &str) {
    let out = run_file(script);
    assert_eq!(
        out.stdout,
        want_stdout,
        "{ctx}: stdout; stderr {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stderr,
        b"",
        "{ctx}: stderr must be silent; got {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0, "{ctx}: exit code");
}

#[test]
fn nested_dollar_name_dollar_pattern() {
    expect(
        "x=notOK; x1=not\necho \"[${x#${$'x1'%$'t'}}]\"\n",
        b"[tOK]\n",
        "nested ${$'x1'%$'t'}",
    );
}

#[test]
fn nested_dollar_name_plain_pattern() {
    expect(
        "x=notOK; x1=not\necho \"[${x#${$'x1'%'t'}}]\"\n",
        b"[tOK]\n",
        "nested ${$'x1'%'t'}",
    );
}

#[test]
fn nested_dollar_name_in_heredoc() {
    expect(
        "x=notOK; x1=not\ncat <<EOF\n[${x#${$'x1'%$'t'}}]\nEOF\n",
        b"[tOK]\n",
        "nested form in heredoc body",
    );
}

#[test]
fn nested_dollar_name_only() {
    expect(
        "x=notOK; x1=not\necho \"[${x#${$'x1'}}]\"\n",
        b"[OK]\n",
        "nested ${$'x1'} as the whole pattern",
    );
}

#[test]
fn nested_dollar_name_unquoted_word_rejected() {
    // GNU rejects the $'name' form in an unquoted word (bad
    // substitution, command abandoned, script continues) — the
    // quoted-context admission is part of the mechanism, not an
    // accident of the double-quoted repro.
    let script = "x=notOK; x1=not\necho ${x#${$'x1'%$'t'}}\necho AFTER\n";
    let out = run_file(script);
    assert_eq!(out.stdout, b"AFTER\n", "the bad command must be abandoned");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        err.contains("bad substitution"),
        "stderr must carry the bad-substitution verdict; got {err:?}"
    );
    assert_eq!(out.code, 0, "script continues; stderr {err:?}");
}

#[test]
fn top_level_dollar_name_expands() {
    expect(
        "x1=not\necho \"[${$'x1'%$'t'}]\"\n",
        b"[no]\n",
        "top-level ${$'x1'%$'t'}",
    );
}

#[test]
fn nested_plain_quoted_name_is_bad_substitution() {
    // No `$`: GNU reports `${'x1'%'t'}: bad substitution`, abandons the
    // command, and continues the script (posixexp7.sub "syntax errors").
    let script = "x=notOK; x1=not\necho \"[${x#${'x1'%'t'}}]\"\necho AFTER\n";
    let out = run_file(script);
    assert_eq!(out.stdout, b"AFTER\n", "the bad command must be abandoned");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        err.contains("${'x1'%") && err.contains("bad substitution"),
        "stderr must carry the bad-substitution verdict on the inner body; got {err:?}"
    );
    assert_eq!(out.code, 0, "script continues; stderr {err:?}");
}

#[test]
fn control_pattern_ansi_c_name() {
    expect(
        "x=notOK\necho \"[${x#$'not'}]\"\n",
        b"[OK]\n",
        "pattern-position $'not'",
    );
}

#[test]
fn control_plain_nested_expansion() {
    expect(
        "x=notOK; x1=not\necho \"[${x#${x1%'t'}}]\"\n",
        b"[tOK]\n",
        "plain nested ${x1%'t'}",
    );
}

#[test]
fn nested_dollar_name_via_stdin() {
    let out = run_stdin("x=notOK; x1=not\necho \"[${x#${$'x1'%$'t'}}]\"\n");
    assert_eq!(
        out.stdout,
        b"[tOK]\n",
        "stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.code, 0);
}
