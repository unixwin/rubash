//! disown module.
//!
//! GNU Bash source ownership:
// - builtins/jobs.def (`disown_builtin`)

use std::io;

use crate::executor::builtin_diag::{Diag, EX_USAGE};
use crate::executor::builtin_opt::{BuiltinOpt, OptEvent};
use crate::executor::builtin_table::BuiltinRow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisownAction {
    Complete(i32),
    /// No jobspec: the current job. `nohup_only` is `disown -h` —
    /// GNU builtins/jobs.def disown_builtin: with -h the job is marked
    /// (jobs.c nohup_job, J_NOHUP) instead of deleted (rubash#1048).
    Current {
        nohup_only: bool,
    },
    All {
        nohup_only: bool,
        /// GNU builtins/jobs.def disown_builtin: `-a` and `-r` both
        /// reach delete_all_jobs, but its argument is the `-r` flag
        /// (jobs.c:5066 delete_all_jobs (running_only)) — set when `-r`
        /// was given, with or without `-a`.
        running_only: bool,
    },
    Jobs {
        specs: Vec<String>,
        nohup_only: bool,
    },
}

/// One `BuiltinOpt` scan over the table row (`"ahr"`,
/// builtins/jobs.def:249): `--help` in option position prints the
/// long help (status 2); any other option word is an invalid option
/// (usage + `EX_USAGE`). The action logic below is unchanged.
pub fn execute_with_io<'a>(
    row: &'a BuiltinRow,
    args: &'a [String],
    diag: &mut Diag<'a>,
    stdout: &mut Vec<u8>,
) -> io::Result<DisownAction> {
    let (all_jobs, running_only, nohup_only, help, operands) = {
        let mut opt = BuiltinOpt::new(row, args, diag, false);
        let mut all_jobs = false;
        let mut running_only = false;
        let mut nohup_only = false;
        let mut help = false;
        loop {
            match opt.next() {
                OptEvent::Opt(b'a') => all_jobs = true,
                OptEvent::Opt(b'r') => {
                    all_jobs = true;
                    running_only = true;
                }
                OptEvent::Opt(b'h') => nohup_only = true,
                OptEvent::Help => {
                    help = true;
                    break;
                }
                OptEvent::End => break,
                // The row's spec is `"ahr"`: no other option character
                // (and no argument-taking option) can ever arrive.
                OptEvent::Opt(_) | OptEvent::OptArg(_, _) => {
                    unreachable!("disown spec is \"ahr\"")
                }
            }
        }
        let operands = opt.operands().to_vec();
        (all_jobs, running_only, nohup_only, help, operands)
    };
    if help {
        crate::builtins::help::print_builtin_help(row.name, stdout)?;
        return Ok(DisownAction::Complete(EX_USAGE));
    }
    if diag.status() != 0 {
        diag.usage();
        return Ok(DisownAction::Complete(diag.status()));
    }

    if all_jobs && operands.is_empty() {
        return Ok(DisownAction::All {
            nohup_only,
            running_only,
        });
    }

    if !operands.is_empty() {
        Ok(DisownAction::Jobs {
            specs: operands,
            nohup_only,
        })
    } else {
        Ok(DisownAction::Current { nohup_only })
    }
}
