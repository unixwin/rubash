//! Issue rubash#334 leftover (errors8.sub ok 5-ok 8): under `set -o posix`,
//! a special builtin executed as the `command' builtin's argument must
//! report its error and CONTINUE — only the bare form is posix-fatal.
//!
//! GNU contract (vendored third_party/bash):
//! - execute_cmd.c:4657-4666: `builtin_is_special' is set only on the
//!   dispatch path where `(cmdflags & CMD_NO_FUNCTIONS) == 0'; the
//!   `command' builtin's re-dispatch (check_command_builtin sets
//!   CMD_NO_FUNCTIONS at :4723) never qualifies — "we don't want to exit
//!   the shell if a special builtin executed with `command builtin'
//!   fails". `special_builtin_failed' (:4888) therefore never fires, and
//!   the :1004-1017 exit (`posixly_correct && interactive == 0 &&
//!   special_builtin_failed') cannot run.
//! - general.c:112 (posix_initialize): `set -o posix' turns
//!   print_shift_error ON — `shift 12' past `$#' prints its sh_erange
//!   diagnostic in posix mode (builtins/shift.def:77-88) even without the
//!   shift_verbose shopt; the failure is plain EXECUTION_FAILURE (1, not >
//!   EX_SHERRBASE), never posix-fatal.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28, wt18/misc17
//! target/misc17/e8*.sh and c1/c2 matrix artifacts).

use std::io::Write;
use std::process::Command;

/// Run `script` as a script FILE and return (stdout, stderr with the
/// invocation-dependent `<path>: ` prolog normalized away, rc).
fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i334-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| match line.find(": line ") {
            // `<path>: line N: msg` -> `SH: line N: msg`
            Some(idx) => format!("SH{}", &line[idx..]),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

/// The full errors8.sub sequence (ok 1-ok 8 plus the continuation marker):
/// every `command <special-builtin> ...' failure reports and lets the `||'
/// branch and the rest of the script run.
#[test]
fn errors8_sequence_command_special_builtins_continue() {
    let (stdout, stderr, code) = run_script(
        "set -o posix\nreadonly v\n\ncommand eval '( ' || echo ok 1\n\ncommand export v=foo || echo ok 2\ncommand readonly v=foo || echo ok 3\n\ncommand shift 12 || echo ok 4\n\ncommand return 16 || echo ok 5\ncommand set -o notanoption || echo ok 6\n\ncommand . /notthere || echo ok 7\ncommand . -x true || echo ok 8\necho DEBUG-after\n",
    );
    assert_eq!(
        stdout,
        "ok 1\nok 2\nok 3\nok 4\nok 5\nok 6\nok 7\nok 8\nDEBUG-after\n"
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        "SH: line 5: syntax error: unexpected end of file from `(' command on line 4\n\
         SH: line 6: v: readonly variable\n\
         SH: line 7: v: readonly variable\n\
         SH: line 9: shift: 12: shift count out of range\n\
         SH: line 11: return: can only `return' from a function or sourced script\n\
         SH: line 12: set: notanoption: invalid option name\n\
         SH: line 14: /notthere: No such file or directory\n\
         SH: line 15: .: -x: invalid option\n\
         .: usage: . [-p path] filename [arguments]"
    );
}

/// ok 5 owner: `command return 16' — return reports and the script
/// continues (execute_cmd.c:4657-4666 comment). GNU's status is 2
/// (EX_... conversion; verified WSL 5.3.0), not 1.
#[test]
fn command_return_outside_function_continues() {
    let (stdout, stderr, code) =
        run_script("set -o posix\ncommand return 16 || echo caught $?\necho after\n");
    assert_eq!(stdout, "caught 2\nafter\n");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        "SH: line 2: return: can only `return' from a function or sourced script"
    );
}

/// The BARE form of ok 5's shape stays posix-fatal (execute_cmd.c:1004-1017
/// with builtin_is_special set): nothing after runs.
#[test]
fn bare_return_outside_function_is_posix_fatal() {
    let (stdout, stderr, code) = run_script("set -o posix\nreturn 16\necho after\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "SH: line 2: return: can only `return' from a function or sourced script"
    );
}

/// ok 6 owner: `command set -o notanoption' — the usage error reports and
/// the script continues.
#[test]
fn command_set_bad_option_continues() {
    let (stdout, stderr, code) =
        run_script("set -o posix\ncommand set -o notanoption || echo caught $?\necho after\n");
    assert_eq!(stdout, "caught 2\nafter\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "SH: line 2: set: notanoption: invalid option name");
}

/// The BARE form of ok 6's shape stays posix-fatal.
#[test]
fn bare_set_bad_option_is_posix_fatal() {
    let (stdout, stderr, code) = run_script("set -o posix\nset -o notanoption\necho after\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(2));
    assert_eq!(stderr, "SH: line 2: set: notanoption: invalid option name");
}

/// ok 7 owner: `command . /notthere' — the missing file reports and the
/// script continues.
#[test]
fn command_source_missing_file_continues() {
    let (stdout, stderr, code) =
        run_script("set -o posix\ncommand . /notthere || echo caught $?\necho after\n");
    assert_eq!(stdout, "caught 1\nafter\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "SH: line 2: /notthere: No such file or directory");
}

/// The BARE form of ok 7's shape stays posix-fatal with rc 1.
#[test]
fn bare_source_missing_file_is_posix_fatal() {
    let (stdout, stderr, code) = run_script("set -o posix\n. /notthere\necho after\n");
    assert_eq!(stdout, "");
    assert_eq!(code, Some(1));
    assert_eq!(stderr, "SH: line 2: /notthere: No such file or directory");
}

/// ok 8 owner: `command . -x true' — the invalid option and usage lines
/// report and the script continues.
#[test]
fn command_source_bad_option_continues() {
    let (stdout, stderr, code) =
        run_script("set -o posix\ncommand . -x true || echo caught $?\necho after\n");
    assert_eq!(stdout, "caught 2\nafter\n");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        "SH: line 2: .: -x: invalid option\n.: usage: . [-p path] filename [arguments]"
    );
}

/// `command eval' containment (rubash#333) is preserved alongside the new
/// gate: the eval parse error reports, the `||' branch runs.
#[test]
fn command_eval_parse_error_continues() {
    let (stdout, stderr, code) =
        run_script("set -o posix\ncommand eval '( ' || echo caught\necho after\n");
    assert_eq!(stdout, "caught\nafter\n");
    assert_eq!(code, Some(0));
    assert_eq!(
        stderr,
        "SH: line 3: syntax error: unexpected end of file from `(' command on line 2"
    );
}

/// general.c:112: `set -o posix' implies print_shift_error — the
/// out-of-range diagnostic prints without the shopt; the failure stays
/// non-fatal (EXECUTION_FAILURE is not > EX_SHERRBASE).
#[test]
fn posix_mode_prints_shift_out_of_range() {
    let (stdout, stderr, code) =
        run_script("set -o posix\nshift 12 || echo caught $?\necho after\n");
    assert_eq!(stdout, "caught 1\nafter\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "SH: line 2: shift: 12: shift count out of range");
}

/// Without posix mode and without the shift_verbose shopt the failure is
/// silent (general.c:127 restores print_shift_error = 0).
#[test]
fn default_mode_shift_out_of_range_stays_silent() {
    let (stdout, stderr, code) = run_script("shift 12 || echo caught $?\necho after\n");
    assert_eq!(stdout, "caught 1\nafter\n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, "");
}
