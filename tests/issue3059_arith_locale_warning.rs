//! Regression pins for rubash#3059 — assigning a locale variable
//! through ARITHMETIC assignment never ran the sv_locale funnel:
//!   ((LC_ALL = 5))
//! stayed silent where GNU bash warns:
//!   warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory
//!
//! GNU ownership: every arithmetic bind goes through expr.c:331
//! expr_bind_variable, which binds and then calls
//! stupidly_hack_special_variables(lhs) with the WRITTEN lhs
//! (expr.c:352); the special_vars table (variables.c:5800) maps LANG
//! and every LC_* category to sv_locale (variables.c:6272), which
//! calls locale.c:185 set_locale_var. On a failed setlocale it warns
//! `setlocale: <VAR>: cannot change locale (<value>)` (locale.c:226 /
//! locale.c:302), appending `: <strerror(errno)>` only when errno is
//! set — glibc leaves errno set only on the FIRST failed lookup of a
//! given locale name in the process, so rebinding the same bad value
//! warns without the suffix while a different bad value earns its own
//! suffix. LANG itself never warns (set_lang is silent). The hook is
//! keyed on the written lhs, so a nameref bind (`((r = 5))` with r a
//! nameref to LC_ALL) and an element bind (`((LC_ALL[0] = 5))`, whose
//! lhs is the literal text `LC_ALL[0]`) do NOT fire it; an
//! unsubscripted bind of an array-marked locale name binds element 0
//! and DOES fire it with the element value. Under noeval the bind is
//! skipped entirely (expr.c:607-615), so nothing warns.
//!
//! All expectations are the literal GNU bash 5.3.20 outputs for the
//! same script files in a locale-clean environment (only C/C.utf8 /
//! POSIX locales installed; the numeric values used here can never
//! resolve to a real locale name).
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3059-{tag}-{}", std::process::id()));
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
fn issue3059_plain_arith_assignment_warns_like_gnu() {
    let (path, stdout, stderr, code) = run_script("assign", "((LC_ALL = 5))\necho \"v=$LC_ALL\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "v=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3059_compound_and_incdec_shapes_warn_like_gnu() {
    // Add-assign from unbound binds the sum; post-inc binds the
    // incremented value (the warning names the BOUND value, not the
    // expression result).
    let (path, stdout, stderr, code) = run_script(
        "compound",
        "((LC_ALL += 5))\necho \"a=$LC_ALL\"\n((LC_ALL++))\necho \"b=$LC_ALL\"\n((++LC_ALL))\necho \"c=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=5\nb=6\nc=7\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 3: warning: setlocale: LC_ALL: cannot change locale (6): No such file or directory\n\
             {path}: line 5: warning: setlocale: LC_ALL: cannot change locale (7): No such file or directory\n"
        )
    );
}

#[test]
fn issue3059_rebind_suffix_rule_is_per_value() {
    // Two binds in ONE expression: each distinct bad value is a first
    // failed lookup of that name, so both warnings carry the suffix;
    // rebinding the SAME value warns again without it.
    let (path, stdout, stderr, code) = run_script(
        "rebind",
        "((LC_ALL = 5, LC_ALL = 6))\necho \"a=$LC_ALL\"\n((LC_ALL = 6))\necho \"b=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=6\nb=6\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (6): No such file or directory\n\
             {path}: line 3: warning: setlocale: LC_ALL: cannot change locale (6)\n"
        )
    );
}

#[test]
fn issue3059_category_and_let_shapes_warn_like_gnu() {
    let (path, stdout, stderr, code) = run_script(
        "category",
        "((LC_CTYPE = 5))\necho \"a=$LC_CTYPE\"\nlet LC_ALL=5\necho \"b=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=5\nb=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_CTYPE: cannot change locale (5): No such file or directory\n\
             {path}: line 3: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3059_for_loop_binds_warn_per_bind() {
    // Slow loop (non-`:` body): init and every update bind warn, each
    // with the for-line prefix.
    let (path, stdout, stderr, code) = run_script(
        "forloop",
        "for ((LC_ALL=5; LC_ALL<7; LC_ALL++)); do echo iter; done\necho \"v=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "iter\niter\nv=7\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (6): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (7): No such file or directory\n"
        )
    );

    // Bare-`:` body (the arith-counter fast loop's shape): init and
    // the final update bind warn the same way.
    let (path, stdout, stderr, code) = run_script(
        "forloopfast",
        "for ((LC_ALL=5; LC_ALL<6; LC_ALL++)); do :; done\necho \"v=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "v=6\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 1: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n\
             {path}: line 1: warning: setlocale: LC_ALL: cannot change locale (6): No such file or directory\n"
        )
    );
}

#[test]
fn issue3059_silent_shapes_stay_silent() {
    // LANG never warns; non-locale arithmetic binds never warn; a
    // nameref bind is keyed on the written name (silent, but the
    // target still lands); an element bind is keyed on `LC_ALL[0]`
    // (silent); a noeval-skipped bind never happens.
    let (_path, stdout, stderr, code) = run_script(
        "silent",
        "((LANG = 5))\n\
         ((x = 5))\n\
         ((x += 2))\n\
         ((x++))\n\
         declare -a LC_ALL\n\
         ((LC_ALL[0] = 5))\n\
         declare -n r=LC_CTYPE\n\
         ((r = 5))\n\
         ((0 && (LC_ALL = 9)))\n\
         echo \"lang=$LANG x=$x lc=${LC_ALL} ct=$LC_CTYPE a0=${LC_ALL[0]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "lang=5 x=8 lc=5 ct=5 a0=5\n");
    assert_eq!(stderr, "", "no setlocale warning in the silent shapes");
}

#[test]
fn issue3059_array_marked_name_scalar_bind_warns_with_element_value() {
    // An unsubscripted arithmetic bind of an array-marked locale name
    // lands on element 0; GNU's hook is keyed on the written name and
    // reads element 0 as the scalar value.
    let (path, stdout, stderr, code) = run_script(
        "arrayname",
        "declare -a LC_ALL\n((LC_ALL = 5))\necho \"v=${LC_ALL[0]}\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "v=5\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 2: warning: setlocale: LC_ALL: cannot change locale (5): No such file or directory\n"
        )
    );
}

#[test]
fn issue3059_noeval_skips_but_taken_branch_binds() {
    let (path, stdout, stderr, code) = run_script(
        "noeval",
        "((0 && (LC_ALL = 5)))\necho \"a=${LC_ALL-unset}\"\n((1 ? (LC_ALL = 7) : 0))\necho \"b=$LC_ALL\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "a=unset\nb=7\n");
    assert_eq!(
        stderr,
        format!(
            "{path}: line 3: warning: setlocale: LC_ALL: cannot change locale (7): No such file or directory\n"
        )
    );
}
