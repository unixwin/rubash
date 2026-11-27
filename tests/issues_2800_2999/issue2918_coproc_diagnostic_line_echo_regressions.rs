//! rubash#2918 — a coproc body that is a syntax error at a `;`
//! (`coproc {;}`, `coproc (;)`) reported the right first line
//! ("syntax error near unexpected token `;'") but echoed only the
//! offending token on the second line, where GNU echoes the whole
//! physical input line (parse.y:6814 print_offending_line over
//! shell_input_line, called from report_syntax_error at
//! parse.y:6833). The coproc body re-parses ran on a bare `parse()`
//! of the body slice, dropping the enclosing parse's diagnostic
//! text, so the echo fell back to joining the body fragment. The
//! reattach covers error nodes nested inside compound bodies of the
//! coproc body as well (an `if`/`while` sequence body, a nested brace
//! group), and the folded-brace re-lex now starts at the brace
//! token's own line so a body on a later script line reports that
//! line, not line 1.
//!
//! Expectations cross-checked against GNU bash (script-file probes;
//! the two headline shapes were confirmed against 5.3.20 in #2918).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2918_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

/// The shared shape: rc 2, the `;` blame on line one, the whole
/// physical input line echoed on line two — never the bare token.
fn assert_full_line_echo(run: &Run, line: usize, echo: &str) {
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "", "stdout: {}", run.stdout);
    assert!(
        run.stderr
            .contains("syntax error near unexpected token `;'"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains(&format!("line {line}: `{echo}'")),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains(&format!("line {line}: `;'\n")),
        "stderr still echoes only the offending token: {}",
        run.stderr
    );
    let _ = &run.dir;
}

#[test]
fn coproc_brace_semicolon_echoes_full_line() {
    let run = run_script("coproc {;}\n");
    assert_full_line_echo(&run, 1, "coproc {;}");
}

#[test]
fn coproc_subshell_semicolon_echoes_full_line() {
    let run = run_script("coproc (;)\n");
    assert_full_line_echo(&run, 1, "coproc (;)");
}

#[test]
fn named_coproc_brace_semicolon_echoes_full_line() {
    let run = run_script("coproc N {;}\n");
    assert_full_line_echo(&run, 1, "coproc N {;}");
}

#[test]
fn coproc_spaced_split_brace_echoes_full_line() {
    let run = run_script("coproc { ; }\n");
    assert_full_line_echo(&run, 1, "coproc { ; }");
}

#[test]
fn coproc_on_second_line_echoes_its_own_line() {
    let run = run_script(":\ncoproc {;}\n");
    assert_full_line_echo(&run, 2, "coproc {;}");
}

#[test]
fn coproc_double_separator_inside_brace_body_echoes_full_line() {
    let run = run_script("coproc { echo hi; ; }\n");
    assert_full_line_echo(&run, 1, "coproc { echo hi; ; }");
}

#[test]
fn coproc_nested_brace_body_error_echoes_full_line() {
    let run = run_script("coproc { { ; }; }\n");
    assert_full_line_echo(&run, 1, "coproc { { ; }; }");
}

#[test]
fn coproc_if_body_error_echoes_full_line() {
    let run = run_script("coproc if true; then ; fi\n");
    assert_full_line_echo(&run, 1, "coproc if true; then ; fi");
}

#[test]
fn coproc_while_body_error_echoes_full_line() {
    let run = run_script("coproc while true; do ; done\n");
    assert_full_line_echo(&run, 1, "coproc while true; do ; done");
}
