//! pushd module.
//!
//! GNU Bash source ownership:
// - builtins/pushd.def

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
pub(crate) const DIR_STACK: &str = "__RUBASH_DIR_STACK";
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum PushdOperand {
    Swap,
    Index {
        index: usize,
        from_right: bool,
        no_cd: bool,
    },
    Dir {
        dir: String,
        no_cd: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PopdOperand {
    Top {
        no_cd: bool,
    },
    Index {
        index: usize,
        from_right: bool,
        no_cd: bool,
    },
}

fn parse_pushd_operand<W>(
    args: &[&str],
    diagnostic_prefix: &str,
    stderr: &mut W,
) -> io::Result<Option<PushdOperand>>
where
    W: Write,
{
    let (no_cd, args) = parse_stack_options(args);
    if args.is_empty() {
        return Ok(Some(PushdOperand::Swap));
    }

    let arg = args[0];
    if arg.starts_with('-') && !is_stack_index(arg) {
        writeln!(stderr, "{diagnostic_prefix}pushd: {arg}: invalid number")?;
        writeln!(stderr, "pushd: usage: pushd [-n] [+N | -N | dir]")?;
        return Ok(None);
    }

    if is_stack_index(arg) {
        return Ok(Some(PushdOperand::Index {
            index: arg[1..].parse::<usize>().unwrap_or(usize::MAX),
            from_right: arg.starts_with('-'),
            no_cd,
        }));
    }

    Ok(Some(PushdOperand::Dir {
        dir: arg.to_string(),
        no_cd,
    }))
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
        return Ok(Some(PopdOperand::Top { no_cd: false }));
    }

    let (no_cd, args) = parse_stack_options(args);
    if args.is_empty() {
        return Ok(Some(PopdOperand::Top { no_cd }));
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
    Ok(Some(PopdOperand::Index {
        index,
        from_right: arg.starts_with('-'),
        no_cd,
    }))
}

fn parse_stack_options<'a>(args: &'a [&str]) -> (bool, &'a [&'a str]) {
    let mut no_cd = false;
    let mut index = 0;
    while let Some(arg) = args.get(index).copied() {
        match arg {
            "--" => return (no_cd, &args[index + 1..]),
            "-n" => {
                no_cd = true;
                index += 1;
            }
            _ => break,
        }
    }
    (no_cd, &args[index..])
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

pub(crate) fn load_stack(env_vars: &HashMap<String, String>) -> Vec<String> {
    if let Some(value) = env_vars.get(DIR_STACK) {
        return value
            .split(SEP)
            .filter(|dir| !dir.is_empty())
            .map(str::to_string)
            .collect();
    }

    vec![env_vars
        .get("PWD")
        .cloned()
        .unwrap_or_else(|| "/".to_string())]
}

pub(crate) fn save_stack(env_vars: &mut HashMap<String, String>, stack: &[String]) {
    env_vars.insert(DIR_STACK.to_string(), stack.join(&SEP.to_string()));
}

pub(crate) fn stack_value(env_vars: &HashMap<String, String>, index: usize) -> Option<String> {
    load_stack(env_vars).get(index).cloned()
}

pub(crate) fn stack_words(env_vars: &HashMap<String, String>) -> String {
    load_stack(env_vars).join(" ")
}

pub(crate) fn set_stack_value(env_vars: &mut HashMap<String, String>, index: usize, value: String) {
    let mut stack = load_stack(env_vars);
    if index < stack.len() {
        stack[index] = value;
        save_stack(env_vars, &stack);
    }
}

fn dirs_index_or_error<W, E>(
    args: &[&str],
    stack: &[String],
    diagnostic_prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<Option<i32>>
where
    W: Write,
    E: Write,
{
    let Some(arg) = args.first().copied().filter(|arg| is_stack_index(arg)) else {
        return Ok(None);
    };
    let Some(index) = stack_index(arg, stack.len()) else {
        writeln!(
            stderr,
            "{diagnostic_prefix}dirs: {}: directory stack index out of range",
            arg.trim_start_matches(['+', '-'])
        )?;
        return Ok(Some(EXECUTION_FAILURE));
    };
    writeln!(stdout, "{}", stack[index])?;
    Ok(Some(EXECUTION_SUCCESS))
}

fn stack_index(arg: &str, len: usize) -> Option<usize> {
    let value = arg[1..].parse::<usize>().ok()?;
    if len == usize::MAX {
        return Some(if arg.starts_with('+') {
            value
        } else {
            usize::MAX
        });
    }
    if arg.starts_with('+') {
        (value < len).then_some(value)
    } else {
        resolved_index(value, true, len)
    }
}

fn resolved_index(value: usize, from_right: bool, len: usize) -> Option<usize> {
    if !from_right {
        return (value < len).then_some(value);
    }
    if value < len {
        Some(len - 1 - value)
    } else {
        None
    }
}

fn set_pwd_from_stack(
    env_vars: &mut HashMap<String, String>,
    stack: &[String],
    update_oldpwd: bool,
) {
    let Some(pwd) = stack.first().cloned() else {
        return;
    };
    if update_oldpwd {
        let old = env_vars
            .get("PWD")
            .cloned()
            .unwrap_or_else(|| "/".to_string());
        env_vars.insert("OLDPWD".to_string(), old);
    }
    env_vars.insert("PWD".to_string(), pwd);
}

fn logical_dir_exists(dir: &str) -> bool {
    matches!(dir, "/" | "/bin" | "/etc" | "/tmp" | "/usr")
}
