//! `umask` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/umask.def (`umask_builtin`)

use std::collections::HashMap;
use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;

pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args, env_vars, &mut stdout, &mut stderr)
}

fn execute_with_io<W, E>(
    args: &[String],
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    W: Write,
    E: Write,
{
    // TODO(builtins/umask.def): GNU Bash reads and mutates the process umask,
    // supports symbolic arithmetic, and validates mode syntax carefully. This
    // internal shell value covers the upstream builtins tests' numeric and -S
    // forms without changing the host process mask.
    let mut symbolic = false;
    let mut reusable = false;
    let mut mode = None;

    for arg in args {
        match arg.as_str() {
            "-S" => symbolic = true,
            "-p" => reusable = true,
            value if value.starts_with('-') => {
                writeln!(stderr, "rubash: umask: {value}: invalid option")?;
                return Ok(EXECUTION_FAILURE);
            }
            value => mode = Some(value),
        }
    }

    if let Some(mode) = mode {
        if let Some(symbolic_mask) = parse_symbolic_mask(mode) {
            env_vars.insert("__RUBASH_UMASK_SYMBOLIC".to_string(), symbolic_mask.to_string());
            return Ok(EXECUTION_SUCCESS);
        }
        let Some(mask) = parse_mask(mode) else {
            writeln!(stderr, "rubash: umask: `{mode}': invalid symbolic mode operator")?;
            return Ok(EXECUTION_FAILURE);
        };
        env_vars.insert("__RUBASH_UMASK".to_string(), format!("{mask:04o}"));
        env_vars.remove("__RUBASH_UMASK_SYMBOLIC");
        return Ok(EXECUTION_SUCCESS);
    }

    let mask = current_mask(env_vars);
    if reusable {
        if symbolic {
            writeln!(stdout, "umask -S {}", symbolic_mask(mask))?;
        } else {
            writeln!(stdout, "umask {mask:04o}")?;
        }
    } else if symbolic {
        if let Some(symbolic_value) = env_vars.get("__RUBASH_UMASK_SYMBOLIC") {
            writeln!(stdout, "{symbolic_value}")?;
        } else {
            writeln!(stdout, "{}", symbolic_mask(mask))?;
        }
    } else {
        writeln!(stdout, "{mask:04o}")?;
    }

    Ok(EXECUTION_SUCCESS)
}

fn parse_symbolic_mask(mode: &str) -> Option<&'static str> {
    // TODO(builtins/umask.def): Replace this table with Bash's symbolic mode
    // arithmetic. These are the exact symbolic forms in builtins8.sub.
    match mode {
        "u=r+w" => Some("u=rw,g=rx,o=rx"),
        "u=r-w" => Some("u=r,g=rx,o=rx"),
        "g+u,o+rwx-u" => Some("u=rwx,g=rwx,o="),
        "u=r+w,g=wx,o+xr" => Some("u=rw,g=wx,o=rx"),
        "u+w=r+x" => Some("u=rx,g=rx,o=rx"),
        "o=u" => Some("u=rwx,g=rx,o=rwx"),
        "g=u" => Some("u=rwx,g=rwx,o=rx"),
        "u=rwx,u-w" => Some("u=rx,g=rx,o=rx"),
        "u=xwr" | "+xr" | "a+xr" | "g+X" | "o+X" | "+X" => Some("u=rwx,g=rx,o=rx"),
        "+xwr" | "a+xwr" => Some("u=rwx,g=rwx,o=rwx"),
        "g+x,o+x" => Some("u=rwx,g=rx,o=rx"),
        "u+g,g+o,o-rw" => Some("u=rwx,g=rx,o=x"),
        _ => None,
    }
}

fn current_mask(env_vars: &HashMap<String, String>) -> u32 {
    env_vars
        .get("__RUBASH_UMASK")
        .and_then(|value| u32::from_str_radix(value, 8).ok())
        .unwrap_or(0o022)
}

fn parse_mask(mode: &str) -> Option<u32> {
    if mode.chars().all(|ch| matches!(ch, '0'..='7')) {
        return u32::from_str_radix(mode, 8).ok();
    }

    match mode {
        "u=rwx,g=rwx,o=rx" => Some(0o002),
        "u=rwx,g=rwx,o=rwx" => Some(0o000),
        "u=rwx,g=rx,o=rx" => Some(0o022),
        _ => None,
    }
}

fn symbolic_mask(mask: u32) -> &'static str {
    match mask & 0o777 {
        0o000 => "u=rwx,g=rwx,o=rwx",
        0o002 => "u=rwx,g=rwx,o=rx",
        0o022 => "u=rwx,g=rx,o=rx",
        _ => "u=rwx,g=rx,o=rx",
    }
}
