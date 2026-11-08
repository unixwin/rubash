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
/// The payload is GNU builtins/times.def:65-88 (rubash#2932):
/// `getrusage(RUSAGE_SELF)` prints as the first line,
/// `getrusage(RUSAGE_CHILDREN)` as the second, each value through
/// `print_timeval`.
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

    let (self_user, self_sys, child_user, child_sys) =
        crate::executor::read_helpers::process_times();
    writeln!(
        stdout,
        "{} {}",
        format_timeval(self_user),
        format_timeval(self_sys)
    )?;
    writeln!(
        stdout,
        "{} {}",
        format_timeval(child_user),
        format_timeval(child_sys)
    )?;
    Ok(EXECUTION_SUCCESS)
}

/// GNU `print_timeval` (lib/sh/timeval.c:158-171): the format is
/// `"%ldm%d%c%03lds"` — whole minutes, the remaining whole seconds,
/// the locale decimal point, and the millisecond remainder TRUNCATED
/// (tv_usec / 1000) to exactly three digits. The decimal point is the
/// shell's C-locale `.` (the form every pinned expectation uses).
fn format_timeval(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    format!(
        "{}m{}.{:03}s",
        secs / 60,
        secs % 60,
        duration.subsec_millis()
    )
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

    /// The shape of the two lines `times` prints with real rusage
    /// values (rubash#2932): each line is `<t> <t>` with `<t>` in
    /// print_timeval's `{mins}m{secs}.{millis:03}s` form. The VALUES
    /// are the process's live CPU times, so only the shape is
    /// pinnable here; value movement is pinned by
    /// tests/issue2932_times_real_rusage_regressions.rs.
    fn assert_times_shape(stdout: &str) {
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 2, "stdout: {stdout}");
        for line in lines {
            let fields: Vec<&str> = line.split(' ').collect();
            assert_eq!(fields.len(), 2, "line: {line}");
            for field in fields {
                let (mins, rest) = field.split_once('m').expect("minutes separator");
                assert!(!mins.is_empty() && mins.bytes().all(|b| b.is_ascii_digit()));
                let (secs, frac) = rest.split_once('.').expect("decimal point");
                assert!(!secs.is_empty() && secs.bytes().all(|b| b.is_ascii_digit()));
                let frac = frac.strip_suffix('s').expect("seconds suffix");
                assert_eq!(frac.len(), 3, "field: {field}");
                assert!(frac.bytes().all(|b| b.is_ascii_digit()));
            }
        }
    }

    #[test]
    fn prints_two_time_lines() {
        let (status, stdout, stderr) = run(&[]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_times_shape(&stdout);
        assert!(stderr.is_empty());
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
        assert_times_shape(&stdout);
        assert!(stderr.is_empty());
    }

    #[test]
    fn bare_dash_is_an_operand() {
        let (status, stdout, stderr) = run(&["-"]);

        assert_eq!(status, EXECUTION_SUCCESS);
        assert_times_shape(&stdout);
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
