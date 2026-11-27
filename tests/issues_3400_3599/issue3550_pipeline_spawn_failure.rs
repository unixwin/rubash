//! Regression pins for rubash#3550 — a pipeline member whose execve
//! fails (here: a stale `hash` entry naming a deleted PATH hit) must
//! keep the pipeline's GNU shape. GNU runs every pipeline element
//! inside its own fork (execute_cmd.c:2620 execute_pipeline ->
//! execute_simple_command -> execute_disk_command); the child reports
//! shell_execve's errno through the element's own fd 2 — naming the
//! resolved PATH spelling (execute_cmd.c:6143) — and exits 127
//! (ENOENT → EX_NOTFOUND, else 126; execute_cmd.c:6126-6159).
//! PIPESTATUS therefore keeps the dead member's slot instead of
//! collapsing into a bare-word diagnostic with status 0, and the
//! element's own `2>` redirect still applies (redir.c:237
//! do_redirections runs in the fork before execve).
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same probe
//! scripts under LC_ALL=C.UTF-8.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3550-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[derive(Debug)]
struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env_remove("BASH_ENV")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

/// `bindir/mycmd` — `hash mycmd` seeds the table entry, the script then
/// deletes the file so the recorded PATH spelling execve-refuses with
/// ENOENT.
fn fixture(tag: &str) -> PathBuf {
    let root = scratch(tag);
    let bindir = root.join("bindir");
    std::fs::create_dir_all(&bindir).expect("bindir");
    let mycmd = bindir.join("mycmd");
    std::fs::write(&mycmd, "echo mycmd-ok\n").expect("mycmd");
    let mut perms = std::fs::metadata(&mycmd).expect("mycmd meta").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&mycmd, perms).expect("chmod mycmd");
    root
}

/// The issue's verbatim repro: a stale-hash member in first position —
/// GNU prints the resolved spelling on stderr and PIPESTATUS is `127 0`.
#[test]
fn stale_hash_first_member_keeps_pipestatus_shape() {
    let root = fixture("first");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
mycmd | cat
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 5: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(out.stdout, "rc=127 0\n", "stdout: {out:?}");
    assert_eq!(out.rc, 0);
}

/// The stale member in last position: its fd-2 diagnostic surfaces and
/// PIPESTATUS is `0 127` (the upstream `cat` ran fine).
#[test]
fn stale_hash_last_member_keeps_pipestatus_shape() {
    let root = fixture("last");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
cat </dev/null | mycmd
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 5: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(out.stdout, "rc=0 127\n", "stdout: {out:?}");
    assert_eq!(out.rc, 0);
}

/// The dead member mid-pipe: `true | mycmd | cat` — GNU keeps three
/// PIPESTATUS slots and the dead fork's closed pipe end feeds `cat`
/// an immediate EOF.
#[test]
fn stale_hash_middle_member_reads_eof_downstream() {
    let root = fixture("mid");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
true | mycmd | cat
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 5: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(out.stdout, "rc=0 127 0\n", "stdout: {out:?}");
    assert_eq!(out.rc, 0);
}

/// The element's own `2>file` applies before the lookup fails
/// (redir.c:237 in the element's fork): the diagnostic lands in the
/// file and the pipeline's stderr stays empty.
#[test]
fn member_stderr_redirect_claims_the_diagnostic() {
    let root = fixture("redir");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
mycmd 2>err | cat
echo "rc=${PIPESTATUS[@]}"
cat err
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected = format!(
        "rc=127 0\ncase.sh: line 5: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stdout, expected, "stdout: {out:?}");
    assert_eq!(out.stderr, "", "stderr: {out:?}");
    assert_eq!(out.rc, 0);
}

/// `mycmd 2>&1 | cat` — the element's fd-2 is dup'd onto its pipe, so
/// the dead fork's diagnostic flows downstream as cat's input.
#[test]
fn member_stderr_merge_feeds_the_downstream() {
    let root = fixture("merge");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
mycmd 2>&1 | cat
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected = format!(
        "case.sh: line 5: {}: No such file or directory\nrc=127 0\n",
        resolved.display()
    );
    assert_eq!(out.stdout, expected, "stdout: {out:?}");
    assert_eq!(out.stderr, "", "stderr: {out:?}");
    assert_eq!(out.rc, 0);
}

/// The shell-writer path (`{ :; } | mycmd` — a compound producer
/// streaming in-process beside spawned consumers): the dead consumer
/// keeps the same dead-fork contract — resolved-name diagnostic and
/// `0 127` in PIPESTATUS.
#[test]
fn shell_writer_pipeline_dead_consumer_keeps_shape() {
    let root = fixture("writer");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
{ :; } | mycmd
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 5: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(out.stdout, "rc=0 127\n", "stdout: {out:?}");
    assert_eq!(out.rc, 0);
}

/// The streaming-sandwich path (one shell function amid externals):
/// the dead first member writes its diagnostic on the ambient fd 2
/// and keeps its PIPESTATUS slot.
#[test]
fn sandwich_pipeline_dead_first_member_keeps_shape() {
    let root = fixture("sandwich-first");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
f(){ cat; }
mycmd | f | cat
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 6: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(out.stdout, "rc=127 0 0\n", "stdout: {out:?}");
    assert_eq!(out.rc, 0);
}

/// Sandwich with the dead member last: its diagnostic rides the
/// captured fd-2 relay and PIPESTATUS is `0 0 127`.
#[test]
fn sandwich_pipeline_dead_last_member_keeps_shape() {
    let root = fixture("sandwich-last");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
f(){ cat; }
true | f | mycmd
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 6: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(out.stdout, "rc=0 0 127\n", "stdout: {out:?}");
    assert_eq!(out.rc, 0);
}

/// With the ambient `2>&1` binding the dead member's diagnostic is
/// part of the merged stream — GNU writes it to fd 1's endpoint, so
/// it lands on stdout ahead of the status echo.
#[test]
fn ambient_stderr_merge_relays_the_diagnostic_on_stdout() {
    let root = fixture("ambient-merge");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
exec 2>&1
{ :; } | mycmd | cat
echo "rc=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected = format!(
        "case.sh: line 6: {}: No such file or directory\nrc=0 127 0\n",
        resolved.display()
    );
    assert_eq!(out.stdout, expected, "stdout: {out:?}");
    assert_eq!(out.stderr, "", "stderr: {out:?}");
    assert_eq!(out.rc, 0);
}

/// This lane's addition (queue-auto-dispatch 2026-11-18): the issue's
/// plain-command line — a stale hash outside a pipeline already
/// reports the hashed path with 127. The pipeline faces above must
/// match this control, not the bare-word/0 shape the issue recorded.
#[test]
fn stale_hash_plain_command_reports_hashed_path_with_127() {
    let root = fixture("plain");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
mycmd
echo "plain rc=$?"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 5: {}: No such file or directory\n",
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(out.stdout, "plain rc=127\n", "stdout: {out:?}");
    assert_eq!(out.rc, 0);
}

/// This lane's addition: an explicit slot-count lock — PIPESTATUS
/// holds one slot per member even when the dead member is first or
/// last (the issue's collapse to a single `0`).
#[test]
fn stale_hash_pipestatus_slot_count_survives_dead_member() {
    let root = fixture("slots");
    let body = r#"cd "$(dirname "$0")"
PATH="$PWD/bindir:/usr/bin:/bin"
hash mycmd
rm bindir/mycmd
mycmd | cat; echo "first n=${#PIPESTATUS[@]} all=${PIPESTATUS[@]}"
cat </dev/null | mycmd; echo "last n=${#PIPESTATUS[@]} all=${PIPESTATUS[@]}"
"#;
    let out = run(&root, body);
    let resolved = root.join("bindir/mycmd");
    let expected_diag = format!(
        "case.sh: line 5: {}: No such file or directory\ncase.sh: line 6: {}: No such file or directory\n",
        resolved.display(),
        resolved.display()
    );
    assert_eq!(out.stderr, expected_diag, "stderr: {out:?}");
    assert_eq!(
        out.stdout, "first n=2 all=127 0\nlast n=2 all=0 127\n",
        "stdout: {out:?}"
    );
    assert_eq!(out.rc, 0);
}
