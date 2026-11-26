//! rubash#2475 — a tempenv prefix whose name is not a valid
//! identifier is diagnosed and dropped by GNU, and the command
//! still runs with its redirections intact.
//!
//! GNU ground: assign_in_env (variables.c:3536) validates the name
//! before the `=` with valid_identifier; on failure sh_invalidid
//! (variables.c:3563; error.c:461) reports
//! `` `NAME': not a valid identifier ``, the assignment is dropped,
//! tempenv_assign_error is bumped (subst.c:13199), and in non-POSIX
//! mode execution simply continues — the command runs, its
//! redirections apply, and its exit status is its own. (The issue
//! text's original "the command does not run" reading came from the
//! `>/dev/null` shape alone: env DID run, GNU's redirect swallowed
//! its output exactly as written.)
//!
//! rubash's tempenv diagnostic + drop + continue were already
//! right; what was lost was the command's redirections. `env` runs
//! in-process (PrimarySlot::Env), and its no-subcommand print branch
//! wrote through write_default_stdout — the fd-table endpoint. A
//! plain command's `>` binding never lands in the fd table (only
//! pipeline/capture/exec bindings do), so the dump escaped to the
//! terminal and the redirect file was never created. env's own
//! diagnostics had the same disease via write_default_stderr. Both
//! now flush through the buffered builtin router
//! (route_builtin_buffered_output — the do_redirections model every
//! other builtin uses, execute_cmd.c:5606), with the print branch
//! folding a failed flush into the exit status (sh_chkwrite, the
//! rubash#1496 pattern).
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle
//! with script-file probes. env's dump content is
//! environment-dependent, so the file assertions target specific
//! lines instead of comparing whole dumps.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2475-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as a script FILE with the scratch dir as cwd;
/// return (status, stdout, stderr) with the scratch path normalized
/// to WORK and the script path to SCRIPT.
fn run_script(dir: &Path, script: &str) -> (i32, String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .current_dir(dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        text.replace(&script_path.to_string_lossy().into_owned(), "SCRIPT")
            .replace(&dir.to_string_lossy().into_owned(), "WORK")
    };
    (
        output.status.code().unwrap_or(-1),
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

const DIAGNOSTIC: &str = "SCRIPT: line 1: `a[abc + 1]': not a valid identifier\n";

#[test]
fn issue_shape_env_output_obeys_dev_null_redirect() {
    let dir = scratch("exact");
    let (status, stdout, stderr) =
        run_script(&dir, "x=1 a[abc + 1]=x env >/dev/null\necho rc=$?\n");
    // GNU: the assignment is diagnosed and dropped, env runs, and
    // >/dev/null swallows its dump — nothing reaches the terminal.
    assert_eq!(status, 0);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, DIAGNOSTIC);
}

#[test]
fn env_dump_lands_in_the_redirect_file() {
    let dir = scratch("tofile");
    let (status, stdout, stderr) =
        run_script(&dir, "ZZQ2475=1 a[abc + 1]=x env > out.txt\necho rc=$?\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, DIAGNOSTIC);
    // The redirect file is created and holds the dump: the valid
    // sibling assignment reaches the child environment, the invalid
    // one was dropped (no line for it).
    let dump = fs::read_to_string(dir.join("out.txt")).expect("env redirect file exists");
    assert!(
        dump.lines().any(|line| line == "ZZQ2475=1"),
        "valid sibling assignment missing from dump:\n{dump}"
    );
    assert!(
        !dump.lines().any(|line| line.starts_with("a[")),
        "invalid assignment leaked into dump:\n{dump}"
    );
}

#[test]
fn command_still_runs_with_the_valid_sibling_assignments() {
    let dir = scratch("runs");
    // GNU does NOT abort the command on a tempenv diagnostic: the
    // (in-process) command runs and sees the valid sibling.
    let (status, stdout, stderr) = run_script(
        &dir,
        "f() { echo \"got:$ZZQ2475\"; }\nZZQ2475=5 a[abc + 1]=x f\n",
    );
    assert_eq!(status, 0);
    assert_eq!(stdout, "got:5\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 2: `a[abc + 1]': not a valid identifier\n"
    );
    // An external command runs too, and the command's own status is
    // what the script sees.
    let (status, stdout, stderr) = run_script(&dir, "a[abc + 1]=x false\necho rc=$?\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(stderr, DIAGNOSTIC);
}

#[test]
fn naked_env_redirect_guard() {
    let dir = scratch("naked");
    // No tempenv involved: the in-process env print branch itself
    // must obey a plain redirect.
    let (status, stdout, stderr) = run_script(&dir, "env >/dev/null\necho rc=$?\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    let (status, stdout, stderr) = run_script(&dir, "ZZQ2475=9 env > out.txt\necho rc=$?\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
    let dump = fs::read_to_string(dir.join("out.txt")).expect("env redirect file exists");
    assert!(dump.lines().any(|line| line == "ZZQ2475=9"));
}

#[test]
fn env_own_diagnostic_obeys_stderr_redirect() {
    let dir = scratch("diagredir");
    let (status, stdout, stderr) = run_script(
        &dir,
        "env nonexistent_prog_xyz_2475 2> err.txt\necho rc=$?\n",
    );
    assert_eq!(status, 0);
    assert_eq!(stdout, "rc=127\n");
    // The diagnostic went into err.txt, not onto the terminal.
    assert_eq!(stderr, "");
    let captured = fs::read_to_string(dir.join("err.txt")).expect("stderr redirect file exists");
    assert!(
        captured.contains("nonexistent_prog_xyz_2475"),
        "diagnostic missing from err.txt: {captured}"
    );
}

#[test]
fn env_in_a_pipeline_still_works() {
    let dir = scratch("pipeline");
    let (status, stdout, stderr) = run_script(&dir, "ZZQ2475=3 env | grep ^ZZQ2475=3$\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "ZZQ2475=3\n");
    assert_eq!(stderr, "");
}
