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
    // TODO(builtins/umask.def): GNU Bash reads and mutates the process umask.
    // This internal shell value preserves shell semantics without changing the
    // host process mask.
    let mut symbolic = false;
    let mut reusable = false;
    let mut mode = None;

    for arg in args {
        match arg.as_str() {
            value if value.starts_with('-') && value != "-" => {
                for option in value[1..].chars() {
                    match option {
                        'S' => symbolic = true,
                        'p' => reusable = true,
                        _ => {
                            writeln!(stderr, "rubash: umask: {value}: invalid option")?;
                            return Ok(EXECUTION_FAILURE);
                        }
                    }
                }
            }
            value if value.starts_with('-') => {
                writeln!(stderr, "rubash: umask: {value}: invalid option")?;
                return Ok(EXECUTION_FAILURE);
            }
            value => mode = Some(value),
        }
    }

    if let Some(mode) = mode {
        let Some(mask) =
            parse_mask(mode).or_else(|| parse_symbolic_mask(mode, current_mask(env_vars)))
        else {
            writeln!(
                stderr,
                "rubash: umask: `{mode}': invalid symbolic mode operator"
            )?;
            return Ok(EXECUTION_FAILURE);
        };
        env_vars.insert("__RUBASH_UMASK".to_string(), format!("{mask:04o}"));
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
        writeln!(stdout, "{}", symbolic_mask(mask))?;
    } else {
        writeln!(stdout, "{mask:04o}")?;
    }

    Ok(EXECUTION_SUCCESS)
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

    None
}

fn parse_symbolic_mask(mode: &str, current_mask: u32) -> Option<u32> {
    let mut allowed = (!current_mask) & 0o777;

    for clause in mode.split(',') {
        if clause.is_empty() {
            return None;
        }
        allowed = apply_symbolic_clause(allowed, clause)?;
    }

    Some((!allowed) & 0o777)
}

fn apply_symbolic_clause(mut allowed: u32, clause: &str) -> Option<u32> {
    let chars: Vec<char> = clause.chars().collect();
    let mut index = 0;
    let mut who = 0;

    while let Some(ch) = chars.get(index) {
        let bits = match ch {
            'u' => 0o700,
            'g' => 0o070,
            'o' => 0o007,
            'a' => 0o777,
            _ => break,
        };
        who |= bits;
        index += 1;
    }

    if who == 0 {
        who = 0o777;
    }

    while index < chars.len() {
        let op = chars[index];
        if !matches!(op, '+' | '-' | '=') {
            return None;
        }
        index += 1;

        let start = index;
        while index < chars.len() && !matches!(chars[index], '+' | '-' | '=') {
            index += 1;
        }

        let perms = symbolic_permission_bits(&chars[start..index], allowed, who)?;
        match op {
            '+' => allowed |= perms,
            '-' => allowed &= !perms,
            '=' => allowed = (allowed & !who) | perms,
            _ => unreachable!("validated symbolic umask operator"),
        }
    }

    Some(allowed & 0o777)
}

fn symbolic_permission_bits(perms: &[char], allowed: u32, who: u32) -> Option<u32> {
    let mut bits = 0;
    for ch in perms {
        match ch {
            'r' => bits |= expand_permission_to_who(0o444, who),
            'w' => bits |= expand_permission_to_who(0o222, who),
            'x' => bits |= expand_permission_to_who(0o111, who),
            'X' => {
                if allowed & 0o111 != 0 {
                    bits |= expand_permission_to_who(0o111, who);
                }
            }
            'u' | 'g' | 'o' => bits |= copy_permission_to_who(*ch, allowed, who),
            _ => return None,
        }
    }
    Some(bits)
}

fn expand_permission_to_who(permission: u32, who: u32) -> u32 {
    permission & who
}

fn copy_permission_to_who(source: char, allowed: u32, who: u32) -> u32 {
    let source_bits = match source {
        'u' => (allowed & 0o700) >> 6,
        'g' => (allowed & 0o070) >> 3,
        'o' => allowed & 0o007,
        _ => 0,
    };

    let mut bits = 0;
    if who & 0o700 != 0 {
        bits |= source_bits << 6;
    }
    if who & 0o070 != 0 {
        bits |= source_bits << 3;
    }
    if who & 0o007 != 0 {
        bits |= source_bits;
    }
    bits
}

fn symbolic_mask(mask: u32) -> String {
    let allowed = (!mask) & 0o777;
    format!(
        "u={},g={},o={}",
        class_permissions((allowed & 0o700) >> 6),
        class_permissions((allowed & 0o070) >> 3),
        class_permissions(allowed & 0o007)
    )
}

fn class_permissions(bits: u32) -> String {
    let mut permissions = String::new();
    if bits & 0o4 != 0 {
        permissions.push('r');
    }
    if bits & 0o2 != 0 {
        permissions.push('w');
    }
    if bits & 0o1 != 0 {
        permissions.push('x');
    }
    permissions
}
