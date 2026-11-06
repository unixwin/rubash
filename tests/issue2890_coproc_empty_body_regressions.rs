//! rubash#2890 — an empty brace body for `coproc` was accepted
//! silently (`coproc { }` exited 0). GNU's grammar has no empty
//! compound_list production (parse.y:1262), and the coproc rule
//! (parse.y:1125) requires a real command, so the split form is a
//! syntax error blaming the closing brace: rc 2, "syntax error near
//! unexpected token `}'". The one-piece spellings (`coproc {}`)
//! already took the simple-command path in both shells.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (script-file
//! probes s90a-s90h in the r75 hunt).

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
    let dir = std::env::temp_dir().join(format!("issue2890_{}_{}", std::process::id(), seq));
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

#[test]
fn coproc_empty_brace_body_is_a_syntax_error() {
    // A parse error aborts the whole script: the trailing echo
    // never runs, the shell exits 2 (both shells, byte-identical
    // stderr including the source-line echo).
    let run = run_script("coproc { }\necho \"rc:$?\"\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "", "stdout: {}", run.stdout);
    assert!(
        run.stderr
            .contains("syntax error near unexpected token `}'"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn named_coproc_empty_brace_body_is_a_syntax_error() {
    let run = run_script("coproc N { }\necho \"rc:$?\"\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "", "stdout: {}", run.stdout);
    assert!(
        run.stderr
            .contains("syntax error near unexpected token `}'"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn coproc_empty_brace_body_split_across_lines_is_a_syntax_error() {
    let run = run_script("coproc {\n}\necho \"rc:$?\"\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "", "stdout: {}", run.stdout);
    assert!(
        run.stderr
            .contains("syntax error near unexpected token `}'"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn coproc_nonempty_brace_body_still_runs() {
    let run =
        run_script("coproc { echo hi; }\nread -t 2 line <&${COPROC[0]}; echo \"got:$line\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "got:hi\n", "stdout: {}", run.stdout);
}
