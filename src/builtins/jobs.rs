//! jobs module.
//!
//! GNU Bash source ownership:
// - builtins/jobs.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EX_USAGE: i32 = 2;

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
}

pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<JobsAction>
where
    E: Write,
{
    let mut index = 0;
    let mut options = JobsListOptions::default();
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
                'l' => options.long = true,
                'p' => options.pids_only = true,
                'n' | 'r' | 's' => {}
                'x' => {
                    let command = args[index + 1..].to_vec();
                    return Ok(if command.is_empty() {
                        JobsAction::Complete(EXECUTION_SUCCESS)
                    } else {
                        JobsAction::Execute(command)
                    });
                }
                other => {
                    writeln!(stderr, "{diagnostic_prefix}jobs: -{other}: invalid option")?;
                    writeln!(
                        stderr,
                        "jobs: usage: jobs [-lnprs] [jobspec ...] or jobs -x command [args]"
                    )?;
                    return Ok(JobsAction::Complete(EX_USAGE));
                }
            }
        }
        index += 1;
    }

    Ok(JobsAction::List {
        options,
        jobs: args[index..].to_vec(),
    })
}
