//! rubash#2942 — `enable -n :` must persist.
//!
//! GNU anchor: `builtins/enable.def` `enable_shell_command` — disabling
//! is a flag clear on the builtin struct (`b->flags &= ~BUILTIN_ENABLED`)
//! with no name-shape restriction; `:` is a static special builtin and
//! disables like any other. Verified against GNU bash 5.3.20:
//!
//!   $ enable -n :; echo rc=$?; enable -n
//!   rc=0
//!   enable -n :
//!
//! rubash carried the disabled set in the colon-joined variable
//! `__RUBASH_DISABLED_BUILTINS`, so the single-element list `[":"]`
//! stored as `":"` and read back as zero names — the disable silently
//! vanished. The carrier is now newline-joined (a builtin name is one
//! shell word from the static table or a loadable symbol name; neither
//! can contain a newline, while `:` is itself a builtin name).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    #[allow(dead_code)]
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2942_{}_{}", std::process::id(), seq));
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
fn disable_colon_persists_and_lists() {
    // The issue seed: disable, list disabled, list all.
    let run = run_script("enable -n :\necho \"rc=$?\"\nenable -n\nenable -a\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let mut lines = run.stdout.lines();
    assert_eq!(lines.next(), Some("rc=0"), "stdout: {}", run.stdout);
    assert_eq!(lines.next(), Some("enable -n :"), "stdout: {}", run.stdout);
    assert!(
        run.stdout
            .lines()
            .filter(|line| *line == "enable -n :")
            .count()
            == 2,
        "enable -a must also show `enable -n :`; stdout: {}",
        run.stdout
    );
    assert!(
        !run.stdout.lines().any(|line| line == "enable :"),
        "`:` must not list as enabled; stdout: {}",
        run.stdout
    );
}

#[test]
fn disable_colon_alongside_another_builtin() {
    // The carrier holds both names; `:` must not eat or corrupt `echo`.
    let run = run_script("enable -n :\nenable -n echo\nenable -n\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "enable -n :\nenable -n echo\n");
}

#[test]
fn reenable_colon_clears_only_colon() {
    let run = run_script("enable -n :\nenable -n echo\nenable :\nenable -n\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "enable -n echo\n");
}

#[test]
fn disabled_colon_dispatches_to_external_lookup() {
    // A disabled builtin falls through to PATH lookup: with no `:` file
    // on PATH the command is simply not found, and the shell lives.
    let run = run_script("enable -n :\n:\necho \"rc=$?\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("rc=127"), "stdout: {}", run.stdout);
}
