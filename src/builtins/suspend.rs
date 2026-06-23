//! suspend module.
//!
//! GNU Bash source ownership:
// - builtins/suspend.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

pub fn execute_with_io<E>(
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let mut force = false;
    for arg in args {
        if arg == "--" {
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        for option in arg[1..].chars() {
            match option {
                'f' => force = true,
                other => {
                    writeln!(
                        stderr,
                        "{diagnostic_prefix}suspend: -{other}: invalid option"
                    )?;
                    writeln!(stderr, "suspend: usage: suspend [-f]")?;
                    return Ok(EX_USAGE);
                }
            }
        }
    }

    if force {
        return Ok(EXECUTION_SUCCESS);
    }

    writeln!(
        stderr,
        "{diagnostic_prefix}suspend: cannot suspend: no job control"
    )?;
    Ok(EXECUTION_FAILURE)
}
