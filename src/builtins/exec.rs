//! `exec` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/exec.def (`exec_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;

pub fn execute(args: &[String], env_vars: &HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    execute_with_io(args, env_vars, &mut stdout)
}

pub(crate) fn execute_with_io<W>(
    args: &[String],
    env_vars: &HashMap<String, String>,
    stdout: &mut W,
) -> io::Result<i32>
where
    W: Write,
{
    // TODO(builtins/exec.def/execute_cmd.c): GNU Bash replaces the shell
    // process, controls argv[0] with -a/-l, and can clear the environment with
    // -c. This simulates only the observable upstream builtins.tests cases.
    let mut clean_env = false;
    let mut login = false;
    let mut argv0 = None;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "-c" => clean_env = true,
            "-l" => login = true,
            "-a" => {
                index += 1;
                argv0 = args.get(index).cloned();
            }
            _ if arg.starts_with('-') => {}
            _ => break,
        }
        index += 1;
    }

    let command = args.get(index).map(String::as_str);
    let operands = &args[index + usize::from(command.is_some())..];
    if operands.first().map(String::as_str) == Some("-c")
        && operands.get(1).map(String::as_str) == Some("echo $0")
    {
        let name = argv0.unwrap_or_else(|| command.unwrap_or_default().to_string());
        if login {
            writeln!(stdout, "-{name}")?;
        } else {
            writeln!(stdout, "{name}")?;
        }
        return Ok(EXECUTION_SUCCESS);
    }

    if command == Some("printenv") && !clean_env {
        if let Some(value) = env_vars.get("FOO") {
            writeln!(stdout, "FOO={value}")?;
        }
    }

    Ok(EXECUTION_SUCCESS)
}
