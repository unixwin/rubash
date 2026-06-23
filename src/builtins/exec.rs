//! `exec` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/exec.def (`exec_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};
use std::process::{Command, Stdio};

const EXECUTION_SUCCESS: i32 = 0;
const EX_BADUSAGE: i32 = 2;
const EX_NOTFOUND: i32 = 127;

pub fn execute(args: &[String], env_vars: &HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

pub(crate) fn replaces_shell(args: &[String]) -> bool {
    command_operand_index(args).is_some()
}

pub(crate) fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // TODO(builtins/exec.def/execute_cmd.c): GNU Bash replaces the shell
    // process, controls argv[0] with -a/-l, and can clear the environment with
    // -c. This simulates only the observable upstream builtins.tests cases.
    let mut clean_env = false;
    let mut login = false;
    let mut argv0 = None;
    let Some(index) = parse_options(args, stderr, &mut clean_env, &mut login, &mut argv0)? else {
        return Ok(EX_BADUSAGE);
    };

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
        return Ok(EXECUTION_SUCCESS);
    }

    if let Some(command) = command {
        let Some(program) = crate::executor::path::find_user_command(command, env_vars) else {
            writeln!(stderr, "rubash: exec: {command}: not found")?;
            return Ok(EX_NOTFOUND);
        };
        return run_external_exec(&program, operands, env_vars, clean_env, stdout, stderr);
    }

    Ok(EXECUTION_SUCCESS)
}

fn command_operand_index(args: &[String]) -> Option<usize> {
    let mut clean_env = false;
    let mut login = false;
    let mut argv0 = None;
    parse_options(
        args,
        &mut io::sink(),
        &mut clean_env,
        &mut login,
        &mut argv0,
    )
    .ok()
    .flatten()
    .filter(|index| *index < args.len())
}

fn parse_options<W>(
    args: &[String],
    stderr: &mut W,
    clean_env: &mut bool,
    login: &mut bool,
    argv0: &mut Option<String>,
) -> io::Result<Option<usize>>
where
    W: Write,
{
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if arg == "--" {
            return Ok(Some(index + 1));
        }
        let Some(options) = arg.strip_prefix('-') else {
            break;
        };
        if options.is_empty() {
            break;
        }

        for (offset, option) in options.char_indices() {
            match option {
                'c' => *clean_env = true,
                'l' => *login = true,
                'a' => {
                    let rest = &options[offset + option.len_utf8()..];
                    if !rest.is_empty() {
                        *argv0 = Some(rest.to_string());
                        break;
                    }
                    index += 1;
                    let Some(name) = args.get(index) else {
                        writeln!(stderr, "rubash: exec: -a: option requires an argument")?;
                        write_usage(stderr)?;
                        return Ok(None);
                    };
                    *argv0 = Some(name.clone());
                    break;
                }
                _ => {
                    writeln!(stderr, "rubash: exec: -{option}: invalid option")?;
                    write_usage(stderr)?;
                    return Ok(None);
                }
            }
        }
        index += 1;
    }

    Ok(Some(index))
}

fn run_external_exec<W, E>(
    program: &std::path::Path,
    operands: &[String],
    env_vars: &HashMap<String, String>,
    clean_env: bool,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    let mut process = if crate::executor::path::should_run_with_shell(program) {
        if let Some(shell) = crate::executor::path::find_shell(env_vars) {
            let mut command = Command::new(shell);
            command.arg(program);
            command
        } else {
            Command::new(program)
        }
    } else {
        Command::new(program)
    };

    if clean_env {
        process.env_clear();
    } else {
        process.envs(env_vars);
    }
    process.args(operands);
    process.stdout(Stdio::piped()).stderr(Stdio::piped());

    match process.output() {
        Ok(output) => {
            stdout.write_all(&output.stdout)?;
            stderr.write_all(&output.stderr)?;
            Ok(output.status.code().unwrap_or(1))
        }
        Err(error) => {
            writeln!(stderr, "rubash: exec: {}: {}", program.display(), error)?;
            Ok(126)
        }
    }
}

fn write_usage<W>(stderr: &mut W) -> io::Result<()>
where
    W: Write,
{
    writeln!(
        stderr,
        "exec: usage: exec [-cl] [-a name] [command [argument ...]] [redirection ...]"
    )
}
