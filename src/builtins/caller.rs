//! caller module.
//!
//! GNU Bash source ownership:
// - builtins/caller.def

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

pub fn execute_with_io<W, E>(
    args: &[String],
    funcname: &[String],
    lineno: &[String],
    source: &[String],
    diagnostic_prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let level = match args.first() {
        Some(arg) if arg.starts_with('-') => {
            writeln!(stderr, "{diagnostic_prefix}caller: {arg}: invalid option")?;
            writeln!(stderr, "caller: usage: caller [expr]")?;
            return Ok(EX_USAGE);
        }
        Some(arg) => match arg.parse::<usize>() {
            Ok(level) => Some(level),
            Err(_) => {
                writeln!(stderr, "{diagnostic_prefix}caller: {arg}: invalid number")?;
                writeln!(stderr, "caller: usage: caller [expr]")?;
                return Ok(EX_USAGE);
            }
        },
        None => None,
    };

    match level {
        Some(level) => print_call_frame(level, funcname, lineno, source, stdout),
        None => print_current_call(funcname, lineno, source, stdout),
    }
}

fn print_current_call<W>(
    funcname: &[String],
    lineno: &[String],
    source: &[String],
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    if funcname.is_empty() {
        return Ok(EXECUTION_FAILURE);
    }

    let line = lineno.first().map(String::as_str).unwrap_or("0");
    let source = if funcname.len() > 1 {
        source_name(source.get(1).or_else(|| source.first()))
    } else {
        "NULL"
    };
    writeln!(stdout, "{line} {source}")?;
    Ok(EXECUTION_SUCCESS)
}

fn print_call_frame<W>(
    level: usize,
    funcname: &[String],
    lineno: &[String],
    source: &[String],
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    let Some(function) = funcname.get(level + 1) else {
        return Ok(EXECUTION_FAILURE);
    };
    let line = lineno.get(level).map(String::as_str).unwrap_or("0");
    let source = source_name(source.get(level + 1).or_else(|| source.first()));
    writeln!(stdout, "{line} {function} {source}")?;
    Ok(EXECUTION_SUCCESS)
}

fn source_name(source: Option<&String>) -> &str {
    source
        .map(String::as_str)
        .filter(|source| !source.is_empty())
        .unwrap_or("environment")
}
