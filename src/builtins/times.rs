//! `times` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/times.def (`times_builtin`)

use std::io::{self, Write};

use crate::executor::builtin_diag::{Diag, EX_USAGE};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;

const EXECUTION_SUCCESS: i32 = 0;

/// Execute `times` on the triple: one `BuiltinOpt` scan over the
/// table row (`no_options`, `builtins/common.c:157-175`), diagnostics
/// through `Diag`. GNU `builtins/times.def:72` runs `no_options`:
/// `--help` in option position prints the long help (status 2); any
/// other option word is an invalid option (usage + `EX_USAGE`); a
/// bare `-` is an operand, and every operand is ignored.
///
/// The two time lines stay the stub zeros (rubash#2932 tracks the
/// real `getrusage` values); this phase migrates the scan only.
pub fn execute_with_io<'a>(
    row: &'a BuiltinRow,
    args: &'a [String],
    diag: &mut Diag<'a>,
    stdout: &mut Vec<u8>,
) -> io::Result<i32> {
    let help = {
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        let mut help = false;
        loop {
            match opt.next() {
                OptEvent::Help => {
                    help = true;
                    break;
                }
                OptEvent::End => break,
                // The row's spec is empty: no option character is ever
                // valid, so an `Opt`/`OptArg` event is unreachable by
                // construction (an option word ends the scan with an
                // `InvalidOpt` report instead).
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {
                    unreachable!("times has an empty getopt spec")
                }
            }
        }
        help
    };
    if help {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(EX_USAGE);
    }
    if diag.status() != 0 {
        diag.usage();
        return Ok(diag.status());
    }

    writeln!(stdout, "0m0.000s 0m0.000s")?;
    writeln!(stdout, "0m0.000s 0m0.000s")?;
    Ok(EXECUTION_SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::builtin_diag::Prolog;
    use crate::executor::builtin_table::builtin_table_row;

    fn run(args: &[&str]) -> (i32, String, String) {
        let row = builtin_table_row("times").unwrap();
        let prolog = Prolog {
            interactive: false,
            shell_basename: "bash".to_string(),
            error_name: "bash".to_string(),
            script_file: None,
            line: Some("1".to_string()),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = {
            let mut diag = Diag::new(row, &mut stderr);
            let words: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let status = execute_with_io(row, &words, &mut diag, &mut stdout).unwrap();
            diag.flush_with(|| prolog);
            status
        };
        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn prints_two_time_lines() {
        assert_eq!(
            run(&[]),
            (
                EXECUTION_SUCCESS,
                "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n".to_string(),
                String::new()
            )
        );
    }

    #[test]
    fn rejects_options() {
        let (status, stdout, stderr) = run(&["-x"]);

        assert_eq!(status, EX_USAGE);
        assert!(stdout.is_empty());
        assert!(stderr.contains("invalid option"));
    }

    #[test]
    fn b6_f2_invalid_option_uses_callers_diagnostic_prefix() {
        // Deep-audit B6-F2: GNU builtins/common.c builtin_error carries the
        // shell diagnostic prefix (`bash: line 1: times: -z: invalid
        // option`); the builtin must not hardcode a program name.
        let (status, _, stderr) = run(&["-z"]);

        assert_eq!(status, EX_USAGE);
        assert_eq!(
            stderr,
            "bash: line 1: times: -z: invalid option\ntimes: usage: times\n"
        );
    }

    #[test]
    fn ignores_non_option_arguments() {
        let (status, stdout, stderr) = run(&["extra"]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn bare_dash_is_an_operand() {
        let (status, stdout, stderr) = run(&["-"]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(stdout, "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn help_prints_long_help() {
        let (status, stdout, stderr) = run(&["--help"]);

        assert_eq!(status, EX_USAGE);
        assert!(stderr.is_empty());
        assert!(stdout.starts_with("times: times\n"));
        assert!(stdout.contains("Display process times."));
    }
}
