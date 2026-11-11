//! rubash#3338 — the declare family's option scans stop at the
//! first non-option operand.
//!
//! GNU's `internal_getopt` returns -1 at the first non-option word
//! (builtins/bashgetopt.c:77-84), so in `declare_builtin`
//! (builtins/declare.def:273-274) and `unset_builtin`
//! (builtins/set.def, `internal_getopt (list, "fnv")`) every word
//! from the first operand on is a NAME, never an option — however
//! option-shaped it looks. Stage-2 ④ Phase 4 put the handlers' core
//! scans on the single BuiltinOpt machine, which stops there; the
//! executor-side predicate scans around the core (the integer
//! pre-evaluation gate, the subscript rewrite's -a/-A hints, the
//! function face's letter scans, unset's -f/-v/-n detectors) still
//! roamed the whole word list and misread post-operand options.
//! These pins lock the converged consumption form: one option
//! region, shared by every scan in the family.
//!
//! The `declare x --help` cell below is the sub-cell extracted from
//! PR #3236's gate pin (its :205), moved here verbatim at the
//! captain's direction — #3338's issue body carries the three-way
//! evidence (GNU / pre-Phase-4 base / gate head).

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
    let dir = std::env::temp_dir().join(format!("issue3338scan_{}_{}", std::process::id(), seq));
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

/// Not a help dispatch: the help synopsis must not appear.
fn assert_not_help(run: &Run, synopsis: &str) {
    assert!(
        !run.stdout.contains(synopsis),
        "unexpected help text on stdout: {}",
        run.stdout
    );
}

#[test]
fn declare_typeset_operand_first_ends_scan() {
    // An operand first ends the scan: both words are names, and
    // `--help` as a name is an invalid identifier (status 1).
    let run = run_script("declare x --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_not_help(&run, "declare: declare [");
    assert!(
        run.stderr.contains("`--help': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );

    let run = run_script("typeset x --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_not_help(&run, "typeset: typeset [");
    assert!(
        run.stderr.contains("`--help': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );

    // Any option-shaped word after the operand is a name:
    // `-p` is not print mode there, it is an invalid identifier.
    let run = run_script("declare x -p\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("`-p': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );

    // Plus-signed options before the operand still scan (the
    // declare spec is plus mode); the word after the operand is
    // a name all the same.
    let run = run_script("declare +x y --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("`--help': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn declare_print_operand_first_ends_scan() {
    // `-p` in the option region is a real option; the scan stops
    // at `x`, so `--help` is a lookup NAME: both names are
    // reported not found, status 1. (Report ORDER between the two
    // names is a separate face and deliberately not asserted.)
    let run = run_script("declare -p x --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("x: not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("--help: not found"),
        "stderr: {}",
        run.stderr
    );
    assert_not_help(&run, "declare: declare [");
}

#[test]
fn unset_operand_first_ends_scan() {
    // Both words are names; neither exists, and a plain unset
    // treats an invalid name as a potential function name —
    // silent, status 0.
    let run = run_script("unset x --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);

    // The operand is still unset: `--help` does not divert the
    // command into an option error.
    let run = run_script("v=1\nunset v --help\necho ${v-GONE}\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "GONE\n", "stdout: {}", run.stdout);

    // With `-v` in the region, the post-operand `--help` name IS
    // reported (as a name): not a valid identifier, status 1.
    let run = run_script("unset -v x --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("`--help': not a valid identifier"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn unset_mode_letters_after_operand_are_names() {
    // `-f` after the operand is a name, not function mode: the
    // variable is unset by the default (variable-or-function)
    // path.
    let run = run_script("v=1\nunset v -f\necho ${v-GONE}\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "GONE\n", "stdout: {}", run.stdout);

    // `-v` after the operand is a name, not variable mode: the
    // function is unset by the default path.
    let run = run_script("f() { :; }\nunset f -v\ntype f\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stderr.contains("not found"), "stderr: {}", run.stderr);

    // `-n` after the operand is a name, not nameref mode: the
    // variable itself is unset.
    let run = run_script("x=1\nunset x -n\necho ${x-GONE}\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "GONE\n", "stdout: {}", run.stdout);
}

#[test]
fn declare_integer_gate_reads_region_only() {
    // `-i` after the operand is a name (reported, status of the
    // declare is 1 for that name) and must NOT switch the command
    // into integer evaluation: `y=zz` binds the literal text,
    // exactly as GNU binds it (zz would evaluate to 0 as an
    // integer expression).
    let run = run_script("declare x -i y=zz\ndeclare -p y\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("y=\"zz\""), "stdout: {}", run.stdout);

    let run = run_script("typeset x -i y=zz\ndeclare -p y\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("y=\"zz\""), "stdout: {}", run.stdout);

    // `--` ends the region too: `-i` behind it is a name.
    let run = run_script("declare -- -i y=zz\ndeclare -p y\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("y=\"zz\""), "stdout: {}", run.stdout);

    // The same gate serves `local` (same handler family).
    let run = run_script("f() { local x -i y=zz; declare -p y; }\nf\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("y=\"zz\""), "stdout: {}", run.stdout);

    // Conservation: `-i` in the region still applies, with and
    // without a `--` behind it.
    let run = run_script("declare -i y=zz\ndeclare -p y\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("y=\"0\""), "stdout: {}", run.stdout);
    let run = run_script("declare -i -- y=zz\ndeclare -p y\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("y=\"0\""), "stdout: {}", run.stdout);
}

#[test]
fn declare_array_hints_read_region_only() {
    // A quoted `(1 2)` value binds as a plain scalar string in
    // GNU unless -a/-A stands in the OPTION REGION. After the
    // operand the letters are a (failing) name, never a hint:
    // the value stays the literal string either way.
    for flag in ["-A", "-a"] {
        let body = format!("declare y='(1 2)' {flag}\ndeclare -p y\n");
        let run = run_script(&body);
        assert_eq!(run.status, 0, "stderr: {}", run.stderr);
        assert!(
            run.stdout.contains("y=\"(1 2)\""),
            "flag {flag}: stdout: {}",
            run.stdout
        );
    }
}

#[test]
fn declare_function_face_scans_region_only() {
    // `-p` after the operand is a function NAME in the -f face:
    // it is looked up, not found, and the command fails (silently
    // — without a `-p` option, missing names are not reported).
    let run = run_script("x() { echo hi; }\ndeclare -f x -p\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("x ()"), "stdout: {}", run.stdout);

    // With `-p` in the region, a post-operand `-t` name IS
    // reported as not found.
    let run = run_script("x() { echo hi; }\ndeclare -fp x -t\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("-t: not found"),
        "stderr: {}",
        run.stderr
    );

    // Conservation: `-p` in the region still reports a missing
    // function name.
    let run = run_script("declare -f -p NOPE\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("NOPE: not found"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn option_region_errors_and_help_stay_put() {
    // An invalid letter in the region is still a usage error.
    let run = run_script("declare -Z x\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("-Z: invalid option"),
        "stderr: {}",
        run.stderr
    );

    // `--help` in option position still prints the long help.
    let run = run_script("declare --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("declare: declare ["),
        "stdout: {}",
        run.stdout
    );
    let run = run_script("unset --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("unset: unset ["),
        "stdout: {}",
        run.stdout
    );
}
