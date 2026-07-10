//! pushd module.
//!
//! GNU Bash source ownership:
// - builtins/pushd.def

mod parse;
mod stack;

pub(crate) use stack::{load_stack, save_stack, set_stack_value, stack_value, stack_words};

use parse::{parse_popd_operand, parse_pushd_operand, PopdOperand, PushdOperand};
use stack::{
    dirs_index_or_error, is_stack_index, logical_dir_exists, resolved_index, set_pwd_from_stack,
    stack_index, strip_double_dash,
};
use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
pub(crate) const DIR_STACK: &str = "__RUBASH_DIR_STACK";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackBuiltin {
    Pushd,
    Popd,
    Dirs,
}

pub fn execute(
    builtin: StackBuiltin,
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    diagnostic_prefix: &str,
) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        builtin,
        args.iter().map(String::as_str),
        env_vars,
        diagnostic_prefix,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    builtin: StackBuiltin,
    args: I,
    env_vars: &mut HashMap<String, String>,
    diagnostic_prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut stack = load_stack(env_vars);

    match builtin {
        StackBuiltin::Dirs => {
            let args = strip_double_dash(&args);
            if args.first().copied() == Some("-c") {
                save_stack(env_vars, &[]);
                return Ok(EXECUTION_SUCCESS);
            }
            if let Some(status) =
                dirs_index_or_error(args, &stack, diagnostic_prefix, stdout, stderr)?
            {
                return Ok(status);
            }
            if args.first().copied() == Some("-v") {
                if let Some(index_arg) = args.get(1).copied().filter(|arg| is_stack_index(arg)) {
                    let Some(index) = stack_index(index_arg, stack.len()) else {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}dirs: {}: directory stack index out of range",
                            index_arg.trim_start_matches(['+', '-'])
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    };
                    writeln!(stdout, "{index:2}  {}", stack[index])?;
                    return Ok(EXECUTION_SUCCESS);
                }
                for (index, dir) in stack.iter().enumerate() {
                    writeln!(stdout, "{index:2}  {dir}")?;
                }
            } else if args.first().copied() == Some("-p") {
                for dir in &stack {
                    writeln!(stdout, "{dir}")?;
                }
            } else if args.first().copied() == Some("-l") {
                writeln!(stdout, "{}", stack.join(" "))?;
            } else if args.first().is_some_and(|arg| arg.starts_with('-')) {
                writeln!(
                    stderr,
                    "{diagnostic_prefix}dirs: {}: invalid number",
                    args[0]
                )?;
                writeln!(stderr, "dirs: usage: dirs [-clpv] [+N] [-N]")?;
                return Ok(EXECUTION_FAILURE);
            } else if !args.is_empty() {
                writeln!(
                    stderr,
                    "{diagnostic_prefix}dirs: {}: invalid option",
                    args[0]
                )?;
                writeln!(stderr, "dirs: usage: dirs [-clpv] [+N] [-N]")?;
                return Ok(EXECUTION_FAILURE);
            } else if stack.is_empty() {
                writeln!(
                    stdout,
                    "{}",
                    env_vars
                        .get("PWD")
                        .cloned()
                        .unwrap_or_else(|| "/".to_string())
                )?;
            } else {
                writeln!(stdout, "{}", stack.join(" "))?;
            }
            Ok(EXECUTION_SUCCESS)
        }
        StackBuiltin::Pushd => {
            let operand = parse_pushd_operand(&args, diagnostic_prefix, stderr)?;
            let Some(operand) = operand else {
                return Ok(EXECUTION_FAILURE);
            };

            match operand {
                PushdOperand::Swap => {
                    if stack.len() < 2 {
                        writeln!(stderr, "{diagnostic_prefix}pushd: no other directory")?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    stack.swap(0, 1);
                    set_pwd_from_stack(env_vars, &stack, true);
                }
                PushdOperand::Index {
                    index,
                    from_right,
                    no_cd,
                } => {
                    let index =
                        resolved_index(index, from_right, stack.len()).unwrap_or(usize::MAX);
                    if index >= stack.len() {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}pushd: {}: directory stack index out of range",
                            args.last().copied().unwrap_or_default()
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    if no_cd {
                        save_stack(env_vars, &stack);
                        return Ok(EXECUTION_SUCCESS);
                    }
                    stack.rotate_left(index);
                    set_pwd_from_stack(env_vars, &stack, true);
                }
                PushdOperand::Dir { dir, no_cd } => {
                    if !logical_dir_exists(&dir) {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}pushd: {dir}: No such file or directory"
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    let old_pwd = stack
                        .first()
                        .cloned()
                        .or_else(|| env_vars.get("PWD").cloned())
                        .unwrap_or_else(|| "/".to_string());
                    if stack.is_empty() {
                        stack.push(old_pwd.clone());
                    }
                    if no_cd && !stack.is_empty() {
                        stack.insert(1, dir);
                    } else {
                        stack.insert(0, dir.clone());
                        env_vars.insert("OLDPWD".to_string(), old_pwd);
                        env_vars.insert("PWD".to_string(), dir);
                    }
                }
            }

            save_stack(env_vars, &stack);
            writeln!(stdout, "{}", stack.join(" "))?;
            Ok(EXECUTION_SUCCESS)
        }
        StackBuiltin::Popd => {
            let operand = parse_popd_operand(&args, diagnostic_prefix, stderr)?;
            let Some(operand) = operand else {
                return Ok(EXECUTION_FAILURE);
            };

            match operand {
                PopdOperand::Top { no_cd } => {
                    if stack.len() <= 1 {
                        writeln!(stderr, "{diagnostic_prefix}popd: directory stack empty")?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    stack.remove(if no_cd { 1 } else { 0 });
                    if no_cd {
                        save_stack(env_vars, &stack);
                        writeln!(stdout, "{}", stack.join(" "))?;
                        return Ok(EXECUTION_SUCCESS);
                    }
                }
                PopdOperand::Index {
                    index,
                    from_right,
                    no_cd,
                } => {
                    let index =
                        resolved_index(index, from_right, stack.len()).unwrap_or(usize::MAX);
                    if index >= stack.len() {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}popd: {}: directory stack index out of range",
                            args.last().copied().unwrap_or_default()
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    stack.remove(index);
                    if no_cd {
                        save_stack(env_vars, &stack);
                        writeln!(stdout, "{}", stack.join(" "))?;
                        return Ok(EXECUTION_SUCCESS);
                    }
                }
            }

            let pwd = stack.first().cloned().unwrap_or_else(|| "/".to_string());
            env_vars.insert("PWD".to_string(), pwd);
            save_stack(env_vars, &stack);
            writeln!(stdout, "{}", stack.join(" "))?;
            Ok(EXECUTION_SUCCESS)
        }
    }
}
