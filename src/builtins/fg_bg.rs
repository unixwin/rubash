//! fg bg module.
//!
//! GNU Bash source ownership:
// - builtins/fg_bg.def

use std::io::{self, Write};

const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobControlBuiltin {
    Fg,
    Bg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FgBgAction {
    Complete(i32),
    Jobs(Vec<String>),
}

pub fn execute_with_io<E>(
    builtin: JobControlBuiltin,
    args: &[String],
    diagnostic_prefix: &str,
    stderr: &mut E,
) -> io::Result<FgBgAction>
where
    E: Write,
{
    let name = match builtin {
        JobControlBuiltin::Fg => "fg",
        JobControlBuiltin::Bg => "bg",
    };

    for arg in args {
        if arg == "--" {
            continue;
        }
        if arg.starts_with('-') && arg != "-" {
            let option = arg.chars().nth(1).unwrap_or('-');
            writeln!(
                stderr,
                "{diagnostic_prefix}{name}: -{option}: invalid option"
            )?;
            writeln!(stderr, "{name}: usage: {name} [job_spec ...]")?;
            return Ok(FgBgAction::Complete(EX_USAGE));
        }
    }

    Ok(FgBgAction::Jobs(
        args.iter()
            .filter(|arg| arg.as_str() != "--")
            .cloned()
            .collect(),
    ))
}

pub fn write_no_job_control<E>(
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
