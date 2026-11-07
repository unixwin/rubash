//! rubash#2957 (Stage-2 ④ Phase 3) — GNU builtins/set.def's
//! apply loop has an explicit `?` arm (set.def:724-728): a `?`
//! character reached in the apply walk (reachable only hidden
//! inside an inline `-o` operand, since the pre-scan's
//! internal_getopt consumes a standalone `?` itself) prints
//! `builtin_usage()` and returns EXECUTION_SUCCESS, with NO
//! invalid-option line. rubash's apply loop had no `?` arm, so
//! `set -o?` fell into the change_flag FLAG_ERROR path
//! (set.def:763-771 shape): extra `-?: invalid option` line and
//! status 1. Expectations are byte-exact GNU bash 5.3.20 outputs
//! (Phase 3 E1 differential).
//!
//! The guard cell pins the neighbouring FLAG_ERROR path: `set
//! -ol` still reports `-l: invalid option` + usage with status 1
//! (the `l` hidden in the inline operand is genuinely unknown to
//! change_flag — flags.c:188-190 keeps it out of shell_flags[]).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2957_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

const EXPECTED_LISTING: &str = "allexport      \toff\nbraceexpand    \ton\nemacs          \toff\nerrexit        \toff\nerrtrace       \toff\nfunctrace      \toff\nhashall        \ton\nhistexpand     \toff\nhistory        \toff\nignoreeof      \toff\ninteractive-comments\ton\nkeyword        \toff\nmonitor        \toff\nnoclobber      \toff\nnoexec         \toff\nnoglob         \toff\nnolog          \toff\nnotify         \toff\nnounset        \toff\nonecmd         \toff\nphysical       \toff\npipefail       \toff\nposix          \toff\nprivileged     \toff\nverbose        \toff\nvi             \toff\nxtrace         \toff\n";
const SET_USAGE: &str =
    "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]\n";

#[test]
fn set_o_question_prints_listing_and_usage_with_success() {
    let run = run_script("set -o?\necho \"rc=$? flags=$-\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{EXPECTED_LISTING}rc=0 flags=hB\n"),
        "stdout: {}",
        run.stdout
    );
    assert_eq!(run.stderr, SET_USAGE, "stderr: {}", run.stderr);
    let _ = run.dir;
}

#[test]
fn set_o_hidden_l_still_takes_the_flag_error_path() {
    let run = run_script("set -ol\necho \"rc=$? flags=$-\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout,
        format!("{EXPECTED_LISTING}rc=1 flags=hB\n"),
        "stdout: {}",
        run.stdout
    );
    assert_eq!(
        run.stderr,
        format!("case.sh: line 1: set: -l: invalid option\n{SET_USAGE}"),
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}
