//! rubash#2940 — `type` misses the `-type`/`-path`/`-all` prescan rewrite
//! (Stage-2 ④ Phase 2 E1).
//!
//! GNU anchor: `builtins/type.def:122-141` — before getopt, `type_builtin`
//! prescans the leading dash-words and rewrites the obsolescent forms
//! in place: `-type`/`--type` → `-t`, `-path`/`--path` → `-p`,
//! `-all`/`--all` → `-a`. The prescan runs while the word starts with
//! `-` (it continues past a bare `-`) and stops at the first non-dash
//! word. A rewritten word that getopt never consumes (because a bare
//! `-` or `--` stopped the scan first) is looked up as a NAME.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (E1 differential
//! + direct probes, master `854eed80`). The long forms in the LEADING
//! position already work on both shells (`type -path echo`,
//! `type --type echo` are SAME) — only words after a scan-stopping
//! `-`/`--` diverge.

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
    let dir = std::env::temp_dir().join(format!("issue2940_{}_{}", std::process::id(), seq));
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
fn dash_type_after_bare_dash_is_rewritten() {
    // GNU: the prescan rewrites `-type` → `-t` even after a bare `-`;
    // getopt stops at the `-`, so `-t` is looked up as a name.
    let run = run_script("type - -type echo\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "echo is a shell builtin\n",
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("type: -: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("type: -t: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("-type"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn dash_type_after_dashdash_is_rewritten() {
    let run = run_script("type -- -type echo\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "echo is a shell builtin\n",
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("type: -t: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("-type"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn dash_path_after_bare_dash_is_rewritten() {
    let run = run_script("type - -path echo\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "echo is a shell builtin\n",
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("type: -p: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("-path"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn dash_all_after_bare_dash_is_rewritten() {
    let run = run_script("type - -all echo\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "echo is a shell builtin\n",
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("type: -a: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("-all"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn dash_path_after_dashdash_is_rewritten() {
    let run = run_script("type -- -path echo\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "echo is a shell builtin\n",
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("type: -p: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("-path"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn long_type_after_bare_dash_is_rewritten() {
    let run = run_script("type - --type echo\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "echo is a shell builtin\n",
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("type: -t: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("--type"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn long_all_after_dashdash_is_rewritten() {
    let run = run_script("type -- --all echo\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "echo is a shell builtin\n",
        "stdout: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("type: -a: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(!run.stderr.contains("--all"), "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn leading_long_forms_still_work() {
    // Positive controls: the long forms in the leading position keep
    // working (they already did on master).
    let run = run_script("type --type echo\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "builtin\n", "stdout: {}", run.stdout);
    let _ = run.dir;

    let run = run_script("type -path echo\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    let _ = run.dir;

    let run = run_script("type -all -t echo\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "builtin\nfile\nfile\n",
        "stdout: {}",
        run.stdout
    );
    let _ = run.dir;
}
