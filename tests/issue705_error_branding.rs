//! unixwin/rubash#705 R7.4: exit builtin error semantics + diagnostic
//! branding, pinned against the repository baseline GNU Bash **5.3.0**
//! (docs/architecture.md:55). Every case below was byte-verified against a
//! locally built bash-5.3.0(1)-release binary (2026-10-18 probes, /tmp
//! battery A-X/W/Y); the source anchors cite bash-5.3 tarball line numbers.
//!
//! TWO 5.2-vs-5.3 divergences matter here (the sandbox's 5.2.21 is NOT the
//! oracle for this file):
//!
//! 1. `exit abc` (numeric argument required): 5.2 exits rc=2; **5.3
//!    continues** with $?=2 (get_exitstat returns EX_USAGE=258, which
//!    execute_cmd.c builtin_status translates to EX_BADUSAGE=2 for the
//!    command status — no jump).
//! 2. `exit 1 2` (too many arguments): both versions abandon the rest of
//!    the current command, but 5.3 leaves **$?=2** (no_args runs
//!    `set_exit_status (EX_BADUSAGE)` before its DISCARD jump) where 5.2
//!    left 1. Under `-c` both end the shell: 5.3 through
//!    shell.c:1477 run_one_command's DISCARD catch
//!    (`return last_command_exit_value = 1`).
//!
//! Case ids follow rubash#746 (`705.<n>`, this file owns 705.20+).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn sandbox(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-705b-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

fn engine(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .current_dir(dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("HOME", dir);
    command
}

fn run(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = engine(dir).args(args).output().expect("spawn rubash");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Write `name` into `dir` and run it as a script file.
fn run_script(dir: &Path, name: &str, body: &str) -> (i32, String, String) {
    let path = dir.join(name);
    fs::write(&path, body).expect("write script");
    run(dir, &[name])
}

// ---------------------------------------------------------------------------
// exit: too many arguments — the 5.3 discard family
// ---------------------------------------------------------------------------

/// 705.20 [ratchet] GNU 5.3.0 probe P1: `-c 'exit 1 2; echo AFTER2'` —
/// no AFTER2, rc=1, prolog `bash: line 1:`. The old code printed the
/// message WITHOUT the line segment (the builtin-level prefix helper read
/// __RUBASH_* from the process environment, which the engine never
/// exports), left $?=1 in place, and let `echo AFTER2` run.
#[test]
fn issue705_exit_too_many_c_mode_stops_with_prolog() {
    let dir = sandbox("toomany-c");
    let (code, stdout, stderr) = run(&dir, &["-c", "exit 1 2; echo AFTER2"]);
    assert_eq!(stdout, "", "the rest of the -c string must be abandoned");
    assert_eq!(code, 1, "run_one_command's DISCARD catch returns 1");
    assert_eq!(stderr, "bash: line 1: exit: too many arguments\n");
}

/// 705.21 [ratchet] GNU probe P2: a NEWLINE-separated -c string is
/// abandoned whole too (top_level_cleanup popped the string stream) —
/// `bash -c 'exit 1 2\necho AFTER'` prints nothing more.
#[test]
fn issue705_exit_too_many_c_mode_abandons_later_lines() {
    let dir = sandbox("toomany-c-nl");
    let (code, stdout, _) = run(&dir, &["-c", "exit 1 2\necho AFTER"]);
    assert_eq!(stdout, "");
    assert_eq!(code, 1);
}

/// 705.22 [ratchet] GNU probes S1/T6/W3: script mode — the rest of the
/// current command list is abandoned, the reader continues with the next
/// one, and $? is 2 (`set_exit_status (EX_BADUSAGE)` survives the DISCARD:
/// eval.c:115-117 only rewrites a zero). The old code left $?=1 and RAN
/// the same-line tail.
#[test]
fn issue705_exit_too_many_script_discards_line_continues_status2() {
    let dir = sandbox("toomany-script");
    let (code, stdout, _) = run_script(
        &dir,
        "s.sh",
        "echo B; exit 1 2; echo X\necho \"dollarq=$?\"\n",
    );
    assert_eq!(
        stdout, "B\ndollarq=2\n",
        "same-line tail dropped, next line runs with $?=2"
    );
    assert_eq!(code, 0);
}

/// 705.23 [ratchet] GNU probe Y8-shape: script mode prolog carries the
/// script name — `<script>: line 1: exit: too many arguments`.
#[test]
fn issue705_exit_too_many_script_prolog_names_the_script() {
    let dir = sandbox("toomany-script-prolog");
    let (_, _, stderr) = run_script(&dir, "s.sh", "exit 1 2\necho NEXT\n");
    assert_eq!(stderr, "s.sh: line 1: exit: too many arguments\n");
}

/// 705.24 [ratchet] GNU probe S2: a discard inside an `if` body abandons
/// the whole compound (`echo IN` never runs) and the NEXT top-level
/// command still runs.
#[test]
fn issue705_exit_too_many_abandons_compound_next_command_runs() {
    let dir = sandbox("toomany-if");
    let (code, stdout, _) = run_script(
        &dir,
        "s.sh",
        "echo B\nif true; then\n  exit 1 2\n  echo IN\nfi\necho A\n",
    );
    assert_eq!(stdout, "B\nA\n");
    assert_eq!(code, 0);
}

/// 705.25 [ratchet] GNU probe S3: the same through a function body — the
/// body unwinds, the caller's next command runs.
#[test]
fn issue705_exit_too_many_function_body_unwinds_caller_continues() {
    let dir = sandbox("toomany-func");
    let (code, stdout, _) = run_script(
        &dir,
        "s.sh",
        "echo B\nf() { exit 1 2; echo IN; }\nf\necho A\n",
    );
    assert_eq!(stdout, "B\nA\n");
    assert_eq!(code, 0);
}

/// 705.26 [ratchet] GNU probes V2/W1: inside `eval` the WHOLE eval'd
/// string is abandoned (no_args' top_level_cleanup pops that
/// parse_and_execute level — even a newline-separated later line does not
/// run, unlike the failglob class) and the caller continues with $?=2.
#[test]
fn issue705_exit_too_many_eval_string_abandons_whole_caller_continues() {
    let dir = sandbox("toomany-eval");
    let (code, stdout, _) = run_script(
        &dir,
        "s.sh",
        "eval \"exit 1 2\necho IN\"\necho \"dollarq=$?\"\n",
    );
    assert_eq!(stdout, "dollarq=2\n");
    assert_eq!(code, 0);
}

/// 705.27 [ratchet] GNU probe W6: `source` behaves like eval — the sourced
/// file's remainder is dropped, the sourcing script continues.
#[test]
fn issue705_exit_too_many_source_file_abandons_rest_caller_continues() {
    let dir = sandbox("toomany-source");
    fs::write(dir.join("inc.sh"), "exit 1 2\necho IN\n").expect("write inc");
    let (code, stdout, _) = run_script(&dir, "s.sh", "echo B\nsource inc.sh\necho A\n");
    assert_eq!(stdout, "B\nA\n");
    assert_eq!(code, 0);
}

/// 705.28 [control] GNU probe W4: `-c 'exit 1 2'` alone exits 1. The old
/// Continue(1) shape coincidentally returned 1 here too; the ratchet for
/// the discard semantics lives in 705.20-705.31.
#[test]
fn issue705_exit_too_many_c_bare_exits_1() {
    let dir = sandbox("toomany-c-bare");
    let (code, _, _) = run(&dir, &["-c", "exit 1 2"]);
    assert_eq!(code, 1);
}

/// 705.29 [ratchet] GNU probe W5: a script ENDING in `exit 1 2` exits 2 —
/// the discard's $?=2 is the last status.
#[test]
fn issue705_exit_too_many_script_tail_exits_2() {
    let dir = sandbox("toomany-script-tail");
    let (code, stdout, _) = run_script(&dir, "s.sh", "echo B\nexit 1 2\n");
    assert_eq!(stdout, "B\n");
    assert_eq!(code, 2);
}

/// 705.30 [control] GNU probe T1: the EXIT trap runs with $?=1 and the
/// process exits 1 under `-c`. Same coincidence as 705.28 under the old
/// Continue(1) shape — kept as regression protection for the trap path.
#[test]
fn issue705_exit_too_many_c_exit_trap_sees_1() {
    let dir = sandbox("toomany-trap");
    let (code, stdout, _) = run(&dir, &["-c", "trap 'echo T:$?' EXIT; exit 1 2"]);
    assert_eq!(stdout, "T:1\n");
    assert_eq!(code, 1);
}

/// 705.31 [ratchet] GNU probe U3: inside a function body the prolog line
/// number is the exit command's own source line, and the script's final
/// status is the discard's 2.
#[test]
fn issue705_exit_too_many_function_prolog_cites_body_line() {
    let dir = sandbox("toomany-func-line");
    let (code, stdout, stderr) = run_script(&dir, "s.sh", "f() {\n  exit 1 2\n}\necho B\nf\n");
    assert_eq!(stderr, "s.sh: line 2: exit: too many arguments\n");
    assert_eq!(stdout, "B\n");
    assert_eq!(code, 2);
}

/// 705.33 [guard] GNU probes X4-X6: a leading `--` is skipped before the
/// status word (get_exitstat's ISOPTION(word, '-') matches "--" exactly) —
/// `exit --` exits with $?, `exit -- 1` exits 1, `exit -- 1 2` is the
/// too-many error.
#[test]
fn issue705_exit_dashdash_is_skipped_before_status_word() {
    let dir = sandbox("exit-dashdash");
    let (code, _, _) = run(&dir, &["-c", "true; exit --"]);
    assert_eq!(code, 0, "exit -- exits with $?");
    let (code, _, _) = run(&dir, &["-c", "exit -- 1"]);
    assert_eq!(code, 1, "exit -- 1 exits 1");
    let (code, _, stderr) = run(&dir, &["-c", "exit -- 1 2"]);
    assert_eq!(code, 1);
    assert_eq!(stderr, "bash: line 1: exit: too many arguments\n");
}

// ---------------------------------------------------------------------------
// exit: numeric argument required — 5.3 continues with $?=2
// ---------------------------------------------------------------------------

/// 705.34 [control] GNU probe D: `-c 'exit abc; echo AFTER'` — 5.3
/// CONTINUES (5.2 exited; the executor interception already implemented
/// the continue-with-2 shape, pinned so nobody "fixes" it back).
#[test]
fn issue705_exit_numeric_c_continues_status2() {
    let dir = sandbox("numeric-c");
    let (code, stdout, stderr) = run(&dir, &["-c", "exit abc; echo AFTER"]);
    assert_eq!(stdout, "AFTER\n");
    assert_eq!(code, 0);
    assert_eq!(
        stderr,
        "bash: line 1: exit: abc: numeric argument required\n"
    );
}

/// 705.35 [control] GNU probe E-shape: script-mode numeric prolog names
/// the script and the next line still runs.
#[test]
fn issue705_exit_numeric_script_prolog_and_continue() {
    let dir = sandbox("numeric-script");
    let (code, stdout, stderr) = run_script(&dir, "s.sh", "exit abc\necho NEXT\n");
    assert_eq!(
        stderr,
        "s.sh: line 1: exit: abc: numeric argument required\n"
    );
    assert_eq!(stdout, "NEXT\n");
    assert_eq!(code, 0);
}

/// 705.36 [guard] GNU probe X3: `exit -` is NOT skipped by the ISOPTION
/// rule (only "--" matches) — it is the numeric error.
#[test]
fn issue705_exit_single_dash_is_numeric_error() {
    let dir = sandbox("exit-single-dash");
    let (code, stdout, stderr) = run(&dir, &["-c", "exit -; echo A"]);
    assert_eq!(stdout, "A\n");
    assert_eq!(code, 0);
    assert_eq!(stderr, "bash: line 1: exit: -: numeric argument required\n");
}

/// 705.37 [control] GNU probe Y7: get_exitstat checks the FIRST status
/// word's numeric validity BEFORE no_args — `exit abc def` reports the
/// numeric error, not too-many-arguments.
#[test]
fn issue705_exit_numeric_check_precedes_too_many() {
    let dir = sandbox("exit-numeric-first");
    let (code, _, stderr) = run(&dir, &["-c", "exit abc def"]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "bash: line 1: exit: abc: numeric argument required\n"
    );
}

/// 705.32 [control] healthy exits stay put: a numeric exit keeps its
/// value and a bare `exit` keeps $?.
#[test]
fn issue705_exit_healthy_paths_unchanged() {
    let dir = sandbox("exit-healthy");
    let (code, _, _) = run(&dir, &["-c", "exit 3"]);
    assert_eq!(code, 3);
    let (code, _, _) = run(&dir, &["-c", "false; exit"]);
    assert_eq!(code, 1);
    let (code, _, _) = run_script(&dir, "s.sh", "exit 7\n");
    assert_eq!(code, 7);
}

// ---------------------------------------------------------------------------
// enable / cd / pwd / export: usage-error status + reader prolog
// ---------------------------------------------------------------------------

/// 705.40 [ratchet] GNU probe Y1: `enable -f` — message + usage line, rc 2
/// (enable.def:163 default arm `return (EX_USAGE)`). The old code returned
/// EXECUTION_FAILURE.
#[test]
fn issue705_enable_f_missing_operand_exits_2() {
    let dir = sandbox("enable-f");
    let (code, _, stderr) = run(&dir, &["-c", "enable -f"]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "bash: line 1: enable: -f: option requires an argument\n\
         enable: usage: enable [-a] [-dnps] [-f filename] [name ...]\n"
    );
}

/// 705.41 [ratchet] GNU probe Y2: `enable -Q` — invalid option, rc 2. The
/// old code returned EXECUTION_FAILURE.
#[test]
fn issue705_enable_invalid_option_exits_2() {
    let dir = sandbox("enable-q");
    let (code, _, stderr) = run(&dir, &["-c", "enable -Q"]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "bash: line 1: enable: -Q: invalid option\n\
         enable: usage: enable [-a] [-dnps] [-f filename] [name ...]\n"
    );
}

/// 705.42 [ratchet] GNU probe Y3 + cd.def:329-330: `cd a b` —
/// `builtin_error` + `return (EX_USAGE)`, rc 2. An earlier pass of this
/// fix recorded 1 from a mis-read probe.
#[test]
fn issue705_cd_too_many_arguments_exits_2() {
    let dir = sandbox("cd-toomany");
    let (code, _, stderr) = run(&dir, &["-c", "cd a b"]);
    assert_eq!(code, 2);
    assert_eq!(stderr, "bash: line 1: cd: too many arguments\n");
}

/// 705.43 [ratchet] GNU probe Y4: `cd nosuch` — the normal cd failure
/// status is 1 (cd.def setpwd failure -> EXECUTION_FAILURE).
#[test]
fn issue705_cd_missing_dir_exits_1() {
    let dir = sandbox("cd-nosuch");
    let (code, _, stderr) = run(&dir, &["-c", "cd nosuch"]);
    assert_eq!(code, 1);
    assert!(stderr.ends_with("cd: nosuch: No such file or directory\n"));
}

/// 705.44 [ratchet] The cd diagnostic must survive `exec 2>file`: the
/// builtin's stderr flush used to be silently dropped when the route
/// reported "no default endpoint" (printf_path_builtins.rs now falls back
/// the same way external_redirects.rs does).
#[test]
fn issue705_cd_diagnostic_survives_exec_stderr_rebind() {
    let dir = sandbox("cd-exec-stderr");
    let (code, _, stderr) = run_script(&dir, "s.sh", "exec 2>err.txt\ncd a b\n");
    assert_eq!(stderr, "", "nothing reaches the old stderr");
    let captured = fs::read_to_string(dir.join("err.txt")).expect("read err.txt");
    assert_eq!(captured, "s.sh: line 2: cd: too many arguments\n");
    assert_eq!(code, 2);
}

/// 705.45 [ratchet] The mirror shape: `exec >file` must not swallow the
/// `cd -` directory report (stdout channel of the same flush bug).
#[test]
fn issue705_cd_stdout_report_survives_exec_stdout_rebind() {
    let dir = sandbox("cd-exec-stdout");
    fs::create_dir_all(dir.join("sub")).expect("mkdir");
    let (code, stdout, _) = run_script(&dir, "s.sh", "cd sub\nexec >out.txt\ncd -\n");
    assert_eq!(stdout, "", "nothing reaches the old stdout");
    let captured = fs::read_to_string(dir.join("sub/out.txt")).expect("read out.txt");
    // `cd -` returns to the directory BEFORE `cd sub` — the sandbox root.
    assert!(captured.contains(dir.to_str().unwrap()));
    assert_eq!(code, 0);
}

/// 705.46 [control] pwd/export invalid options keep their 5.3 shape:
/// rc 2, prolog, usage line (already matching; pinned against drift).
#[test]
fn issue705_pwd_export_invalid_option_shape() {
    let dir = sandbox("pwd-export");
    let (code, _, stderr) = run(&dir, &["-c", "pwd -Q"]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "bash: line 1: pwd: -Q: invalid option\npwd: usage: pwd [-LP]\n"
    );
    let (code, _, stderr) = run(&dir, &["-c", "export -Q"]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr,
        "bash: line 1: export: -Q: invalid option\n\
         export: usage: export [-fn] [name[=value] ...] or export -p [-f]\n"
    );
}
