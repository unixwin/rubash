//! disown module.
//!
//! GNU Bash source ownership:
// - builtins/jobs.def (`disown_builtin`)

use std::io::{self, Write};

const EX_USAGE: i32 = 2;

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

pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<DisownAction>
where
    E: Write,
{
    let mut all_jobs = false;
    let mut running_only = false;
    let mut nohup_only = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'a' => all_jobs = true,
                'r' => {
                    all_jobs = true;
                    running_only = true;
                }
                'h' => nohup_only = true,
                other => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}disown: -{other}: invalid option"
                    )?;
                    write_usage(stderr)?;
                    return Ok(DisownAction::Complete(EX_USAGE));
                }
            }
        }
        index += 1;
    }

    if all_jobs && args.get(index).is_none() {
        return Ok(DisownAction::All {
            nohup_only,
            running_only,
        });
    }

    if index < args.len() {
        Ok(DisownAction::Jobs {
            specs: args[index..].to_vec(),
            nohup_only,
        })
    } else {
        Ok(DisownAction::Current { nohup_only })
    }
}

fn write_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(
        stderr,
        "disown: usage: disown [-h] [-ar] [jobspec ... | pid ...]"
    )
}
