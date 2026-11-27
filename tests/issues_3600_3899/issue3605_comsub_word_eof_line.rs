//! rubash#3605 class B (run-braces) — a `$(` surviving into a `${}`
//! word expansion (braces.tests `"${a+'$('\'}")` — the `'...'` quotes
//! make `$(` literal at parse time, but the `${a+word}`/`${a-word}`
//! word expansion re-processes it as a real command substitution)
//! reports `unexpected EOF while looking for matching `)'` through the
//! `command substitution:` prefix, but rubash stamped the construct's
//! own line where GNU stamps construct_line + 2.
//!
//! GNU mechanics (bash 5.3.20, vendored source): command_substitute
//! re-parses the extracted body via `parse_and_execute (string,
//! "command substitution", pflags|SEVAL_NOHIST)` (subst.c:7413) with
//! `pflags = 0` non-interactively (subst.c:7210 — no SEVAL_RESETLINE),
//! so `push_stream(0)` keeps the script's `line_number` and
//! `line_number--` (evalstring.c:345) backs it up one; the body's own
//! lines plus its EOF read then carry the counter to
//! `exec_line + body_lines + 1`. Measured on GNU bash 5.3.20:
//! `echo "${a+'$(\'}"` on file line N reports `line N+2`, and a body
//! spanning two lines reports `line N+3`.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! 2026-11-15).

use std::io::Write;
use std::process::Command;

const EOF_PAREN: &str = "unexpected EOF while looking for matching `)'";

/// Run `script` as a script FILE and return (stdout, stderr lines with
/// the `<path>: ` prolog and the `command substitution:` whom stripped
/// to `line N: ...`).
fn run_script(script: &str) -> (String, Vec<String>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| match line.find(": line ") {
            Some(idx) => line[idx + 2..].to_string(),
            None => line.to_string(),
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

/// The braces.tests seed: `${a-word}` expands the word when `a` is
/// unset; the `'$('` literal becomes a live comsub whose body runs out
/// at the word end. GNU reports exec line + 2.
#[test]
fn issue3605_dollar_minus_word_comsub_eof_reports_plus_two() {
    let (_stdout, stderr, _rc) = run_script("echo \"${a-'$(\\'}\"\n");
    assert_eq!(stderr, vec![format!("line 3: {EOF_PAREN}")]);
}

/// `${a+word}` expands the word when `a` IS set — the suite's actual
/// shape (`a=4` earlier in braces.tests). Same +2 at the construct's
/// line 2.
#[test]
fn issue3605_dollar_plus_word_comsub_eof_reports_plus_two() {
    let (_stdout, stderr, _rc) = run_script("a=x\necho \"${a+'$(\\'}\"\n");
    assert_eq!(stderr, vec![format!("line 4: {EOF_PAREN}")]);
}

/// The +2 tracks the construct's own line, not a fixed number.
#[test]
fn issue3605_comsub_eof_line_scales_with_construct_line() {
    let (_stdout, stderr, _rc) = run_script("a=x\ntrue\necho \"${a+'$(\\'}\"\n");
    assert_eq!(stderr, vec![format!("line 5: {EOF_PAREN}")]);
}
