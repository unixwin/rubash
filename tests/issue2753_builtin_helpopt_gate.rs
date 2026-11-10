//! rubash#2753 — the shared `--help` gate (CHECK_HELPOPT /
//! CASE_HELPOPT, builtins/common.h:29-41) in the simple-command
//! dispatch funnel.
//!
//! The issue's census found 36 of 55 builtins feeding `--help` to
//! their own parsers (`wait: --: invalid option`, `kill --help`
//! misread as a signal spec, `fc --help` printing a short text with
//! status 0). Stage-2 ④ Phase 1 fixed the pilot family
//! (issue2753_p4_phase1_help_regressions); this stem pins the
//! dispatch-gate coverage for the remaining families plus the
//! position semantics GNU's internal_getopt defines
//! (builtins/bashgetopt.c:61-192):
//!
//! * `--help` in option position (first word, or after no-arg
//!   options) prints the builtin's long help to stdout, status 2,
//!   empty stderr.
//! * `--help` after `--`, after the first operand, after an invalid
//!   option, or swallowed as another option's argument is NOT a
//!   help request.
//! * Rows with `help_entry == false` (`echo`, `test`, `:`, `true`,
//!   `false`) never take the help path.
//! * The gate loses to a shadowing function and to `enable -n`,
//!   and applies through redirections, command substitution,
//!   `builtin` re-dispatch and trap's pipeline stage.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2753gate_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

/// A help dispatch: status 2, the long help (synopsis line first)
/// on stdout, nothing on stderr.
fn assert_help(run: &Run, synopsis: &str) {
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.starts_with(synopsis), "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
}

/// Not a help dispatch: the help synopsis must not appear.
fn assert_not_help(run: &Run, synopsis: &str) {
    assert!(
        !run.stdout.contains(synopsis),
        "unexpected help text on stdout: {}",
        run.stdout
    );
}

#[test]
fn cd_help_first_and_after_option() {
    assert_help(
        &run_script("cd --help\n"),
        "cd: cd [-L|[-P [-e]]] [-@] [dir]\n",
    );
    assert_help(
        &run_script("cd -L --help\n"),
        "cd: cd [-L|[-P [-e]]] [-@] [dir]\n",
    );
}

#[test]
fn kill_help_first_word_only() {
    assert_help(
        &run_script("kill --help\n"),
        "kill: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]\n",
    );
    // CHECK_HELPOPT (kill.def:97) is a first-word check: after -l
    // the word is -l's operand and the signal list prints.
    let run = run_script("kill -l --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("SIGTERM"), "stdout: {}", run.stdout);
    assert_not_help(&run, "Send a signal to a job");
}

#[test]
fn trap_help_and_after_option() {
    assert_help(
        &run_script("trap --help\n"),
        "trap: trap [-Plp] [[action] signal_spec ...]\n",
    );
    assert_help(
        &run_script("trap -p --help\n"),
        "trap: trap [-Plp] [[action] signal_spec ...]\n",
    );
    // After `--` the word is an action operand, not a help request.
    let run = run_script("trap -- --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert_not_help(&run, "trap: trap [-Plp]");
}

#[test]
fn fc_help_is_long_form_with_usage_status() {
    // GNU prints the long topic and returns EX_USAGE (2); the old
    // hand-rolled face printed a short custom text with status 0.
    assert_help(
        &run_script("fc --help\n"),
        "fc: fc [-e ename] [-lnr] [first] [last] or fc -s [pat=rep] [command]\n",
    );
    assert_help(
        &run_script("fc -l --help\n"),
        "fc: fc [-e ename] [-lnr] [first] [last] or fc -s [pat=rep] [command]\n",
    );
}

#[test]
fn fc_dash_h_is_an_invalid_option() {
    // fc's spec is ":e:lnrs" (fc.def) — there is no -h option.
    let run = run_script("fc -h\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("invalid option"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn read_help_and_prompt_argument_consumption() {
    assert_help(
        &run_script("read --help\n"),
        "read: read [-Eers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]\n",
    );
    assert_help(
        &run_script("read -r --help\n"),
        "read: read [-Eers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]\n",
    );
    // `-p` takes the next word as its argument (bashgetopt.c ':'):
    // `--help` there is the prompt, and the read hits EOF.
    let run = run_script("read -p --help x < /dev/null\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_not_help(&run, "Read a line from the standard input");
}

#[test]
fn wait_var_argument_consumption() {
    // `wait -p --help`: -p's argument is the word itself
    // (bashgetopt.c ':' takes the next word unconditionally), so
    // this is the invalid-identifier error, not a help request.
    let run = run_script("wait -p --help\n");
    assert_not_help(&run, "Wait for job completion");
    assert!(!run.stderr.is_empty(), "stderr: {}", run.stderr);
}

#[test]
fn printf_var_argument_consumption() {
    // `printf -v --help`: --help is -v's variable-name argument.
    let run = run_script("printf -v --help\n");
    assert_not_help(&run, "Formats and prints ARGUMENTS under control");
}

#[test]
fn declare_family_help() {
    assert_help(
        &run_script("declare --help\n"),
        "declare: declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]\n",
    );
    assert_help(
        &run_script("declare -p --help\n"),
        "declare: declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]\n",
    );
    assert_help(
        &run_script("typeset --help\n"),
        "typeset: typeset [-aAfFgiIlnrtux] name[=value] ... or typeset -p [-aAfFilnrtux] [name ...]\n",
    );
    assert_help(
        &run_script("export --help\n"),
        "export: export [-fn] [name[=value] ...] or export -p [-f]\n",
    );
    assert_help(
        &run_script("export -p --help\n"),
        "export: export [-fn] [name[=value] ...] or export -p [-f]\n",
    );
    assert_help(
        &run_script("readonly --help\n"),
        "readonly: readonly [-aAf] [name[=value] ...] or readonly -p\n",
    );
    // An operand first ends the scan: both words are names, and
    // `--help` as a name is an invalid identifier (status 1).
    let run = run_script("declare x --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_not_help(&run, "declare: declare [");
}

#[test]
fn local_help_first_word_only() {
    // local_builtin (declare.def:126-141) catches a straight
    // first-word `--help` before the function-context check.
    assert_help(
        &run_script("local --help\n"),
        "local: local [option] name[=value] ...\n",
    );
    // Any other position belongs to the context check: at top
    // level `local -p --help` is the context error, status 1.
    let run = run_script("local -p --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("can only be used in a function"),
        "stderr: {}",
        run.stderr
    );
    assert_not_help(&run, "local: local [");
}

#[test]
fn alias_unalias_unset_help() {
    assert_help(
        &run_script("alias --help\n"),
        "alias: alias [-p] [name[=value] ... ]\n",
    );
    assert_help(
        &run_script("alias -p --help\n"),
        "alias: alias [-p] [name[=value] ... ]\n",
    );
    assert_help(
        &run_script("unalias --help\n"),
        "unalias: unalias [-a] name [name ...]\n",
    );
    assert_help(
        &run_script("unalias -a --help\n"),
        "unalias: unalias [-a] name [name ...]\n",
    );
    assert_help(
        &run_script("unset --help\n"),
        "unset: unset [-f] [-v] [-n] [name ...]\n",
    );
    assert_help(
        &run_script("unset -f --help\n"),
        "unset: unset [-f] [-v] [-n] [name ...]\n",
    );
}

#[test]
fn completion_family_help() {
    assert_help(
        &run_script("compgen --help\n"),
        "compgen: compgen [-V varname] [-abcdefgjksuv] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [word]\n",
    );
    assert_help(
        &run_script("complete --help\n"),
        "complete: complete [-abcdefgjksuv] [-pr] [-DEI] [-o option] [-A action] [-G globpat] [-W wordlist] [-F function] [-C command] [-X filterpat] [-P prefix] [-S suffix] [name ...]\n",
    );
    assert_help(
        &run_script("compopt --help\n"),
        "compopt: compopt [-o|+o option] [-DEI] [name ...]\n",
    );
}

#[test]
fn remaining_divergent_help() {
    assert_help(&run_script("pwd --help\n"), "pwd: pwd [-LP]\n");
    assert_help(&run_script("pwd -L --help\n"), "pwd: pwd [-LP]\n");
    assert_help(&run_script("let --help\n"), "let: let arg [arg ...]\n");
    assert_help(
        &run_script("mapfile --help\n"),
        "mapfile: mapfile [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]\n",
    );
    assert_help(
        &run_script("mapfile -t --help\n"),
        "mapfile: mapfile [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]\n",
    );
    assert_help(
        &run_script("history --help\n"),
        "history: history [-c] [-d offset] [n] or history -anrw [filename] or history -ps arg [arg...]\n",
    );
    assert_help(
        &run_script("getopts --help\n"),
        "getopts: getopts optstring name [arg ...]\n",
    );
    assert_help(
        &run_script("dirs --help\n"),
        "dirs: dirs [-clpv] [+N] [-N]\n",
    );
    assert_help(
        &run_script("pushd --help\n"),
        "pushd: pushd [-n] [+N | -N | dir]\n",
    );
    assert_help(&run_script("popd --help\n"), "popd: popd [-n] [+N | -N]\n");
    assert_help(&run_script("caller --help\n"), "caller: caller [expr]\n");
}

#[test]
fn bind_help() {
    // GNU's bind also emits a line-editing warning on stderr in a
    // non-interactive shell; the help face is stdout + status 2.
    let run = run_script("bind --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("bind: bind [-lpsvPSVX]"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn positions_that_are_not_help() {
    // `--` ends the scan: the word is cd's directory operand.
    let run = run_script("cd -- --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("No such file or directory"),
        "stderr: {}",
        run.stderr
    );
    // An operand first ends the scan: two directories is the
    // too-many-arguments error (status 2, but no help text).
    let run = run_script("cd /tmp --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    assert!(
        run.stderr.contains("too many arguments"),
        "stderr: {}",
        run.stderr
    );
    // An invalid option before the word ends the scan in error.
    let run = run_script("cd -Z --help\n");
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert_not_help(&run, "cd: cd [");
    // let is a CHECK_HELPOPT row: words after the first are
    // arithmetic expressions, evaluated normally.
    let run = run_script("let x=1 --help\n");
    assert_not_help(&run, "let: let arg");
}

#[test]
fn no_help_entry_rows_unchanged() {
    // help_entry == false: `--help` is data, never a help request.
    let run = run_script("echo --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "--help\n");
    let run = run_script("test --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
    let run = run_script(": --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let run = run_script("true --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let run = run_script("false --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
}

#[test]
fn gate_respects_redirection_and_capture() {
    let run = run_script("cd --help > help.txt\necho status=$?\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "status=2\n");
    let captured = std::fs::read_to_string(run.dir.join("help.txt")).expect("help.txt");
    assert!(
        captured.starts_with("cd: cd [-L|[-P [-e]]] [-@] [dir]\n"),
        "help.txt: {captured}"
    );
    // Command substitution captures the help text (a subshell — the
    // cd must NOT happen) and sees status 2 inside.
    let run = run_script(
        "out=$(cd --help)\nstatus=$?\nprintf 's=%s first=%.7s\\n' \"$status\" \"$out\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "s=2 first=cd: cd \n", "stdout: {}", run.stdout);
}

#[test]
fn gate_loses_to_function_and_enable_disable() {
    // A function shadows the builtin, help word and all.
    let run = run_script("cd() { echo SHADOWED; }\ncd --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "SHADOWED\n");
    // `enable -n` resolves the name externally: the gate must not
    // print the builtin help.
    let run = run_script("enable -n cd\ncd --help\n");
    assert_not_help(&run, "Change the shell working directory");
}

#[test]
fn gate_applies_through_builtin_and_pipeline() {
    assert_help(
        &run_script("builtin cd --help\n"),
        "cd: cd [-L|[-P [-e]]] [-@] [dir]\n",
    );
    assert_help(
        &run_script("builtin kill --help\n"),
        "kill: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]\n",
    );
    // trap's inline pipeline stage honours CASE_HELPOPT too.
    let run = run_script("trap --help | cat\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout
            .starts_with("trap: trap [-Plp] [[action] signal_spec ...]\n"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn conformant_samples_still_help() {
    assert_help(
        &run_script("help --help\n"),
        "help: help [-dms] [pattern ...]\n",
    );
    assert_help(
        &run_script("wait --help\n"),
        "wait: wait [-fn] [-p var] [id ...]\n",
    );
    assert_help(
        &run_script("exec --help\n"),
        "exec: exec [-cl] [-a name] [command [argument ...]] [redirection ...]\n",
    );
    // exit --help must not exit: the script continues afterwards.
    let run = run_script("exit --help\necho AFTER\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.starts_with("exit: exit [n]\n"),
        "stdout: {}",
        run.stdout
    );
    assert!(run.stdout.contains("AFTER"), "stdout: {}", run.stdout);
}
