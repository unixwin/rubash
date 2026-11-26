//! rubash#3906: a NON-exec simple command (`:` builtin — the funnel
//! face, `apply_dynamic_fd_var_redirects` in src/executor/trap_exec.rs)
//! must allocate a `{var}` heredoc / here-string descriptor at that
//! entry's list position in GNU's single ordered walk (redir.c:237
//! do_redirections), not after the plain list. GNU's heredoc arm
//! materializes the body with here_document_to_fd (redir.c:1065-1072)
//! and, for REDIR_VARASSIGN, dups it with fcntl F_DUPFD from
//! SHELL_FD_BASE (redir.c:1082) before redir_varassign binds the
//! variable (redir.c:1113) — so a later entry's undo-save for an
//! already-open redirector (add_undo_redirect, redir.c:1091, F_DUPFD
//! from SHELL_FD_BASE at redir.c:1303-1308) claims the slot AFTER the
//! var's. The exec-only face of the same defect was fixed by PR #3898
//! (rubash#3893); this nail covers the funnel face, which is a
//! different function and was left untouched there.
//!
//! Every expectation below was byte-verified against GNU bash 5.3.20
//! script-file probes (the same script text, including the
//! `ulimit -n 16` prelude, which mirrors the issue3893 pin harness
//! so fds 13-15 stay below the soft limit). After the `:` command the
//! plain fd 14 is restored (closed again) while the `{var}` fd stays
//! bound (varredir_close off), so the nail reads the body back
//! through `$v` only.

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3906-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

#[test]
fn issue3906_funnel_var_heredoc_allocates_at_list_position() {
    // The issue's verbatim shape through the funnel: `14<<X` opens fd
    // 14, so the later `14<<B` registers an undo-save; GNU allocates
    // `{v}` at its own list position (v=10) and the save lands on 11.
    // A tail-only bind gives v=11 (the save takes 10 first).
    let (stdout, stderr, code, _dir) = run_script(
        "list-position",
        "ulimit -n 16\n\
         : 14<<X {v}<<A 14<<B\n\
         x\n\
         X\n\
         va\n\
         A\n\
         nb\n\
         B\n\
         echo \"v=$v\"\n\
         read -u \"$v\" p; echo \"p=$p v=$v\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "v=10\np=va v=10\n", "stdout={stdout:?}");
}

#[test]
fn issue3906_funnel_var_heredoc_after_leftover_open_fd() {
    // A prior `exec` left fds 14/15 open, so the second list's `14<<B`
    // undo-save again must follow the var's allocation, not precede it.
    let (stdout, stderr, code, _dir) = run_script(
        "leftover-open",
        "ulimit -n 16\n\
         exec 14<<P 15<<Q\n\
         p14\n\
         P\n\
         q15\n\
         Q\n\
         : {v}<<A 14<<B\n\
         va\n\
         A\n\
         nb\n\
         B\n\
         echo \"v=$v\"\n\
         read -u \"$v\" p; echo \"p=$p v=$v\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "v=10\np=va v=10\n", "stdout={stdout:?}");
}

#[test]
fn issue3906_funnel_var_here_string_allocates_at_list_position() {
    // The same ordering through the here-string kind (`{v}<<<word`
    // shares the r_reading_string arm): the save for the later
    // `14<<<z` must not jump ahead of the var's allocation.
    let (stdout, stderr, code, _dir) = run_script(
        "here-string",
        "ulimit -n 16\n\
         : 14<<<x {v}<<<y 14<<<z\n\
         read -u \"$v\" p; echo \"p=$p v=$v\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "p=y v=10\n", "stdout={stdout:?}");
}

#[test]
fn issue3906_funnel_var_heredoc_exact_value_without_leading_open() {
    // Control: `{v}<<A 14<<B` with fd 14 still closed at the `14<<B`
    // position registers no undo-save, so GNU gives exactly v=10.
    let (stdout, stderr, code, _dir) = run_script(
        "exact-no-save",
        "ulimit -n 16\n\
         : {v}<<A 14<<B\n\
         va\n\
         A\n\
         nb\n\
         B\n\
         echo \"v=$v\"\n\
         read -u \"$v\" p; echo \"p=$p v=$v\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "v=10\np=va v=10\n", "stdout={stdout:?}");
}

#[test]
fn issue3906_funnel_two_var_heredocs_with_numbered_rebind_between() {
    // Control: with fd 14 closed at the `14<<B` position there is no
    // undo-save to claim a slot, so the second var takes the very
    // next free descriptor (w=11, not 12) — the fix must not
    // over-claim slots for the later allocation.
    let (stdout, stderr, code, _dir) = run_script(
        "two-vars",
        "ulimit -n 16\n\
         : {v}<<A 14<<B {w}<<C\n\
         va\n\
         A\n\
         nb\n\
         B\n\
         wc\n\
         C\n\
         read -u \"$v\" p; read -u \"$w\" r; echo \"p=$p r=$r v=$v w=$w\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "p=va r=wc v=10 w=11\n", "stdout={stdout:?}");
}

#[test]
fn issue3906_funnel_var_heredoc_alone_gets_lowest_free() {
    // Control: a lone `{v}<<A` allocates the lowest free descriptor
    // at or above SHELL_FD_BASE (v=10).
    let (stdout, stderr, code, _dir) = run_script(
        "alone",
        "ulimit -n 16\n\
         : {v}<<A\n\
         va\n\
         A\n\
         read -u \"$v\" p; echo \"p=$p v=$v\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "p=va v=10\n", "stdout={stdout:?}");
}
