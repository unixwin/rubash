//! Glob / expansion / quoting regression pins (backfill wave bf-c).
//!
//! rubash#1029 — `export -f` serialization stripped quotes from the
//! function body and leaked carrier bytes (\037 / \023), so a real shell
//! reading `BASH_FUNC_q%%` from the environment collapsed `"a  b"` to
//! `a b`. The serialized environment bytes are the contract: quotes and
//! spacing preserved, no marker bytes.
//! rubash#1067 — pathname expansion was skipped for the word list built
//! from unquoted `${arr[@]}` / `${arr[*]}` in a `for` list (quoted forms
//! must stay unglobbed).
//! rubash#1241 — an arithmetic expansion error (`$((1/0))`) in a for-list
//! or case word printed a SECOND, spurious `No such file or directory`
//! line; GNU prints exactly one diagnostic and the command fails rc=1.
//! rubash#1253 — under `set -e`, an arithmetic expansion error must
//! DISCARD the current command and CONTINUE the script (final rc 0, the
//! failed assignment does not land), instead of aborting the shell.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! in this environment (byte-level stdout/stderr/rc).
//!
//! DRIFT (found by backfill bf-c, 2026-10-25; FIXED by rubash#1518):
//! the three `set -e` #1253 pins below failed on master — the shell
//! aborted with rc=1 on the arithmetic error instead of discarding the
//! command and continuing. They are live again; the no-errexit control
//! stays live and passes.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("rubash-bfc-glob-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Extract the (possibly multi-line) `BASH_FUNC_<name>%%=` entry from
/// `env` output: from its header line through the closing `}` line.
fn bash_func_entry(env_out: &str, name: &str) -> String {
    let header = format!("BASH_FUNC_{name}%%=");
    let start = env_out.find(&header).expect("BASH_FUNC entry present");
    let rest = &env_out[start..];
    let end = rest.find("\n}\n").expect("entry closing brace line");
    rest[..end + 3].to_string()
}

#[test]
fn issue_1029_export_f_serialization_preserves_quotes() {
    let (stdout, _stderr, code) =
        run_c("q() { x=\"a  b\"; echo \"[$x]\"; }; p() { echo \"$1\"; }; export -f q p; env");
    assert_eq!(code, Some(0));
    let q = bash_func_entry(&stdout, "q");
    assert!(q.contains("x=\"a  b\""), "serialized q: {q:?}");
    assert!(q.contains("echo \"[$x]\""), "serialized q: {q:?}");
    let p = bash_func_entry(&stdout, "p");
    assert!(p.contains("echo \"$1\""), "serialized p: {p:?}");
    // No carrier/marker bytes may leak into the environment form.
    for entry in [&q, &p] {
        assert!(!entry.contains('\u{1f}'), "US marker leaked: {entry:?}");
        assert!(!entry.contains('\u{13}'), "DC3 marker leaked: {entry:?}");
    }
}

#[test]
fn issue_1029_exported_function_round_trips_to_child_shell() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args([
            "-c",
            "q() { x=\"a  b\"; echo \"[$x]\"; }; export -f q; \"$RUBASH_SELF\" -c 'q'",
        ])
        .env("RUBASH_SELF", env!("CARGO_BIN_EXE_rubash"))
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout, "[a  b]\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn issue_1067_for_list_globs_unquoted_array_expansions() {
    let dir = scratch("glob");
    std::fs::write(dir.join("a.sh"), "").expect("touch a.sh");
    std::fs::write(dir.join("b.sh"), "").expect("touch b.sh");
    let script = "arr=('*.sh' 'x')\nfor i in ${arr[@]}; do echo \"[$i]\"; done\nfor i in \"${arr[@]}\"; do echo \"q[$i]\"; done\nfor i in ${arr[*]}; do echo \"s[$i]\"; done\n";
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout,
        "[a.sh]\n[b.sh]\n[x]\nq[*.sh]\nq[x]\ns[a.sh]\ns[b.sh]\ns[x]\n"
    );
    assert_eq!(output.status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue_1241_arith_error_in_for_list_prints_one_diagnostic() {
    let (stdout, stderr, code) =
        run_script("for i in $((1/0)) 2 3; do echo \"i=$i\"; done\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(stderr.lines().count(), 1, "stderr: {stderr}");
    assert!(stderr.contains("division by 0"), "stderr: {stderr}");
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1241_arith_error_in_case_word_prints_one_diagnostic() {
    let (stdout, stderr, code) = run_script("case $((1/0)) in x) echo x;; esac\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(stderr.lines().count(), 1, "stderr: {stderr}");
    assert!(stderr.contains("division by 0"), "stderr: {stderr}");
    assert!(
        !stderr.contains("No such file or directory"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1253_errexit_arith_error_discards_assignment_and_continues() {
    let (stdout, stderr, code) = run_script("set -e\nb=$((1/0))\necho AFTER\n");
    assert_eq!(stdout, "AFTER\n", "stderr: {stderr}");
    assert!(stderr.contains("division by 0"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1253_errexit_arith_error_discards_echo_and_continues() {
    let (stdout, stderr, code) = run_script("set -e\necho $((1/0))\necho AFTER\n");
    assert_eq!(stdout, "AFTER\n", "stderr: {stderr}");
    assert!(stderr.contains("division by 0"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1253_failed_assignment_does_not_land() {
    let (stdout, stderr, code) = run_script("set -e\nb=7\nb=$((1/0))\necho \"b=$b\"\n");
    assert_eq!(stdout, "b=7\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1253_without_errexit_arith_error_continues_too() {
    // Control: no `set -e` — the error is reported and the script goes on.
    let (stdout, stderr, code) = run_script("echo $((1/0))\necho REACHED\n");
    assert_eq!(stdout, "REACHED\n", "stderr: {stderr}");
    assert!(stderr.contains("division by 0"), "stderr: {stderr}");
    assert_eq!(code, Some(0));
}
