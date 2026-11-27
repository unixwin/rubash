//! niubash #803 (rubash engine half) — the completion-function state that
//! the bare `compopt` form requires, and the PATH command-candidate
//! executability filter.
//!
//! GNU ownership:
//! - `complete.def:862-886 compopt_builtin`: the bare form (`list == 0 &&
//!   wl == 0`) errors with "not currently executing completion function"
//!   only when `RL_ISSTATE (RL_STATE_COMPLETING) == 0 || pcomp_curcs == 0`;
//!   inside a completion it prints the governing compspec's option line
//!   (no -o/+o given) or sets the option bits on the copy
//!   (`pcomp_set_compspec_options`).
//! - `pcomplete.c:1519-1531 gen_progcomp_completions`: `pcomp_curcs` (the
//!   resolved compspec COPY) and `pcomp_curcmd` are installed for the
//!   duration of the dynamic-action run and restored afterwards.
//! - `bashline.c:2005 command_word_completion_function` +
//!   `findcmd.c:104-135 file_status`: PATH command candidates require the
//!   execute bit; on Windows that rule is the spawn rule
//!   (`executable_candidate`): PATHEXT extension or extensionless — a
//!   data file (`.dll`) is not a command.
//!
//! Verified against WSL GNU bash 5.3.20: inside a completion function
//! `compopt -o nospace; echo $?` prints 0 and bare `compopt` prints the
//! compopts line; at the command line `compopt` fails with rc 1.

use std::process::Command;

use rubash::executor::Executor;
use rubash::lexer::tokenize;
use rubash::parser::parse;

fn run_script(executor: &mut Executor, script: &str) {
    let tokens = tokenize(script);
    let ast = parse(&tokens);
    let result = executor.execute_ast(&ast);
    assert!(result.is_ok(), "script failed: {script:?} -> {result:?}");
}

/// Inside the completion function, `compopt -o nospace` is accepted (rc 0,
/// no diagnostic) and a following bare `compopt` reports the modified
/// option set on the governing compspec copy (complete.def:866-886).
#[test]
fn compopt_accepted_inside_completion_function() {
    let mut executor = Executor::new();
    run_script(
        &mut executor,
        r#"_probe() { compopt -o nospace; rc1=$?; line=$(compopt 2>&1); rc2=$?;
            COMPREPLY=("${rc1}|${rc2}|${line}"); }
           complete -F _probe cmd803"#,
    );
    let candidates = executor.complete_line("cmd803 ", 7);
    let joined = candidates.join("\n");
    assert!(
        joined.contains("0|0|"),
        "compopt inside the completion function must succeed twice: {joined:?}"
    );
    assert!(
        joined.contains("-o nospace"),
        "the bare report form must reflect the -o nospace set on the copy: {joined:?}"
    );
    assert!(
        !joined.contains("not currently executing completion function"),
        "no rejection diagnostic inside a completion: {joined:?}"
    );
}

/// At the command line (no completion running) the bare form still fails
/// exactly as GNU does (complete.def:865-869) — the state port must not
/// leak outside the completion window.
#[test]
fn compopt_still_fails_outside_completion() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("compopt")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("compopt: not currently executing completion function"),
        "stderr: {stderr}"
    );
}

/// A `compgen -c` PATH walk lists only executable candidates: on Windows a
/// `.dll` data file is never a command (niubash #803's System32 menu), while
/// real executables from PATH directories are still listed.
#[cfg(windows)]
#[test]
fn compgen_command_candidates_exclude_windows_data_files() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("compgen -c")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout
            .lines()
            .all(|line| !line.to_ascii_lowercase().ends_with(".dll")),
        "compgen -c must not list .dll data files"
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.eq_ignore_ascii_case("where.exe")),
        "compgen -c must still list real PATH executables (where.exe)"
    );
}
