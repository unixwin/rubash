//! Regression pins for rubash#3078 — inside ONE arithmetic
//! expression, rubash emits the non-fatal diagnostics and the
//! #3059/#3064 locale bind warnings in opposite order vs GNU:
//! the evaluator records diags in `ARITH_EVAL_DIAGS` and locale
//! binds in `ARITH_LOCALE_BINDS` (two separate logs), and the
//! boundary prints ALL diags first (`flush_arith_diags`,
//! arithmetic/mod.rs) and only then drains the warning log
//! (`sync_arith_writes_to_shell_state`), so the intra-expression
//! interleaving is lost.
//!
//! GNU's rule is uniform: every arithmetic stderr line is emitted
//! synchronously at its point in EVALUATION ORDER.
//!   - A bind warning fires at the bind: expr.c:331
//!     expr_bind_variable calls stupidly_hack_special_variables
//!     (expr.c:352) -> variables.c:6272 sv_locale -> locale.c:185
//!     set_locale_var -> internal_warning (locale.c:226/228 for
//!     LC_ALL, locale.c:301/303 for a category). A category bind
//!     made while LC_ALL is set never warns at all (the category's
//!     effective value is LC_ALL's; set_locale_var is not reached
//!     with a failing category value in that state — oracle p14).
//!   - `` `a[]': not a valid identifier `` is NON-fatal: the whole
//!     `a[]` token reaches bind_variable as a name, fails
//!     valid_array_reference and valid_identifier, and is rejected
//!     by sh_invalidid (variables.c:3403; builtins/common.c:208)
//!     while evaluation CONTINUES — later comma operands still
//!     bind (oracle p02/p03) and the failed assignment yields its
//!     RHS value (`((a[]=1))` exits 0).
//!   - A read of `a[]` reports `a[]: bad array subscript` TWICE
//!     (err_badarraysub, error.c:440 — a non-fatal report_error —
//!     reached from arrayfunc.c:402 via get_array_value, called
//!     at expr.c:1227), the element reads as 0, and the enclosing
//!     `(( ))` fails (status 1) without aborting the script.
//!   - A fatal diag (evalerror, expr.c:1525 — e.g. division by 0
//!     at expr.c:551) prints at its point and ABORTS the
//!     expression: operands after it never evaluate, binds before
//!     it stand. A bind to a readonly variable likewise aborts
//!     the rest of the expression at its point (oracle p18:
//!     `((r=2, LC_ALL=5))` leaves LC_ALL unset).
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for
//! the same script files in a locale-clean environment (only
//! C/C.utf8/POSIX locales installed; the numeric values used here
//! can never resolve to a real locale name), captured 2026-11-09
//! from probes p01-p21 in ~/workspace/fix-B4/probes/.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3078-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Base command with the whole locale family removed from the
/// inherited environment so each case starts from a known state.
fn base() -> Command {
    let mut cmd = Command::new(BIN);
    for var in [
        "LANG",
        "LC_ALL",
        "LC_COLLATE",
        "LC_CTYPE",
        "LC_MESSAGES",
        "LC_NUMERIC",
        "LC_TIME",
    ] {
        cmd.env_remove(var);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd
}

fn run_script(tag: &str, body: &str) -> (String, String, String, Option<i32>) {
    let dir = scratch(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = base()
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let result = (
        script.to_string_lossy().into_owned(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn issue3078_repro_warning_precedes_diag() {
    // The issue's reproducer: the warning bind (LC_ALL=5) evaluates
    // BEFORE the a[]=1 diag, so GNU prints warning, then diag.
    let (path, stdout, stderr, code) = run_script(
        "repro",
        "((LC_ALL=5, a[]=1))\necho \"rc=$? lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: ((: `a[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_diag_first_then_warning_binds() {
    // Mirror order: diag evaluates first, the warning bind still
    // happens after it (the identifier diag is non-fatal in GNU).
    let (path, stdout, stderr, code) = run_script(
        "diagfirst",
        "((a[]=1, LC_ALL=5))\necho \"rc=$? lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: ((: `a[]': not a valid identifier\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3078_bind_after_diag_lands() {
    // Continuation past a non-fatal diag: a plain bind after the
    // bad lvalue still lands.
    let (path, stdout, stderr, code) = run_script(
        "bindafter",
        "((a[]=1, x=42))\necho \"rc=$? x=${x-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 x=42\n");
    assert_eq!(
        stderr,
        format!("{path}: line 1: ((: `a[]': not a valid identifier\n")
    );
}

#[test]
fn issue3078_two_warnings_then_diag() {
    // Two warnings that both fire (category first, while LC_ALL is
    // still unset), then the diag — full three-line interleave.
    let (path, stdout, stderr, code) = run_script(
        "warnwarn",
        "((LC_CTYPE=6, LC_ALL=5, a[]=1))\necho \"rc=$? ct=$LC_CTYPE lc=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 ct=6 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_CTYPE: cannot change locale (6): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: ((: `a[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_diag_then_two_warnings() {
    let (path, stdout, stderr, code) = run_script(
        "diagwarnwarn",
        "((a[]=1, LC_CTYPE=6, LC_ALL=5))\necho \"rc=$? ct=$LC_CTYPE lc=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 ct=6 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: ((: `a[]': not a valid identifier\n\
             {path}: line 1: warning: setlocale: LC_CTYPE: cannot change locale (6): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3078_diag_warning_diag_sandwich() {
    // The strongest interleave: diag, warning, diag in one
    // expression. A two-log boundary flush cannot reproduce this
    // order with any fixed log order.
    let (path, stdout, stderr, code) = run_script(
        "sandwich",
        "((a[]=1, LC_ALL=5, b[]=2))\necho \"rc=$? lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: ((: `a[]': not a valid identifier\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: ((: `b[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_two_warnings_keep_bind_order() {
    // Warnings alone already drain in bind order (#3059); the fix
    // must not disturb that.
    let (path, stdout, stderr, code) = run_script(
        "warnorder",
        "((LC_CTYPE=6, LC_ALL=5))\necho \"rc=$? ct=$LC_CTYPE lc=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 ct=6 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_CTYPE: cannot change locale (6): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3078_lc_all_suppresses_category_warning() {
    // Once LC_ALL is bound (even to a bad value), a category bind
    // in the same expression does NOT warn — GNU's category arm
    // sees LC_ALL's value as the effective one. Only one line.
    let (path, stdout, stderr, code) = run_script(
        "suppress",
        "((LC_ALL=5, LC_CTYPE=6))\necho \"rc=$? lc=$LC_ALL ct=$LC_CTYPE\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 lc=5 ct=6\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3078_two_diags_in_eval_order() {
    let (path, stdout, stderr, code) = run_script("twodiags", "((a[]=1, b[]=2))\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: ((: `a[]': not a valid identifier\n\
             {path}: line 1: ((: `b[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_fatal_diag_after_warning() {
    // Division by zero is fatal (evalerror): the warning bind
    // before it prints first AND its value stands.
    let (path, stdout, stderr, code) = run_script(
        "fatalafter",
        "((LC_ALL=5, 1/0))\necho \"rc=$? lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: ((: LC_ALL=5, 1/0: division by 0 (error token is \"0\")\n"
        )
    );
}

#[test]
fn issue3078_fatal_diag_first_suppresses_rest() {
    // Fatal point first: nothing after it evaluates — no warning,
    // no bind. (GNU's error token here is the unparsed remainder
    // verbatim, expr.c evalerror lasttp semantics.)
    let (path, stdout, stderr, code) = run_script(
        "fatalfirst",
        "((1/0, LC_ALL=5))\necho \"rc=$? lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 lc=unset\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: ((: 1/0, LC_ALL=5: division by 0 (error token is \"0, LC_ALL=5\")\n"
        )
    );
}

#[test]
fn issue3078_nonfatal_diag_then_fatal_diag() {
    let (path, stdout, stderr, code) = run_script("diagfatal", "((a[]=1, 1/0))\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: ((: `a[]': not a valid identifier\n\
             {path}: line 1: ((: a[]=1, 1/0: division by 0 (error token is \"0\")\n"
        )
    );
}

#[test]
fn issue3078_read_side_bad_subscript_after_warning() {
    // Read-side diag (bad array subscript, reported twice by GNU)
    // follows the warning bind that precedes it in the expression;
    // the element reads as 0 and the command fails.
    let (path, stdout, stderr, code) = run_script(
        "readside",
        "a[3]=9\n((LC_ALL=5, y=a[]))\necho \"rc=$? y=${y-unset} a3=${a[3]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 y=0 a3=9\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 2: a[]: bad array subscript\n\
             {path}: line 2: a[]: bad array subscript\n"
        )
    );
}

#[test]
fn issue3078_let_form_warning_precedes_diag() {
    // Same evaluator through the `let` builtin: label is `let`,
    // order is the same evaluation order.
    let (path, stdout, stderr, code) = run_script(
        "letform",
        "let 'LC_ALL=5, a[]=1'\necho \"rc=$? lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: let: `a[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_expansion_form_warning_precedes_diag() {
    // $(( )) expansion context: diagnostics carry no command
    // label, order is still evaluation order.
    let (path, stdout, stderr, code) = run_script(
        "expansion",
        "v=$((LC_ALL=5, a[]=1))\necho \"rc=$? v=${v-unset} lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 v=1 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: `a[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_chain_state_feeds_next_step() {
    // Chained: step 1 mixes a warning bind and a diag (the bind
    // lands, the pre-existing element is untouched); step 2 spends
    // that state — its category bind is silent because LC_ALL is
    // already set, and the stored text value reads as 0 in
    // arithmetic, so the post-inc stores 1 and the command fails.
    let (path, stdout, stderr, code) = run_script(
        "chain",
        "a[7]=keep\n\
         ((LC_ALL=5, a[]=1))\n\
         echo \"step1 rc=$? a7=${a[7]} lc=$LC_ALL\"\n\
         ((LC_CTYPE=6, a[7]++))\n\
         echo \"step2 rc=$? a7=${a[7]} ct=$LC_CTYPE\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "step1 rc=0 a7=keep lc=5\nstep2 rc=1 a7=1 ct=6\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 2: ((: `a[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_for_loop_incr_diag_after_init_warning() {
    // arith-for: the init list's warning bind prints in the init
    // evaluation; the incr list's diag prints in the incr
    // evaluation one iteration later — order across the loop's
    // own sub-expressions is still evaluation order.
    let (path, stdout, stderr, code) = run_script(
        "forloop",
        "for ((LC_ALL=5, i=0; i<1; i++, a[]=1)); do echo iter; done\necho \"rc=$? i=$i lc=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "iter\nrc=0 i=1 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: ((: `a[]': not a valid identifier\n"
        )
    );
}

#[test]
fn issue3078_warning_then_readonly_diag() {
    // The readonly bind evaluates AFTER the warning bind, so GNU
    // prints the warning first; the readonly value stands.
    let (path, stdout, stderr, code) = run_script(
        "warnreadonly",
        "readonly r=1\n((LC_ALL=5, r=2))\necho \"rc=$? r=$r lc=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 r=1 lc=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 2: r: readonly variable\n"
        )
    );
}

#[test]
fn issue3078_readonly_bind_aborts_rest() {
    // A readonly bind failure aborts the rest of the expression
    // at its point: the later locale bind never happens (no
    // warning, value unset), mirroring evalerror's cut.
    let (path, stdout, stderr, code) = run_script(
        "readonlyabort",
        "readonly r=1\n((r=2, LC_ALL=5))\necho \"rc=$? r=$r lc=${LC_ALL-unset}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1 r=1 lc=unset\n");
    assert_eq!(stderr, format!("{path}: line 2: r: readonly variable\n"));
}
