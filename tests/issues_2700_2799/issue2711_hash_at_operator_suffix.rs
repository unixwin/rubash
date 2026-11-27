//! rubash#2711 — `${#@...}` / `${#*...}` with an operator suffix is
//! silently accepted instead of bad substitution.
//!
//! GNU: the `#` length prefix is accepted only when the rest of the
//! body is exactly one length operand. parameter_brace_expand
//! (subst.c:9931-9941): a `#`-led name that does not consume the whole
//! body up to `}`, or that fails valid_length_expression
//! (subst.c:8244-8253 — `${#}`, one special-parameter char, all
//! digits, a valid array reference, or a valid identifier), goes to
//! bad_substitution (subst.c:10273-10288: set_exit_status
//! (EXECUTION_FAILURE), report, error sentinel). `${#@}` / `${#*}`
//! themselves are the positional count (subst.c:8271). Any operator
//! text after the `@`/`*` — substring `:`, removal `%`, default `:-` —
//! is therefore a bad substitution and the non-interactive shell
//! exits 1 without running the command.
//!
//! rubash at base: prints an empty line, rc 0 — the braced walk
//! accepts the `#@`/`#*` head and silently discards the trailing
//! operator text (no bad-substitution verdict from the length gate in
//! expand_braced_indices.rs / the substring dispatch in
//! parameter_words.rs).
//!
//! Expectations measured byte-for-byte against GNU bash 5.3.20 in
//! script-file mode (2026-11-09). The four `plain_*` cells are the
//! conformant controls.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2711-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), format!("{script}\n")).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn check(tag: &str, run: &Run, stdout: &str, stderr: &str, code: i32) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, stderr, "{tag}: stderr");
    assert_eq!(run.code, Some(code), "{tag}: rc");
}

#[test]
fn issue2711_hash_at_substring_suffix_is_bad_substitution() {
    let run = run_file("a", "set -- ab cd; echo \"<${#@: 2: 1}>\"");
    check(
        "a",
        &run,
        "",
        "case.sh: line 1: <${#@: 2: 1}>: bad substitution\n",
        1,
    );
}

#[test]
fn issue2711_hash_at_removal_suffix_is_bad_substitution() {
    let run = run_file("b", "set -- ab cd; echo \"<${#@%b}>\"");
    check(
        "b",
        &run,
        "",
        "case.sh: line 1: <${#@%b}>: bad substitution\n",
        1,
    );
}

#[test]
fn issue2711_hash_star_substring_suffix_is_bad_substitution() {
    let run = run_file("c", "set -- ab cd; echo \"<${#*: 1}>\"");
    check(
        "c",
        &run,
        "",
        "case.sh: line 1: <${#*: 1}>: bad substitution\n",
        1,
    );
}

#[test]
fn issue2711_hash_at_default_suffix_is_bad_substitution() {
    let run = run_file("d", "set -- ab cd; echo \"<${#@:-2}>\"");
    check(
        "d",
        &run,
        "",
        "case.sh: line 1: <${#@:-2}>: bad substitution\n",
        1,
    );
}

#[test]
fn issue2711_plain_hash_at_count_control() {
    let run = run_file("ctl1", "set -- ab cd; echo \"<${#@}>\"");
    check("ctl1", &run, "<2>\n", "", 0);
}

#[test]
fn issue2711_plain_hash_star_count_control() {
    let run = run_file("ctl2", "set -- ab cd; echo \"<${#*}>\"");
    check("ctl2", &run, "<2>\n", "", 0);
}

#[test]
fn issue2711_plain_hash_array_count_control() {
    let run = run_file("ctl3", "a=(x y z); echo \"<${#a[@]}>\"");
    check("ctl3", &run, "<3>\n", "", 0);
}

#[test]
fn issue2711_plain_hash_var_length_control() {
    let run = run_file("ctl4", "v=hello; echo \"<${#v}>\"");
    check("ctl4", &run, "<5>\n", "", 0);
}
