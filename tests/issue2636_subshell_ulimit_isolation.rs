//! Issue rubash#2636: a `ulimit` change inside a subshell (or command
//! substitution, or pipeline element) leaked back into the parent
//! shell — `( ulimit -n 777; ulimit -n ); ulimit -n` printed the
//! child's 777 for the outer query too.
//!
//! GNU semantics: every one of those contexts is a forked child
//! (execute_cmd.c:1576 execute_in_subshell; subst.c:7143
//! command_substitute; execute_cmd.c:2620 execute_pipeline), and the
//! `ulimit` builtin drives the PROCESS rlimits (builtins/ulimit.def
//! set_limit -> setrlimit, ulimit.def:632), so the child's setrlimit
//! dies with the child and the parent's limits never move. A `{ }`
//! group is the current shell and its change persists in both.
//!
//! Root cause (same family as rubash#1907's umask leak): rubash's
//! fork boundaries are flat — the region runs against this same
//! process, so its setrlimit re-limited the parent process itself,
//! and no boundary snapshotted/restored the process rlimit set. Fix:
//! every flat fork boundary that already brackets the process umask
//! (clone_for_child_save/restore_flat_subshell, the command-
//! substitution interior snapshot, the pipeline-stage bracket, the
//! in-process child-script restore) now also snapshots the process
//! rlimits and re-applies them when the region ends.
//!
//! Cell shapes: most cells set the SOFT limit only (`-Sn`), which is
//! restorable for any user. Two cells use the plain/both form and
//! the hard limit: restoring a lowered HARD limit means raising it
//! back, which the kernel allows only with CAP_SYS_RESOURCE, so
//! those cells run only where a probe shows a hard limit can
//! actually be raised back (uid 0 inside a user namespace fails
//! that probe — GNU bash itself cannot raise there either); the
//! soft cells pin the same boundary mechanism everywhere else.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-11-08). Unix-only:
//! Windows has no process rlimits (the emulated table lives in the
//! env map, which already rides the state save/restore).

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

/// Whether this environment lets a process RAISE a hard limit back
/// after lowering it (the restore direction for a hard-side set).
/// uid 0 is NOT sufficient evidence — root inside a user namespace
/// still gets EPERM (verified on the captain's VM: GNU bash itself
/// cannot raise `ulimit -Hn` back there) — so probe the operation
/// itself in a throwaway shell: lower the hard limit by one, then
/// raise it back; both steps must succeed.
fn hard_limit_is_raisable() -> bool {
    let (hard, _, _) = rubash("ulimit -Hn");
    let hard: u64 = match hard.trim().parse() {
        Ok(value) if value >= 1024 => value,
        _ => return false,
    };
    let probe = format!("ulimit -Hn {} && ulimit -Hn {}", hard - 1, hard);
    let (_, stderr, status) = rubash(&probe);
    status == Some(0) && stderr.is_empty()
}

/// The issue's exact seed: the parent's limit survives the subshell.
/// (Raisable-gated: the plain set form lowers the hard limit too,
/// and the boundary restore must raise it back — only possible
/// where hard limits can be raised at all; see REPORT.)
#[test]
fn subshell_ulimit_seed_shape_does_not_leak() {
    if !hard_limit_is_raisable() {
        return;
    }
    let (stdout, stderr, _) = rubash("ulimit -n 512\n( ulimit -n 256; ulimit -n )\nulimit -n");
    assert_clean(&stderr);
    assert_eq!(stdout, "256\n512\n");
}

/// Soft-limit form of the seed — the same boundary, restorable for
/// any user.
#[test]
fn subshell_ulimit_soft_does_not_leak_to_parent() {
    let (stdout, stderr, _) = rubash("ulimit -Sn 512\n( ulimit -Sn 256; ulimit -Sn )\nulimit -Sn");
    assert_clean(&stderr);
    assert_eq!(stdout, "256\n512\n");
}

/// Command substitution is the same fork boundary, set form included.
#[test]
fn command_substitution_ulimit_does_not_leak() {
    let (stdout, stderr, _) =
        rubash("ulimit -Sn 512\nx=$(ulimit -Sn 256; ulimit -Sn)\necho \"comsub=$x\"\nulimit -Sn");
    assert_clean(&stderr);
    assert_eq!(stdout, "comsub=256\n512\n");
}

/// The single-word `$(ulimit ...)` fast path runs the real builtin
/// engine on the shared executor — its set must die at the boundary
/// too.
#[test]
fn command_substitution_fast_path_ulimit_set_does_not_leak() {
    let (stdout, stderr, _) = rubash("ulimit -Sn 512\nx=$(ulimit -Sn 256)\nulimit -Sn");
    assert_clean(&stderr);
    assert_eq!(stdout, "512\n");
}

/// A pipeline element is a forked child in GNU: a builtin element's
/// setrlimit must not reach the parent.
#[test]
fn pipeline_element_ulimit_does_not_leak() {
    let (stdout, stderr, _) = rubash("ulimit -Sn 512\nulimit -Sn 256 | cat\nulimit -Sn");
    assert_clean(&stderr);
    assert_eq!(stdout, "512\n");
}

/// A subshell as a pipeline element restores at its own boundary.
#[test]
fn pipeline_subshell_ulimit_does_not_leak() {
    let (stdout, stderr, _) =
        rubash("ulimit -Sn 512\n( ulimit -Sn 256; ulimit -Sn ) | cat\nulimit -Sn");
    assert_clean(&stderr);
    assert_eq!(stdout, "256\n512\n");
}

/// Nested subshells restore level by level.
#[test]
fn nested_subshell_ulimit_restores_level_by_level() {
    let (stdout, stderr, _) =
        rubash("ulimit -Sn 512\n( ulimit -Sn 256; ( ulimit -Sn 128 ); ulimit -Sn )\nulimit -Sn");
    assert_clean(&stderr);
    assert_eq!(stdout, "256\n512\n");
}

/// A different resource (`-s`, stack) rides the same snapshot — the
/// fix covers the whole rlimit set, not just NOFILE.
#[test]
fn subshell_ulimit_other_resource_does_not_leak() {
    let (stdout, stderr, _) =
        rubash("ulimit -Ss 8192\n( ulimit -Ss 4096; ulimit -Ss )\nulimit -Ss");
    assert_clean(&stderr);
    assert_eq!(stdout, "4096\n8192\n");
}

/// The hard limit is process state too: lowering it inside a
/// subshell must not lower the parent's (the set is one-way for
/// non-root, so a leak would be unrecoverable for the parent).
/// Raisable-gated for the restore reason in the header.
#[test]
fn subshell_ulimit_hard_limit_does_not_leak() {
    if !hard_limit_is_raisable() {
        return;
    }
    // Read the inherited hard limit first; only run the cell when it
    // leaves room to lower by a visible step.
    let (hard, _, _) = rubash("ulimit -Hn");
    let hard: u64 = match hard.trim().parse() {
        Ok(value) if value >= 1024 => value,
        _ => return,
    };
    let lowered = hard - 512;
    let script = format!("( ulimit -Hn {lowered}; ulimit -Hn )\nulimit -Hn");
    let (stdout, stderr, _) = rubash(&script);
    assert_clean(&stderr);
    assert_eq!(stdout, format!("{lowered}\n{hard}\n"));
}

/// An in-process exec-model child (`${THIS_SH} script`) is a process
/// boundary too: the child script's ulimit dies when it returns.
#[test]
fn exec_model_child_script_ulimit_does_not_leak() {
    let dir = std::env::temp_dir().join(format!("rubash-issue2636-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("child.sh"), "ulimit -Sn 256\n").expect("write child script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("ulimit -Sn 512\n\"${THIS_SH}\" child.sh\nulimit -Sn")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.stderr.is_empty(),
        "stderr not empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "512\n");
}

/// Control: a `{ }` group is NOT a fork boundary — its ulimit change
/// must persist (both shells agree; pinning so the fix cannot
/// over-isolate).
#[test]
fn brace_group_ulimit_persists() {
    let (stdout, stderr, _) = rubash("ulimit -Sn 512\n{ ulimit -Sn 256; }\nulimit -Sn");
    assert_clean(&stderr);
    assert_eq!(stdout, "256\n");
}
