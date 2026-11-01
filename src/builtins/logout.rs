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

/// The CHECK_HELPOPT rendering (builtins/exit.def:81): the logout
/// $BUILTIN doc in the standard help shape, matching exit's
/// print_help. GNU prints it to stdout and the builtin returns
/// EX_USAGE — the shell continues.
pub fn write_help<W>(stdout: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(stdout, "logout: logout [n]")?;
    writeln!(stdout, "    Exit a login shell.")?;
    writeln!(stdout, "    ")?;
    writeln!(
        stdout,
        "    Exits a login shell with exit status N.  Returns an error if not executed"
    )?;
    writeln!(stdout, "    in a login shell.")
}
