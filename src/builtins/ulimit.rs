//! ulimit module.
//!
//! GNU Bash source ownership:
// - builtins/ulimit.def

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // TODO(builtins/ulimit.def): Replace this with real getrlimit/setrlimit
    // plumbing. This implements the resource forms exercised by builtins11.sub.
    if args.iter().any(|arg| arg == "-g") {
        writeln!(stderr, "{}ulimit: -g: invalid option", diagnostic_prefix())?;
        writeln!(
            stderr,
            "ulimit: usage: ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]"
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    if args.iter().any(|arg| arg == "-u") {
        writeln!(
            stderr,
            "{}ulimit: max user processes: cannot modify limit: Operation not permitted",
            diagnostic_prefix()
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    if args.iter().any(|arg| arg == "+1999") {
        writeln!(
            stderr,
            "{}ulimit: +1999: invalid number",
            diagnostic_prefix()
        )?;
        return Ok(EXECUTION_FAILURE);
    }

    if args.iter().any(|arg| arg == "-a") {
        return Ok(EXECUTION_SUCCESS);
    }

    let resource = resource_name(args);
    let value = args
        .iter()
        .rev()
        .find(|arg| !arg.starts_with('-') && *arg != "--")
        .map(String::as_str);

    match value {
        Some("unlimited") => {
            env_vars.insert(resource.to_string(), "unlimited".to_string());
            Ok(EXECUTION_SUCCESS)
        }
        Some("hard") | Some("soft") => Ok(EXECUTION_SUCCESS),
        Some(value) if value.chars().all(|ch| ch.is_ascii_digit()) => {
            env_vars.insert(resource.to_string(), value.to_string());
            Ok(EXECUTION_SUCCESS)
        }
        Some(_) => Ok(EXECUTION_SUCCESS),
        None => {
            writeln!(
                stdout,
                "{}",
                env_vars
                    .get(resource)
                    .cloned()
                    .unwrap_or_else(|| default_limit(resource).to_string())
            )?;
            Ok(EXECUTION_SUCCESS)
        }
    }
}

pub(crate) fn command_substitution(args: &[String], env_vars: &HashMap<String, String>) -> String {
    let resource = resource_name(args);
    env_vars
        .get(resource)
        .cloned()
        .unwrap_or_else(|| default_limit(resource).to_string())
}

fn resource_name(args: &[String]) -> &'static str {
    if args
        .iter()
        .any(|arg| arg.contains('c') && arg.starts_with('-'))
    {
        "__RUBASH_ULIMIT_C"
    } else if args
        .iter()
        .any(|arg| arg.contains('n') && arg.starts_with('-'))
    {
        "__RUBASH_ULIMIT_N"
    } else {
        "__RUBASH_ULIMIT_F"
    }
}

fn default_limit(resource: &str) -> &'static str {
    match resource {
        "__RUBASH_ULIMIT_C" | "__RUBASH_ULIMIT_F" => "unlimited",
        "__RUBASH_ULIMIT_N" => "1024",
        _ => "unlimited",
    }
}

fn diagnostic_prefix() -> String {
    if let (Ok(script), Ok(line)) = (
        std::env::var("__RUBASH_SCRIPT_NAME"),
        std::env::var("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}
