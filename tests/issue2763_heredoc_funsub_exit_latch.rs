//! rubash#2763 — a `${ ...; exit N; }` funsub inside a heredoc body
//! latched `current_shell_substitution_exit`, whose only consumer ran
//! before the heredoc pre-expansion, so the latch lingered and surfaced
//! in the NEXT command instead of being consumed by the command that
//! carried the body.
//!
//! GNU anchor: redir.c:348 heredoc_expand runs the body inside the
//! command's do_redirections. Where those redirects run decides the
//! blast radius (execute_cmd.c:5522 execute_disk_command forks first
//! and the CHILD applies the redirects; execute_cmd.c:5606
//! execute_builtin_or_function applies them in the current shell):
//!
//! - Builtin/function target: the funsub `exit` aborts the shell with
//!   N — the command never runs (probes A/C: nothing after the
//!   heredoc line prints, rc 42).
//! - Disk-command target: the child dies with N during do_redirections
//!   before the command execs (probe G: `cat` never prints the body's
//!   `hello`), so the command never runs, N is its status, and the
//!   script continues (probe B: B2 prints status=42, B3 runs).
//! - Word expansion (probe D) already aborted via the consumer in
//!   command_execute.rs — pinned here as a guard.
//!
//! Expectations verified against GNU bash 5.3.20 (script files).
//! Fail-first (code-derived, pre-fix): probe B printed nothing after
//! B1 — the next command's word-expansion consumer took the leaked
//! latch and aborted the shell with 42 before B2 printed.

use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue2763-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script_path = dir.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn builtin_read_heredoc_funsub_exit_aborts_shell() {
    let (stdout, _, code) = run_script(
        "read-aborts",
        "echo \"A1-before\"\nread x <<EOF\n${ exit 42; }\nEOF\necho \"A2-after-read status=$? x=[$x]\"\necho \"A3-next-ran\"\n",
    );
    assert_eq!(stdout, "A1-before\n");
    assert_eq!(code, Some(42));
}

#[test]
fn external_cat_heredoc_funsub_exit_is_command_status() {
    let (stdout, _, code) = run_script(
        "cat-status",
        "echo \"B1-before\"\ncat <<EOF\n${ exit 42; }\nEOF\necho \"B2-after-cat status=$?\"\necho \"B3-next-ran\"\n",
    );
    assert_eq!(stdout, "B1-before\nB2-after-cat status=42\nB3-next-ran\n");
    assert_eq!(code, Some(0));
}

#[test]
fn null_builtin_heredoc_funsub_exit_aborts_shell() {
    let (stdout, _, code) = run_script(
        "null-aborts",
        "echo \"C1-before\"\n: <<EOF\n${ exit 42; }\nEOF\necho \"C2-after-null status=$?\"\necho \"C3-next-ran\"\n",
    );
    assert_eq!(stdout, "C1-before\n");
    assert_eq!(code, Some(42));
}

#[test]
fn word_funsub_exit_aborts_shell() {
    let (stdout, _, code) = run_script(
        "word-aborts",
        "echo \"D1-before\"\necho \"D2-word ${ exit 42; } tail\"\necho \"D3-after status=$?\"\n",
    );
    assert_eq!(stdout, "D1-before\n");
    assert_eq!(code, Some(42));
}

#[test]
fn external_cat_heredoc_funsub_exit_never_runs_command() {
    let (stdout, _, code) = run_script(
        "cat-never-runs",
        "echo \"G1-before\"\ncat <<EOF\n${ echo hello; exit 42; }\nEOF\necho \"G2-after status=$?\"\necho \"G3-next\"\n",
    );
    assert_eq!(stdout, "G1-before\nG2-after status=42\nG3-next\n");
    assert_eq!(code, Some(0));
}

#[test]
fn pipeline_stage_heredoc_funsub_exit_is_stage_status() {
    let (stdout, _, code) = run_script(
        "pipe-stage",
        "echo \"F1-before\"\ncat <<EOF | cat\n${ exit 42; }\nEOF\necho \"F2-after status=$? pipe=${PIPESTATUS[*]}\"\necho \"F3-next\"\n",
    );
    assert_eq!(stdout, "F1-before\nF2-after status=0 pipe=42 0\nF3-next\n");
    assert_eq!(code, Some(0));
}
