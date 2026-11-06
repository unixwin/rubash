//! jobs module.
//!
//! GNU Bash source ownership:
// - builtins/jobs.def

use std::io;

use crate::executor::builtin_diag::{Diag, EX_USAGE};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobsAction {
    Complete(i32),
    Execute(Vec<String>),
    List {
        options: JobsListOptions,
        jobs: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobsListOptions {
    pub long: bool,
    pub pids_only: bool,
    pub changed_only: bool,
    pub running_only: bool,
    pub stopped_only: bool,
}

/// One `BuiltinOpt` scan over the table row (`"lpnxrs"`,
/// builtins/jobs.def:93): `--help` in option position prints the
/// long help (status 2); any other option word is an invalid option
/// (usage + `EX_USAGE`). The `-x` form check (jobs.def:106-113)
/// fires AT the `x` event against the form set so far: a form-setting
/// option (`-l`/`-p`/`-n`) before `x` is an error (`no other options
/// allowed with \`-x'`, status 1, no usage line); state-only `-r`/`-s`
/// and any letter AFTER `x` do not trip it, and the scan keeps going —
/// the command is the words after the last option word (rubash#2931).
pub fn execute_with_io<'a>(
    row: &'a BuiltinRow,
    args: &'a [String],
    diag: &mut Diag<'a>,
    stdout: &mut Vec<u8>,
) -> io::Result<JobsAction> {
    let (options, help, execute_command, x_violation, operands) = {
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        let mut options = JobsListOptions::default();
        let mut help = false;
        let mut execute_command = false;
        let mut x_violation = false;
        loop {
            match opt.next() {
                OptEvent::Opt(b'l') => options.long = true,
                OptEvent::Opt(b'p') => options.pids_only = true,
                OptEvent::Opt(b'n') => options.changed_only = true,
                OptEvent::Opt(b'r') => options.running_only = true,
                OptEvent::Opt(b's') => options.stopped_only = true,
                OptEvent::Opt(b'x') => {
                    if options.long || options.pids_only || options.changed_only {
                        // The diagnostic goes out after the scan (the
                        // machine owns the Diag borrow during it); the
                        // scan stops here so no later word can
                        // overwrite the report.
                        x_violation = true;
                        break;
                    }
                    execute_command = true;
                }
                OptEvent::Help => {
                    help = true;
                    break;
                }
                OptEvent::End => break,
                // The row's spec is `"lpnxrs"`: no other option
                // character (and no argument-taking option) can ever
                // arrive.
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {
                    unreachable!("jobs spec is \"lpnxrs\"")
                }
            }
        }
        let operands = opt.operands().to_vec();
        (options, help, execute_command, x_violation, operands)
    };
    if help {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(JobsAction::Complete(EX_USAGE));
    }
    if x_violation {
        diag.free(format_args!("no other options allowed with `-x'"));
        return Ok(JobsAction::Complete(EXECUTION_FAILURE));
    }
    if diag.status() != 0 {
        diag.usage();
        return Ok(JobsAction::Complete(diag.status()));
    }
    if execute_command {
        return Ok(if operands.is_empty() {
            JobsAction::Complete(EXECUTION_SUCCESS)
        } else {
            JobsAction::Execute(operands)
        });
    }
    Ok(JobsAction::List {
        options,
        jobs: operands,
    })
}
