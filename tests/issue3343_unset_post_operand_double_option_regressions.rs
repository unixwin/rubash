//! rubash#3343 — `unset`'s diagnostic routing must not re-read
//! post-operand `-f`/`-v` as options.
//!
//! GNU's `internal_getopt` stops at the first operand
//! (builtins/bashgetopt.c:77-84), so in `unset_builtin`
//! (builtins/set.def:827, `internal_getopt (list, "fnv")`) every word
//! from the first operand on is a NAME — `-f`/`-v` after an operand
//! are names, never options, and `-f` + `-v` together only conflict
//! when both sit in the option region. PR #3342 converged the five
//! executor-side predicate scans onto the one shared
//! `builtin_opt::option_region_end` definition, but the diagnostic
//! routing predicate `unset_args_need_builtin_diagnostics` kept its
//! own hand scan, and the executor route's `variable_args` rebuild
//! re-emitted post-operand `-` words verbatim: whenever the operand
//! ahead of them was consumed upstream (nameref/array/local/invalid
//! name — its `pushed` slot left empty), `-f`/`-v` slid to the head
//! of the lib's argument list and BuiltinOpt re-read them as
//! options, producing a spurious `cannot simultaneously unset a
//! function and a variable` (or a spurious invalid-option / help
//! dispatch) where GNU is silent.
//!
//! The fix reuses the same `option_region_end` definition in both
//! places: the predicate iterates only the option region, and the
//! rebuild emits `--` before the first post-region `-` word whenever
//! no operand has been emitted yet, so the lib's scan ends exactly
//! where the original region ended.
//!
//! Every expectation below was measured against the GNU 5.3.20
//! oracle built from `third_party/bash` (bash.git @9c465866).
//!
//! Note on status: each script's exit status is the unset command's
//! own — single-shape scripts end on the unset line, and value
//! checks save `$?` and `exit` with it instead of echoing it (an
//! `echo` after the unset would reset the script status to 0).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    #[allow(dead_code)]
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue3343unset_{}_{}", std::process::id(), seq));
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

/// GNU is silent here: no diagnostic of any kind on stderr, and no
/// help text on stdout.
fn assert_silent(run: &Run) {
    assert!(
        run.stderr.is_empty(),
        "expected silence, stderr: {}",
        run.stderr
    );
    assert!(
        !run.stdout.contains("unset: unset ["),
        "unexpected help text on stdout: {}",
        run.stdout
    );
}

#[test]
fn unset_nameref_flag_invalid_name_post_operand_double_option_is_silent() {
    // The issue's named shape: -n in the region, invalid name first,
    // -f and -v both after the operand. GNU: getopt stops at the
    // operand, both are names, silent status 0. Base: the -n branch
    // consumed the name upstream without pushing it, so -f/-v slid to
    // the head of the lib's argument list and the lib reported the
    // -f/-v conflict.
    let run = run_script("unset -n 1bad -f -v\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_silent(&run);
}

#[test]
fn unset_invalid_array_name_post_operand_double_option_is_silent() {
    // Invalid `[`-shaped operand, no -n: the executor's
    // invalid-array-reference arm continues without pushing the
    // operand, so the same slide-up produced the conflict error.
    let run = run_script("unset 'a[bad' -f -v\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_silent(&run);
}

#[test]
fn unset_nameref_post_operand_double_option_unsets_both_silently() {
    // The operand is a nameref: the executor unbinds the referenced
    // variable through it and leaves the pushed slot empty; the
    // slide-up must not turn the -f/-v names into a conflict error.
    let run = run_script(
        "t=1\ndeclare -n v=t\nunset v -f -v\ns=$?\necho \"v=${v-unset} t=${t-unset}\"\nexit $s\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "v=unset t=unset\n", "stderr: {}", run.stderr);
    assert_silent(&run);
}

#[test]
fn unset_nameref_post_operand_unknown_option_is_a_name() {
    // One post-operand dash word, consumed-operand shape: `-x` is a
    // name (invalid identifier, silent without -v), not an option.
    // Base reported `-x: invalid option` + usage, status 2.
    let run = run_script("t=1\ndeclare -n v=t\nunset v -x\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_silent(&run);
}

#[test]
fn unset_nameref_post_operand_help_is_a_name() {
    // `--help` after the operand is a name, not a help dispatch.
    // Base printed the full help synopsis, status 2.
    let run = run_script("t=1\ndeclare -n v=t\nunset v --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_silent(&run);
}

#[test]
fn unset_variable_flag_invalid_array_name_reports_both_names() {
    // -v in the region, invalid array operand, -f after it: GNU
    // reports BOTH invalid names (`a[bad' and `-f'), status 1.
    // Base reported the first correctly, then the spurious conflict
    // error instead of the `-f' diagnostic.
    let run = run_script("unset -v 'a[bad' -f\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("`a[bad': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("`-f': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("cannot simultaneously"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn unset_nameref_flag_nameref_post_operand_double_option_keeps_target() {
    // -n in the region and the operand is a nameref: the nameref
    // itself is unset, the target survives, and -f/-v after the
    // operand stay names. Base: conflict error, status 1.
    let run = run_script(
        "t=1\ndeclare -n v=t\nunset -n v -f -v\ns=$?\necho \"v=${v-unset} t=${t-unset}\"\nexit $s\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "v=unset t=1\n", "stderr: {}", run.stderr);
    assert_silent(&run);
}

#[test]
fn unset_array_element_post_operand_double_option_is_silent() {
    // The operand is an array element, consumed upstream by
    // unset_array_element; the element is removed and -f/-v stay
    // names. Base: element removed, but conflict error, status 1.
    let run = run_script("a=(1 2)\nunset a[0] -f -v\ns=$?\necho \"a=${a[@]}\"\nexit $s\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "a=2\n", "stderr: {}", run.stderr);
    assert_silent(&run);
}

// Conservation cells: these already matched GNU on the unpatched
// tree and must keep matching after the fix.

#[test]
fn unset_both_modes_in_region_still_conflict() {
    let run = run_script("unset -f -v 1bad\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr
            .contains("unset: cannot simultaneously unset a function and a variable"),
        "stderr: {}",
        run.stderr
    );

    let run = run_script("unset -fv 1bad\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr
            .contains("unset: cannot simultaneously unset a function and a variable"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn unset_plain_invalid_name_post_operand_double_option_still_silent() {
    // No upstream consumption here (plain name, pushed through), so
    // the base already agreed with GNU; the fix must not regress it.
    let run = run_script("unset 1bad -f -v\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_silent(&run);
}

#[test]
fn unset_variable_flag_invalid_names_still_reported() {
    // -v in the region: both the operand and the post-operand -f
    // are invalid identifiers, both reported, status 1.
    let run = run_script("unset -v 1bad -f\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("`1bad': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("`-f': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn unset_help_before_operand_still_dispatches() {
    let run = run_script("unset --help -f -v\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .contains("unset: unset [-f] [-v] [-n] [name ...]"),
        "stdout: {}",
        run.stdout
    );
}
