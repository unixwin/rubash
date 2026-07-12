//! source module.
//!
//! GNU Bash source ownership:
// - builtins/source.def
// - execute_cmd.c
// - redir.c
// - subst.c

mod execution;
mod flow;
mod if_alias;
mod invocation;
mod pipe_source;
mod simple_if;

pub use execution::{execute_text, execute_text_with_args};
pub(crate) use flow::normalize_inline_compound_commands;
pub use pipe_source::execute_pipe_into_source;
pub use simple_if::execute_simple_if;

use invocation::{SourceInvocation, SourceParseError};

use crate::executor::{ExecuteError, Executor};
use crate::parser::CommandNode;
use std::fs;
use std::io::Write;

pub fn execute(executor: &mut Executor, args: &[String]) -> Result<(), ExecuteError> {
    execute_named(executor, "source", args)
}

pub fn execute_named(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
) -> Result<(), ExecuteError> {
    execute_named_with_io(executor, command_name, args, &mut std::io::stderr().lock())
}

pub fn execute_named_with_io<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    execute_named_with_io_impl(executor, command_name, args, stderr, None)
}

pub fn execute_named_with_io_and_redirects<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
    redirect_cmd: &CommandNode,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    execute_named_with_io_impl(executor, command_name, args, stderr, Some(redirect_cmd))
}

fn execute_named_with_io_impl<E>(
    executor: &mut Executor,
    command_name: &str,
    args: &[String],
    stderr: &mut E,
    redirect_cmd: Option<&CommandNode>,
) -> Result<(), ExecuteError>
where
    E: Write,
{
    // TODO(builtins/source.def): GNU Bash `source_builtin` uses unwind/trap
    // machinery around `source_file`.
    let invocation = match SourceInvocation::parse(args) {
        Ok(invocation) => invocation,
        Err(error) => {
            match error {
                SourceParseError::MissingFilename => {
                    writeln!(
                        stderr,
                        "{}{command_name}: filename argument required",
                        executor.diagnostic_prefix()
                    )?;
                }
                SourceParseError::MissingPathArgument => {
                    writeln!(
                        stderr,
                        "{}{command_name}: -p: option requires an argument",
                        executor.diagnostic_prefix()
                    )?;
                }
                SourceParseError::InvalidOption(option) => {
                    writeln!(
                        stderr,
                        "{}{command_name}: -{option}: invalid option",
                        executor.diagnostic_prefix()
                    )?;
                }
            }
            writeln!(
                stderr,
                "{command_name}: usage: {command_name} [-p path] filename [arguments]"
            )?;
            executor.set_exit_code(2);
            return Ok(());
        }
    };
    let filename = invocation.filename;

    if is_null_device(filename) {
        executor.set_exit_code(0);
        return Ok(());
    }

    if filename == "echo" {
        // TODO(subst.c/execute_cmd.c): Process substitution should create a
        // /dev/fd path whose content is the command's stdout. The current
        // parser sees `. <(echo "echo two - OK")` as `source echo ...`; source
        // that generated text directly until process substitution is parsed.
        let source = args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        if !source.is_empty() {
            return execution::execute_text_maybe_redirected(executor, &source, &[], redirect_cmd);
        }
    }

    let Some(source_path) = invocation.resolve_path(executor) else {
        if invocation.path.is_some() || posix_plain_name_lookup(executor, filename) {
            writeln!(
                stderr,
                "{}.: {filename}: file not found",
                executor.diagnostic_prefix()
            )?;
        } else {
            writeln!(
                stderr,
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            )?;
        }
        executor.set_exit_code(1);
        if executor.get_env("__RUBASH_POSIX_MODE") == Some("1") {
            return Err(ExecuteError::ExitCode(1));
        }
        return Ok(());
    };

    let source = match fs::read_to_string(&source_path) {
        Ok(source) => source,
        Err(_) => {
            writeln!(
                stderr,
                "{}{filename}: No such file or directory",
                executor.diagnostic_prefix()
            )?;
            executor.set_exit_code(1);
            if executor.get_env("__RUBASH_POSIX_MODE") == Some("1") {
                return Err(ExecuteError::ExitCode(1));
            }
            return Ok(());
        }
    };

    execution::execute_text_maybe_redirected(executor, &source, invocation.args, redirect_cmd)
}

fn is_null_device(path: &str) -> bool {
    matches!(path, "/dev/null" | "NUL")
}

fn posix_plain_name_lookup(executor: &Executor, filename: &str) -> bool {
    executor.get_env("__RUBASH_POSIX_MODE") == Some("1")
        && !filename.contains('/')
        && !filename.contains('\\')
}
