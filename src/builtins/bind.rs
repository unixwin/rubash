//! bind module.
//!
//! GNU Bash source ownership:
// - builtins/bind.def

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
    writeln!(
        stderr,
        "{diagnostic_prefix}bind: warning: line editing not enabled"
    )?;

    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'l' | 'p' | 's' | 'v' | 'P' | 'S' | 'V' | 'X' => {}
                'm' | 'f' | 'q' | 'u' | 'r' | 'x' => {
                    if arg.len() > 2 {
                        continue;
                    }
                    index += 1;
                    if args.get(index).is_none() {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}bind: -{option}: option requires an argument"
                        )?;
                        write_usage(stderr)?;
                        return Ok(EX_USAGE);
                    }
                }
                other => {
                    writeln!(stderr, "{diagnostic_prefix}bind: -{other}: invalid option")?;
                    write_usage(stderr)?;
                    return Ok(EX_USAGE);
                }
            }
        }
        index += 1;
    }

    Ok(EXECUTION_SUCCESS)
}

fn write_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(
        stderr,
        "bind: usage: bind [-lpsvPSVX] [-m keymap] [-f filename] [-q name] [-u name] [-r keyseq] [-x keyseq:shell-command] [keyseq:readline-function or readline-command]"
    )
}
