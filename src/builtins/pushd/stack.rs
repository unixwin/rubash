use super::{DIR_STACK, EXECUTION_FAILURE, EXECUTION_SUCCESS};
use std::collections::HashMap;
use std::io::{self, Write};

const SEP: char = '\x1f';

pub(super) fn strip_double_dash<'a>(args: &'a [&str]) -> &'a [&'a str] {
    if args.first().copied() == Some("--") {
        &args[1..]
    } else {
        args
    }
}

pub(super) fn is_stack_index(arg: &str) -> bool {
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

pub(super) fn dirs_index_or_error<W, E>(
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

pub(super) fn resolved_index(value: usize, from_right: bool, len: usize) -> Option<usize> {
    if !from_right {
        return (value < len).then_some(value);
    }
    if value < len {
        Some(len - 1 - value)
    } else {
        None
    }
}

pub(super) fn set_pwd_from_stack(
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

pub(super) fn logical_dir_exists(dir: &str) -> bool {
    matches!(dir, "/" | "/bin" | "/etc" | "/tmp" | "/usr")
}

pub(super) fn stack_index(arg: &str, len: usize) -> Option<usize> {
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
