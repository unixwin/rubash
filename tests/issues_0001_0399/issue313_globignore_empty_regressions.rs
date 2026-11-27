//! Issue rubash#313 regressions (wt13/sweepfix lane).
//!
//! `GLOBIGNORE=` (set-but-null) wrongly enabled the implicit dotglob side
//! effect, so pathname expansion `*` matched leading-dot files while GNU
//! kept them excluded. Blast radius: bash-completion 2.18's
//! `_comp_expand_glob()` does `local GLOBIGNORE=""` before its
//! `eval "$1=($2)"` glob, so rubash's compat-directory scan included
//! `.gitignore` and the loader SOURCED `completions-core/.gitignore` as a
//! script — ~460 `No such file or directory` stderr lines on every
//! `source bash_completion` (GNU: zero).
//!
//! GNU contract (vendored third_party/bash):
//! - pathexp.c:507-515 setup_glob_ignore (reached via variables.c:5778
//!   `{ "GLOBIGNORE", sv_globignore }` at bind time):
//!   `v = get_string_value(name); setup_ignore_patterns(&globignore);`
//!   `if (globignore.num_ignores) glob_dot_filenames = 1;`
//!   `else if (v == 0) glob_dot_filenames = 0;`
//!   — a set-but-null value takes NEITHER branch: the dotglob cell keeps
//!   its previous value. num_ignores is >= 1 for any non-null value
//!   (setup_ignore_patterns early-returns on NULL/empty input,
//!   pathexp.c:611-614).
//! - The side effect and the `dotglob` shopt share ONE cell
//!   (glob_dot_filenames): `shopt -s/-u dotglob` writes it directly, a
//!   non-null GLOBIGNORE assignment writes 1, unsetting GLOBIGNORE writes
//!   0 (clobbering even an explicit `shopt -s dotglob`).
//!
//! Fixes:
//! - executor/glob.rs: globignore_assigned now requires a NON-NULL value
//!   (set-but-null contributes nothing at expansion time).
//! - executor/temporary_assignments.rs: a successful non-null GLOBIGNORE
//!   bind materializes the side effect into the dotglob option storage
//!   (the shared cell).
//! - builtins/set/unset.rs: unsetting GLOBIGNORE turns the cell off.
//!
//! Every expectation is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-29).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i313-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(dir.join("d")).expect("scratch dir");
    std::fs::write(dir.join("d/.hidden"), b"").expect("dotfile");
    std::fs::write(dir.join("d/visible"), b"").expect("file");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue matrix: A (set-but-null, dotglob off) is the bug; unset,
/// non-null, set-after-non-null, shopt interactions, and the `:` value
/// all match GNU.
#[test]
fn globignore_empty_keeps_dotglob_state() {
    let (stdout, stderr, code) = rubash_file(concat!(
        "cd d\n",
        "shopt -u dotglob\n",
        "GLOBIGNORE=\n",
        "echo \"A=[$(echo *)]\"\n",
        "unset GLOBIGNORE\n",
        "echo \"B=[$(echo *)]\"\n",
        "GLOBIGNORE=x\n",
        "echo \"C=[$(echo *)]\"\n",
        "GLOBIGNORE=\n",
        "echo \"D=[$(echo *)]\"\n",
        "shopt -s dotglob\n",
        "GLOBIGNORE=\n",
        "echo \"E=[$(echo *)]\"\n",
        "shopt -u dotglob\n",
        "GLOBIGNORE=*\n",
        "echo \"F=[$(echo *)]\"\n",
        "shopt -s dotglob\n",
        "unset GLOBIGNORE\n",
        "echo \"P1=[$(echo *)]\"\n",
        "shopt -u dotglob\n",
        "GLOBIGNORE=:\n",
        "echo \"P4=[$(echo *)]\"\n",
    ));
    let expected = concat!(
        "A=[visible]\n",
        "B=[visible]\n",
        "C=[.hidden visible]\n",
        "D=[.hidden visible]\n",
        "E=[.hidden visible]\n",
        "F=[*]\n",
        "P1=[visible]\n",
        "P4=[.hidden visible]\n",
    );
    assert_eq!(stdout, expected);
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// The bash-completion `_comp_expand_glob` shape: a `local GLOBIGNORE=""`,
/// then a glob through eval — the dotfile must stay excluded and nothing
/// errors.
#[test]
fn globignore_empty_local_guard_expands_without_dotfiles() {
    let (stdout, stderr, code) = rubash_file(concat!(
        "cd d\n",
        "shopt -u dotglob\n",
        "_comp_expand_glob() {\n",
        "  local GLOBIGNORE=\"\"\n",
        "  eval \"$1=($2)\"\n",
        "}\n",
        "_comp_expand_glob out '*'\n",
        "echo \"count=${#out[@]} first=[${out[0]}]\"\n",
    ));
    assert_eq!(stdout, "count=1 first=[visible]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// GLOBIGNORE pattern filtering itself keeps working for non-null values:
/// matched basenames drop, `.`/`..` always drop, and the dotfile side
/// effect applies.
#[test]
fn globignore_pattern_filtering_unchanged() {
    let (stdout, stderr, code) = rubash_file(concat!(
        "cd d\n",
        "shopt -u dotglob\n",
        "GLOBIGNORE=hidden\n",
        "echo \"A=[$(echo *)]\"\n",
        "GLOBIGNORE=.hidden\n",
        "echo \"B=[$(echo *)]\"\n",
    ));
    assert_eq!(stdout, "A=[.hidden visible]\nB=[visible]\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
