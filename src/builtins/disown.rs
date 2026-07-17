//! disown module.
//!
//! GNU Bash source ownership:
// - builtins/jobs.def (`disown_builtin`)

use std::io::{self, Write};

const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisownAction {
    Complete(i32),
    Current,
    All,
    Jobs(Vec<String>),
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
                'a' | 'r' => all_jobs = true,
                'h' => {}
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
        return Ok(DisownAction::All);
    }

    if index < args.len() {
        Ok(DisownAction::Jobs(args[index..].to_vec()))
    } else {
        Ok(DisownAction::Current)
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
