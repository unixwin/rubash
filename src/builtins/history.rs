//! history module.
//!
//! GNU Bash source ownership:
// - builtins/history.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EX_USAGE: i32 = 2;

pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
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
                'a' | 'c' | 'n' | 'r' | 'w' => {}
                'd' => {
                    index += 1;
                    if args.get(index).is_none() {
                        return Ok(EXECUTION_SUCCESS);
                    }
                    break;
                }
                'p' | 's' => {
                    return Ok(EXECUTION_SUCCESS);
                }
                other => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}history: -{other}: invalid option"
                    )?;
                    write_usage(stderr)?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    if let Some(arg) = args.get(index) {
        if !arg.chars().all(|ch| ch.is_ascii_digit()) {
            writeln!(
                stderr,
                "{diagnostic_prefix}history: {arg}: numeric argument required"
            )?;
            write_usage(stderr)?;
            return Ok(EX_USAGE);
        }
    }

    Ok(EXECUTION_SUCCESS)
}

fn write_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(
        stderr,
        "history: usage: history [-c] [-d offset] [n] or history -anrw [filename] or history -ps arg [arg...]"
    )
}
