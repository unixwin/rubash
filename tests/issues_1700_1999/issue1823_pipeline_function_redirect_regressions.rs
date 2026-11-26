//! rubash#1823: a function named as a pipeline stage carrying an
//! explicit redirect resolved as `command not found` (127) — the
//! fd-0 redirect routing in the pipeline stage dispatcher sent the
//! stage straight to the external spawner without consulting the
//! function table.
//!
//! GNU: execute_pipeline (execute_cmd.c:2620) routes every element
//! through execute_simple_command, which classifies the element in
//! the forked child — special builtins, then find_function
//! (execute_cmd.c:4674-4677), then builtins, then disk — regardless
//! of the element's own redirections; the chosen path applies the
//! redirections internally (execute_function's do_redirections,
//! execute_cmd.c:5606). Classification therefore precedes any
//! redirect-based routing.
//!
//! Oracle: GNU bash 5.3.20, script file. The prefix function mirrors
//! bats-core's `bats_prefix_lines_for_tap_output` shape (the real
//! world consumer): it reads its stdin — the redirect target, not
//! the upstream pipe payload — and prefixes each line with `# `.

#![cfg(unix)]

use std::process::{Command, Stdio};

/// Local stand-in for the `tempfile` crate (not a dev-dependency of
/// this repo): a unique scratch directory removed on drop.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_tempdir() -> ScratchDir {
    let dir = std::env::temp_dir().join(format!(
        "rubash-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    ScratchDir(dir)
}

fn rubash_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

/// Write `in.txt` (content `line1\n`) plus `case.sh` holding `script`
/// into a scratch dir, run the script file under rubash, and return
/// (stdout, stderr, success).
fn run_script(script: &str) -> (String, String, bool) {
    let dir = scratch_tempdir();
    std::fs::write(dir.0.join("in.txt"), "line1\n").expect("write in.txt");
    let script_path = dir.0.join("case.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&script_path)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", &dir.0)
        .current_dir(&dir.0)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

const PFX: &str = "pfx() { while IFS= read -r l; do echo \"# $l\"; done; };\n";

#[test]
fn issue1823_function_stage_stdin_redirect_final() {
    // The bats shape: upstream payload is ignored, the function's
    // stdin is the redirect target.
    let (stdout, stderr, ok) = run_script(&format!("{PFX}printf 'ignored\\n' | pfx <in.txt\n"));
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "# line1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1823_function_stage_stdin_redirect_nonfinal() {
    let (stdout, stderr, ok) = run_script(&format!("{PFX}pfx <in.txt | cat\n"));
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "# line1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1823_function_stage_stdin_redirect_two_functions() {
    let (stdout, stderr, ok) = run_script(&format!(
        "{PFX}pfx2() {{ while IFS= read -r l; do echo \"@ $l\"; done; }};\npfx <in.txt | pfx2\n"
    ));
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "@ # line1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1823_function_stage_redirect_shadows_builtin() {
    // Classification order under a redirect is GNU's: a function
    // shadows the builtin of the same name (here `cat`).
    let (stdout, stderr, ok) = run_script("cat() { echo FN; };\nprintf 'x\\n' | cat <in.txt\n");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "FN\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1823_function_stage_no_redirect_control() {
    let (stdout, stderr, ok) = run_script(&format!("{PFX}printf 'x\\n' | pfx\n"));
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "# x\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1823_external_stage_stdin_redirect_control() {
    // The external route the gate exists for stays put: a real
    // external command with `< file` reads the file, not the payload.
    let (stdout, stderr, ok) = run_script("printf 'ignored\\n' | cat <in.txt\n");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "line1\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue1823_builtin_stage_stdin_redirect_control() {
    // The builtin carve-out (rubash#525) stays put: a builtin stage
    // with `< file` runs as the builtin, never `command not found`.
    let (stdout, stderr, ok) = run_script("printf 'z\\n' | read -r v <in.txt | wc -c\n");
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "0");
    assert_eq!(stderr, "");
}

#[test]
fn issue1823_function_stage_output_redirect_control() {
    // Output redirects on a function stage already took the function
    // path; guard that the classification move keeps them working.
    let (stdout, stderr, ok) =
        run_script(&format!("{PFX}printf 'x\\n' | pfx >out.txt\ncat out.txt\n"));
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout, "# x\n");
    assert_eq!(stderr, "");
}
