//! rubash#1115: entering a subshell reseeds `RANDOM` — the child
//! must not replay the parent's post-seed sequence.
//!
//! GNU anchor: `variables.c` (dynamic RANDOM) seeds the generator
//! per shell; subshell creation (`execute_cmd.c` / `subst.c` for
//! `( )` and `$( )`) reseeds from time/pid, so the child's draws
//! diverge from the parent's stream while the parent's own stream
//! continues undisturbed. rubash cloned the parent's generator
//! state, so `(echo $RANDOM $RANDOM)` after `RANDOM=42` repeated
//! the parent's `17772 26794` bit-for-bit.
//!
//! Expectations verified against GNU bash 5.3.20: the top-level
//! post-seed stream is pinned exactly (both shells agree on it);
//! subshell streams are pinned by the reseed invariant — never
//! equal to the parent's stream.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1115-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1115 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The shared baseline: after `RANDOM=42` the top-level stream is
/// exactly GNU's (both shells agreed even before the fix).
#[test]
fn top_level_post_seed_stream_matches_gnu() {
    let (code, stdout, _) = run_script("RANDOM=42; echo \"top:$RANDOM $RANDOM $RANDOM\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "top:17772 26794 1435\n");
}

/// The reported case: a `( )` subshell draws a reseeded stream.
#[test]
fn paren_subshell_reseeds_random() {
    let (_, stdout, _) = run_script("RANDOM=42; (echo \"sub:$RANDOM $RANDOM\")\n");
    assert!(stdout.starts_with("sub:"), "stdout: {stdout}");
    assert_ne!(stdout, "sub:17772 26794\n");
}

/// The `$( )` form of the same report.
#[test]
fn command_substitution_reseeds_random() {
    let (_, stdout, _) = run_script("RANDOM=42; s=$(echo \"cs:$RANDOM $RANDOM\"); echo \"$s\"\n");
    assert!(stdout.starts_with("cs:"), "stdout: {stdout}");
    assert_ne!(stdout, "cs:17772 26794\n");
}

/// Reseeding is one-way: the parent keeps its own stream — the
/// first draw stays 17772 and the subshell pair equals neither the
/// parent's opening pair nor its continuation pair.
#[test]
fn subshell_reseed_leaves_parent_stream_untouched() {
    let (_, stdout, _) =
        run_script("RANDOM=42; x=$RANDOM; (echo \"sub2:$RANDOM $RANDOM\"); echo \"after:$x\"\n");
    let mut lines = stdout.lines();
    let sub2 = lines.next().unwrap_or("");
    let after = lines.next().unwrap_or("");
    assert_eq!(after, "after:17772");
    assert!(sub2.starts_with("sub2:"), "stdout: {stdout}");
    assert_ne!(sub2, "sub2:17772 26794");
    assert_ne!(sub2, "sub2:26794 1435");
}
