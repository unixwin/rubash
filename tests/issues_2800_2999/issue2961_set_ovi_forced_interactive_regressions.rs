//! rubash#2961 (Stage-2 ④ Phase 3 E1 residual) — `set -ovi` must
//! behave as GNU's apply loop does, character by character
//! (builtins/set.def:672-772): the pre-scan's internal_getopt
//! swallows `vi` as `o`'s inline operand, the apply loop reaches
//! `o` with no following word and prints the `-o` listing, then
//! CONTINUES walking the word's characters — `v` flips verbose and
//! `i` flips forced_interactive through change_flag (flags.c:226),
//! which has no special case for `i`. `which_set_flags` renders
//! `$-` from that same forced_interactive state, so a
//! non-interactive shell shows `i` in `$-` after `set -ovi`, and
//! `set +ovi` removes it again — even in a genuinely interactive
//! shell, whose behavior does not change, only the letter.
//!
//! The refusal of a STANDALONE `set -i`/`set +i` (and of a bundle
//! like `set -vi`, which the pre-scan rejects whole, applying
//! nothing) lives in the pre-scan getopt and is a separate face;
//! the guard cells pin that it survives.
//!
//! Pre-fix rubash: the apply loop admitted `i` but
//! `apply_short_set_flag('i')` was a no-op and `$-` rendered the
//! letter from the immutable invocation state, so `set -ovi`
//! printed `flags=hvB` and interactive `set +ovi` kept the
//! letter it should have dropped.
//!
//! Every expectation below was measured against GNU bash 5.3.20.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn case_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2961_{tag}_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    dir
}

fn finish(dir: PathBuf, output: std::process::Output) -> Run {
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

fn run_script(body: &str) -> Run {
    let dir = case_dir("script");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    finish(dir, output)
}

fn run_args(args: &[&str]) -> Run {
    let dir = case_dir("args");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    finish(dir, output)
}

fn assert_stdout(run: &Run, expected_status: i32, expected_stdout: &str) {
    assert_eq!(
        run.status, expected_status,
        "status (stdout: {:?}, stderr: {})",
        run.stdout, run.stderr
    );
    assert_eq!(
        run.stdout, expected_stdout,
        "stdout (stderr: {})",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&run.dir);
}

/// The issue reproducer: `set -ovi` prints the `-o` listing as it
/// stood BEFORE the trailing letters apply (verbose still off),
/// then `v` and `i` both land — `$-` gains the `i` in a
/// non-interactive shell.
#[test]
fn set_minus_ovi_prints_listing_then_sets_v_and_i() {
    let run = run_script("set -ovi\necho \"flags=$-\"\n");
    let expected = concat!(
        "allexport      \toff\n",
        "braceexpand    \ton\n",
        "emacs          \toff\n",
        "errexit        \toff\n",
        "errtrace       \toff\n",
        "functrace      \toff\n",
        "hashall        \ton\n",
        "histexpand     \toff\n",
        "history        \toff\n",
        "ignoreeof      \toff\n",
        "interactive-comments\ton\n",
        "keyword        \toff\n",
        "monitor        \toff\n",
        "noclobber      \toff\n",
        "noexec         \toff\n",
        "noglob         \toff\n",
        "nolog          \toff\n",
        "notify         \toff\n",
        "nounset        \toff\n",
        "onecmd         \toff\n",
        "physical       \toff\n",
        "pipefail       \toff\n",
        "posix          \toff\n",
        "privileged     \toff\n",
        "verbose        \toff\n",
        "vi             \toff\n",
        "xtrace         \toff\n",
        "flags=hivB\n",
    );
    assert_stdout(&run, 0, expected);
}

/// The `+` form prints the listing in command form (showing the
/// state the `-ovi` left: verbose on) and removes BOTH trailing
/// letters from `$-`.
#[test]
fn set_plus_ovi_prints_command_form_and_drops_v_and_i() {
    let run = run_script("set -ovi >/dev/null\nset +ovi\necho \"flags=$-\"\n");
    let expected = concat!(
        "set +o allexport\n",
        "set -o braceexpand\n",
        "set +o emacs\n",
        "set +o errexit\n",
        "set +o errtrace\n",
        "set +o functrace\n",
        "set -o hashall\n",
        "set +o histexpand\n",
        "set +o history\n",
        "set +o ignoreeof\n",
        "set -o interactive-comments\n",
        "set +o keyword\n",
        "set +o monitor\n",
        "set +o noclobber\n",
        "set +o noexec\n",
        "set +o noglob\n",
        "set +o nolog\n",
        "set +o notify\n",
        "set +o nounset\n",
        "set +o onecmd\n",
        "set +o physical\n",
        "set +o pipefail\n",
        "set +o posix\n",
        "set +o privileged\n",
        "set -o verbose\n",
        "set +o vi\n",
        "set +o xtrace\n",
        "flags=hB\n",
    );
    assert_stdout(&run, 0, expected);
}

/// In an interactive shell the `i` letter is already rendered;
/// `set -ovi` adds `v`, and `set +ovi` drops BOTH letters — the
/// shell stays interactive, only the `$-` rendering changes.
#[test]
fn interactive_plus_ovi_drops_the_letter_not_the_shell() {
    let run = run_args(&[
        "-i",
        "-c",
        "set -ovi >/dev/null; echo \"flags=$-\"; set +ovi >/dev/null; echo \"flags2=$-\"",
    ]);
    assert_stdout(&run, 0, "flags=hivBHc\nflags2=hBHc\n");
}

/// forced_interactive is ordinary shell state: it rides the
/// subshell fork boundary, so a `( )` subshell renders it too.
#[test]
fn forced_interactive_survives_subshell() {
    let run = run_script("set -ovi >/dev/null\n( echo \"sub=$-\" )\necho \"outer=$-\"\n");
    assert_stdout(&run, 0, "sub=hivB\nouter=hivB\n");
}

/// Guard: a standalone `set -i` is refused by the pre-scan
/// (invalid option + usage, status 2) and the script continues.
#[test]
fn standalone_set_minus_i_still_refused() {
    let run = run_script("set -i\necho \"after rc=$?\"\n");
    assert_eq!(run.status, 0, "status (stderr: {})", run.stderr);
    assert_eq!(run.stdout, "after rc=2\n");
    assert!(
        run.stderr.contains("set: -i: invalid option"),
        "stderr should carry the refusal: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("usage: set [-abefhkmnptuvxBCEHPT]"),
        "stderr should carry the usage line: {}",
        run.stderr
    );
    let _ = std::fs::remove_dir_all(&run.dir);
}

/// Guard: a bundle the pre-scan can see (`set -vi`) is rejected
/// whole — no flag is applied, status 2.
#[test]
fn visible_i_bundle_still_refused_wholesale() {
    let run = run_args(&["-c", "set -vi; echo \"flags=$- rc=$?\""]);
    assert_stdout(&run, 0, "flags=hBc rc=2\n");
}
