//! rubash#3487 — bare `declare` (and bare `set`) omit the function
//! definitions GNU prints.
//!
//! GNU ground: with no operands, no `-p`, and no minus-form attribute
//! options, `declare` dispatches to `set_builtin ((WORD_LIST *)NULL)`
//! (builtins/declare.def, the `list == 0` / `flags_on == 0` arm), whose
//! listing is print_all_shell_variables (builtins/set.def:506-527):
//! print_var_list over all_shell_variables first, then — only when
//! posixly_correct == 0 — print_func_list over all_shell_functions.
//! print_func_list (variables.c:1079-1090) prints, per function,
//! `name ` + named_function_string (FUNC_MULTILINE|FUNC_EXTERNAL,
//! variables.c:1152-1162) + newline: the multiline form
//! `name () \n{ \n    body\n}` with definition redirects after the
//! closing brace and subshell bodies inside the braces. The function
//! table order in the listing is name order, after ALL variables
//! (a variable named `zzzz_last` still precedes a function `afunc`).
//!
//! The `-p` listing is a different enumerator (show_all_var_attributes)
//! and never carries function definitions; POSIX mode drops the
//! function section entirely (PASC Interp #202, set.def:518-520).
//!
//! Rubash's bare `declare` never consulted the function table
//! (ShellState.functions): the builtin's plain branch printed the
//! variable half only. Bare `set` printed the function half only
//! because execute_set appended it after the builtin returned
//! (rubash#1056) — an executor-side patch on one of the two routes
//! into the shared enumerator. The enumerator now prints both halves
//! itself, exactly like GNU's single print_all_shell_variables, and
//! the executor append is gone. Expectations byte-verified against
//! the GNU bash 5.3.20 oracle with clean-environment script-file
//! probes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3487-{tag}-{}", std::process::id()));
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

/// Index of the first line exactly equal to `line`.
fn line_index(out: &str, line: &str) -> Option<usize> {
    out.lines().position(|candidate| candidate == line)
}

/// Assert the consecutive block `lines` appears in `out`, returning the
/// index of its first line.
fn block_index(out: &str, lines: &[&str]) -> Option<usize> {
    let all: Vec<&str> = out.lines().collect();
    all.windows(lines.len()).position(|window| window == lines)
}

const SEED: &str = "\
myfn() { echo body; }
some_var=42
";

#[test]
fn bare_declare_prints_function_definition_block() {
    let dir = scratch("seed");
    let (out, err, ok) = run_full(&dir, &format!("{SEED}declare\n"));
    assert!(ok, "bare declare failed: {err}");
    assert!(
        line_index(&out, "some_var=42").is_some(),
        "variable line missing from bare declare:\n{out}"
    );
    assert!(
        block_index(&out, &["myfn () ", "{ ", "    echo body", "}"]).is_some(),
        "function definition block missing from bare declare:\n{out}"
    );
}

#[test]
fn bare_declare_functions_sorted_and_after_all_variables() {
    let dir = scratch("order");
    let script = "\
zfunc() { echo zbody; }
afunc() { echo abody; }
mfunc() { x=1; echo mbody; }
zzzz_last=1
declare
";
    let (out, err, ok) = run_full(&dir, script);
    assert!(ok, "bare declare failed: {err}");
    let var = line_index(&out, "zzzz_last=1").expect("zzzz_last line missing");
    let a = block_index(&out, &["afunc () ", "{ ", "    echo abody", "}"])
        .expect("afunc block missing");
    let m = block_index(
        &out,
        &["mfunc () ", "{ ", "    x=1;", "    echo mbody", "}"],
    )
    .expect("mfunc block missing");
    let z = block_index(&out, &["zfunc () ", "{ ", "    echo zbody", "}"])
        .expect("zfunc block missing");
    assert!(
        var < a && a < m && m < z,
        "functions must follow all variables in name order:\n{out}"
    );
}

#[test]
fn bare_set_prints_function_definitions_too() {
    let dir = scratch("set");
    let (out, err, ok) = run_full(&dir, &format!("{SEED}set\n"));
    assert!(ok, "bare set failed: {err}");
    assert!(
        block_index(&out, &["myfn () ", "{ ", "    echo body", "}"]).is_some(),
        "function definition block missing from bare set:\n{out}"
    );
}

#[test]
fn bare_declare_posix_mode_omits_functions() {
    let dir = scratch("posix");
    let script = format!("{SEED}set -o posix\ndeclare\n");
    let (out, err, ok) = run_full(&dir, &script);
    assert!(ok, "posix bare declare failed: {err}");
    assert!(
        line_index(&out, "some_var=42").is_some(),
        "variable line missing in posix mode:\n{out}"
    );
    assert!(
        line_index(&out, "myfn () ").is_none(),
        "POSIX mode must omit the function section:\n{out}"
    );
}

#[test]
fn bare_declare_function_shapes() {
    let dir = scratch("shapes");
    let script = "\
sfn() ( echo sub )
dfn() { echo dbody; } 2>/dev/null
function kfn { echo kbody; }
declare
";
    let (out, err, ok) = run_full(&dir, script);
    assert!(ok, "bare declare failed: {err}");
    assert!(
        block_index(&out, &["dfn () ", "{ ", "    echo dbody", "} 2> /dev/null"]).is_some(),
        "definition-redirect shape wrong:\n{out}"
    );
    assert!(
        block_index(&out, &["kfn () ", "{ ", "    echo kbody", "}"]).is_some(),
        "keyword-form shape wrong:\n{out}"
    );
    assert!(
        block_index(&out, &["sfn () ", "{ ", "    ( echo sub )", "}"]).is_some(),
        "subshell-body shape wrong:\n{out}"
    );
}

#[test]
fn declare_p_listing_omits_function_definitions() {
    let dir = scratch("declarep");
    let (out, err, ok) = run_full(&dir, &format!("{SEED}declare -p\n"));
    assert!(ok, "declare -p failed: {err}");
    assert!(
        out.lines()
            .any(|line| line.starts_with("declare") && line.contains("some_var")),
        "declare -p variable line missing:\n{out}"
    );
    assert!(
        line_index(&out, "myfn () ").is_none(),
        "declare -p must not print function definitions:\n{out}"
    );
}

#[test]
fn bare_declare_after_unset_f_omits_function() {
    let dir = scratch("unsetf");
    let script = format!("{SEED}unset -f myfn\ndeclare\n");
    let (out, err, ok) = run_full(&dir, &script);
    assert!(ok, "bare declare after unset -f failed: {err}");
    assert!(
        line_index(&out, "some_var=42").is_some(),
        "variable line missing:\n{out}"
    );
    assert!(
        line_index(&out, "myfn () ").is_none(),
        "unset function must leave the listing:\n{out}"
    );
}

#[test]
fn declare_f_still_prints_definition() {
    let dir = scratch("declaref");
    let (out, err, ok) = run_full(&dir, &format!("{SEED}declare -f myfn\n"));
    assert!(ok, "declare -f failed: {err}");
    assert!(
        block_index(&out, &["myfn () ", "{ ", "    echo body", "}"]).is_some(),
        "declare -f output regressed:\n{out}"
    );
}
