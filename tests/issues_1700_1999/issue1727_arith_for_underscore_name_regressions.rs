//! rubash#1727 — Variable names containing `_` (any position) resolved as
//! unset inside C-style `for ((init; test; update))` clauses: the section
//! text reached the evaluator with every underscore deleted (`_x` read as
//! the unset `x`, `x_y` as `xy`, `${#_x}` collapsed to `${#}` = `$#`), while
//! the same names read correctly in every other context.
//!
//! Root cause: `arith_for_section_text` (parser/arithmetic_for.rs) treated
//! a char as an in-band carrier when its codepoint matched ANY row of the
//! marker registry — but `MarkerInfo.code` is the FIRST codepoint of the
//! marker, and the multi-char string markers "__RUBASH_HD1__" /
//! "__RUBASH_CA1__" register `'_'` (0x5F). The predicate now strips only
//! genuine single-char carriers (marker zone U+E000..=U+E401 plus C0/C1
//! controls), preserving the #548 diagnostic-boundary defense.
//!
//! GNU law: make_cmd.c:245-275 `make_arith_for_command` splits the (( ))
//! text into the three sub-expressions verbatim (leading whitespace
//! skipped) and execute_cmd.c:3163 `eval_arith_for_expr` evaluates that
//! text as-is — GNU deletes no characters. Expectations below were
//! measured against GNU bash 5.3.20. Every loop carries an iteration
//! guard: on an unfixed binary the `c*=2` shapes are a real unbounded
//! loop (string doubling), so the guards keep a regression bounded.

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

fn assert_stdout(script: &str, expected: &str) {
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stdout, expected, "script: {script}\nstderr: {stderr}");
    assert_eq!(code, Some(0), "script: {script}");
}

#[test]
fn arith_for_init_length_read_underscore_positions() {
    // ${#name} in the init clause, underscore at start / middle / end.
    assert_stdout(
        "_x='        '; n=8; g=0; for ((c=${#_x}; c<n; c*=2)); do g=$((g+1)); [ $g -ge 3 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=0\n",
    );
    assert_stdout(
        "x_y='        '; n=8; g=0; for ((c=${#x_y}; c<n; c*=2)); do g=$((g+1)); [ $g -ge 3 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=0\n",
    );
    assert_stdout(
        "x_='        '; n=8; g=0; for ((c=${#x_}; c<n; c*=2)); do g=$((g+1)); [ $g -ge 3 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=0\n",
    );
}

#[test]
fn arith_for_init_plain_read_underscore_positions() {
    // Bare-name reads in the init clause, underscore at start / middle / end.
    assert_stdout(
        "_x=5; g=0; for ((c=_x; c<8; c++)); do g=$((g+1)); [ $g -ge 20 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=3\n",
    );
    assert_stdout(
        "x_y=7; g=0; for ((c=x_y; c<8; c++)); do g=$((g+1)); [ $g -ge 20 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=1\n",
    );
    assert_stdout(
        "x_=6; g=0; for ((c=x_; c<8; c++)); do g=$((g+1)); [ $g -ge 20 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=2\n",
    );
}

#[test]
fn arith_for_test_and_update_read_underscore_names() {
    // ${#name} in the test clause.
    assert_stdout(
        "_x='        '; cnt=0; for ((i=0; i<${#_x}; i++)); do cnt=$((cnt+1)); [ $cnt -ge 20 ] && break; done; echo \"cnt=$cnt i=$i\"",
        "cnt=8 i=8\n",
    );
    // Bare name in the test clause.
    assert_stdout(
        "lim_v=3; g=0; for ((i=0; i<lim_v; i++)); do g=$((g+1)); [ $g -ge 9 ] && break; done; echo \"i=$i g=$g\"",
        "i=3 g=3\n",
    );
    // ${#name} in the update clause.
    assert_stdout(
        "_x='        '; seq=\"\"; for ((i=0; i<3; i+=${#_x})); do seq=\"$seq $i\"; [ ${#seq} -gt 30 ] && break; done; echo \"seq=[$seq] i=$i\"",
        "seq=[ 0] i=8\n",
    );
    // Bare name in the update clause.
    assert_stdout(
        "step_v=2; g=0; for ((i=0; i<5; i+=step_v)); do g=$((g+1)); [ $g -ge 9 ] && break; done; echo \"i=$i g=$g\"",
        "i=6 g=3\n",
    );
}

#[test]
fn arith_for_loop_variable_with_underscore_keeps_its_binding() {
    // Writes to an underscored loop variable must land on that variable:
    // it drives the test/update and stays visible after the loop.
    assert_stdout(
        "g=0; for ((my_i=0; my_i<3; my_i++)); do g=$((g+1)); [ $g -ge 9 ] && break; done; echo \"my_i=$my_i g=$g\"",
        "my_i=3 g=3\n",
    );
}

#[test]
fn arith_for_underscore_name_is_not_a_prefix_truncation() {
    // With `x` also defined (length 5), ${#x_y} must still read x_y's own
    // length 8 — the miss is a whole-name miss, not a prefix resolution.
    assert_stdout(
        "x='abcde'; x_y='        '; for ((c=${#x_y}; c<0; c++)); do :; done; echo \"c=$c\"",
        "c=8\n",
    );
    assert_stdout(
        "_y=7; for ((c=_y; c<0; c++)); do :; done; echo \"c=$c\"",
        "c=7\n",
    );
}

#[test]
fn arith_for_init_brace_and_nested_arith_reads() {
    assert_stdout(
        "_x=5; g=0; for ((c=${_x}; c<8; c++)); do g=$((g+1)); [ $g -ge 20 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=3\n",
    );
    assert_stdout(
        "_x=5; g=0; for ((c=$((_x)); c<8; c++)); do g=$((g+1)); [ $g -ge 20 ] && break; done; echo \"c=$c g=$g\"",
        "c=8 g=3\n",
    );
}

#[test]
fn arith_for_ble_reserve_prototype_shape_terminates() {
    // The ble.sh reserve-prototype shape that found the bug: the init read
    // is the prototype's length, so GNU runs zero iterations. Guarded like
    // the original probe (an unfixed binary doubles the string unbounded).
    assert_stdout(
        "_ble_string_prototype='        '\nreserve() {\n  local n=$1 c guard=0\n  for ((c=${#_ble_string_prototype}; c<n; c*=2)); do\n    _ble_string_prototype=$_ble_string_prototype$_ble_string_prototype\n    guard=$((guard+1)); [ $guard -ge 6 ] && break\n  done\n  echo \"c=$c len=${#_ble_string_prototype} iters=$guard\"\n}\nreserve 8",
        "c=8 len=8 iters=0\n",
    );
}

#[test]
fn underscore_names_outside_arith_for_stay_correct() {
    // Controls: the same reads in expansion, standalone (( )) and $(( ))
    // contexts always worked and must not move.
    assert_stdout(
        "_x='        '; echo \"len=${#_x}\"; ((a=${#_x})); echo \"a=$a\"; b=${#_x}; echo \"b=$b\"; echo \"d=$((${#_x}))\"; _x=5; ((e=_x)); echo \"e=$e\"; echo \"s=$((_x+1))\"",
        "len=8\na=8\nb=8\nd=8\ne=5\ns=6\n",
    );
}
