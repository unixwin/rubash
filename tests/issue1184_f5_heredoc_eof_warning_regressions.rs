//! Issue unixwin/rubash#1184 finding 5: the here-document EOF warning
//! leaked the U+E001 carrier byte when the delimiter word carried an
//! ANSI-C escape — `cat <<EOF3$'\n'x\nEOF3` printed
//! `` wanted `EOF3<U+E001>xnEOF3' `` instead of naming the decoded
//! delimiter with a real newline.
//!
//! GNU owner: the delimiter word goes through read_token_word's full
//! quoting pipeline (parse.y:5305), so `$'\n'` is decoded before the
//! gather path ever stores it; the EOF warning then prints the decoded
//! delimiter verbatim (parse.y gather_here_documents /
//! read_secondary_line family). Rubash stores the delimiter in carrier
//! form, so the warning site must decode it like every other consumer —
//! the same diagnostic site as rubash#1151 (fb0dd160, quoted-delimiter
//! shape) and its regression pin
//! tests/issue1151_heredoc_delimiter_warning_regressions.rs. The fix
//! already on master decodes the whole carrier set there, so these pins
//! cover the ANSI-C member of the class: a real newline for `$'\n'`, a
//! real TAB for `$'\t'`, and no EE 90 81 byte anywhere in stderr.
//!
//! Expectations verified byte-identical against WSL GNU Bash 5.3.20
//! script files (2026-10-29, master 25039770), including the wanted
//! text spanning a physical line and the trailing literal `\n` of the
//! filed repro folding to `n` (wanted `EOF3<newline>xnEOF3'`).

use std::process::{Command, Stdio};

fn run_rubash_file(script: &str, tag: &str) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!("issue1184_f5_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// The filed repro, verbatim: the delimiter word is
/// `EOF3$'\n'x\nEOF3` — `$'\n'` decodes to a real newline and the
/// trailing literal `\n` folds to `n`, so the warning names
/// `` EOF3<newline>xnEOF3' `` across a physical line. GNU exit status
/// for the warning path is 0 (the warning does not fail the script).
#[test]
fn ansi_c_newline_delimiter_warning_names_decoded_text() {
    let script = r#"cat <<EOF3$'\n'x\nEOF3
"#;
    let (stdout, stderr, code) = run_rubash_file(script, "nl");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("warning: here-document at line 1 delimited by end-of-file"),
        "stderr: {stderr}"
    );
    // The wanted text carries a REAL newline (the pin's byte-level
    // point), then the folded `n` tail.
    assert!(
        stderr.contains("wanted `EOF3\nxnEOF3'"),
        "stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains('\u{E001}'),
        "stderr must not leak the U+E001 carrier: {stderr:?}"
    );
    assert_eq!(stdout, "");
}

/// Class member, not just the filed char: `$'\t'` in the delimiter
/// decodes to a real TAB in the wanted text, and the body still runs to
/// end-of-file (GNU prints `b` then the warning, exit 0).
#[test]
fn ansi_c_tab_delimiter_warning_names_decoded_text() {
    let script = r#"cat <<E$'\t'F
b
"#;
    let (stdout, stderr, code) = run_rubash_file(script, "tab");
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("warning: here-document at line 1 delimited by end-of-file"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("wanted `E\tF'"), "stderr: {stderr:?}");
    assert!(!stderr.contains('\u{E001}'), "stderr: {stderr:?}");
    assert_eq!(stdout, "b\n");
}
