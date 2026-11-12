//! rubash#3339 — `command <builtin> --help` must go through the
//! shared `--help` gate (CHECK_HELPOPT / CASE_HELPOPT,
//! builtins/common.h:29-41), exactly as the `builtin` re-dispatch
//! face already does (rubash#2753).
//!
//! GNU semantics (builtins/command.def:68-146): `command_builtin`
//! parses only its own `-p`/`-v`/`-V` options, then builds a bare
//! simple command from the remaining words and calls
//! `execute_command` (command.def:141) — the re-dispatched builtin
//! runs through the normal dispatch, so its own `internal_getopt`
//! scan meets `--help` and prints the long help with EX_USAGE (2).
//! rubash's `execute_command_without_aliases`
//! (command_no_alias.rs) calls the handlers directly and never
//! re-enters the funnel gate, so the target's own scan reported
//! `invalid option` instead (status 2, no help).
//!
//! The fix reuses the one shared `help_opt_requested` decision on
//! the `command` re-dispatch face — no per-builtin special cases.
//! Every expectation below was probed against GNU bash 5.3.20
//! (built from third_party/bash) before pinning:
//!
//! * `--help` in option position prints the target's long help to
//!   stdout, status 2, empty stderr.
//! * `--help` after `--`, after the first operand, after an invalid
//!   option, or swallowed as another option's argument is NOT a
//!   help request.
//! * Rows with `help_entry == false` (`echo`, `test`, `:`, `true`)
//!   never take the help path.
//! * The gate loses to `enable -n`, and applies through `-p`,
//!   nested `command`/`builtin`, a shadowing function,
//!   redirections, command substitution and pipelines.

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
    let dir = std::env::temp_dir().join(format!("issue3339gate_{}_{}", std::process::id(), seq));
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

const CD: &str = "cd: cd [-L|[-P [-e]]] [-@] [dir]\n";
const KILL: &str =
    "kill: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]\n";
const TRAP: &str = "trap: trap [-Plp] [[action] signal_spec ...]\n";
const WAIT: &str = "wait: wait [-fn] [-p var] [id ...]\n";
const DECLARE: &str =
    "declare: declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]\n";
const SET: &str = "set: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]\n";
const EVAL: &str = "eval: eval [arg ...]\n";
const UMASK: &str = "umask: umask [-p] [-S] [mode]\n";
const TYPE: &str = "type: type [-afptP] name [name ...]\n";
const SHIFT: &str = "shift: shift [n]\n";
const EXEC: &str = "exec: exec [-cl] [-a name] [command [argument ...]] [redirection ...]\n";
const SOURCE: &str = "source: source [-p path] filename [arguments]\n";
const EXIT: &str = "exit: exit [n]\n";
const PRINTF: &str = "printf: printf [-v var] format [arguments]\n";
const RETURN: &str = "return: return [n]\n";
const BREAK: &str = "break: break [n]\n";
const READ: &str = "read: read [-ers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]\n";

#[test]
fn command_cd_help() {
    assert_help(&run_script("command cd --help\n"), CD);
}

#[test]
fn command_cd_help_after_option() {
    assert_help(&run_script("command cd -L --help\n"), CD);
}

#[test]
fn command_kill_help() {
    assert_help(&run_script("command kill --help\n"), KILL);
}

#[test]
fn command_trap_help_and_after_option() {
    assert_help(&run_script("command trap --help\n"), TRAP);
    assert_help(&run_script("command trap -p --help\n"), TRAP);
}

#[test]
fn command_wait_n_help() {
    assert_help(&run_script("command wait -n --help\n"), WAIT);
}

#[test]
fn command_declare_help() {
    assert_help(&run_script("command declare --help\n"), DECLARE);
}

#[test]
fn command_set_help() {
    assert_help(&run_script("command set --help\n"), SET);
}

#[test]
fn command_eval_help() {
    assert_help(&run_script("command eval --help\n"), EVAL);
}

#[test]
fn command_umask_help() {
    assert_help(&run_script("command umask --help\n"), UMASK);
}

#[test]
fn command_type_help() {
    assert_help(&run_script("command type --help\n"), TYPE);
}

#[test]
fn command_shift_help() {
    assert_help(&run_script("command shift --help\n"), SHIFT);
}

#[test]
fn command_exec_help() {
    // The gate must fire before the exec handler: no process
    // replacement, just the long help with status 2.
    assert_help(&run_script("command exec --help\n"), EXEC);
}

#[test]
fn command_source_help() {
    assert_help(&run_script("command source --help\n"), SOURCE);
}

#[test]
fn command_exit_help() {
    // The gate must fire before the exit handler: no exit, just
    // the long help with status 2.
    assert_help(&run_script("command exit --help\n"), EXIT);
}

#[test]
fn command_printf_help() {
    assert_help(&run_script("command printf --help\n"), PRINTF);
}

#[test]
fn command_return_help() {
    assert_help(&run_script("command return --help\n"), RETURN);
}

#[test]
fn command_break_help() {
    assert_help(&run_script("command break --help\n"), BREAK);
}

#[test]
fn command_p_cd_help() {
    // `-p` changes only the PATH lookup; the builtin target still
    // takes the help path.
    assert_help(&run_script("command -p cd --help\n"), CD);
}

#[test]
fn command_builtin_cd_help() {
    assert_help(&run_script("command builtin cd --help\n"), CD);
}

#[test]
fn command_command_cd_help() {
    assert_help(&run_script("command command cd --help\n"), CD);
}

#[test]
fn command_cd_help_shadowed_function() {
    // `command` suppresses functions, so the shadowing function
    // must not win over the builtin's help.
    assert_help(
        &run_script("cd() { echo shadow; }\ncommand cd --help\n"),
        CD,
    );
}

#[test]
fn command_cd_help_redirected() {
    let run = run_script(
        "command cd --help > out.txt\ns=$?\nprintf 'status=%s\\n' \"$s\"\nhead -1 out.txt\n",
    );
    assert!(run.stdout.contains("status=2"), "stdout: {}", run.stdout);
    assert!(run.stdout.contains(CD.trim_end()), "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
    // The script's own exit status is the last echo's; the file
    // must hold exactly the help text.
    let help = std::fs::read_to_string(run.dir.join("out.txt")).expect("read out.txt");
    assert!(help.starts_with(CD), "out.txt: {}", help);
}

#[test]
fn command_cd_help_in_comsub() {
    let run = run_script("x=$(command cd --help)\ns=$?\nprintf 'status=%s\\n' \"$s\"\nprintf '%s\\n' \"$x\" | head -1\n");
    assert!(run.stdout.contains("status=2"), "stdout: {}", run.stdout);
    assert!(run.stdout.contains(CD.trim_end()), "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
}

#[test]
fn command_cd_help_in_pipeline() {
    // The pipeline's status is the last stage's (cat, 0); the help
    // text must still flow down the pipe.
    let run = run_script("command cd --help | cat\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.starts_with(CD), "stdout: {}", run.stdout);
    assert!(run.stderr.is_empty(), "stderr: {}", run.stderr);
}

#[test]
fn command_own_help() {
    // Control: `command`'s own `--help` already goes through its
    // own CASE_HELPOPT scan; it must stay green.
    assert_help(
        &run_script("command --help\n"),
        "command: command [-pVv] command [arg ...]\n",
    );
}

#[test]
fn command_kill_l_help_lists_signals() {
    // CHECK_HELPOPT is a first-word check for kill: after -l the
    // word is -l's operand and the signal list prints.
    let run = run_script("command kill -l --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("SIGTERM"), "stdout: {}", run.stdout);
    assert_not_help(&run, "Send a signal to a job");
}

#[test]
fn command_echo_help_is_operand() {
    let run = run_script("command echo --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "--help\n", "stdout: {}", run.stdout);
}

#[test]
fn command_test_help_is_operand() {
    let run = run_script("command test --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
}

#[test]
fn command_colon_true_help_noop() {
    for name in [":", "true"] {
        let run = run_script(&format!("command {name} --help\n"));
        assert_eq!(run.status, 0, "name={name} stderr: {}", run.stderr);
        assert!(run.stdout.is_empty(), "name={name} stdout: {}", run.stdout);
    }
}

#[test]
fn command_read_p_help_is_prompt() {
    // `-p` takes an argument: `--help` is the prompt, not a help
    // request; the read hits EOF on the harness's null stdin.
    let run = run_script("command read -p --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert_not_help(&run, READ);
}

#[test]
fn command_cd_dashdash_help_is_operand() {
    // After `--` the word is a directory operand, not a help
    // request.
    let run = run_script("command cd -- --help\n");
    assert_eq!(run.status, 1, "stderr: {}", run.stderr);
    assert!(run.stderr.contains("--help"), "stderr: {}", run.stderr);
    assert_not_help(&run, CD);
}

#[test]
fn command_v_cd_help_is_describe() {
    // `-v` is describe mode: `--help` is the described name.
    let run = run_script("command -v cd --help\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "cd\n", "stdout: {}", run.stdout);
}

#[test]
fn command_cd_help_disabled_falls_to_path() {
    // The gate loses to `enable -n`: a disabled target falls to
    // the PATH lookup and is not found.
    let run = run_script("enable -n cd\ncommand cd --help\n");
    assert_eq!(run.status, 127, "stderr: {}", run.stderr);
    assert!(
        run.stderr.contains("command not found"),
        "stderr: {}",
        run.stderr
    );
    assert_not_help(&run, CD);
}

#[test]
fn command_let_operand_help_not_help() {
    // `let` is a Grammar row: the first word `x=1` is an operand,
    // so `--help` is not a help request and the assignment runs.
    let run = run_script("command let x=1 --help\necho \"x=$x\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("x=1"), "stdout: {}", run.stdout);
}
