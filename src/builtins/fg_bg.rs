//! fg bg module.
//!
//! GNU Bash source ownership:
// - builtins/fg_bg.def

use std::io::{self, Write};

const EXECUTION_FAILURE: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobControlBuiltin {
    Fg,
    Bg,
}

pub fn execute_with_io<E>(
    builtin: JobControlBuiltin,
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<i32>
where
    E: Write,
{
    let name = match builtin {
        JobControlBuiltin::Fg => "fg",
        JobControlBuiltin::Bg => "bg",
    };
    writeln!(stderr, "{diagnostic_prefix}{name}: no job control")?;
    Ok(EXECUTION_FAILURE)
}
