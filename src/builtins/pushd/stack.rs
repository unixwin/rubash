use super::DIR_STACK;
use crate::executor::path::shell_path_to_windows;
use std::collections::HashMap;

const SEP: char = crate::executor::markers::DIR_STACK_FIELD_SEP;

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
        let dirs: Vec<String> = value
            .split(SEP)
            .filter(|dir| !dir.is_empty())
            .map(str::to_string)
            .collect();
        if !dirs.is_empty() {
            return dirs;
        }
        // An emptied stack (`dirs -c`) still exposes the working directory
        // as element 0: GNU get_directory_stack (builtins/pushd.def:669)
        // always appends the live cwd after the pushed entries, so
        // `dirs -c; declare -p DIRSTACK` prints ([0]="<cwd>") and
        // ${#DIRSTACK[@]} stays 1.
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

pub(super) fn logical_dir_exists(dir: &str, env_vars: &HashMap<String, String>) -> bool {
    // The shell keeps POSIX-looking paths in PWD even on Windows.  Resolve
    // those paths before checking them, while allowing relative operands such
    // as `pushd .` and `pushd ..` against the process working directory.
    (!crate::executor::path::shell_root_configured(env_vars)
        && matches!(dir, "/" | "/bin" | "/etc" | "/tmp" | "/usr"))
        || shell_path_to_windows(dir, env_vars).is_dir()
}

/// Resolve a directory operand to the logical name stored in the stack.
///
/// Relative operands are resolved against the shell logical PWD. This lexical
/// step stays separate from Windows filesystem mapping so POSIX paths remain
/// visible as POSIX paths.
pub(super) fn normalize_stack_dir(dir: &str, env_vars: &HashMap<String, String>) -> String {
    let dir = dir.replace('\\', "/");
    let base = env_vars
        .get("PWD")
        .map(|value| value.replace('\\', "/"))
        .unwrap_or_else(|| "/".to_string());
    let combined = if dir.starts_with('/') || is_drive_absolute(&dir) {
        dir
    } else {
        format!("{base}/{dir}")
    };

    let (prefix, rest) = if combined.starts_with('/') {
        ("/", combined.as_str())
    } else if is_drive_absolute(&combined) {
        (&combined[..2], &combined[2..])
    } else {
        ("", combined.as_str())
    };
    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }

    let joined = parts.join("/");
    if prefix == "/" {
        if joined.is_empty() {
            "/".to_string()
        } else {
            format!("/{joined}")
        }
    } else if prefix.is_empty() {
        joined
    } else if joined.is_empty() {
        format!("{prefix}/")
    } else {
        format!("{prefix}/{joined}")
    }
}

fn is_drive_absolute(path: &str) -> bool {
    path.len() >= 3
        && path.as_bytes()[1] == b':'
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[2] == b'/'
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
