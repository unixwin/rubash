//! pushd module.
//!
//! GNU Bash source ownership:
// - builtins/pushd.def

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const DIR_STACK: &str = "__RUBASH_DIR_STACK";
const SEP: char = '\x1f';

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

fn execute_with_io<'a, I, W, E>(
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
            if args.first().copied() == Some("-p") {
                for dir in &stack {
                    writeln!(stdout, "{dir}")?;
                }
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
                    if stack.len() >= 2 {
                        stack.swap(0, 1);
                    }
                }
                PushdOperand::Dir(dir) => {
                    let old_pwd = stack
                        .first()
                        .cloned()
                        .or_else(|| env_vars.get("PWD").cloned())
                        .unwrap_or_else(|| "/".to_string());
                    stack.insert(0, dir.clone());
                    env_vars.insert("OLDPWD".to_string(), old_pwd);
                    env_vars.insert("PWD".to_string(), dir);
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
                PopdOperand::Top => {
                    if !stack.is_empty() {
                        stack.remove(0);
                    }
                }
                PopdOperand::Index(index) => {
                    if index >= stack.len() {
                        writeln!(
                            stderr,
                            "{diagnostic_prefix}popd: {}: directory stack index out of range",
                            args.last().copied().unwrap_or_default()
                        )?;
                        return Ok(EXECUTION_FAILURE);
                    }
                    stack.remove(index);
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum PushdOperand {
    Swap,
    Dir(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PopdOperand {
    Top,
    Index(usize),
}

fn parse_pushd_operand<W>(
    args: &[&str],
    diagnostic_prefix: &str,
    stderr: &mut W,
) -> io::Result<Option<PushdOperand>>
where
    W: Write,
{
    let args = strip_double_dash(args);
    if args.is_empty() {
        return Ok(Some(PushdOperand::Swap));
    }

    let arg = args[0];
    if arg.starts_with('-') && !is_stack_index(arg) {
        writeln!(stderr, "{diagnostic_prefix}pushd: {arg}: invalid number")?;
        writeln!(stderr, "pushd: usage: pushd [-n] [+N | -N | dir]")?;
        return Ok(None);
    }

    Ok(Some(PushdOperand::Dir(arg.to_string())))
}

fn parse_popd_operand<W>(
    args: &[&str],
    diagnostic_prefix: &str,
    stderr: &mut W,
) -> io::Result<Option<PopdOperand>>
where
    W: Write,
{
    if args.first().copied() == Some("--") {
        // TODO(builtins/pushd.def): Bash's popd option parser accepts `--`
        // and, in the builtins12.sub regression, treats following +N/-N
        // operands as non-options. Keep this narrow top-pop behavior until
        // the real directory-stack parser is ported.
        return Ok(Some(PopdOperand::Top));
    }

    let args = strip_double_dash(args);
    if args.is_empty() {
        return Ok(Some(PopdOperand::Top));
    }

    let arg = args[0];
    if !is_stack_index(arg) {
        if arg.starts_with('-') {
            writeln!(stderr, "{diagnostic_prefix}popd: {arg}: invalid number")?;
        } else {
            writeln!(stderr, "{diagnostic_prefix}popd: {arg}: invalid argument")?;
        }
        writeln!(stderr, "popd: usage: popd [-n] [+N | -N]")?;
        return Ok(None);
    }

    let index = arg[1..].parse::<usize>().unwrap_or(usize::MAX);
    Ok(Some(PopdOperand::Index(index)))
}

fn strip_double_dash<'a>(args: &'a [&str]) -> &'a [&'a str] {
    if args.first().copied() == Some("--") {
        &args[1..]
    } else {
        args
    }
}

fn is_stack_index(arg: &str) -> bool {
    let Some(rest) = arg.strip_prefix('+').or_else(|| arg.strip_prefix('-')) else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())
}

fn load_stack(env_vars: &HashMap<String, String>) -> Vec<String> {
    if let Some(value) = env_vars.get(DIR_STACK) {
        let stack: Vec<String> = value
            .split(SEP)
            .filter(|dir| !dir.is_empty())
            .map(str::to_string)
            .collect();
        if !stack.is_empty() {
            return stack;
        }
    }

    vec![env_vars
        .get("PWD")
        .cloned()
        .unwrap_or_else(|| "/".to_string())]
}

fn save_stack(env_vars: &mut HashMap<String, String>, stack: &[String]) {
    env_vars.insert(DIR_STACK.to_string(), stack.join(&SEP.to_string()));
}

