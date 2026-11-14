//! rubash#2658 — a readonly error inside a function is diagnosed
//! twice and does not abort the function (attr1.sub wholesale:
//! rubash's stderr is GNU's plus exactly the two duplicate lines at
//! the unquoted compound forms, `readonly r=(4)` and
//! `readonly -a r=(6)`).
//!
//! GNU ground (bash 5.3.20 oracle, script-file probes, byte-exact):
//! `readonly`/`export` live in builtins/setattr.def
//! (set_or_show_attributes); an unflagged assignment word goes
//! through do_assignment_no_expand (subst.c do_assignment family),
//! while `-a`/`-A` words are rewritten to `declare -g[rx][aA]` and
//! dispatched to declare_builtin. A compound assignment that
//! actually reaches the bind layer against a readonly global fails
//! ONCE (bind-layer err_readonly, error.c:453 — bare
//! `file: line N: name: readonly variable`) and the assignment
//! error unwinds the whole function-call chain (bypassing `||`,
//! callers do not continue), status 1, script resumes at top level
//! — the EX_BADASSIGN discipline (builtins/declare.def
//! declare_internal's `assign_error ? EX_BADASSIGN` return). A
//! scalar value is stopped by the declare.def:885 pre-check before
//! any bind, so it also diagnoses once but the function CONTINUES.
//! `declare`/`typeset`/`local` diagnose twice by design (local-copy
//! refusal at variables.c:2658, then the builtin's own pre-check —
//! the two sh_readonly layers, builtins/common.c:240) and never
//! unwind. Expectations below are the literal oracle outputs.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2658-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> Run {
    fs::write(dir.join("probe.sh"), script).expect("write probe script");
    let output = Command::new(RUBASH)
        .current_dir(dir)
        .arg("probe.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn check(tag: &str, script: &str, expected_stdout: &str, expected_stderr: &str) {
    let dir = scratch(tag);
    let run = run_script(&dir, script);
    assert_eq!(run.stdout, expected_stdout, "{tag}: stdout");
    assert_eq!(run.stderr, expected_stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(0), "{tag}: script rc");
}

/// The issue's minimal case: one diagnostic, function aborted at
/// the readonly compound assignment (no inner declare, no f-end),
/// f's status is 1, the outer variable is untouched.
#[test]
fn issue2658_compound_readonly_aborts_function_once() {
    check(
        "minimal",
        "r=(1); readonly r\nf() { readonly r=(4); declare -p r; echo \"f-end\"; }\nf\necho \"rc-after-f=$?\"\ndeclare -p r\n",
        "rc-after-f=1\ndeclare -ar r=([0]=\"1\")\n",
        "probe.sh: line 2: r: readonly variable\n",
    );
}

/// The `-a` flag form (attr1.sub line 40): setattr rewrites it to
/// `declare -gra`, so the single diagnostic carries the innermost
/// function name; the function still aborts.
#[test]
fn issue2658_dash_a_form_function_named_diagnostic() {
    check(
        "dash-a",
        "declare -a r=(1); readonly r\nf() { readonly -a r=(6); declare -p r; echo \"f-end\"; }\nf\necho \"rc-after-f=$?\"\ndeclare -p r\n",
        "rc-after-f=1\ndeclare -ar r=([0]=\"1\")\n",
        "probe.sh: line 2: f: r: readonly variable\n",
    );
}

/// The `-A` assoc form follows the same rewritten path.
#[test]
fn issue2658_dash_capital_a_assoc_form() {
    check(
        "dash-A",
        "declare -A m=( [k]=1 ); readonly m\nf() { readonly -A m=([k]=2); echo \"f-end\"; }\nf\necho \"rc=$?\"\ndeclare -p m\n",
        "rc=1\ndeclare -Ar m=([k]=\"1\" )\n",
        "probe.sh: line 2: f: m: readonly variable\n",
    );
}

/// A compound value aimed at a readonly SCALAR global also reaches
/// the bind layer: one bare diagnostic, function aborts.
#[test]
fn issue2658_compound_value_on_scalar_target_aborts() {
    check(
        "scalar-target",
        "r=1; readonly r\nf() { readonly r=(4); echo \"f-end\"; }\nf\necho \"rc=$?\"\ndeclare -p r\n",
        "rc=1\ndeclare -r r=\"1\"\n",
        "probe.sh: line 2: r: readonly variable\n",
    );
}

/// `export` shares setattr's machinery: compound value on a
/// readonly array aborts the same way.
#[test]
fn issue2658_export_compound_aborts() {
    check(
        "export",
        "r=(1); readonly r\nf() { export r=(4); echo \"f-end\"; }\nf\necho \"rc=$?\"\n",
        "rc=1\n",
        "probe.sh: line 2: r: readonly variable\n",
    );
}

/// Chain form: the unwind crosses every function frame — the
/// caller's continuation never runs — and the diagnostic names the
/// innermost function where the error occurred.
#[test]
fn issue2658_unwind_crosses_all_frames() {
    check(
        "nested",
        "r=(1); readonly r\nmyfunc() { readonly -a r=(6); echo \"mf-end\"; }\nouter() { myfunc; echo \"outer-end rc=$?\"; }\nouter\necho \"rc=$?\"\n",
        "rc=1\n",
        "probe.sh: line 2: myfunc: r: readonly variable\n",
    );
}

/// The abort is an unwind, not a failure status: `||` cannot catch
/// it and the rest of the function body never runs.
#[test]
fn issue2658_unwind_bypasses_or_list() {
    check(
        "or-bypass",
        "r=(1); readonly r\nf() { readonly r=(4) || echo \"caught\"; echo \"after rc=$?\"; }\nf\necho \"rc=$?\"\n",
        "rc=1\n",
        "probe.sh: line 2: r: readonly variable\n",
    );
}

/// A bare compound assignment statement against a readonly array
/// is the sibling face of the same assignment-error unwind.
#[test]
fn issue2658_bare_compound_assignment_aborts() {
    check(
        "bare-assign",
        "r=(1); readonly r\nf() { r=(4); echo \"f-end\"; }\nf\necho \"rc=$?\"\ndeclare -p r\n",
        "rc=1\ndeclare -ar r=([0]=\"1\")\n",
        "probe.sh: line 2: r: readonly variable\n",
    );
}

/// Control: a SCALAR readonly assignment inside a function
/// diagnoses once and the function CONTINUES (the pre-check stops
/// it before the bind layer; no unwind).
#[test]
fn issue2658_scalar_value_diagnoses_once_and_continues() {
    check(
        "scalar-value",
        "r=1; readonly r\nf() { readonly r=4; declare -p r; echo \"f-end\"; }\nf\necho \"rc-after-f=$?\"\ndeclare -p r\n",
        "declare -r r=\"1\"\nf-end\nrc-after-f=0\ndeclare -r r=\"1\"\n",
        "probe.sh: line 2: r: readonly variable\n",
    );
}

/// Control: a QUOTED compound string is a scalar value by the time
/// it is assigned — one diagnostic, function continues (this is
/// attr1.sub's `'(5)'` form, already byte-identical in the
/// wholesale run).
#[test]
fn issue2658_quoted_compound_is_scalar_path() {
    check(
        "quoted",
        "r=(1); readonly r\nf() { readonly r='(5)'; declare -p r; echo \"f-end\"; }\nf\necho \"rc-after-f=$?\"\ndeclare -p r\n",
        "declare -ar r=([0]=\"1\")\nf-end\nrc-after-f=0\ndeclare -ar r=([0]=\"1\")\n",
        "probe.sh: line 2: r: readonly variable\n",
    );
}

/// Contrast guard: `declare` (not readonly/export) on the same
/// readonly global diagnoses TWICE — function-named layer, then
/// builtin-named layer — and does NOT unwind. A fix for this issue
/// must not collapse declare's two layers or make declare abort.
#[test]
fn issue2658_declare_contrast_two_diagnostics_no_abort() {
    check(
        "declare-contrast",
        "r=(1)\nreadonly r\nf() {\n  declare r=(4)\n  echo \"f-end\"\n}\nf\necho \"rc=$?\"\n",
        "f-end\nrc=0\n",
        "probe.sh: line 4: f: r: readonly variable\nprobe.sh: line 4: declare: r: readonly variable\n",
    );
}

/// Top level: both the compound and the scalar readonly errors
/// diagnose once each and the script continues (no exit in
/// non-POSIX mode); each failing command's status is 1.
#[test]
fn issue2658_top_level_errors_diagnose_once_and_continue() {
    check(
        "top-level",
        "r=(1); readonly r\nreadonly r=(4)\necho \"after-compound rc=$?\"\nr2=1; readonly r2\nreadonly r2=4\necho \"after-scalar rc=$?\"\ndeclare -p r\ndeclare -p r2\n",
        "after-compound rc=1\nafter-scalar rc=1\ndeclare -ar r=([0]=\"1\")\ndeclare -r r2=\"1\"\n",
        "probe.sh: line 2: r: readonly variable\nprobe.sh: line 5: r2: readonly variable\n",
    );
}
