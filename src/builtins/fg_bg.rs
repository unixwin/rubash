//! fg bg module.
//!
//! GNU Bash source ownership:
// - builtins/fg_bg.def

use std::io::{self, Write};

use crate::executor::builtin_diag::Diag;
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;

const EXECUTION_FAILURE: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobControlBuiltin {
    Fg,
    Bg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FgBgAction {
    Complete(i32),
    /// `--help` in option position: the long help is already in the
    /// stdout buffer; the executor returns `EX_USAGE` WITHOUT the
    /// job-control gate (GNU fg_bg.def `CHECK_HELPOPT` runs before
    /// the `job_control == 0` check).
    Help,
    Jobs(Vec<String>),
}

/// One `BuiltinOpt` scan over the table row (the empty spec,
/// fg_bg.def `no_options`, `builtins/common.c:157-175`): `--help` in
/// option position prints the long help (status 2); any other option
/// word is an invalid option (usage + `EX_USAGE`). The scan stops at
/// the first non-option word; everything from there (including a
/// later `--`) is the job list, as GNU's `no_options` leaves it.
pub fn execute_with_io<'a>(
    row: &'a BuiltinRow,
    args: &'a [String],
    diag: &mut Diag<'a>,
    stdout: &mut Vec<u8>,
) -> io::Result<FgBgAction> {
    let (help, operands) = {
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        let mut help = false;
        loop {
            match opt.next() {
                OptEvent::Help => {
                    help = true;
                    break;
                }
                OptEvent::End => break,
                // fg/bg have an empty getopt spec: no option
                // character (and no argument-taking option) can ever
                // arrive.
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {
                    unreachable!("fg/bg have an empty getopt spec")
                }
            }
        }
        let operands = opt.operands().to_vec();
        (help, operands)
    };
    if help {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(FgBgAction::Help);
    }
    if diag.status() != 0 {
        diag.usage();
        return Ok(FgBgAction::Complete(diag.status()));
    }
    Ok(FgBgAction::Jobs(operands))
}

pub fn write_no_job_control<E>(
    builtin: JobControlBuiltin,
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let name = match builtin {
        JobControlBuiltin::Fg => "fg",
        JobControlBuiltin::Bg => "bg",
    };
    writeln!(stderr, "{diagnostic_prefix}{name}: no job control")?;
    Ok(EXECUTION_FAILURE)
}
