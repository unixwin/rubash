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
                            return Ok(2);
                        }
                    }
                }
            }
            value if value.starts_with('-') => {
                writeln!(stderr, "rubash: umask: {value}: invalid option")?;
                return Ok(2);
            }
            value => mode = Some(value),
        }
    }

    if let Some(mode) = mode {
        let mask = if mode.starts_with(|ch: char| ch.is_ascii_digit()) {
            match parse_mask(mode) {
                Some(mask) => mask,
                None => {
                    writeln!(stderr, "rubash: umask: {mode}: octal number out of range")?;
                    return Ok(EXECUTION_FAILURE);
                }
            }
        } else {
            match parse_symbolic_mask(mode, current_mask(env_vars)) {
                Ok(mask) => mask,
                Err(SymbolicModeError::Operator(ch)) => {
                    writeln!(
                        stderr,
                        "rubash: umask: `{ch}': invalid symbolic mode operator"
                    )?;
                    return Ok(EXECUTION_FAILURE);
                }
                Err(SymbolicModeError::Character(ch)) => {
                    writeln!(
                        stderr,
                        "rubash: umask: `{ch}': invalid symbolic mode character"
                    )?;
                    return Ok(EXECUTION_FAILURE);
                }
            }
        };
        env_vars.insert("__RUBASH_UMASK".to_string(), format!("{mask:04o}"));
        if symbolic {
            // Bash's symbolic form is already human-readable; `-p` only
            // changes the octal form into a reusable command and is ignored
            // when `-S` is also present.
            writeln!(stdout, "{}", symbolic_mask(mask))?;
        }
        return Ok(EXECUTION_SUCCESS);
    }

    let mask = current_mask(env_vars);
    if symbolic {
        // GNU umask -p -S prints a reusable command including the -S flag
        // (umask -p -S -> "umask -S u=...,g=...,o=...").
        if reusable {
            writeln!(stdout, "umask -S {}", symbolic_mask(mask))?;
        } else {
            writeln!(stdout, "{}", symbolic_mask(mask))?;
        }
    } else if reusable {
        writeln!(stdout, "umask {mask:04o}")?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SymbolicModeError {
    Operator(char),
    Character(char),
}

// Port of GNU bash 5.3 builtins/umask.def parse_symbolic_mode: `mode` is a
// chmod-style symbolic spec applied to the allowed-permission bits (the
// complement of the mask).  A clause is `who op action`; after an op is
// applied, a following + - or = continues the SAME who with a fresh perm
// set (the goto start_op continuation), and a comma restarts who parsing.
// Actions accept rwxXst (s/t bits are masked away by who for umask) and
// the copy specifications u/g/o, which assign that class's permissions
// from the INITIAL bits expanded to all classes.
fn parse_symbolic_mask(mode: &str, current_mask: u32) -> Result<u32, SymbolicModeError> {
    let initial_bits = (!current_mask) & 0o777;
    let mut bits = initial_bits;
    let chars: Vec<char> = mode.chars().collect();
    let mut s = 0usize;

    loop {
        // Parse the `who` portion of the symbolic mode clause.
        let mut who = 0u32;
        while let Some(&c) = chars.get(s) {
            match c {
                'u' => who |= 0o700,
                'g' => who |= 0o070,
                'o' => who |= 0o007,
                'a' => who |= 0o777,
                _ => break,
            }
            s += 1;
        }
        // The default `who` is `a`.
        if who == 0 {
            who = 0o777;
        }

        // start_op: parse one operator and its action list, apply it, then
        // continue with the same `who` while the next character is another
        // operator.
        loop {
            let mut perm = 0u32;
            let op = chars.get(s).copied().unwrap_or('\0');
            s += 1;
            if !matches!(op, '+' | '-' | '=') {
                return Err(SymbolicModeError::Operator(op));
            }

            while let Some(&c) = chars.get(s) {
                match c {
                    // Copy specifications assign (not OR) the referenced
                    // class's initial permissions, expanded to all classes.
                    'u' => perm = copy_class(initial_bits, 0o400, 0o200, 0o100),
                    'g' => perm = copy_class(initial_bits, 0o040, 0o020, 0o010),
                    'o' => perm = copy_class(initial_bits, 0o004, 0o002, 0o001),
                    'r' => perm |= 0o444,
                    'w' => perm |= 0o222,
                    // X acts as x only when the initial bits carry execute
                    // permission; otherwise it is consumed as a no-op.
                    'X' => {
                        if initial_bits & 0o111 != 0 {
                            perm |= 0o111;
                        }
                    }
                    'x' => perm |= 0o111,
                    // setuid/setgid/sticky are accepted but cannot survive
                    // the `perm &= who` mask below (who is IRWX only).
                    's' => perm |= 0o6000,
                    't' => perm |= 0o1000,
                    _ => break,
                }
                s += 1;
            }

            perm &= who;
            match op {
                '+' => bits |= perm,
                '-' => bits &= !perm,
                '=' => {
                    bits &= !who;
                    bits |= perm;
                }
                _ => unreachable!("validated symbolic umask operator"),
            }

            match chars.get(s) {
                None => return Ok((!bits) & 0o777),
                Some(',') => {
                    s += 1;
                    break;
                }
                Some('+' | '-' | '=') => continue,
                Some(&c) => return Err(SymbolicModeError::Character(c)),
            }
        }
    }
}

fn copy_class(initial_bits: u32, read: u32, write: u32, exec: u32) -> u32 {
    let mut bits = 0;
    if initial_bits & read != 0 {
        bits |= 0o444;
    }
    if initial_bits & write != 0 {
        bits |= 0o222;
    }
    if initial_bits & exec != 0 {
        bits |= 0o111;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mode: &str, initial: &str) -> (i32, String, String) {
        let mut env = HashMap::from([(String::from("__RUBASH_UMASK"), initial.to_string())]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = execute_with_io(
            &vec![mode.to_string(), "-S".to_string()],
            &mut env,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn symbolic_mode_accepts_who_copy_specifications() {
        // GNU umask.def parses u/g/o as copy specifications: the referenced
        // class's INITIAL permissions are assigned (expanded to all classes,
        // then masked by who).  Initial mask 022 -> allowed u=rwx,g=rx,o=rx.
        for (mode, expected) in [
            ("o=u", "u=rwx,g=rx,o=rwx"),
            ("g=u", "u=rwx,g=rwx,o=rx"),
            ("g+u", "u=rwx,g=rwx,o=rx"),
            ("o+g", "u=rwx,g=rx,o=rx"),
            ("u+g,g+o,o-rw", "u=rwx,g=rx,o=x"),
            ("g+u,o+rwx-u", "u=rwx,g=rwx,o="),
        ] {
            let (status, stdout, stderr) = run(mode, "022");
            assert_eq!(status, EXECUTION_SUCCESS, "mode {mode}");
            assert!(stderr.is_empty(), "mode {mode}: {stderr:?}");
            assert_eq!(stdout, format!("{expected}\n"), "mode {mode}");
        }
    }

    #[test]
    fn symbolic_mode_accepts_multiple_operators_per_clause() {
        // A following + - or = continues the same who with a fresh perm set
        // (umask.def "goto start_op" continuation).
        for (mode, expected) in [
            ("u=r+w", "u=rw,g=rx,o=rx"),
            ("u=r-w", "u=r,g=rx,o=rx"),
            ("u+w=r+x", "u=rx,g=rx,o=rx"),
            ("u=r+w=x", "u=x,g=rx,o=rx"),
            ("u=rwx,u-w", "u=rx,g=rx,o=rx"),
            ("u=xwr", "u=rwx,g=rx,o=rx"),
            ("+xwr", "u=rwx,g=rwx,o=rwx"),
            ("+xr", "u=rwx,g=rx,o=rx"),
        ] {
            let (status, stdout, stderr) = run(mode, "022");
            assert_eq!(status, EXECUTION_SUCCESS, "mode {mode}");
            assert!(stderr.is_empty(), "mode {mode}: {stderr:?}");
            assert_eq!(stdout, format!("{expected}\n"), "mode {mode}");
        }
    }

    #[test]
    fn symbolic_mode_accepts_chmod_style_characters_as_noops() {
        // X acts as x only when the initial bits include execute; s/t bits
        // are accepted but masked away by who for umask.  Initial mask 022
        // already allows execute, so these all leave the mask unchanged.
        for mode in ["a+X", "g+X", "o+X", "+X", "u+s", "u+t"] {
            let (status, stdout, stderr) = run(mode, "022");
            assert_eq!(status, EXECUTION_SUCCESS, "mode {mode}");
            assert!(stderr.is_empty(), "mode {mode}: {stderr:?}");
            assert_eq!(stdout, "u=rwx,g=rx,o=rx\n", "mode {mode}");
        }
    }

    #[test]
    fn symbolic_mode_empty_action_clears_who_for_equals() {
        for (mode, expected) in [("u=", "u=,g=rx,o=rx"), ("u==r", "u=r,g=rx,o=rx")] {
            let (status, stdout, _stderr) = run(mode, "022");
            assert_eq!(status, EXECUTION_SUCCESS, "mode {mode}");
            assert_eq!(stdout, format!("{expected}\n"), "mode {mode}");
        }
    }

    #[test]
    fn symbolic_mode_requires_an_operator() {
        for mode in ["u", "g", "a", "u,g=o"] {
            let (status, stdout, stderr) = run(mode, "022");
            assert_eq!(status, EXECUTION_FAILURE);
            assert!(stdout.is_empty());
            assert!(stderr.contains("invalid symbolic mode operator"));
        }
    }

    #[test]
    fn invalid_option_is_usage_error() {
        let mut env = HashMap::new();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status =
            execute_with_io(&["-Z".to_string()], &mut env, &mut stdout, &mut stderr).unwrap();
        assert_eq!(status, 2);
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "rubash: umask: -Z: invalid option\n"
        );
    }

    #[test]
    fn invalid_modes_keep_bash_error_categories() {
        let mut env = HashMap::new();
        for (mode, expected) in [
            ("09", "rubash: umask: 09: octal number out of range\n"),
            (
                "g=p",
                "rubash: umask: `p': invalid symbolic mode character\n",
            ),
            (
                "u:rwx",
                "rubash: umask: `:': invalid symbolic mode operator\n",
            ),
        ] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status =
                execute_with_io(&[mode.to_string()], &mut env, &mut stdout, &mut stderr).unwrap();
            assert_eq!(status, EXECUTION_FAILURE);
            assert_eq!(String::from_utf8(stderr).unwrap(), expected);
        }
    }
}
