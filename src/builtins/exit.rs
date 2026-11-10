//! `exit` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/exit.def (`exit_builtin`)

use crate::executor::builtin_diag::{Diag, DiagKind, NumericFatal, Prolog};
use crate::executor::builtin_table::BuiltinRow;
use std::io::{self, Write};

const EXECUTION_FAILURE: i32 = 1;
pub(crate) const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAction {
    Exit(i32),
    Continue(i32),
}

/// Execute `exit` with arguments after the command name.
pub fn execute(args: &[String], last_status: i32) -> io::Result<ExitAction> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let row = crate::executor::builtin_table::builtin_table_row("exit")
        .expect("exit row in BUILTIN_TABLE");
    let mut diag = Diag::new(row, &mut stderr);
    let action = execute_with_io(row, args, last_status, &mut diag, &mut stdout)?;
    diag.flush_with(|| {
        // The name cells derive from the single argv[0] carrier
        // chain (builtins::prefix, rubash#2791): the old literal
        // "rubash" named every renamed shell "rubash" on this
        // standalone entry (the executor path supplies its own
        // prolog and never reaches this one).
        let get = |key: &str| std::env::var(key).ok();
        Prolog {
            interactive: false,
            shell_basename: crate::builtins::prefix::shell_basename(&get),
            error_name: crate::builtins::prefix::fallback_error_name(&get),
            script_file: std::env::var("__RUBASH_SCRIPT_NAME").ok(),
            line: std::env::var("__RUBASH_CURRENT_LINE").ok(),
        }
    });
    Ok(action)
}

/// Stage-2 ④ Phase 3: the status word goes through
/// `Diag::numeric_arg` (builtins/common.c:525+ `get_exitstat`,
/// over `general.c:248` valid_number) — head validity before the
/// no_args surplus tail (rubash#2956). The executor's
/// `execute_exit_command_word` intercept still owns the
/// production diagnostic + special-builtin/DISCARD channels for
/// these shapes; this direct path produces the same actions with
/// the caller's Diag as the voice.
pub(crate) fn execute_with_io<'a, W>(
    row: &'a BuiltinRow,
    args: &'a [String],
    last_status: i32,
    diag: &mut Diag<'a>,
    stdout: &mut W,
) -> io::Result<ExitAction>
where
    W: Write,
{
    if args.is_empty() {
        return Ok(ExitAction::Exit(normalize_status(last_status)));
    }

    if args[0] == "--help" {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        // GNU builtins/exit.def exit_builtin: CHECK_HELPOPT prints the help
        // and RETURNS EX_USAGE from the builtin itself — the builtin's
        // nonzero status never reaches jump_to_top_level, so the shell
        // CONTINUES (probe: `bash -c 'exit --help; echo AFTER'` prints
        // AFTER, and an interactive `exit --help' keeps prompting). Only a
        // parsed status word takes the EXITPROG path below.
        return Ok(ExitAction::Continue(EX_USAGE));
    }

    // GNU builtins/common.c get_exitstat (bash 5.3): ISOPTION(word, '-')
    // matches "--" exactly and a leading `--` is skipped before the status
    // word — `exit --` exits with $?, `exit -- 1` exits 1 (2026-10-18
    // probes X4/X5). `exit -` is not skipped: valid_number("-") fails and
    // it becomes the numeric argument error.
    let start = usize::from(args[0] == "--");
    if args.len() == start {
        return Ok(ExitAction::Exit(normalize_status(last_status)));
    }

    let head_end = (start + 1).min(args.len());
    let value = match diag.numeric_arg(&args[..head_end], NumericFatal::Soft) {
        Some(value) => value,
        // sh_neednumarg was already reported by numeric_arg; the
        // builtin's failure return is EX_USAGE (the shell does not
        // exit — see the intercept's commentary).
        None => return Ok(ExitAction::Exit(EX_USAGE)),
    };

    if args.len() > start + 1 {
        // no_args (builtins/common.c, fatal == 0): the surplus
        // report; the builtin does NOT exit (the executor's
        // intercept owns the DISCARD channel for this shape).
        diag.report(DiagKind::TooManyArgs);
        return Ok(ExitAction::Continue(EXECUTION_FAILURE));
    }

    Ok(ExitAction::Exit(normalize_status(value)))
}

pub(crate) fn normalize_status<T>(status: T) -> i32
where
    T: Into<i128>,
{
    status.into().rem_euclid(256) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], last_status: i32) -> (ExitAction, String) {
        let row = crate::executor::builtin_table::builtin_table_row("exit").unwrap();
        let words: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action = {
            let mut diag = Diag::new(row, &mut stderr);
            let action = execute_with_io(row, &words, last_status, &mut diag, &mut stdout).unwrap();
            diag.flush_with(|| Prolog {
                interactive: false,
                shell_basename: "bash".to_string(),
                error_name: "bash".to_string(),
                script_file: Some("t.sh".to_string()),
                line: Some("1".to_string()),
            });
            action
        };
        let _ = stdout;
        (action, String::from_utf8(stderr).unwrap())
    }

    #[test]
    fn exits_with_last_status_without_arguments() {
        assert_eq!(run(&[], 42).0, ExitAction::Exit(42));
    }

    #[test]
    fn normalizes_numeric_status_to_eight_bits() {
        assert_eq!(run(&["258"], 0).0, ExitAction::Exit(2));
        assert_eq!(run(&["-1"], 0).0, ExitAction::Exit(255));
    }

    #[test]
    fn invalid_numeric_argument_exits_with_usage() {
        let (action, stderr) = run(&["abc"], 0);

        assert_eq!(action, ExitAction::Exit(EX_USAGE));
        assert!(stderr.contains("numeric argument required"));
    }

    #[test]
    fn too_many_arguments_does_not_exit() {
        let (action, stderr) = run(&["1", "2"], 0);

        assert_eq!(action, ExitAction::Continue(EXECUTION_FAILURE));
        assert!(stderr.contains("too many arguments"));
    }
}
