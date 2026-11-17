//! DEBUG/RETURN/ERR trap actions with a syntax error (rubash#3327; GNU bash
//! 5.3.20 oracle-verified 2026-11-15).
//!
//! GNU `_run_trap_internal` (trap.c:1200) runs each trap action through
//! `parse_and_execute (trap_command, tag, ...)` where the tag is the
//! trap's source name — "debug trap" (trap.c:1288), "error trap"
//! (trap.c:1324), "return trap" (trap.c:1340) — so parser diagnostics
//! read `<prog>: <tag>: line N: syntax error ...`. GNU does NOT set
//! SEVAL_RESETLINE for these three traps (trap.c:1196-1198), so the
//! action parses at the caller's current line: a one-line action whose
//! firing line is L reports `line L+1 ... from \`if' command on line L`.
//! And a parse failure of the action is contained: the diagnostic
//! stands, the pending `$?` is preserved, and the script continues
//! (only an explicit exit jump out of the action, or an ERREXIT jump,
//! propagates).
//!
//! Rubash re-parsed the three actions without the context name, at
//! line 1, and propagated the re-parse error out of the trap runner,
//! so the script aborted with rc=2 and context-free diagnostics.
//!
//! Script-file pins; the script path in diagnostics is normalized
//! to `S`. Diagnostics are asserted on their stable parts (the
//! `<tag>:` infix and the firing-line numbers) plus exact stdout and
//! exact exit status. The RETURN function form needs `set -T`
//! (functrace); without it GNU does not fire the RETURN trap inside
//! a function at all (execute_cmd.c:5293) — that is inheritance, not
//! a bug, so the function pins set it.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

/// Run a script file; return (normalized stdout, normalized stderr, code).
/// `extra` files are written into the same temp dir (name -> body); the
/// main script may source them via `$(dirname "$0")/<name>`.
fn run_script(tag: &str, body: &str, extra: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("issue3327-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    for (name, content) in extra {
        fs::write(dir.join(name), content).expect("write extra file");
    }
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        let raw = path.to_str().expect("utf8 path");
        // Diagnostics echo the script path in rubash's internal form:
        // verbatim on Unix, MSYS-style `/c/...` for a Windows drive path
        // (drive letter lowercased, `:` dropped, `/` separators).
        let fwd = raw.replace('\\', "/");
        let mut needles = vec![raw.to_string(), fwd.clone()];
        if fwd.len() > 2 && fwd.as_bytes()[1] == b':' && fwd.as_bytes()[0].is_ascii_alphabetic() {
            needles.push(format!("/{}{}", fwd[..1].to_lowercase(), &fwd[2..]));
        }
        for needle in needles {
            if !needle.is_empty() {
                text = text.replace(&needle, "S");
            }
        }
        text
    };
    (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.code(),
    )
}

const BAD_ACTION: &str = "if true; then echo SHOULD_NOT_PRINT";

#[test]
fn issue3327_debug_trap_syntax_error_carries_context_and_continues() {
    // DEBUG fires once per command (lines 2, 3, 4): each firing's
    // diagnostic names its firing line (+1 for the EOF line).
    let (stdout, stderr, code) = run_script(
        "debug",
        &format!(
            "trap '{BAD_ACTION}' DEBUG\necho after-debug-1\necho after-debug-2\necho \"rc=$?\"\n"
        ),
        &[],
    );
    assert_eq!(stdout, "after-debug-1\nafter-debug-2\nrc=0\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: debug trap: line 3: syntax error: unexpected end of file from `if' command on line 2"),
        "first firing must name the firing line: {stderr:?}"
    );
    assert!(
        stderr.contains("S: debug trap: line 4: syntax error: unexpected end of file from `if' command on line 3"),
        "second firing must name its firing line: {stderr:?}"
    );
    assert!(
        stderr.contains("S: debug trap: line 5: syntax error: unexpected end of file from `if' command on line 4"),
        "third firing must name its firing line: {stderr:?}"
    );
}

#[test]
fn issue3327_err_trap_syntax_error_carries_context_and_preserves_status() {
    let (stdout, stderr, code) = run_script(
        "err",
        &format!("trap '{BAD_ACTION}' ERR\nfalse\necho \"rc=$?\"\necho done-err\n"),
        &[],
    );
    assert_eq!(stdout, "rc=1\ndone-err\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: error trap: line 3: syntax error: unexpected end of file from `if' command on line 2"),
        "diagnostic must carry the error trap context and the failed line: {stderr:?}"
    );
}

#[test]
fn issue3327_return_trap_functrace_syntax_error_carries_context_and_preserves_status() {
    // The function returns 3; the trap fires at the `return` statement
    // (script line 2, the one-line definition) — hence line 3 / on line 2.
    let (stdout, stderr, code) = run_script(
        "return-ftrace",
        &format!(
            "set -T\nf() {{ return 3; }}\ntrap '{BAD_ACTION}' RETURN\nf\necho \"rc=$?\"\necho done-return\n"
        ),
        &[],
    );
    assert_eq!(stdout, "rc=3\ndone-return\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: return trap: line 3: syntax error: unexpected end of file from `if' command on line 2"),
        "diagnostic must carry the return trap context and the return line: {stderr:?}"
    );
}

#[test]
fn issue3327_return_trap_sourced_syntax_error_carries_context_and_preserves_status() {
    // The RETURN trap fires at the sourced file's exit; the firing line
    // is the `.` command's line (2) in the outer script, whose name the
    // diagnostic carries.
    let (stdout, stderr, code) = run_script(
        "return-source",
        &format!(
            "trap '{BAD_ACTION}' RETURN\n. \"$(dirname \"$0\")/sourced.sh\"\necho \"rc=$?\"\necho done-return\n"
        ),
        &[("sourced.sh", "echo \"in sourced\"\n")],
    );
    assert_eq!(stdout, "in sourced\nrc=0\ndone-return\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("S: return trap: line 3: syntax error: unexpected end of file from `if' command on line 2"),
        "diagnostic must carry the return trap context and the source line: {stderr:?}"
    );
}

#[test]
fn issue3327_valid_debug_action_still_runs_and_preserves_status() {
    // Regression lock: a well-formed DEBUG action runs and the pending
    // status passes through.
    let (stdout, stderr, code) = run_script(
        "debug-valid",
        "trap 'echo DBG' DEBUG\necho hi\necho \"rc=$?\"\n",
        &[],
    );
    assert_eq!(stdout, "DBG\nhi\nDBG\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3327_valid_return_action_still_runs_and_preserves_status() {
    let (stdout, stderr, code) = run_script(
        "return-valid",
        "set -T\nf() { return 3; }\ntrap 'echo RET-FIRED' RETURN\nf\necho \"rc=$?\"\necho done-return\n",
        &[],
    );
    assert_eq!(stdout, "RET-FIRED\nrc=3\ndone-return\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3327_valid_err_action_still_runs_and_preserves_status() {
    let (stdout, stderr, code) = run_script(
        "err-valid",
        "trap 'echo ETRAP' ERR\nfalse\necho \"rc=$?\"\necho done-err\n",
        &[],
    );
    assert_eq!(stdout, "ETRAP\nrc=1\ndone-err\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
