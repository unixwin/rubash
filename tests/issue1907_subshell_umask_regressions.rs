//! Issue rubash#1907: a `umask` set inside a `( )` subshell leaked
//! into the parent shell — after `(umask 077)`, the parent's `umask`
//! still printed 0077 and files it created came out 0600 instead of
//! the 0660/0644 the parent's mask implies.
//!
//! GNU semantics: a subshell runs in a forked child
//! (execute_cmd.c:1576 execute_in_subshell), so the child's process
//! umask dies with the child and the parent keeps its own. The
//! `umask` builtin itself applies the mask to the PROCESS
//! (builtins/umask.def umask_builtin), which is what file creation
//! and spawned children observe.
//!
//! Root cause: rubash's flat subshell boundary
//! (`clone_for_child_save` / `restore_flat_subshell`) saved and
//! restored the shell STATE — including the `__RUBASH_UMASK` env
//! mirror — but never the PROCESS mask the builtin had applied, and
//! the parent's mask reader falls back to the process mask when the
//! mirror is absent, so the child's value became the parent's. Fix:
//! the save point snapshots the process umask into the saved state
//! (`fork_saved_process_umask`) and the restore point re-applies it
//! after the region's own work is done.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-10-28). Unix-only:
//! Windows has no process umask.

#![cfg(unix)]

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The issue's exact seed: the parent's mask survives the subshell.
#[test]
#[cfg(unix)]
fn subshell_umask_does_not_leak_to_parent() {
    let (stdout, stderr, _) = rubash("umask 022\n(umask 077)\numask");
    assert_clean(&stderr);
    assert_eq!(stdout, "0022\n");
}

/// The child still sees (and uses) its own mask while it runs.
#[test]
#[cfg(unix)]
fn subshell_sees_its_own_umask() {
    let (stdout, stderr, _) = rubash("umask 022\n(umask 077; umask)\numask");
    assert_clean(&stderr);
    assert_eq!(stdout, "0077\n0022\n");
}

/// Nested subshells restore level by level: the inner child's mask
/// dies at the inner boundary, the outer's at the outer boundary.
#[test]
#[cfg(unix)]
fn nested_subshell_umask_restores_level_by_level() {
    let (stdout, stderr, _) = rubash("umask 022\n( umask 077; (umask 000); umask )\numask");
    assert_clean(&stderr);
    assert_eq!(stdout, "0077\n0022\n");
}

/// The issue's file-creation shape: a file created by the PARENT
/// after the subshell carries the parent's mask (0644 under 022),
/// while a file created INSIDE the subshell carries the child's
/// (0600 under 077).
#[test]
#[cfg(unix)]
fn file_creation_after_subshell_uses_parent_mask() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("rubash-issue1907-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = format!(
        "cd {} && umask 022 && (umask 077) && touch after.txt && (umask 077; touch inside.txt)",
        dir.display()
    );
    let (_, stderr, status) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(status, Some(0));

    let after = std::fs::metadata(dir.join("after.txt"))
        .expect("after.txt exists")
        .permissions()
        .mode()
        & 0o777;
    let inside = std::fs::metadata(dir.join("inside.txt"))
        .expect("inside.txt exists")
        .permissions()
        .mode()
        & 0o777;
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(after, 0o644, "parent-created file must use parent mask");
    assert_eq!(inside, 0o600, "subshell-created file must use child mask");
}

/// A subshell in a pipeline is the same fork boundary.
#[test]
#[cfg(unix)]
fn pipeline_subshell_umask_does_not_leak() {
    let (stdout, stderr, _) = rubash("umask 022\n(umask 077) | cat\numask");
    assert_clean(&stderr);
    assert_eq!(stdout, "0022\n");
}

/// A subshell nested in a command substitution restores at its own
/// boundary, before the substitution result is even consumed.
#[test]
#[cfg(unix)]
fn subshell_inside_command_substitution_restores() {
    let (stdout, stderr, _) =
        rubash("umask 022\ny=$( (umask 077); echo done )\necho \"$y\"\numask");
    assert_clean(&stderr);
    assert_eq!(stdout, "done\n0022\n");
}

/// The general command-substitution boundary is the same flat fork:
/// a `umask` directly in a multi-command `$( )` body must not leak
/// either (the single-command `$(umask)` fast path already bracketed
/// the process mask; the general path's interior snapshot now does).
#[test]
#[cfg(unix)]
fn command_substitution_umask_does_not_leak() {
    let (stdout, stderr, _) =
        rubash("umask 022\nx=$(umask 077; echo done)\necho \"$x\"\numask");
    assert_clean(&stderr);
    assert_eq!(stdout, "done\n0022\n");
}

/// An in-process exec-model child (`${THIS_SH} script`) is a process
/// boundary too: the child script's `umask` dies when it returns.
#[test]
#[cfg(unix)]
fn exec_model_child_script_umask_does_not_leak() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1907-this-sh-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("child.sh"), "umask 077\n").expect("write child script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("umask 022\n\"${THIS_SH}\" child.sh\numask")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.stderr.is_empty(),
        "stderr not empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0022\n");
}

/// Control: a `{ }` group is NOT a fork boundary — its umask change
/// must persist (both shells agree; pinning so the fix cannot
/// over-isolate).
#[test]
#[cfg(unix)]
fn brace_group_umask_persists() {
    let (stdout, stderr, _) = rubash("umask 022\n{ umask 077; }\numask");
    assert_clean(&stderr);
    assert_eq!(stdout, "0077\n");
}
