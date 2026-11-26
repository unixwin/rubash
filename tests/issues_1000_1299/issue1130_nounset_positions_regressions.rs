//! rubash#1130: `set -u` must fire in every expansion position —
//! rubash silently skipped four of them.
//!
//! GNU anchor: `subst.c` reports `unbound variable` for an unset
//! expansion under nounset wherever the expansion happens: a
//! for-list, a `case` subject, a `[[ ]]` word and a herestring all
//! diagnose (fatally outside a pipeline, non-fatally inside one).
//! Simple-command words, assignment RHS, array literals, `(( ))`,
//! `[ ]`, redirect targets and `${v:-d}` already agreed.
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1130-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue1130 scratch dir");
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

/// (a) for-list: the loop never runs, the script dies with rc 1.
#[test]
fn nounset_fires_in_for_list() {
    let (code, stdout, stderr) =
        run_script("set -u\nfor s in $q hello; do echo \"S=$s\"; done\necho \"END rc=$?\"\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(stderr.contains("q: unbound variable"), "stderr: {stderr}");
}

/// (b) case subject: no branch is taken, rc 1.
#[test]
fn nounset_fires_in_case_subject() {
    let (code, stdout, stderr) =
        run_script("set -u\ncase \"$q\" in x) echo x;; *) echo other;; esac\necho \"END rc=$?\"\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(stderr.contains("q: unbound variable"), "stderr: {stderr}");
}

/// (c) `[[ ]]` word: neither side of the `&&`/`||` runs, rc 1.
#[test]
fn nounset_fires_in_conditional_word() {
    let (code, stdout, stderr) =
        run_script("set -u\n[[ $v != *o* ]] && echo yes || echo no\necho \"END rc=$?\"\n");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(stderr.contains("v: unbound variable"), "stderr: {stderr}");
}

/// (d) herestring in a pipeline: the diagnostic fires but is
/// non-fatal there — `wc` still runs and counts 0 words, rc 0.
/// (rubash printed 1 word and no diagnostic.)
#[test]
#[cfg(unix)]
fn nounset_fires_in_herestring() {
    let (code, stdout, stderr) =
        run_script("set -u\ncat <<< prefix$p | wc -w\necho \"END rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "0\nEND rc=0\n");
    assert!(stderr.contains("p: unbound variable"), "stderr: {stderr}");
}

/// Control that already agreed: `${q:-def}` is the sanctioned
/// escape hatch and stays silent under `set -u`.
#[test]
fn nounset_default_expansion_stays_silent() {
    let (code, stdout, stderr) = run_script(
        "set -u\necho \"D=${q:-def}\"\n[ -n \"${v2:-}\" ] && echo nonempty || echo empty-ok\necho \"END rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "D=def\nempty-ok\nEND rc=0\n");
    assert_eq!(stderr, "");
}
