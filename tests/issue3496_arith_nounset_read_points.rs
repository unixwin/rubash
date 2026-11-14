//! rubash#3496 / #3497 / #3498 — arithmetic `set -u` traps follow GNU's
//! read points and expansion phases, not a text scan.
//!
//! GNU anchors (bash 5.3, vendored tree @9c465866):
//! - expr.c:1150-1216 expr_streval: the nounset trap fires only when
//!   evalexp actually READS a variable — never under `noeval`
//!   (expr.c:1164-1167, `0 && i`), never for assignment targets
//!   (expr.c:1393-1401 readtok's `peektok != EQ`), and a surviving `$`
//!   in already-expanded text is readtok data ("operand expected"),
//!   so `let '$undef'` is a syntax error, not an unbound read (#3496).
//!   For an array reference (`e == ']'`) the ARRAY cell is looked up
//!   first: `v == 0 || invisible_p (v)` reports the array name.
//! - subst.c:11395-11405 / 11107 expand_array_subscript: inside
//!   `(( ))` / `$(( ))` text that holds an expansion character, a
//!   subscript is expanded by expand_subscript_string with
//!   `quoted & ~(Q_ARITH|Q_DOUBLE_QUOTES)` (subst.c:11147) — unquoted,
//!   so `b['$undef']` hands evalexp the literal `$undef` (operand
//!   error, #3497). The `let` operand only reaches array_expand_index's
//!   `expand_arith_string (exp, Q_DOUBLE_QUOTES|...)` (arrayfunc.c:1370),
//!   where `'` is data and `$undef` still expands. Without an expansion
//!   character the string takes expand_arith_string's quote-removal
//!   branch (subst.c:3992-4033), which keeps `'` (`b['1']` errors).
//! - subst.c:11229 expand_word_internal: each `$((...))` is expanded
//!   and evaluated when the left-to-right word walk reaches it, so its
//!   bare-name trap precedes a later word's `$undef` (#3498).
//!
//! Every expectation below is the GNU bash 5.3.20 `bash -c` measurement
//! (`/usr/local/bin/bash` in WSL), byte-for-byte on the message tail.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: String,
    stderr_tail: String,
    code: i32,
}

/// Run `rubash -c script`; the stderr lines lose their `<argv0>: line N: `
/// prefix (it names the binary, which differs per checkout).
fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_tail = stderr
        .lines()
        .map(|line| match line.find(": line 1: ") {
            Some(at) => &line[at + ": line 1: ".len()..],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr_tail,
        code: output.status.code().unwrap_or(-1),
    }
}

fn expect(script: &str, stdout: &str, stderr_tail: &str, code: i32) {
    let out = run(script);
    assert_eq!(out.stdout, stdout, "stdout of {script:?}");
    assert_eq!(out.stderr_tail, stderr_tail, "stderr of {script:?}");
    assert_eq!(out.code, code, "exit status of {script:?}");
}

fn expect_unbound(script: &str, name: &str) {
    expect(script, "", &format!("{name}: unbound variable"), 127);
}

// ---- #3496: evalexp-time reads, not a text scan ----

#[test]
fn let_dollar_operand_is_a_syntax_error() {
    expect(
        "set -u; let '$undef'; echo AFTER rc=$?",
        "AFTER rc=1\n",
        "let: $undef: arithmetic syntax error: operand expected (error token is \"$undef\")",
        0,
    );
}

#[test]
fn noeval_short_circuits_never_trap() {
    expect(
        "set -u; let '0 && undef'; echo AFTER rc=$?",
        "AFTER rc=1\n",
        "",
        0,
    );
    expect(
        "set -u; echo $(( 0 && i )) $(( 1 || i )) $(( 0 ? i : 2 )); echo AFTER",
        "0 1 2\nAFTER\n",
        "",
        0,
    );
    expect(
        "set -u; for (( q=0; 0 && n; q++ )); do :; done; echo AFTER",
        "AFTER\n",
        "",
        0,
    );
}

#[test]
fn real_reads_still_trap() {
    expect_unbound("set -u; let 'x = undef + 1'; echo AFTER", "undef");
    expect_unbound("set -u; b=(x y); (( b[i]++ )); echo AFTER", "i");
}

#[test]
fn array_cell_lookup_precedes_the_element() {
    expect_unbound("set -u; declare -a e; echo $(( e[0] )); echo AFTER", "e");
    expect_unbound("set -u; declare -a e; echo $(( e )); echo AFTER", "e");
    expect_unbound("set -u; echo $(( nn[0] )); echo AFTER", "nn");
    expect(
        "set -u; declare -A h=(); echo $(( h[k] )); echo AFTER",
        "0\nAFTER\n",
        "",
        0,
    );
    expect("set -u; (( nn[0]=3 )); echo ${nn[0]}", "3\n", "", 0);
}

// ---- #3497: subscript quoting context ----

#[test]
fn single_quoted_subscript_dollar_is_data() {
    let tail = "$undef: arithmetic syntax error: operand expected (error token is \"$undef\")";
    expect(
        "set -u; b=(x y); (( b['$undef'] )); echo AFTER",
        "",
        tail,
        1,
    );
    expect(
        "set -u; b=(x y); echo $(( b['$undef'] )); echo AFTER",
        "",
        tail,
        1,
    );
    expect(
        "set -u; b=(x y); x=1; (( b['\\$x'] )); echo AFTER",
        "",
        "\\$x: arithmetic syntax error: operand expected (error token is \"\\$x\")",
        1,
    );
}

#[test]
fn other_subscript_contexts_keep_their_reads() {
    expect_unbound("set -u; b=(x y); (( b[\"$undef\"] )); echo AFTER", "undef");
    // `let`: array_expand_index's double-quoted pass expands inside `'`.
    expect_unbound(
        "set -u; b=(x y); let \"b['\\$undef']\"; echo AFTER",
        "undef",
    );
    // No expansion character: quote removal keeps `'` as data.
    expect(
        "set -u; b=(x y); (( b['1'] )); echo AFTER",
        "",
        "'1': arithmetic syntax error: operand expected (error token is \"'1'\")",
        1,
    );
    // With one, the subscript is quote-removed: element 1 is `y`.
    expect_unbound("set -u; b=(x y); x=0; (( b['1'] + $x )); echo AFTER", "y");
}

// ---- #3498: the word walk evaluates `$((...))` in place ----

#[test]
fn earlier_arith_expansion_traps_first() {
    expect_unbound("set -u; echo $(( i++ )) + b[$undef]; echo AFTER", "i");
    expect_unbound("set -u; echo $(( i )) $undef; echo AFTER", "i");
    expect_unbound("set -u; v=$(( i ))$undef; echo AFTER", "i");
}

#[test]
fn walk_order_is_kept() {
    expect_unbound("set -u; echo $undef $(( i )); echo AFTER", "undef");
    // Expansion precedes evaluation inside one `$((...))`.
    expect_unbound("set -u; echo $(( $undef + i )); echo AFTER", "undef");
    // A non-trapping evaluation lets the walk move on.
    expect_unbound(
        "set -u; i=1; echo $(( i )) $(( 0 && j )) $undef; echo AFTER",
        "undef",
    );
}
