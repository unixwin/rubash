//! rubash#2770 — FUNCNAME has no listing view in declare/set
//! output (split from rubash#2700, whose PR #2766 made the
//! declare/set listings registry-driven for the dynamic variables
//! but left FUNCNAME out).
//!
//! GNU ground (bash 5.3.20 oracle, script-file probes, byte-exact):
//! FUNCNAME is a real array cell in the variable table
//! (variables.c:1839 initialize_dynamic_variables region), so
//! print_var_list enumerates it like the other frame arrays — but
//! its shape depends on the frame stack:
//!   * outside any function the cell exists with no elements:
//!     `declare -a`, `declare -p` and even `declare -p FUNCNAME`
//!     render the VALUELESS line `declare -a FUNCNAME` (rc 0),
//!     while the assignment-form listings (bare `declare`, `set`)
//!     omit it entirely;
//!   * inside a function every listing form shows the live frame
//!     composition: bare `declare`/`set` print
//!     `FUNCNAME=([0]="f" [1]="main")`, the declare forms print
//!     `declare -a FUNCNAME=([0]="f" [1]="main")`.
//! Rubash composes FUNCNAME for expansion, the test -v overlay
//! and the named `declare -p FUNCNAME` form, but the ENUMERATION
//! supply (dynamic_declaration_class / dynamic_set_views in
//! src/executor/dynamic_arrays.rs omit the name) renders it as a
//! static valueless entry in the declare forms and omits it from
//! the assignment forms — regardless of the live frame stack.
//! Expectations are the literal oracle outputs.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2770-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> (String, Option<i32>) {
    fs::write(dir.join("probe.sh"), script).expect("write probe script");
    let output = Command::new(RUBASH)
        .current_dir(dir)
        .arg("probe.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

fn has_exact_line(out: &str, want: &str) -> bool {
    out.lines().any(|line| line == want)
}

fn has_line_prefix(out: &str, prefix: &str) -> bool {
    out.lines().any(|line| line.starts_with(prefix))
}

/// Top level: the declare-format listings expose the valueless
/// cell; the assignment-form listings omit the name entirely.
#[test]
fn issue2770_top_level_listing_forms() {
    let dir = scratch("toplevel");
    let (out, _) = run_script(&dir, "declare -a\n");
    assert!(
        has_exact_line(&out, "declare -a FUNCNAME"),
        "declare -a must render the valueless FUNCNAME cell: {out}"
    );
    let (out, _) = run_script(&dir, "declare -p\n");
    assert!(
        has_exact_line(&out, "declare -a FUNCNAME"),
        "declare -p must render the valueless FUNCNAME cell: {out}"
    );
    let (out, code) = run_script(&dir, "declare -p FUNCNAME\n");
    assert_eq!(out, "declare -a FUNCNAME\n", "declare -p FUNCNAME output");
    assert_eq!(code, Some(0), "declare -p FUNCNAME rc");
    let (out, _) = run_script(&dir, "declare\n");
    assert!(
        !has_line_prefix(&out, "FUNCNAME"),
        "bare declare must not list FUNCNAME outside a function: {out}"
    );
    let (out, _) = run_script(&dir, "set\n");
    assert!(
        !has_line_prefix(&out, "FUNCNAME="),
        "set must not list FUNCNAME outside a function: {out}"
    );
}

/// Inside a function: the assignment-form listings carry the live
/// frame composition.
#[test]
fn issue2770_in_function_assignment_listings() {
    let dir = scratch("infunc-assign");
    let (out, _) = run_script(&dir, "f() { local x=1; declare; }\nf\n");
    assert!(
        has_exact_line(&out, "FUNCNAME=([0]=\"f\" [1]=\"main\")"),
        "bare declare inside f must list FUNCNAME with its frames: {out}"
    );
    let (out, _) = run_script(&dir, "f() { local x=1; set; }\nf\n");
    assert!(
        has_exact_line(&out, "FUNCNAME=([0]=\"f\" [1]=\"main\")"),
        "set inside f must list FUNCNAME with its frames: {out}"
    );
}

/// Inside a function: the declare forms carry the same live
/// composition, including the named `-p` form.
#[test]
fn issue2770_in_function_declare_forms() {
    let dir = scratch("infunc-declare");
    let (out, code) = run_script(&dir, "f() { declare -p FUNCNAME; }\nf\n");
    assert_eq!(
        out, "declare -a FUNCNAME=([0]=\"f\" [1]=\"main\")\n",
        "declare -p FUNCNAME inside f"
    );
    assert_eq!(code, Some(0), "declare -p FUNCNAME rc");
    let (out, _) = run_script(&dir, "g() { declare -a; }\nf() { g; }\nf\n");
    assert!(
        has_exact_line(
            &out,
            "declare -a FUNCNAME=([0]=\"g\" [1]=\"f\" [2]=\"main\")"
        ),
        "declare -a in a nested frame must show the whole stack: {out}"
    );
}

/// Chain form: the listing follows the frame stack as it changes —
/// valued inside the call, valueless again after the return.
#[test]
fn issue2770_listing_tracks_frame_lifecycle() {
    let dir = scratch("lifecycle");
    let (out, code) = run_script(
        &dir,
        "f() { declare -p FUNCNAME; }\nf\ndeclare -p FUNCNAME\n",
    );
    assert_eq!(
        out, "declare -a FUNCNAME=([0]=\"f\" [1]=\"main\")\ndeclare -a FUNCNAME\n",
        "FUNCNAME listing inside vs after the call"
    );
    assert_eq!(code, Some(0), "script rc");
}
