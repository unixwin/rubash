//! rubash#3783 — `test -v` / `[[ -v ]]` treated an all-digit operand as
//! an invalid name and always returned false, instead of resolving it as
//! a positional parameter the way GNU bash does.
//!
//! GNU ground: test.c:650-685 (unary `-v`) tries `valid_array_reference`
//! first, then `valid_number(arg, &r)` (general.c:248-281, strtoimax base
//! 10: optional sign, surrounding whitespace, errno on overflow) and
//! answers `r >= 0 && r <= number_of_args()` (builtins/common.c:429-446,
//! posparam_count, the count of $1..$N) — so `0` is always set, `N` is
//! set iff that positional exists, and leading zeros parse numerically
//! (`00` is `$0`, `01` is `$1`). The same holds for `[[ -v N ]]`
//! (execute_cmd.c routes `[[ -v ]]` through the same unary_test) and
//! inside functions (frame positionals).
//!
//! Rubash's `builtins::test::variable_is_set` only consulted the
//! variable table (plus dynamic parameters), which never holds
//! positional entries, so every all-digit `-v` operand returned false.
//! The fix threads the live positional window length
//! (ShellState::positional_len) from the executor into the `-v`
//! verdict and applies GNU's valid_number + range check there.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle
//! (self-compiled, `~/workspace/rubash-audit/build-gnu/bash-5.3/bash`)
//! with `-c` probes, including the edge matrix (`+1`, leading/trailing
//! whitespace, `-1`, overflow, `0x1`, `1e1`, empty-string positional).

use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run_dash_c(script: &str) -> (String, String) {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn check(script: &str, expected_stdout: &str) {
    let (stdout, stderr) = run_dash_c(script);
    assert_eq!(stderr, "", "script {script:?} wrote to stderr");
    assert_eq!(stdout, expected_stdout, "script {script:?}");
}

#[test]
fn issue3783_test_v_zero_is_always_set() {
    check("test -v 0; echo $?", "0\n");
    check("test -v 00; echo $?", "0\n");
    check("[ -v 0 ]; echo $?", "0\n");
    check("[[ -v 0 ]]; echo $?", "0\n");
}

#[test]
fn issue3783_test_v_positionals_set_and_unset() {
    check("set -- x y; test -v 1; echo $?", "0\n");
    check("set -- x y; test -v 2; echo $?", "0\n");
    check("set -- x y; test -v 3; echo $?", "1\n");
    check("set -- a b c d e f g h i j; test -v 10; echo $?", "0\n");
    check("set -- a b c d e f g h i j; test -v 11; echo $?", "1\n");
    check("set -- x y; [[ -v 2 ]]; echo $?", "0\n");
    check("set -- x y; [[ -v 3 ]]; echo $?", "1\n");
}

#[test]
fn issue3783_test_v_leading_zeros_parse_numerically() {
    check("set -- x y; test -v 01; echo $?", "0\n");
    check("test -v 01; echo $?", "1\n");
    check("set -- x; [[ -v 01 ]]; echo $?", "0\n");
    check("set -- x y; test -v 002; echo $?", "0\n");
    check("set -- x y; test -v 003; echo $?", "1\n");
}

#[test]
fn issue3783_test_v_function_frame_positionals() {
    check("f() { test -v 1; echo $?; }; f p", "0\n");
    check("f() { test -v 1; echo $?; }; f", "1\n");
    check("f() { test -v 2; echo $?; }; f p q", "0\n");
    check("f() { [[ -v 1 ]]; echo $?; }; f p", "0\n");
    check("f() { test -v 0; echo $?; }; f", "0\n");
}

#[test]
fn issue3783_test_v_number_edge_matrix_matches_gnu() {
    // GNU valid_number (general.c:248-281): optional sign and
    // surrounding whitespace parse; overflow / junk / negative fail.
    check("set -- x y; test -v '+1'; echo $?", "0\n");
    check("set -- x y; test -v ' 1'; echo $?", "0\n");
    check("set -- x y; test -v '1 '; echo $?", "0\n");
    check("set -- x y; test -v '-1'; echo $?", "1\n");
    check(
        "set -- x y; test -v '9999999999999999999999'; echo $?",
        "1\n",
    );
    check("set -- x y; test -v '0x1'; echo $?", "1\n");
    check("set -- x y; test -v '1e1'; echo $?", "1\n");
    check("set -- x y; test -v '1abc'; echo $?", "1\n");
    // An empty-but-present positional still counts as set (GNU
    // answers from the count, not the value).
    check("set -- ''; test -v 1; echo $?", "0\n");
    check("set -- ''; test -v 2; echo $?", "1\n");
}

#[test]
fn issue3783_test_v_controls_still_false() {
    check("test -v 1; echo $?", "1\n");
    check("test -v NOSUCHVAR_xyz; echo $?", "1\n");
    check("test -v ''; echo $?", "1\n");
    check("test -v '1abc'; echo $?", "1\n");
}
