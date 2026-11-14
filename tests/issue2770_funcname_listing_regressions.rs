//! rubash#2770 — FUNCNAME has no listing view in declare/set output.
//!
//! GNU ground: FUNCNAME is a real dynamic array cell
//! (variables.c:1835-1839 init_funcname_var: INIT_DYNAMIC_ARRAY_VAR
//! with att_invisible|att_noassign); make_funcname_visible
//! (variables.c:1812-1824) clears att_invisible at function entry.
//! So the listing forms expose it exactly like any other variable
//! of its class:
//!
//! * `declare -p FUNCNAME` / `typeset -p FUNCNAME` — named: bare
//!   `declare -a FUNCNAME` at depth 0 (show_var_attributes,
//!   builtins/setattr.def:510-511 prints an invisible array as its
//!   bare name), `declare -a FUNCNAME=([0]="f" [1]="main")` inside
//!   a function, and `FUNCNAME: not found` (rc 1) after `unset`.
//! * `declare -p` / `declare -a` listings — the bare form at depth
//!   0, the live stack inside a function (the enumeration walks the
//!   registry cell, variables.c:1844 initialize_dynamic_variables).
//! * bare `declare` / `set` — print_var_list skips att_invisible
//!   cells (variables.c:1066-1073), so no line at depth 0; the live
//!   stack renders inside a function.
//! * Filters adjudicate by the cell's own class: `-a` admits it,
//!   `-i` does not; `declare -p FUNCNAME[0]` and `local -p FUNCNAME`
//!   are both "not found" (rc 1) — a subscript is not a name, and
//!   local -p only reports current-frame locals
//!   (builtins/declare.def:555-570 show_localname_attributes).
//!
//! Rubash modeled FUNCNAME virtually (parameter_array_storage) and
//! the named print was rescued by a seed hack (rubash#183), but the
//! listings rendered the init mirror — a birth snapshot ("") function
//! entry never refreshes — so every listing form inside a function
//! showed the stale valueless cell (or nothing, for bare
//! declare/set), and the seed fabricated a cell after `unset`.
//! The listings are now composed from the live provider view, the
//! same registry-driven direction as rubash#2700: the dynamic
//! declaration view renders the composed stack, the cell view
//! feeds the -p/filtered listings, and the assignment-form supply
//! feeds bare `declare`/`set` (non-empty stacks only).
//!
//! Expectations byte-verified against the GNU bash 5.3 oracle with
//! script-file probes (probe2770.sh / probe2770c.sh, run through the
//! vendored 5.3 build). The probe scripts deliberately avoid
//! `export -p FUNCNAME` / `readonly -p FUNCNAME` before the listing
//! cells: GNU's `-p` with an operand SETS the attribute as a side
//! effect (`declare -ax` / `declare -ar` afterwards), which would
//! pollute every later listing expectation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2770-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_full(dir: &Path, script: &str) -> (String, String, bool) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm")
        .env("HOME", dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

fn has_line(out: &str, line: &str) -> bool {
    out.lines().any(|candidate| candidate == line)
}

#[test]
fn issue2770_funcname_named_and_listing_forms_at_top_level() {
    let dir = scratch("top");
    // N1: named -p at depth 0 renders the bare invisible array.
    let (out, err, ok) = run_full(&dir, "declare -p FUNCNAME\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert_eq!(out, "declare -a FUNCNAME\n", "declare -p FUNCNAME: {out}");
    // N2: typeset -p is the same builtin.
    let (out, err, ok) = run_full(&dir, "typeset -p FUNCNAME\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert_eq!(out, "declare -a FUNCNAME\n", "typeset -p FUNCNAME: {out}");
    // L1/L2: the -p and -a listings include the bare form.
    let (out, err, ok) = run_full(&dir, "declare -p\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        has_line(&out, "declare -a FUNCNAME"),
        "declare -p listing: {out}"
    );
    let (out, err, ok) = run_full(&dir, "declare -a\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        has_line(&out, "declare -a FUNCNAME"),
        "declare -a listing: {out}"
    );
    // A1/A2: bare declare / set skip the invisible cell at depth 0.
    let (out, err, ok) = run_full(&dir, "declare\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        !out.lines().any(|line| line.starts_with("FUNCNAME=")),
        "bare declare must not list FUNCNAME at depth 0: {out}"
    );
    let (out, err, ok) = run_full(&dir, "set\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        !out.lines().any(|line| line.starts_with("FUNCNAME=")),
        "set must not list FUNCNAME at depth 0: {out}"
    );
    // F1: the -i filter excludes the non-integer array.
    let (out, err, ok) = run_full(&dir, "declare -i\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        !out.lines().any(|line| line.contains("FUNCNAME")),
        "declare -i must not list FUNCNAME: {out}"
    );
    // S1: a subscript operand is not a name.
    let (out, err, ok) = run_full(&dir, "declare -p FUNCNAME[0]\necho \"rc=$?\"\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(out, "rc=1\n", "declare -p FUNCNAME[0] rc: {out}");
    assert!(
        err.contains("FUNCNAME[0]: not found"),
        "declare -p FUNCNAME[0] diagnostic: {err}"
    );
}

#[test]
fn issue2770_funcname_listing_forms_inside_a_function() {
    let dir = scratch("in-function");
    let live = "declare -a FUNCNAME=([0]=\"f\" [1]=\"main\")";
    let assigned = "FUNCNAME=([0]=\"f\" [1]=\"main\")";
    // N3: named -p renders the live stack.
    let (out, err, ok) = run_full(&dir, "f() { declare -p FUNCNAME; }\nf\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert_eq!(out, format!("{live}\n"), "declare -p FUNCNAME in f: {out}");
    // N4: typeset -p is the same builtin.
    let (out, err, ok) = run_full(&dir, "f() { typeset -p FUNCNAME; }\nf\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert_eq!(out, format!("{live}\n"), "typeset -p FUNCNAME in f: {out}");
    // L3/L4: the -p and -a listings render the live stack, not the
    // stale valueless mirror cell.
    let (out, err, ok) = run_full(&dir, "f() { declare -p; }\nf\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(has_line(&out, live), "declare -p listing in f: {out}");
    let (out, err, ok) = run_full(&dir, "f() { declare -a; }\nf\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(has_line(&out, live), "declare -a listing in f: {out}");
    // A3/A4: bare declare / set render the live stack once visible.
    let (out, err, ok) = run_full(&dir, "f() { declare; }\nf\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(has_line(&out, assigned), "bare declare in f: {out}");
    let (out, err, ok) = run_full(&dir, "f() { set; }\nf\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(has_line(&out, assigned), "set in f: {out}");
    // S2: local -p only reports current-frame locals.
    let (out, err, ok) = run_full(&dir, "f() { local -p FUNCNAME; echo \"rc=$?\"; }\nf\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(out, "rc=1\n", "local -p FUNCNAME rc: {out}");
    assert!(
        err.contains("local: FUNCNAME: not found"),
        "local -p FUNCNAME diagnostic: {err}"
    );
    // N5: the nested stack composes (g, h, main).
    let (out, err, ok) = run_full(&dir, "g() { declare -p FUNCNAME; }\nh() { g; }\nh\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert_eq!(
        out, "declare -a FUNCNAME=([0]=\"g\" [1]=\"h\" [2]=\"main\")\n",
        "declare -p FUNCNAME nested: {out}"
    );
    // N6: after return the array is bare again.
    let (out, err, ok) = run_full(&dir, "f() { :; }\nf\ndeclare -p FUNCNAME\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert_eq!(
        out, "declare -a FUNCNAME\n",
        "declare -p FUNCNAME after return: {out}"
    );
}

#[test]
fn issue2770_funcname_unset_removes_the_listing_view() {
    let dir = scratch("unset");
    // U1: after unbind, the named form reports not found — the cell
    // is gone for the rest of the shell's lifetime (variables.c:3839).
    let (out, err, ok) = run_full(
        &dir,
        "unset FUNCNAME\ndeclare -p FUNCNAME\necho \"rc=$?\"\n",
    );
    assert!(ok, "stderr: {err}");
    assert_eq!(out, "rc=1\n", "declare -p FUNCNAME after unset rc: {out}");
    assert!(
        err.contains("FUNCNAME: not found"),
        "declare -p FUNCNAME after unset diagnostic: {err}"
    );
    // U2/U3: the listings lose the name too. (U2 asserts on the
    // anchored declaration line, not a substring: `_` still holds
    // the unset operand "FUNCNAME" here, and its own
    // `declare -- _="FUNCNAME"` line is a different variable.)
    let (out, err, ok) = run_full(&dir, "unset FUNCNAME\ndeclare -p\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        !out.lines()
            .any(|line| line.starts_with("declare -a FUNCNAME")),
        "declare -p listing after unset: {out}"
    );
    let (out, err, ok) = run_full(&dir, "unset FUNCNAME\ndeclare -a\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        !out.lines().any(|line| line.contains("FUNCNAME")),
        "declare -a listing after unset: {out}"
    );
    let (out, err, ok) = run_full(&dir, "unset FUNCNAME\nset\n");
    assert!(ok, "stderr: {err}");
    assert_eq!(err, "", "stderr: {err}");
    assert!(
        !out.lines().any(|line| line.starts_with("FUNCNAME=")),
        "set after unset: {out}"
    );
}
