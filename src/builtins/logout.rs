//! logout module.
//!
//! GNU Bash source ownership:
// - builtins/exit.def (`logout_builtin`)

use std::io::{self, Write};

const EXECUTION_FAILURE: i32 = 1;

pub fn execute_with_io<E>(diagnostic_prefix: &str, stderr: &mut E) -> io::Result<i32>
where
    E: Write,
{
    writeln!(
        stderr,
        "{diagnostic_prefix}logout: not login shell: use `exit'"
    )?;
    Ok(EXECUTION_FAILURE)
}
