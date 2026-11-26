//! Issue unixwin/rubash#2520: inside a `<(...)` process substitution
//! child, `for VAR do ... done` (no `in` clause) iterated zero times over
//! a function's positional parameters while `$#` stayed correct.
//!
//! Root cause: GNU materializes the no-`in` for/select word list at parse
//! time as the literal quoted word `"$@"` (`parse.y:895` / `parse.y:981`,
//! `add_string_to_list ("\"$@\"", NULL)`), so `print_for_command_head`
//! (`print_cmd.c:602`) and `print_select_command_head`
//! (`print_cmd.c:656`) print `for VAR in "$@"`. Rubash's canonical
//! printer (`src/parser/ast_print.rs print_for_command`) printed the
//! no-`in` form with an EMPTY word list (`for VAR in ;`), which re-parses
//! as an explicit empty list — zero iterations. The divergence only
//! surfaced across a respawn boundary: a process-substitution child
//! (`subst.c:6362 process_substitute`) re-imports its function table from
//! `BASH_FUNC_name%%` env entries (built via `exported_function_string`,
//! mirroring `variables.c:4875 make_env_array_from_var_list`), so the
//! mangled body executed in the child. Same-process, subshell, command
//! substitution, and `&` backgrounds were unaffected (no env roundtrip).
//!
//! Explicit `for VAR in ;` is different in GNU (`parse.y:937` passes a
//! NULL word list) and must keep iterating zero times; that form keeps
//! `default_positional = false` and still prints empty.
//!
//! Verified against WSL GNU Bash 5.3.20 (script-file probes,
//! `MSYS_NO_PATHCONV=1 wsl bash <file>`): every expected byte string
//! below was taken from the GNU run.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Drain an fd written by a procsub producer: run the script under a
/// shell, assert stdout matches the GNU byte string exactly.
fn assert_gnu_bytes(script: &str, expected: &str, what: &str) {
    let (stdout, stderr, code) = rubash_stdin(script);
    assert_eq!(code, Some(0), "{what} exit: stderr: {stderr}");
    assert_eq!(
        stdout, expected,
        "{what} stdout mismatch (stderr: {stderr})"
    );
}

#[test]
fn issue_reproducer_for_do_over_function_positionals() {
    // GNU: `L: X=a`, `L: X=b`, `L: X=c`, `L: argc=3`.
    assert_gnu_bytes(
        concat!(
            "fn() { for A do echo \"X=$A\"; done; echo \"argc=$#\"; }\n",
            "exec 9< <( fn a b c )\n",
            "while IFS= read -r l; do echo \"L: $l\"; done <&9\n",
            "exec 9<&-\n",
        ),
        "L: X=a\nL: X=b\nL: X=c\nL: argc=3\n",
        "issue 2520 reproducer",
    );
}

#[test]
fn procsub_function_at_and_first_positional_direct_read() {
    // `$@` / `$1` direct reads already crossed the respawn boundary
    // before the fix (the transport was fine; only the for-loop word
    // list was lost). GNU: `at=[a b c] one=[a] argc=3`.
    assert_gnu_bytes(
        concat!(
            "fn2() { echo \"at=[$@] one=[$1] argc=$#\"; }\n",
            "exec 9< <( fn2 a b c )\n",
            "while IFS= read -r l; do echo \"R: $l\"; done <&9\n",
            "exec 9<&-\n",
        ),
        "R: at=[a b c] one=[a] argc=3\n",
        "procsub $@/$1 direct read",
    );
}

#[test]
fn procsub_function_set_args_then_for_do() {
    // A function that materializes its OWN positionals with `set --`
    // and then runs the no-`in` for loop — the decisive bisect case:
    // zero iterations before the fix because the imported body was
    // `for A in ;`. GNU: `X=x`, `X=y`.
    assert_gnu_bytes(
        concat!(
            "fn4() { set -- x y; for A do echo \"X=$A\"; done; }\n",
            "exec 9< <( fn4 )\n",
            "while IFS= read -r l; do echo \"S: $l\"; done <&9\n",
            "exec 9<&-\n",
        ),
        "S: X=x\nS: X=y\n",
        "set -- inside procsub function",
    );
}

#[test]
fn procsub_toplevel_positionals_for_do() {
    // The parent's positional window crosses to the procsub child and
    // the TOP-LEVEL no-`in` for loop iterates it (this path worked
    // before the fix; pins it against regressions of the transport).
    // GNU: `n=2`, `TB=t1`, `TB=t2`.
    assert_gnu_bytes(
        concat!(
            "set -- t1 t2\n",
            "exec 9< <( echo \"n=$#\"; for B do echo \"TB=$B\"; done )\n",
            "while IFS= read -r l; do echo \"T: $l\"; done <&9\n",
            "exec 9<&-\n",
        ),
        "T: n=2\nT: TB=t1\nT: TB=t2\n",
        "toplevel positionals in procsub child",
    );
}

#[test]
fn explicit_empty_in_list_still_zero_iterations() {
    // `for A in ;` is a different GNU production (parse.y:937, NULL word
    // list) — zero iterations even with positionals set — and must keep
    // printing/rendering as an empty list. GNU: only `end`.
    assert_gnu_bytes(
        concat!(
            "fn5() { for A in ; do echo \"E=$A\"; done; }\n",
            "fn5 a b c\n",
            "echo end\n",
        ),
        "end\n",
        "explicit empty in-list",
    );
}

#[test]
fn declare_f_renders_no_in_for_as_dollar_at() {
    // GNU `declare -f` output for the no-`in` form (print_cmd.c:602,
    // FUNC_MULTILINE form): `for A in "$@";`.
    let (stdout, stderr, code) = rubash_stdin(concat!(
        "fn3() { for A do echo \"X=$A\"; done; }\n",
        "declare -f fn3\n",
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        concat!(
            "fn3 () \n",
            "{ \n",
            "    for A in \"$@\";\n",
            "    do\n",
            "        echo \"X=$A\";\n",
            "    done\n",
            "}\n",
        ),
        "declare -f no-in form (GNU: `for A in \"$@\";`)"
    );
}

#[test]
fn export_f_env_entry_renders_dollar_at_word() {
    // The BASH_FUNC_name%% env value is the respawn transport (the
    // procsub child re-parses this exact text). GNU renders the no-`in`
    // for and select word lists as the literal `"$@"` word
    // (parse.y:895 / parse.y:981).
    let (stdout, stderr, code) = rubash_stdin(concat!(
        "fn3() { for A do echo \"X=$A\"; done; }\n",
        "fs() { select S do echo \"S=$S\"; break; done; }\n",
        "export -f fn3 fs\n",
        "env | grep '^BASH_FUNC_' | sort\n",
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let for_line = stdout
        .lines()
        .find(|l| l.starts_with("BASH_FUNC_fn3%%"))
        .unwrap_or_else(|| panic!("no fn3 entry in {stdout}"));
    let select_line = stdout
        .lines()
        .find(|l| l.starts_with("BASH_FUNC_fs%%"))
        .unwrap_or_else(|| panic!("no fs entry in {stdout}"));
    assert_eq!(for_line, "BASH_FUNC_fn3%%=() {  for A in \"$@\";");
    assert_eq!(select_line, "BASH_FUNC_fs%%=() {  select S in \"$@\";");
}

#[test]
fn coproc_child_calls_function_with_for_do() {
    // The coproc spawn shares the BASH_FUNC function-table transport
    // (init.rs BASH_FUNC import). GNU: `X=c1`, `X=c2` in the file, then
    // echoed. Uses a regular file so the assertion is not an
    // interactive-pipe read race.
    let (stdout, stderr, code) = rubash_stdin(concat!(
        "fn() { for A do echo \"X=$A\"; done; }\n",
        "coproc CP { fn c1 c2 > coproc2520.out; }\n",
        "wait\n",
        "cat coproc2520.out\n",
    ));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "X=c1\nX=c2\n", "stderr: {stderr}");
}

#[test]
fn procsub_select_no_in_reads_function_positionals() {
    // select with no `in` clause shares the parse.y:981 `"$@"`
    // materialization. With stdin at EOF the loop exits after printing
    // the menu once; the menu items are the FUNCTION's positionals —
    // before the fix the imported body was `select S in ;` (empty list),
    // which prints no menu at all. GNU (WSL 5.3.20, script-file probe):
    // stdout `V: \n`, stderr `1) a\n2) b\n#? ` (trailing space, no
    // newline), exit 0 — byte-identical.
    let (stdout, stderr, code) = rubash_stdin(concat!(
        "fs() { select S do echo \"S=$S\"; done; }\n",
        "exec 9< <( fs a b < /dev/null )\n",
        "while IFS= read -r l; do echo \"V: $l\"; done <&9\n",
        "exec 9<&-\n",
    ));
    assert_eq!(code, Some(0), "stdout: {stdout}");
    assert_eq!(stdout, "V: \n", "stdout: {stdout}");
    assert_eq!(
        stderr, "1) a\n2) b\n#? ",
        "select menu from function positionals"
    );
}
