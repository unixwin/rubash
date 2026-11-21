//! rubash#4046 — unified `RX_UNDOABLE` gate for move-redirect `cannot duplicate fd`
//! diagnostics across simple and compound preflights (including respawn faces 2/3).
//!
//! GNU `redir.c:1169-1178` (`do_redirection_internal`):
//! `(flags & RX_UNDOABLE) && (ri == r_move_input || ri == r_move_output)` calls
//! `add_undo_redirect(redir_fd, r_close_this, -1)` (`redir.c:1296-1316`) only when
//! `fcntl(redirector, F_GETFD, 0) != -1`, which emits `redirection error: cannot duplicate fd`
//! before `redirection_error` (`redir.c:149-158`).
//!
//! `execute_command_internal` (`execute_cmd.c:854`) and `execute_builtin_or_function`
//! (`execute_cmd.c:5606`) run with `RX_UNDOABLE` — including inside respawn face 2
//! (subshell body) and face 3 (procsub body) — whereas forked contexts (`execute_in_subshell`
//! `execute_cmd.c:1761`, pipeline stages `execute_cmd.c:5522`, async commands
//! `execute_cmd.c:4551`, and external commands) run `RX_ACTIVE` only.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash4046-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let case = dir.join("case.sh");
    std::fs::write(&case, script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg(&case)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue4046_main_compound_move_open_redirector_emits_two_lines() {
    let (_stdout, stderr, code) = run("main_open", "exec 8>/dev/null\n{ :; } 8>&10-\nexec 8>&-\n");
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Bad file descriptor"),
        "expected cannot duplicate fd line, got: {stderr}"
    );
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "expected 10: Bad file descriptor line, got: {stderr}"
    );
}

#[test]
fn issue4046_main_compound_move_closed_redirector_emits_one_line() {
    let (_stdout, stderr, code) = run("main_closed", "{ :; } 8>&10-\n");
    assert_eq!(code, Some(1));
    assert!(
        !stderr.contains("cannot duplicate fd"),
        "closed redirector must not emit cannot duplicate fd, got: {stderr}"
    );
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "expected 10: Bad file descriptor line, got: {stderr}"
    );
}

#[test]
fn issue4046_async_body_compound_move_emits_two_lines() {
    let (_stdout, stderr, code) = run(
        "async_body_group",
        "exec 8>/dev/null\n{ { :; } 8>&10-; } &\nwait\nexec 8>&-\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Bad file descriptor"),
        "expected cannot duplicate fd line in async body group, got: {stderr}"
    );
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "expected 10: Bad file descriptor line, got: {stderr}"
    );
}

#[test]
fn issue4046_external_command_omits_cannot_duplicate_fd() {
    let (_stdout, stderr, code) = run("external_open", "exec 8>/dev/null\ncat 8>&10-\nexec 8>&-\n");
    assert_eq!(code, Some(0));
    assert!(
        !stderr.contains("cannot duplicate fd"),
        "external command runs RX_ACTIVE only, got: {stderr}"
    );
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor line, got: {stderr}"
    );
}

#[test]
fn issue4046_procsub_compound_move_emits_two_lines() {
    let (_stdout, stderr, code) = run(
        "procsub_group",
        "exec 8>/dev/null\ncat <( { :; } 8>&10- )\nexec 8>&-\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Bad file descriptor"),
        "expected cannot duplicate fd line in procsub group, got: {stderr}"
    );
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "expected 10: Bad file descriptor line, got: {stderr}"
    );
}

#[test]
fn issue4046_procsub_simple_builtin_move_emits_two_lines() {
    let (_stdout, stderr, code) = run(
        "procsub_simple",
        "exec 8>/dev/null\ncat <( : 8>&10- )\nexec 8>&-\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("redirection error: cannot duplicate fd: Bad file descriptor"),
        "expected cannot duplicate fd line in procsub builtin, got: {stderr}"
    );
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor line, got: {stderr}"
    );
}

#[test]
fn issue4046_main_simple_closed_redirector_omits_cannot_duplicate_fd() {
    let (_stdout, stderr, code) = run("simple_closed", ": 8>&10-\n");
    assert_eq!(code, Some(1));
    assert!(
        !stderr.contains("cannot duplicate fd"),
        "closed redirector on simple builtin must not emit cannot duplicate fd, got: {stderr}"
    );
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected Bad file descriptor line, got: {stderr}"
    );
}

#[test]
fn issue4046_pipeline_compound_omits_cannot_duplicate_fd() {
    let (_stdout, stderr, code) = run(
        "pipeline_group",
        "exec 8>/dev/null\n{ :; } 8>&10- | cat\nexec 8>&-\n",
    );
    assert_eq!(code, Some(0));
    assert!(
        !stderr.contains("cannot duplicate fd"),
        "pipeline compound runs RX_ACTIVE only, got: {stderr}"
    );
    assert!(
        stderr.contains("10: Bad file descriptor"),
        "expected 10: Bad file descriptor line, got: {stderr}"
    );
}
